//! Structured documents: finds every image block and resolves its `file` the way core's command line does.

use std::fmt::Write as _;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::api::FieldError;

const FILE_IMAGE_INVALID: &str = "FILE_IMAGE_INVALID";
const SNIFF_BYTES: u64 = 8192;
const SVG_PROLOG_BYTES: u64 = 1 << 20;

/// An image block (`pointer`, a JSON Pointer) and the file it names, resolved to an absolute path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef {
    pub pointer: String,
    pub path: PathBuf,
    pub content_type: &'static str,
}

#[derive(Debug)]
pub enum DocumentError {
    NotFound,
    Unreadable(String),
    NotJson(String),
    Invalid(Vec<FieldError>),
}

/// Reads the document and resolves its images; schema validation is left to the server.
pub fn prepare(json_path: &Path) -> Result<(Value, Vec<ImageRef>), DocumentError> {
    if !json_path.is_file() {
        return Err(DocumentError::NotFound);
    }
    let text = fs::read(json_path).map_err(|e| DocumentError::Unreadable(e.to_string()))?;
    let document: Value =
        serde_json::from_slice(&text).map_err(|e| DocumentError::NotJson(e.to_string()))?;
    if !document.is_object() {
        return Err(DocumentError::Invalid(vec![FieldError {
            path: String::new(),
            code: "SCHEMA".into(),
            message: "The document must be a JSON object.".into(),
        }]));
    }
    let full =
        std::path::absolute(json_path).map_err(|e| DocumentError::Unreadable(e.to_string()))?;
    let directory = full.parent().unwrap_or(Path::new("")).to_path_buf();

    let mut found = Vec::new();
    find_images(&document, String::new(), &mut found);
    let mut problems = Vec::new();
    let mut images = Vec::new();
    for (pointer, block) in found {
        let refuse = |message: &str| FieldError {
            path: pointer.clone(),
            code: FILE_IMAGE_INVALID.into(),
            message: message.into(),
        };
        if block.get("uploadId").is_some_and(Value::is_string) {
            problems.push(FieldError {
                path: pointer.clone(),
                code: "UPLOAD_ID_NOT_ALLOWED".into(),
                message: "uploadId is only available over HTTP; name the image with a file path relative to the JSON file.".into(),
            });
            continue;
        }
        // Without a file name the block is left for the server's schema check.
        let Some(file) = block.get("file").and_then(Value::as_str) else {
            continue;
        };
        let path = match resolve_image(&directory, file) {
            Ok(path) => path,
            Err(reason) => {
                problems.push(refuse(reason));
                continue;
            }
        };
        let Some(content_type) = content_type_of(&path) else {
            problems.push(refuse(
                "an image file must end in .png, .jpg, .jpeg or .svg.",
            ));
            continue;
        };
        if let Some(mismatch) = sniff(&path, content_type) {
            problems.push(refuse(&mismatch));
            continue;
        }
        images.push(ImageRef {
            pointer,
            path,
            content_type,
        });
    }
    if problems.is_empty() {
        Ok((document, images))
    } else {
        Err(DocumentError::Invalid(problems))
    }
}

/// Replaces the `file` of the image block at `pointer` by `uploadId` (appended last, like core).
pub fn use_upload(document: &mut Value, pointer: &str, upload_id: &str) {
    if let Some(block) = document.pointer_mut(pointer).and_then(Value::as_object_mut) {
        block.shift_remove("file");
        block.insert("uploadId".into(), Value::String(upload_id.into()));
    }
}

/// Core's invalid-document report: a heading line, then one line per problem.
pub fn invalid_text(display_path: &str, errors: &[FieldError]) -> String {
    let mut s = format!("koldera: invalid document {display_path}\n");
    for e in errors {
        let path = if e.path.is_empty() {
            "(document)"
        } else {
            &e.path
        };
        let _ = writeln!(s, "  {path}  {}: {}", e.code, e.message);
    }
    s
}

fn find_images<'a>(node: &'a Value, pointer: String, found: &mut Vec<(String, &'a Value)>) {
    match node {
        Value::Object(map) => {
            if map.get("type").and_then(Value::as_str) == Some("image") {
                found.push((pointer.clone(), node));
            }
            for (key, value) in map {
                let key = key.replace('~', "~0").replace('/', "~1");
                find_images(value, format!("{pointer}/{key}"), found);
            }
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                find_images(item, format!("{pointer}/{i}"), found);
            }
        }
        _ => {}
    }
}

/// Core's `ResolveImage`: absolute paths, `..` escapes and links (file or directory) leading out are refused.
fn resolve_image(directory: &Path, file: &str) -> Result<PathBuf, &'static str> {
    let rooted = Path::new(file)
        .components()
        .any(|c| matches!(c, Component::Prefix(_) | Component::RootDir));
    if rooted || file.starts_with('/') || file.starts_with('\\') {
        return Err("an image file path must be relative to the JSON file.");
    }
    let root = normalize(directory);
    let full = normalize(&root.join(file));
    let Some(relative) = strip_root(&full, &root) else {
        return Err("the image file is outside the directory of the JSON file.");
    };
    if !full.is_file() {
        return Err("the image file does not exist.");
    }
    const LINK_OUT: &str =
        "the image file is a link that leads outside the directory of the JSON file.";
    let real_root = fs::canonicalize(&root).map_err(|_| LINK_OUT)?;
    let mut current = root.clone();
    for segment in relative {
        current.push(segment);
        let is_link = fs::symlink_metadata(&current).is_ok_and(|m| m.file_type().is_symlink());
        if !is_link {
            continue;
        }
        match fs::canonicalize(&current) {
            Ok(target) if target.starts_with(&real_root) => {}
            _ => return Err(LINK_OUT),
        }
    }
    Ok(full)
}

/// Like `Path.GetFullPath` on an absolute path: `.` and `..` are resolved without touching the file system.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    out.components().next_back(),
                    None | Some(Component::RootDir | Component::Prefix(_))
                ) {
                    out.pop();
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// The components of `full` below `root`, compared case-insensitively on Windows; `None` when outside.
fn strip_root(full: &Path, root: &Path) -> Option<Vec<std::ffi::OsString>> {
    let same = |a: &Component<'_>, b: &Component<'_>| {
        if cfg!(windows) {
            a.as_os_str().to_string_lossy().to_lowercase()
                == b.as_os_str().to_string_lossy().to_lowercase()
        } else {
            a == b
        }
    };
    let mut rest = full.components();
    for r in root.components() {
        match rest.next() {
            Some(c) if same(&c, &r) => {}
            _ => return None,
        }
    }
    let relative: Vec<_> = rest.map(|c| c.as_os_str().to_owned()).collect();
    (!relative.is_empty()).then_some(relative)
}

fn content_type_of(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_string_lossy().to_lowercase();
    match extension.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "svg" => Some("image/svg+xml"),
        _ => None,
    }
}

/// Core's `ImageSniffer`: `None` when the content matches the type, otherwise the reason.
fn sniff(path: &Path, content_type: &str) -> Option<String> {
    let limit = if content_type == "image/svg+xml" {
        SVG_PROLOG_BYTES
    } else {
        SNIFF_BYTES
    };
    let mut head = Vec::new();
    if let Err(e) = fs::File::open(path).and_then(|f| f.take(limit).read_to_end(&mut head)) {
        return Some(format!("the image file cannot be read: {e}"));
    }
    match content_type {
        "image/png" => (!head.starts_with(b"\x89PNG\r\n\x1a\n"))
            .then(|| "uploaded file is not a PNG image (signature mismatch)".into()),
        "image/jpeg" => (!head.starts_with(b"\xff\xd8\xff"))
            .then(|| "uploaded file is not a JPEG image (signature mismatch)".into()),
        _ => svg_root(&head, head.len() as u64 == limit).map(Into::into),
    }
}

/// Reads the XML prolog (declaration, comments, processing instructions, a DOCTYPE) up to the root element.
fn svg_root(bytes: &[u8], truncated: bool) -> Option<&'static str> {
    const NOT_XML: &str = "uploaded file is not an SVG image (not well-formed UTF-8 XML)";
    let text = match std::str::from_utf8(bytes) {
        Ok(t) => t,
        Err(e) if truncated && e.error_len().is_none() => {
            std::str::from_utf8(&bytes[..e.valid_up_to()]).ok()?
        }
        Err(_) => return Some(NOT_XML),
    };
    let mut rest = text.strip_prefix('\u{feff}').unwrap_or(text);
    loop {
        rest = rest.trim_start_matches([' ', '\t', '\r', '\n']);
        if rest.is_empty() {
            return Some(if truncated {
                NOT_XML
            } else {
                "uploaded file is not an SVG image (no root element)"
            });
        }
        let skipped = if rest.starts_with("<?") {
            rest.find("?>").map(|i| i + 2)
        } else if rest.starts_with("<!--") {
            rest.find("-->").map(|i| i + 3)
        } else if rest.starts_with("<!DOCTYPE") {
            doctype_end(rest)
        } else if let Some(tag) = rest.strip_prefix('<') {
            let name: String = tag
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '/' && *c != '>')
                .collect();
            let local = name.rsplit(':').next().unwrap_or("");
            if name.is_empty() {
                return Some(NOT_XML);
            }
            return (local != "svg")
                .then_some("uploaded file is not an SVG image (the root element is not svg)");
        } else {
            return Some(NOT_XML);
        };
        match skipped {
            Some(n) => rest = &rest[n..],
            None => return Some(NOT_XML),
        }
    }
}

/// The length of a DOCTYPE declaration, including an internal subset in brackets.
fn doctype_end(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    for (i, c) in text.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '[') => depth += 1,
            (None, ']') => depth = depth.saturating_sub(1),
            (None, '>') if depth == 0 => return Some(i + 1),
            _ => {}
        }
    }
    None
}
