//! Engine-diagnostics badge surface (36 GUI half).
//!
//! A minimal, pure mapping from a first-class [`Diagnostic`](photonic_core::diag::Diagnostic)
//! to a compact badge — a severity colour token, a severity word, and a
//! `code — message` line — plus a tiny renderer. This is the GUI-side of the
//! 36 diagnostic taxonomy: the *shape* a status-bar badge / toast renders.
//!
//! ## Wiring status (trust-the-code note)
//! Spec 36 anticipates `EngineStatus.last_error` carrying a `Diagnostic`, but
//! as committed [`photonic_video::EngineStatus::last_error`] is still an
//! `Option<String>` — no `Diagnostic` reaches the GUI yet. The program monitor
//! (`app/monitor.rs`) already surfaces that `String` as an error line, so the
//! *surface* exists. This module supplies the ready-to-consume badge mapping so
//! that the moment the engine publishes a real `Diagnostic` on `EngineStatus`,
//! rendering it (code + message + severity colour) is a one-line call to
//! [`diag_badge`] — no view-model work left. Until then the mapping is verified
//! purely by its own tests.
#![allow(dead_code)] // wired-but-unconsumed until `EngineStatus` carries a `Diagnostic`.

use egui::{Color32, RichText};
use photonic_core::diag::{Diagnostic, Severity};

// Severity → foreground colour, matching the dark-theme design tokens
// (`theme.rs`: `warning` #FBBF24, `error` #F87171) and the shared `secondary`
// muted / `primary` accent used across the video panels.
const INFO: Color32 = Color32::from_rgb(0x8A, 0x8A, 0xA8); // `secondary` (muted)
const WARNING: Color32 = Color32::from_rgb(0xFB, 0xBF, 0x24); // `warning`
const ERROR: Color32 = Color32::from_rgb(0xF8, 0x71, 0x71); // `error`
const FATAL: Color32 = Color32::from_rgb(0xDC, 0x26, 0x26); // deep `error` — worst severity

/// The compact, egui-free view-model of a diagnostic badge — the honest
/// unit-test seam for the severity→colour / label mapping.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DiagBadge {
    /// Severity foreground colour token.
    pub color: Color32,
    /// The severity word ("Info" / "Warning" / "Error" / "Fatal").
    pub severity_label: &'static str,
    /// `"{code} — {message}"`, the one-line badge text.
    pub text: String,
}

/// The foreground colour token for a [`Severity`].
pub(crate) fn severity_color(severity: Severity) -> Color32 {
    match severity {
        Severity::Info => INFO,
        Severity::Warning => WARNING,
        Severity::Error => ERROR,
        Severity::Fatal => FATAL,
    }
}

/// The severity word shown in the badge.
pub(crate) fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Info => "Info",
        Severity::Warning => "Warning",
        Severity::Error => "Error",
        Severity::Fatal => "Fatal",
    }
}

/// Map a [`Diagnostic`] to its compact [`DiagBadge`]. Pure: the stable
/// machine code ([`DiagCode::as_str`](photonic_core::diag::DiagCode::as_str))
/// leads, then the human message. `detail` is deliberately excluded (36 §4.2:
/// technical detail never rides the primary presentation).
pub(crate) fn diag_badge(diag: &Diagnostic) -> DiagBadge {
    DiagBadge {
        color: severity_color(diag.severity),
        severity_label: severity_label(diag.severity),
        text: format!("{} — {}", diag.code.as_str(), diag.message),
    }
}

/// Render a badge inline: a coloured severity chip followed by the code +
/// message line. `count > 1` appends a coalescing multiplier (36 §4.1) so a
/// storm of repeats stays one compact badge.
pub(crate) fn draw_diag_badge(ui: &mut egui::Ui, badge: &DiagBadge, count: u64) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(badge.severity_label)
                .color(badge.color)
                .strong()
                .small(),
        );
        ui.label(RichText::new(&badge.text).color(badge.color).small());
        if count > 1 {
            ui.label(
                RichText::new(format!("×{count}"))
                    .color(badge.color)
                    .small(),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use photonic_core::diag::{DiagCode, Subject};

    fn diag(code: DiagCode, severity: Severity, msg: &str) -> Diagnostic {
        Diagnostic::new(code, Subject::Engine, msg).with_severity(severity)
    }

    #[test]
    fn badge_text_leads_with_code_then_message() {
        let d = diag(
            DiagCode::ExportEncoderFailed,
            Severity::Error,
            "x264 exited 1",
        );
        let b = diag_badge(&d);
        assert_eq!(b.text, "ExportEncoderFailed — x264 exited 1");
    }

    #[test]
    fn each_severity_maps_to_its_colour_and_label() {
        let cases = [
            (Severity::Info, INFO, "Info"),
            (Severity::Warning, WARNING, "Warning"),
            (Severity::Error, ERROR, "Error"),
            (Severity::Fatal, FATAL, "Fatal"),
        ];
        for (severity, color, label) in cases {
            let d = diag(DiagCode::ExportEncoderFailed, severity, "msg");
            let b = diag_badge(&d);
            assert_eq!(b.color, color, "colour for {severity:?}");
            assert_eq!(b.severity_label, label, "label for {severity:?}");
        }
    }

    #[test]
    fn severity_colours_are_all_distinct() {
        let all = [INFO, WARNING, ERROR, FATAL];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b, "severity colours must be visually distinct");
            }
        }
    }

    #[test]
    fn badge_excludes_technical_detail() {
        // 36 §4.2: `detail` (ffmpeg stderr tail etc.) never rides the badge.
        let d = diag(DiagCode::ExportEncoderFailed, Severity::Error, "failed")
            .with_detail("ffmpeg: giant stderr tail that must not surface");
        let b = diag_badge(&d);
        assert!(!b.text.contains("stderr"));
    }
}
