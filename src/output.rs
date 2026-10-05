//! Writing artifacts next to the user's files: `<stem>.<kind><ext>`, never overwriting, all or nothing.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::model::Artifact;

/// How many numbered alternatives (`-1` ... `-1000`) are tried when a name is taken.
pub const MAX_COLLISIONS: u32 = 1000;

/// An artifact written to disk, at its absolute path.
#[derive(Debug, Clone, PartialEq)]
pub struct Written {
    pub artifact: Artifact,
    pub path: PathBuf,
}

/// A file-name component: characters invalid in Windows file names (on every platform, so names stay portable)
/// become `_`; surrounding whitespace is trimmed; an empty result is `output`.
pub fn safe_name(name: &str) -> String {
    const INVALID: &str = "<>:\"/\\|?*";
    let replaced: String = name
        .chars()
        .map(|c| {
            if c < ' ' || INVALID.contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = replaced.trim();
    if trimmed.is_empty() {
        "output".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// The file extension core gives an artifact of this kind (empty for a kind this client does not know).
pub fn extension_for(kind: &str) -> &'static str {
    match kind {
        "original" | "remediated" | "generated_pdf" | "reordered" => ".pdf",
        "source_html" => ".html",
        "baseline_report" | "final_report" | "web_report" => ".json",
        _ => "",
    }
}

/// Verifies every artifact's bytes against its declared size and SHA-256, then writes each to `out_dir`
/// (created when missing). On any error the files of this run are removed again.
pub fn write_artifacts(
    out_dir: &Path,
    stem: &str,
    items: Vec<(Artifact, Vec<u8>)>,
) -> io::Result<Vec<Written>> {
    if items.is_empty() {
        return Ok(Vec::new());
    }
    for (artifact, bytes) in &items {
        verify(artifact, bytes)?;
    }
    fs::create_dir_all(out_dir)?;
    let stem = safe_name(stem);
    let mut written: Vec<Written> = Vec::with_capacity(items.len());
    for (artifact, bytes) in items {
        let base = format!("{stem}.{}", safe_name(&artifact.kind));
        match write_one(out_dir, &base, extension_for(&artifact.kind), &bytes) {
            Ok(path) => written.push(Written { artifact, path }),
            Err(e) => {
                for w in &written {
                    let _ = fs::remove_file(&w.path);
                }
                return Err(e);
            }
        }
    }
    Ok(written)
}

fn verify(artifact: &Artifact, bytes: &[u8]) -> io::Result<()> {
    let actual = sha256_hex(bytes);
    if bytes.len() as u64 != artifact.size_bytes || !actual.eq_ignore_ascii_case(&artifact.sha256) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "the {} artifact does not match its declared size and SHA-256",
                artifact.kind
            ),
        ));
    }
    Ok(())
}

/// Lower-case hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn write_one(out_dir: &Path, base: &str, extension: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    let (path, mut file) = create_new(out_dir, base, extension)?;
    if let Err(e) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&path); // never leave a partial file behind
        return Err(e);
    }
    Ok(path)
}

/// Creates `base.ext`, else `base-1.ext`, ... Only an existing target counts as a collision; any other error is raised.
fn create_new(out_dir: &Path, base: &str, extension: &str) -> io::Result<(PathBuf, File)> {
    for n in 0..=MAX_COLLISIONS {
        let name = if n == 0 {
            format!("{base}{extension}")
        } else {
            format!("{base}-{n}{extension}")
        };
        let path = std::path::absolute(out_dir.join(name))?;
        // create_new refuses an existing file, also one another process created after a check.
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(_) if path.exists() => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!(
            "too many existing files named {base}*{extension} in {}",
            out_dir.display()
        ),
    ))
}
