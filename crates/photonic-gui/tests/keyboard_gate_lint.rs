//! Lint: keyboard handling must not be gated on pointer position or on global
//! focus-emptiness.
//!
//! Two real defects motivated this, both in `panels/video/`, and both shared a
//! signature — a keyboard path that was written correctly and then disabled by
//! its own guard:
//!
//! * the node canvas gated `Tab`/arrows/`Delete` on `rect_contains_pointer`, so
//!   every shortcut required the mouse to hover the canvas;
//! * the curve editor gated arrow-nudge on `!keyboard_captured(ui)`, which means
//!   *nothing anywhere* has focus — so the nudge died the moment the plot itself
//!   was focused.
//!
//! Neither is visible to a structural spec-drift checker: the symbols exist, the
//! signatures match, nothing is stale. Only a pattern lint finds this class.
//! See `docs/specs/video-editor/41-accessibility.md` §3 R-5 and §8 item 3.

use std::fs;
use std::path::Path;

/// Substrings that must not appear inside a block that also handles key input.
const BANNED: &[(&str, &str)] = &[
    (
        "rect_contains_pointer",
        "gates keyboard handling on pointer position — use `Response::has_focus()`",
    ),
    (
        "focused().is_none()",
        "gates keyboard handling on global focus-emptiness — use `Response::has_focus()`",
    ),
];

/// Key-input calls that mark a region as keyboard handling.
const KEY_MARKERS: &[&str] = &["key_pressed(", "key_down(", "consume_key(", "key_released("];

fn is_comment(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("//") || t.starts_with("/*") || t.starts_with('*')
}

fn rs_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn keyboard_handling_is_not_gated_on_pointer_or_global_focus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/panels/video");
    let mut files = Vec::new();
    rs_files(&root, &mut files);
    assert!(!files.is_empty(), "no sources found under {}", root.display());

    // A violation is a banned call within `WINDOW` lines *above* a key-input
    // call — i.e. plausibly guarding it. Distant, unrelated uses are ignored.
    const WINDOW: usize = 12;
    let mut violations = Vec::new();

    for path in &files {
        let src = fs::read_to_string(path).expect("read source");
        let lines: Vec<&str> = src.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            // Scan code, not prose. A comment *describing* a banned pattern —
            // including the ones documenting why these gates were replaced — is
            // not a violation, and a lint that fires on prose gets disabled.
            if is_comment(line) {
                continue;
            }
            let Some((pat, why)) = BANNED.iter().find(|(p, _)| line.contains(*p)) else {
                continue;
            };
            let end = (i + WINDOW).min(lines.len());
            let guards_keys = lines[i..end]
                .iter()
                .filter(|l| !is_comment(l))
                .any(|l| KEY_MARKERS.iter().any(|m| l.contains(m)));
            if guards_keys {
                violations.push(format!(
                    "{}:{}: `{}` {}",
                    path.file_name().unwrap().to_string_lossy(),
                    i + 1,
                    pat,
                    why
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "keyboard handling gated on pointer position or global focus \
         (41 §3 R-5):\n  {}",
        violations.join("\n  ")
    );
}
