mod common;

use std::fs;
use std::path::Path;

use koldera_cli::model::Artifact;
use koldera_cli::output::{self, MAX_COLLISIONS, safe_name, write_artifacts};

fn golden_items() -> Vec<(Artifact, Vec<u8>)> {
    let a = common::artifacts();
    vec![
        (a[0].clone(), common::raw_body("artifact-download-original")),
        (
            a[1].clone(),
            common::raw_body("artifact-download-remediated"),
        ),
    ]
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[test]
fn safe_name_replaces_the_windows_invalid_characters_everywhere() {
    assert_eq!(safe_name("report 2026"), "report 2026");
    assert_eq!(safe_name(r#"a<b>c:d"e/f\g|h?i*j"#), "a_b_c_d_e_f_g_h_i_j");
    assert_eq!(safe_name("tab\there\u{1}\u{1f}"), "tab_here__");
    assert_eq!(safe_name("  padded  "), "padded");
    assert_eq!(safe_name(""), "output");
    assert_eq!(safe_name("   "), "output");
    assert_eq!(safe_name(".."), "..");
    assert_eq!(safe_name("résumé"), "résumé");
}

#[test]
fn extensions_by_kind() {
    for kind in ["original", "remediated", "generated_pdf", "reordered"] {
        assert_eq!(output::extension_for(kind), ".pdf", "{kind}");
    }
    assert_eq!(output::extension_for("source_html"), ".html");
    for kind in ["baseline_report", "final_report", "web_report"] {
        assert_eq!(output::extension_for(kind), ".json", "{kind}");
    }
    assert_eq!(output::extension_for("something_new"), "");
}

#[test]
fn writes_stem_kind_ext_and_creates_the_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("nested").join("out");
    let written = write_artifacts(&out, "sample", golden_items()).unwrap();
    assert_eq!(
        names(&out),
        ["sample.original.pdf", "sample.remediated.pdf"]
    );
    assert_eq!(written.len(), 2);
    assert_eq!(written[0].artifact.kind, "original");
    assert!(written[0].path.is_absolute());
    assert_eq!(written[0].path.file_name().unwrap(), "sample.original.pdf");
    assert_eq!(
        fs::read(&written[1].path).unwrap(),
        common::raw_body("artifact-download-remediated")
    );
}

#[test]
fn nothing_to_write_creates_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("never");
    assert!(write_artifacts(&out, "x", vec![]).unwrap().is_empty());
    assert!(!out.exists());
}

#[test]
fn the_stem_and_kind_are_sanitised() {
    let tmp = tempfile::tempdir().unwrap();
    let mut items = golden_items();
    items.truncate(1);
    items[0].0.kind = "orig/../inal".into();
    let written = write_artifacts(tmp.path(), "../a:b", items).unwrap();
    assert_eq!(names(tmp.path()), [".._a_b.orig_.._inal"]);
    assert_eq!(
        written[0].path.parent().unwrap(),
        std::path::absolute(tmp.path()).unwrap()
    );
}

#[test]
fn existing_files_are_never_overwritten() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("sample.original.pdf"), b"keep").unwrap();
    fs::write(tmp.path().join("sample.original-1.pdf"), b"keep").unwrap();
    let written = write_artifacts(tmp.path(), "sample", golden_items()).unwrap();
    assert_eq!(
        written[0].path.file_name().unwrap(),
        "sample.original-2.pdf"
    );
    assert_eq!(
        written[1].path.file_name().unwrap(),
        "sample.remediated.pdf"
    );
    assert_eq!(
        fs::read(tmp.path().join("sample.original.pdf")).unwrap(),
        b"keep"
    );
    assert_eq!(
        fs::read(tmp.path().join("sample.original-1.pdf")).unwrap(),
        b"keep"
    );
}

#[test]
fn a_second_run_gets_suffixes() {
    let tmp = tempfile::tempdir().unwrap();
    write_artifacts(tmp.path(), "sample", golden_items()).unwrap();
    let second = write_artifacts(tmp.path(), "sample", golden_items()).unwrap();
    assert_eq!(second[0].path.file_name().unwrap(), "sample.original-1.pdf");
    assert_eq!(
        second[1].path.file_name().unwrap(),
        "sample.remediated-1.pdf"
    );
}

#[test]
fn too_many_collisions_fail_and_remove_this_runs_files() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("sample.remediated.pdf"), b"x").unwrap();
    for n in 1..=MAX_COLLISIONS {
        fs::write(tmp.path().join(format!("sample.remediated-{n}.pdf")), b"x").unwrap();
    }
    let before = names(tmp.path());
    let err = write_artifacts(tmp.path(), "sample", golden_items()).unwrap_err();
    assert!(
        err.to_string()
            .starts_with("too many existing files named sample.remediated*.pdf in "),
        "{err}"
    );
    // The original was written first and must be gone again: all or nothing.
    assert_eq!(names(tmp.path()), before);
}

#[test]
fn bytes_that_do_not_match_the_record_are_refused_before_anything_is_written() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");

    let mut wrong_hash = golden_items();
    wrong_hash[1].1[0] ^= 1;
    let err = write_artifacts(&out, "sample", wrong_hash).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(
        err.to_string(),
        "the remediated artifact does not match its declared size and SHA-256"
    );
    assert!(!out.exists());

    let mut wrong_size = golden_items();
    wrong_size[0].1.push(b'\n');
    assert!(write_artifacts(&out, "sample", wrong_size).is_err());
    assert!(!out.exists());

    // An upper-case hash in the record still matches.
    let mut upper = golden_items();
    upper[0].0.sha256 = upper[0].0.sha256.to_uppercase();
    assert_eq!(write_artifacts(&out, "sample", upper).unwrap().len(), 2);
}
