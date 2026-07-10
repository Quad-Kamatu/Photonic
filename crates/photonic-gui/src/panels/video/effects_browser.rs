//! `DrawerGroup::Effects` panel (04 §4.1) — the effect browser/catalog with
//! drag-to-apply onto the selected clip. Interior owned by 08-fusion-node-flows.md.
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
use photonic_core::timeline::{ops, ClipEffect, EffectKind};

const MUTED: Color32 = Color32::from_rgb(0x7A, 0x7A, 0x9A); // `secondary`
const SECTION: Color32 = Color32::from_rgb(0x50, 0x50, 0x6E); // section-header dim-muted

/// Drag payload for an effects-browser row → drop target (consumed today by
/// `clip_inspector.rs`'s effects-stack section; a future timeline-lane drop
/// target would consume the same payload type).
#[derive(Clone, Copy, Debug)]
pub(crate) struct EffectDrag {
    pub(crate) kind: EffectKind,
}

struct CatalogEntry {
    kind: EffectKind,
    name: &'static str,
    description: &'static str,
}

const FILTERS: &[CatalogEntry] = &[
    CatalogEntry {
        kind: EffectKind::Blur,
        name: "Blur",
        description: "Gaussian blur — softens detail by a pixel radius.",
    },
    CatalogEntry {
        kind: EffectKind::Sharpen,
        name: "Sharpen",
        description: "Unsharp-mask style edge enhancement.",
    },
    CatalogEntry {
        kind: EffectKind::Glow,
        name: "Glow",
        description: "Bloom highlights above a brightness threshold.",
    },
    CatalogEntry {
        kind: EffectKind::Invert,
        name: "Invert",
        description: "Inverts color channels.",
    },
];

const KEYS: &[CatalogEntry] = &[
    CatalogEntry {
        kind: EffectKind::ChromaKey,
        name: "Chroma Key",
        description: "Keys out a chosen color (green/blue screen).",
    },
    CatalogEntry {
        kind: EffectKind::LumaKey,
        name: "Luma Key",
        description: "Keys out a luminance range.",
    },
];

const GENERATORS: &[CatalogEntry] = &[CatalogEntry {
    kind: EffectKind::MaskShapeGen,
    name: "Mask Shape",
    description: "Generates an ellipse/rectangle power-window mask.",
}];

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

    let mut action: Option<PanelAction> = None;
    draw_family(ui, ctx, project, &selection, "FILTERS", FILTERS, &mut action);
    draw_family(ui, ctx, project, &selection, "KEYS", KEYS, &mut action);
    draw_family(
        ui,
        ctx,
        project,
        &selection,
        "GENERATORS",
        GENERATORS,
        &mut action,
    );

    if action.is_some() {
        ctx.action = action;
    }
}

fn draw_family(
    ui: &mut Ui,
    ctx: &PropPanelCtx,
    project: &photonic_core::timeline::TimelineProject,
    selection: &[photonic_core::timeline::ClipId],
    title: &str,
    entries: &[CatalogEntry],
    action: &mut Option<PanelAction>,
) {
    let visible: Vec<&CatalogEntry> = entries.iter().filter(|e| ctx.matches(e.name)).collect();
    if visible.is_empty() {
        return;
    }
    section_header(ui, title);
    for entry in visible {
        draw_row(ui, project, selection, entry, action);
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
    entry: &CatalogEntry,
    action: &mut Option<PanelAction>,
) {
    let disabled = selection.is_empty();
    let drag_id = ui.id().with(("effect_drag", entry.kind));
    let label = format!("{}  {}", effect_glyph(entry.kind), entry.name);
    let inner = ui
        .dnd_drag_source(drag_id, EffectDrag { kind: entry.kind }, |ui| {
            ui.add_enabled(!disabled, egui::SelectableLabel::new(false, label))
        })
        .inner;
    let hover_text = if disabled {
        format!(
            "{} — select a clip first (drag onto the Clip Inspector's Effects \
             section, or double-click once a clip is selected)",
            entry.description
        )
    } else {
        format!(
            "{} — drag onto the Clip Inspector's Effects section, or \
             double-click to apply to the current selection",
            entry.description
        )
    };
    let inner = inner.on_hover_text(hover_text);
    // Double-click-to-apply (13 §6.3/§6.6): the load-bearing no-drag path,
    // applying to every selected clip as one undo step.
    if !disabled && inner.double_clicked() {
        let mut cmds = Vec::new();
        for &clip_id in selection {
            if let Some((seq_id, track_id)) = locate(project, clip_id) {
                if let Ok(cmd) = ops::add_effect(
                    project,
                    seq_id,
                    track_id,
                    clip_id,
                    ClipEffect::new(entry.kind),
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

fn effect_glyph(kind: EffectKind) -> &'static str {
    match kind {
        EffectKind::Blur => "\u{25CF}",       // ● generic filter dot
        EffectKind::Sharpen => "\u{25C6}",    // ♦
        EffectKind::Glow => "\u{2726}",       // ✦
        EffectKind::ChromaKey => "\u{25D1}",  // ◑ key
        EffectKind::LumaKey => "\u{25D0}",    // ◐ key
        EffectKind::Invert => "\u{25D3}",     // ◓
        EffectKind::MaskShapeGen => "\u{25A1}", // □ mask
        _ => "\u{25CF}",
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
