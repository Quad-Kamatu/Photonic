//! `DrawerGroup::Effects` panel (04 §4.1) — the effect browser/catalog with
//! drag-to-apply onto the selected clip. Interior owned by 08-fusion-node-flows.md.
//!
//! The catalogue is driven by [`MANIFESTS`] (30 / K-B16) rather than a hard-coded
//! `EffectKind` subset so every bridged effect is one double-click / drag away
//! from a clip. Application uses [`ClipEffect::from_manifest`] so params seed
//! from the manifest defaults (not the legacy neutral seed alone).
//!
//! Two application paths (13 §6.3), both real (no stub): drag a row onto the
//! Clip Inspector's effects-stack drop target (`clip_inspector.rs`, consuming
//! [`EffectDrag`] exactly like the media-pool → timeline `AssetDrag` idiom),
//! or double-click a row to apply straight to every currently selected clip
//! (13 §6.6: "document it as load-bearing, not incidental" — the primary
//! keyboard/no-drag path, not a fallback afterthought). A timeline-lane drop
//! target is a documented seam for whoever finishes `timeline/clips.rs`'s
//! drag-and-drop (mirrors this codebase's existing seam-documentation
//! convention, e.g. `app/engine.rs`'s thumbnail/waveform notes).

use crate::panels::PanelAction;
use crate::panels::PropPanelCtx;
use egui::{Color32, RichText, Ui};
use photonic_core::timeline::{
    ops, ClipEffect, EffectCategory, EffectId, EffectKind, EffectManifest, MANIFESTS,
};

const MUTED: Color32 = Color32::from_rgb(0x7A, 0x7A, 0x9A); // `secondary`
const SECTION: Color32 = Color32::from_rgb(0x50, 0x50, 0x6E); // section-header dim-muted

/// Drag payload for an effects-browser row → drop target (consumed today by
/// `clip_inspector.rs`'s effects-stack section; a future timeline-lane drop
/// target would consume the same payload type).
#[derive(Clone, Debug)]
pub(crate) struct EffectDrag {
    pub(crate) id: EffectId,
}

/// Category display order (palette sections).
const CATEGORY_ORDER: &[EffectCategory] = &[
    EffectCategory::Blur,
    EffectCategory::Sharpen,
    EffectCategory::Color,
    EffectCategory::Stylize,
    EffectCategory::Key,
    EffectCategory::Geo,
    EffectCategory::Noise,
    EffectCategory::Util,
];

fn category_title(cat: EffectCategory) -> &'static str {
    match cat {
        EffectCategory::Blur => "BLUR",
        EffectCategory::Sharpen => "SHARPEN",
        EffectCategory::Color => "COLOR",
        EffectCategory::Stylize => "STYLIZE",
        EffectCategory::Key => "KEYS",
        EffectCategory::Geo => "GEOMETRY",
        EffectCategory::Noise => "NOISE",
        EffectCategory::Util => "UTIL / GENERATORS",
    }
}

/// Left-rail Effects Browser drawer.
pub(crate) fn draw_effects_browser(ui: &mut Ui, ctx: &mut PropPanelCtx) {
    let Some(project) = ctx.doc.timeline.as_ref() else {
        ui.label(RichText::new("No video project yet.").color(MUTED));
        return;
    };
    let selection: Vec<_> = ctx.video.selection.to_vec();
    if selection.is_empty() {
        ui.label(
            RichText::new("Select a clip to apply effects to it.")
                .color(MUTED)
                .small(),
        );
    }

    ui.label(
        RichText::new(format!("{} effects", MANIFESTS.len()))
            .small()
            .color(MUTED),
    );

    let mut action: Option<PanelAction> = None;
    for &cat in CATEGORY_ORDER {
        let entries: Vec<&EffectManifest> = MANIFESTS
            .iter()
            .filter(|m| m.category == cat)
            .filter(|m| ctx.matches(m.name) || ctx.matches(m.id.as_str()))
            .collect();
        if entries.is_empty() {
            continue;
        }
        section_header(ui, category_title(cat));
        for m in entries {
            draw_row(ui, project, &selection, m, &mut action);
        }
    }

    if action.is_some() {
        ctx.action = action;
    }
}

/// A small, muted section heading (mirrors `tools_panel.rs`'s `section_header`
/// idiom — DESIGN.md's `#50506E` "dim-muted" section-header token, 13 §6.5).
fn section_header(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(text).small().color(SECTION));
    ui.add_space(2.0);
}

fn draw_row(
    ui: &mut Ui,
    project: &photonic_core::timeline::TimelineProject,
    selection: &[photonic_core::timeline::ClipId],
    m: &EffectManifest,
    action: &mut Option<PanelAction>,
) {
    let disabled = selection.is_empty();
    let drag_id = ui.id().with(("effect_drag", m.id.as_str()));
    let glyph = effect_glyph(m);
    let label = format!("{glyph}  {}", m.name);
    let drag_payload = EffectDrag { id: m.id.clone() };
    let inner = ui
        .dnd_drag_source(drag_id, drag_payload, |ui| {
            ui.add_enabled(!disabled, egui::SelectableLabel::new(false, label))
        })
        .inner;
    let hover_text = if disabled {
        format!(
            "{} ({}) — select a clip first (drag onto the Clip Inspector's Effects \
             section, or double-click once a clip is selected)",
            m.name,
            m.id.as_str()
        )
    } else {
        format!(
            "{} ({}) — drag onto the Clip Inspector's Effects section, or \
             double-click to apply to the current selection",
            m.name,
            m.id.as_str()
        )
    };
    let inner = inner.on_hover_text(hover_text);
    // Double-click-to-apply (13 §6.3/§6.6): the load-bearing no-drag path,
    // applying to every selected clip as one undo step.
    if !disabled && inner.double_clicked() {
        let Some(effect_seed) = ClipEffect::from_manifest(m.id.clone()) else {
            return;
        };
        let mut cmds = Vec::new();
        for &clip_id in selection {
            if let Some((seq_id, track_id)) = locate(project, clip_id) {
                if let Ok(cmd) = ops::add_effect(
                    project,
                    seq_id,
                    track_id,
                    clip_id,
                    effect_seed.clone(),
                    None,
                ) {
                    cmds.push(cmd);
                }
            }
        }
        if !cmds.is_empty() {
            *action = Some(PanelAction::ClipEditBatch(cmds));
        }
    }
}

fn effect_glyph(m: &EffectManifest) -> &'static str {
    // Prefer legacy glyphs when the id maps to a v1 EffectKind; otherwise
    // category-based dots so the palette stays scannable.
    if let Some(kind) = m.id.legacy_kind() {
        return match kind {
            EffectKind::Blur => "\u{25CF}",         // ●
            EffectKind::Sharpen => "\u{25C6}",      // ♦
            EffectKind::Glow => "\u{2726}",         // ✦
            EffectKind::ChromaKey => "\u{25D1}",    // ◑
            EffectKind::LumaKey => "\u{25D0}",      // ◐
            EffectKind::Invert => "\u{25D3}",       // ◓
            EffectKind::MaskShapeGen => "\u{25A1}", // □
            _ => "\u{25CF}",
        };
    }
    match m.category {
        EffectCategory::Blur => "\u{25CF}",
        EffectCategory::Sharpen => "\u{25C6}",
        EffectCategory::Color => "\u{25D3}",
        EffectCategory::Stylize => "\u{2726}",
        EffectCategory::Key => "\u{25D1}",
        EffectCategory::Geo => "\u{25B3}",
        EffectCategory::Noise => "\u{2591}",
        EffectCategory::Util => "\u{25A1}",
    }
}

/// Resolve a `ClipId`'s owning (sequence, track) — same lookup
/// `clip_inspector.rs` needs, duplicated locally rather than sharing a
/// `pub(crate)` helper across the two files (kept each panel independently
/// buildable per the story split; both are O(clips) and clip counts are
/// small).
fn locate(
    project: &photonic_core::timeline::TimelineProject,
    clip_id: photonic_core::timeline::ClipId,
) -> Option<(
    photonic_core::timeline::SequenceId,
    photonic_core::timeline::TrackId,
)> {
    for (seq_id, seq) in &project.sequences {
        for track in seq.tracks() {
            if track.clips.iter().any(|c| c.id == clip_id) {
                return Some((*seq_id, track.id));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_covers_every_manifest_category() {
        for m in MANIFESTS {
            assert!(
                CATEGORY_ORDER.contains(&m.category),
                "manifest {} category {:?} missing from CATEGORY_ORDER",
                m.id.as_str(),
                m.category
            );
        }
    }

    #[test]
    fn from_manifest_seeds_all_catalogue_entries() {
        for m in MANIFESTS {
            let fx = ClipEffect::from_manifest(m.id.clone())
                .unwrap_or_else(|| panic!("from_manifest failed for {}", m.id.as_str()));
            assert_eq!(fx.id, m.id);
            assert!(!fx.inert);
            assert!(fx.enabled);
        }
    }
}
