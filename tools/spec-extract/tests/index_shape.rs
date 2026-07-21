//! Proves ACC-40-06-01: the extractor emits a deterministic structural index
//! whose records match the real workspace source. Runs the built binary over
//! the actual repo root and asserts on ground-truth items.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// Repo root = two levels above this crate (tools/spec-extract/../..).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("resolve repo root")
}

/// Run `spec-extract --root <repo>` and return its stdout.
fn run_extract() -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_spec-extract"))
        .arg("--root")
        .arg(repo_root())
        .output()
        .expect("run spec-extract");
    assert!(
        out.status.success(),
        "spec-extract failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf8 stdout")
}

fn items(json: &Value) -> &Vec<Value> {
    assert_eq!(json["schema"], 1, "schema tag");
    json["items"].as_array().expect("items array")
}

fn find<'a>(items: &'a [Value], file: &str, name: &str) -> &'a Value {
    items
        .iter()
        .find(|it| it["file"] == file && it["name"] == name)
        .unwrap_or_else(|| panic!("no item {name} in {file}"))
}

#[test]
fn indexes_const_enum_and_fields_in_order() {
    let raw = run_extract();
    let json: Value = serde_json::from_str(&raw).expect("parse index json");
    let items = items(&json);

    // (a) CURRENT_FORMAT_VERSION const in document.rs.
    let ver = find(
        items,
        "crates/photonic-core/src/document.rs",
        "CURRENT_FORMAT_VERSION",
    );
    assert_eq!(ver["kind"], "Const", "kind of CURRENT_FORMAT_VERSION");
    assert_eq!(ver["value"], "4", "value of CURRENT_FORMAT_VERSION");
    assert_eq!(ver["line"], 110, "line of CURRENT_FORMAT_VERSION");

    // (b) EngineCmd enum in session.rs, variants in exact declaration order.
    let cmd = find(
        items,
        "crates/photonic-video/src/session.rs",
        "EngineCmd",
    );
    assert_eq!(cmd["kind"], "Enum", "kind of EngineCmd");
    let variant_names: Vec<String> = cmd["variants"]
        .as_array()
        .expect("variants array")
        .iter()
        .map(|v| v["name"].as_str().expect("variant name").to_string())
        .collect();
    assert_eq!(
        variant_names,
        vec![
            "Play",
            "Pause",
            "Seek",
            "ScrubSeek",
            "Step",
            "SetLoop",
            "SetActiveSequence",
            "SetProxyMode",
            "SetPreviewTarget",
            "SetPreviewQuality",
            "SeekSource",
            "InvalidateRange",
            "Export",
            "Probe",
            "Shutdown",
        ],
        "EngineCmd variants must match source declaration order"
    );

    // (c) SeekSource struct-variant field names in order.
    let seek_source = cmd["variants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "SeekSource")
        .expect("SeekSource variant");
    let fields: Vec<String> = seek_source["fields"]
        .as_array()
        .expect("fields array")
        .iter()
        .map(|f| f.as_str().unwrap().to_string())
        .collect();
    assert_eq!(fields, vec!["asset", "time"], "SeekSource fields");
}

/// Run `spec-extract --root <root>` and return its stdout.
fn run_extract_root(root: &Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_spec-extract"))
        .arg("--root")
        .arg(root)
        .output()
        .expect("run spec-extract");
    assert!(
        out.status.success(),
        "spec-extract failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf8 stdout")
}

#[test]
fn output_is_byte_identical_across_runs() {
    // (d) determinism: two independent runs over the SAME tree must produce
    // identical bytes. We run over a private fixture workspace rather than the
    // real repo root — sibling agents mutate the live tree concurrently, which
    // would make a repo-root comparison flaky for reasons unrelated to the tool.
    let dir = std::env::temp_dir().join(format!(
        "spec-extract-determinism-{}-{}",
        std::process::id(),
        env!("CARGO_PKG_NAME"),
    ));
    let src = dir.join("crates").join("mini").join("src");
    std::fs::create_dir_all(&src).expect("create fixture dirs");
    std::fs::write(
        src.join("lib.rs"),
        "pub const N: u32 = 4;\n\
         pub enum E { A, B(u8), C { x: i32, y: i32 } }\n\
         pub struct S { pub a: u8, b: u16 }\n\
         mod inner { pub fn f(a: u8) -> u8 { a } }\n",
    )
    .expect("write fixture source");

    let a = run_extract_root(&dir);
    let b = run_extract_root(&dir);
    std::fs::remove_dir_all(&dir).ok();

    assert_eq!(a, b, "spec-extract output must be deterministic");
}
