use std::fs;
use std::path::{Path, PathBuf};

use koldera_cli::api::FieldError;
use koldera_cli::document::{self, DocumentError, ImageRef};
use serde_json::{Value, json};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
const JPEG: &[u8] = b"\xff\xd8\xff\xe0\0\x10JFIF";
const SVG: &str = "\u{feff}<?xml version=\"1.0\"?>\n<!-- logo -->\n<!DOCTYPE svg [ <!ENTITY x \"y\"> ]>\n<svg:svg xmlns:svg=\"http://www.w3.org/2000/svg\"/>";

const RELATIVE: &str = "an image file path must be relative to the JSON file.";
const OUTSIDE: &str = "the image file is outside the directory of the JSON file.";
const MISSING: &str = "the image file does not exist.";
const LINK_OUT: &str =
    "the image file is a link that leads outside the directory of the JSON file.";
const EXTENSION: &str = "an image file must end in .png, .jpg, .jpeg or .svg.";

struct Fixture {
    _root: tempfile::TempDir,
    dir: PathBuf,
    outside: PathBuf,
}

/// `<tmp>/doc/` holds the JSON file and images; `<tmp>/outside.png` lies next to it.
fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("doc");
    fs::create_dir_all(dir.join("img")).unwrap();
    fs::write(dir.join("logo.png"), PNG).unwrap();
    fs::write(dir.join("img").join("photo.JPG"), JPEG).unwrap();
    fs::write(dir.join("chart.svg"), SVG).unwrap();
    let outside = root.path().join("outside.png");
    fs::write(&outside, PNG).unwrap();
    Fixture {
        dir,
        outside,
        _root: root,
    }
}

fn image(file: &str) -> Value {
    json!({"type": "image", "file": file, "alt": "An image"})
}

fn write_doc(dir: &Path, body: Vec<Value>) -> PathBuf {
    let path = dir.join("doc.json");
    let doc = json!({"title": "T", "language": "en", "body": body});
    fs::write(&path, serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
    path
}

fn invalid(result: Result<(Value, Vec<ImageRef>), DocumentError>) -> Vec<FieldError> {
    match result {
        Err(DocumentError::Invalid(errors)) => errors,
        other => panic!("expected an invalid document, got {other:?}"),
    }
}

fn field(path: &str, code: &str, message: &str) -> FieldError {
    FieldError {
        path: path.into(),
        code: code.into(),
        message: message.into(),
    }
}

#[test]
fn images_resolve_relative_to_the_json_file_and_the_same_file_is_one_path() {
    let f = fixture();
    let json = write_doc(
        &f.dir,
        vec![
            json!({"type": "heading", "level": 1, "text": "T"}),
            image("logo.png"),
            image("./img/../logo.png"),
            image("img/photo.JPG"),
            image("chart.svg"),
        ],
    );
    let (doc, images) = document::prepare(&json).unwrap();
    assert_eq!(doc["body"][1]["file"], "logo.png");
    let pointers: Vec<&str> = images.iter().map(|i| i.pointer.as_str()).collect();
    assert_eq!(pointers, ["/body/1", "/body/2", "/body/3", "/body/4"]);
    assert_eq!(images[0].path, images[1].path);
    assert!(images[0].path.is_absolute());
    let types: Vec<&str> = images.iter().map(|i| i.content_type).collect();
    assert_eq!(
        types,
        ["image/png", "image/png", "image/jpeg", "image/svg+xml"]
    );
}

#[test]
fn images_are_found_anywhere_in_the_document_in_document_order() {
    let f = fixture();
    let path = f.dir.join("doc.json");
    fs::write(
        &path,
        r#"{"z": {"type": "image", "file": "logo.png"}, "a/b~": [{"type": "image", "file": "chart.svg"}]}"#,
    )
    .unwrap();
    let (_, images) = document::prepare(&path).unwrap();
    let pointers: Vec<&str> = images.iter().map(|i| i.pointer.as_str()).collect();
    assert_eq!(pointers, ["/z", "/a~1b~0/0"]);
}

#[test]
fn refused_image_paths_are_reported_with_core_codes_and_texts() {
    let f = fixture();
    fs::write(f.dir.join("notes.gif"), b"GIF89a").unwrap();
    fs::write(f.dir.join("fake.png"), b"not a png").unwrap();
    fs::write(f.dir.join("fake.jpeg"), PNG).unwrap();
    fs::write(f.dir.join("page.svg"), "<html></html>").unwrap();
    fs::write(f.dir.join("bad.svg"), b"<svg \xff>").unwrap();
    fs::write(f.dir.join("empty.svg"), "<!-- nothing -->").unwrap();
    let absolute = f.dir.join("logo.png").to_string_lossy().into_owned();
    let json = write_doc(
        &f.dir,
        vec![
            image(&absolute),
            image("/logo.png"),
            image("\\logo.png"),
            image("../outside.png"),
            image("img/../../outside.png"),
            image("nothing.png"),
            image("notes.gif"),
            image("fake.png"),
            image("fake.jpeg"),
            image("page.svg"),
            image("bad.svg"),
            image("empty.svg"),
            json!({"type": "image", "uploadId": "33333333-3333-4333-8333-333333333333", "alt": "x"}),
            image("logo.png"),
        ],
    );
    let errors = invalid(document::prepare(&json));
    let bad = "FILE_IMAGE_INVALID";
    let mut expected = vec![
        field("/body/0", bad, RELATIVE),
        field("/body/1", bad, RELATIVE),
    ];
    // A backslash is a separator (and so a root) only on Windows; elsewhere it is part of a missing file's name.
    expected.push(field(
        "/body/2",
        bad,
        if cfg!(windows) { RELATIVE } else { MISSING },
    ));
    expected.extend([
        field("/body/3", bad, OUTSIDE),
        field("/body/4", bad, OUTSIDE),
        field("/body/5", bad, MISSING),
        field("/body/6", bad, EXTENSION),
        field(
            "/body/7",
            bad,
            "uploaded file is not a PNG image (signature mismatch)",
        ),
        field(
            "/body/8",
            bad,
            "uploaded file is not a JPEG image (signature mismatch)",
        ),
        field(
            "/body/9",
            bad,
            "uploaded file is not an SVG image (the root element is not svg)",
        ),
        field(
            "/body/10",
            bad,
            "uploaded file is not an SVG image (not well-formed UTF-8 XML)",
        ),
        field(
            "/body/11",
            bad,
            "uploaded file is not an SVG image (no root element)",
        ),
        field(
            "/body/12",
            "UPLOAD_ID_NOT_ALLOWED",
            "uploadId is only available over HTTP; name the image with a file path relative to the JSON file.",
        ),
    ]);
    assert_eq!(errors, expected);
}

#[test]
fn a_document_that_is_not_an_object_is_invalid_and_bad_json_is_reported() {
    let f = fixture();
    let path = f.dir.join("doc.json");
    fs::write(&path, "[1, 2]").unwrap();
    assert_eq!(
        invalid(document::prepare(&path)),
        [field("", "SCHEMA", "The document must be a JSON object.")]
    );
    fs::write(&path, "{\"title\": ").unwrap();
    assert!(matches!(
        document::prepare(&path),
        Err(DocumentError::NotJson(_))
    ));
    assert!(matches!(
        document::prepare(&f.dir.join("none.json")),
        Err(DocumentError::NotFound)
    ));
    assert!(matches!(
        document::prepare(&f.dir),
        Err(DocumentError::NotFound)
    ));
}

#[test]
fn use_upload_replaces_the_file_and_keeps_the_other_keys_in_order() {
    let f = fixture();
    let json = write_doc(&f.dir, vec![image("logo.png")]);
    let (mut doc, images) = document::prepare(&json).unwrap();
    document::use_upload(
        &mut doc,
        &images[0].pointer,
        "33333333-3333-4333-8333-333333333333",
    );
    assert_eq!(
        serde_json::to_string(&doc["body"][0]).unwrap(),
        r#"{"type":"image","alt":"An image","uploadId":"33333333-3333-4333-8333-333333333333"}"#
    );
    let keys: Vec<&String> = doc.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["title", "language", "body"]);
}

#[test]
fn invalid_document_lines_follow_core() {
    let text = document::invalid_text(
        "in/doc.json",
        &[
            field("", "SCHEMA", "The document must be a JSON object."),
            field("/body/0", "FILE_IMAGE_INVALID", MISSING),
        ],
    );
    assert_eq!(
        text,
        "koldera: invalid document in/doc.json\n  (document)  SCHEMA: The document must be a JSON object.\n  /body/0  FILE_IMAGE_INVALID: the image file does not exist.\n"
    );
}

/// Creates a symbolic link, or `None` where this platform or account cannot.
fn symlink(target: &Path, link: &Path) -> Option<()> {
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let made = if target.is_dir() {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    };
    match made {
        Ok(()) => Some(()),
        Err(e) => {
            eprintln!("skipped: symbolic links cannot be created here ({e})");
            None
        }
    }
}

#[test]
fn links_leading_out_are_refused_and_links_inside_are_accepted() {
    let f = fixture();
    let outside_dir = f.outside.parent().unwrap().join("elsewhere");
    fs::create_dir_all(&outside_dir).unwrap();
    fs::write(outside_dir.join("x.png"), PNG).unwrap();
    if symlink(&f.outside, &f.dir.join("out.png")).is_none() {
        return;
    }
    symlink(&outside_dir, &f.dir.join("linked")).unwrap();
    symlink(&f.dir.join("logo.png"), &f.dir.join("alias.png")).unwrap();
    let json = write_doc(
        &f.dir,
        vec![image("out.png"), image("linked/x.png"), image("alias.png")],
    );
    let errors = invalid(document::prepare(&json));
    assert_eq!(
        errors,
        [
            field("/body/0", "FILE_IMAGE_INVALID", LINK_OUT),
            field("/body/1", "FILE_IMAGE_INVALID", LINK_OUT),
        ]
    );
    let json = write_doc(&f.dir, vec![image("alias.png")]);
    let (_, images) = document::prepare(&json).unwrap();
    assert_eq!(images[0].content_type, "image/png");
}
