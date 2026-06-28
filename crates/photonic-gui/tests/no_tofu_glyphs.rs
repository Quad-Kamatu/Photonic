//! Regression guard for missing-glyph ("tofu", □) icons.
//!
//! The app loads only the default egui font plus egui-phosphor. A handful of
//! Unicode symbols that were used as button labels (✕, ✓, ✗, fullwidth ＋ / －)
//! are in *neither* font, so they rendered as placeholder boxes. They were
//! replaced with phosphor icons (`egui_phosphor::regular::{X, CHECK, PLUS,
//! MINUS}`). This test scans the crate's source and fails if any of those
//! code points reappear as a literal, so the fix can't silently regress.
//!
//! Forbidden code points are listed by escape (not literal) so this test file
//! never contains the offending characters itself.

use std::fs;
use std::path::{Path, PathBuf};

/// (code point, human-readable guidance) for glyphs that render as tofu.
const FORBIDDEN: &[(char, &str)] = &[
    ('\u{2715}', "U+2715 MULTIPLICATION X — use egui_phosphor::regular::X"),
    ('\u{2713}', "U+2713 CHECK MARK — use egui_phosphor::regular::CHECK"),
    ('\u{2717}', "U+2717 BALLOT X — use egui_phosphor::regular::X"),
    ('\u{FF0B}', "U+FF0B FULLWIDTH PLUS — use egui_phosphor::regular::PLUS"),
    ('\u{FF0D}', "U+FF0D FULLWIDTH HYPHEN-MINUS — use egui_phosphor::regular::MINUS"),
    ('\u{2139}', "U+2139 INFORMATION SOURCE — use egui_phosphor::regular::INFO"),
    ('\u{26A0}', "U+26A0 WARNING SIGN — use egui_phosphor::regular::WARNING"),
    ('\u{25B2}', "U+25B2 BLACK UP-POINTING TRIANGLE — use egui_phosphor::regular::CARET_UP"),
    ('\u{25BC}', "U+25BC BLACK DOWN-POINTING TRIANGLE — use egui_phosphor::regular::CARET_DOWN"),
    ('\u{25BE}', "U+25BE SMALL DOWN TRIANGLE — use egui_phosphor::regular::CARET_DOWN"),
    ('\u{25B4}', "U+25B4 SMALL UP TRIANGLE — use egui_phosphor::regular::CARET_UP"),
];

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rs_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

#[test]
fn source_has_no_tofu_glyphs() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rs_files(&src, &mut files);
    assert!(!files.is_empty(), "no source files found under {src:?}");

    let mut violations = Vec::new();
    for path in &files {
        let content = fs::read_to_string(path).expect("read source file");
        for (line_no, line) in content.lines().enumerate() {
            for (ch, guidance) in FORBIDDEN {
                if line.contains(*ch) {
                    violations.push(format!(
                        "{}:{}: contains {} (renders as a missing-glyph box) — {}",
                        path.display(),
                        line_no + 1,
                        ch.escape_unicode(),
                        guidance
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "Found {} tofu-prone glyph(s) in the GUI source. Replace with phosphor icons:\n{}",
        violations.len(),
        violations.join("\n")
    );
}
