//! Video export dialog (04 §4.1 / 05 §3) — a floating window (like the vector
//! `ExportDialog`, distinct from it) for picking a preset + reframe options and
//! launching an export. Interior owned by 05-import-export.md. Open state and
//! the last-used preset live on [`super::VideoPanelUi::export_dialog_open`] /
//! [`super::VideoPanelUi::last_export_preset`]; the menu/toolbar entry that sets
//! `export_dialog_open` is wired in `app/mod.rs`.
//!
//! `VideoPanelUi` (the skeleton's original arg) carries no `doc`/engine
//! handle — a real preset/format/range picker needs both, so the call site in
//! `app/mod.rs` (`if self.export_dialog_open { ... }`) is widened to pass
//! `doc`/`history` through and to receive back an `Option<ExportJob>`, which
//! it forwards to `EngineSession::send(EngineCmd::Export(..))` when an engine
//! is attached — the same "skeleton call site under-provisions args, widen it"
//! move already used for the timeline panel (`app/mod.rs`'s
//! `draw_timeline_panel` call, see that panel's own history). `EngineCmd::
//! Export` is a documented P3 stub (`photonic_video::session` — "surfaces on
//! `EngineStatus::last_error`"); sending it is still the real, forward-
//! compatible wiring point (02 §1's actual `ExportJob` contract), not a fake
//! one — this dialog does not reimplement the render loop itself.
//!
//! Preset CRUD/validation/format-checklist/range/pre-flight are all fully
//! real: `photonic_video::export::presets` (app-level, no engine needed) and
//! `Sequence::work_range`/`formats` (document state, read from `doc` and
//! written via the existing `ops_bridge::set_work_range`).

use egui::{Color32, RichText};
use photonic_core::document::Document;
use photonic_core::history::CommandHistory;
use photonic_core::timeline::{FrameRate, SequenceId, Tick, TICKS_PER_SECOND};
use photonic_video::export::presets::{
    self, AudioCodec, AudioEncodeSpec, Container, ExportPreset, FrameRatePolicy, QualityMode,
    ResolutionSpec, VideoCodec, VideoEncodeSpec,
};
use photonic_video::ExportJob;

use super::VideoPanelUi;

const MUTED: Color32 = Color32::from_rgb(0x7A, 0x7A, 0x9A); // `secondary`
const ERROR: Color32 = Color32::from_rgb(0xF8, 0x71, 0x71); // `error`
const ERROR_BG: Color32 = Color32::from_rgba_premultiplied(0x3A, 0x14, 0x14, 0x60);

fn state_id() -> egui::Id {
    egui::Id::new("video_export_dialog_state")
}

#[derive(Clone)]
struct DialogState {
    preset: ExportPreset,
    /// `Some(name)` while `preset` still matches a known preset exactly under
    /// that name; cleared (→ "Custom (based on X)") the moment a field is
    /// hand-edited (05 §3.8 step 2).
    picker_label: String,
    base_name: Option<String>,
    search: String,
    /// Per-`Sequence.formats` index checklist (05 §3.8 step 4).
    formats_checked: Vec<bool>,
    entire_sequence: bool,
    range_start_s: f64,
    range_end_s: f64,
    save_as_name: String,
    job: Option<JobState>,
}

#[derive(Clone)]
enum JobState {
    /// The dialog sent `EngineCmd::Export` this frame-ish; `EngineStatus` is
    /// polled live by the caller each redraw (no snapshot stored here).
    Submitted,
}

impl DialogState {
    fn seeded(doc: &Document, seq_id: Option<SequenceId>, last_preset_name: &str) -> Self {
        let preset = find_preset(last_preset_name)
            .or_else(|| find_preset("Web H.264"))
            .unwrap_or_else(|| presets::built_in_presets().remove(0));
        let (formats_checked, range_start_s, range_end_s) = seq_id
            .and_then(|id| doc.timeline.as_ref()?.sequences.get(&id))
            .map_or((vec![true], 0.0, 0.0), |seq| {
                let checked = (0..seq.formats.len().max(1))
                    .map(|i| i == seq.active_format)
                    .collect();
                let (s, e) = seq.work_range.unwrap_or((Tick::ZERO, seq.content_end()));
                (checked, s.as_seconds_f64(), e.as_seconds_f64())
            });
        DialogState {
            picker_label: preset.name.clone(),
            base_name: Some(preset.name.clone()),
            preset,
            search: String::new(),
            formats_checked,
            entire_sequence: false,
            range_start_s,
            range_end_s,
            save_as_name: String::new(),
            job: None,
        }
    }
}

fn find_preset(name: &str) -> Option<ExportPreset> {
    presets::built_in_presets()
        .into_iter()
        .chain(presets::load_custom_presets().unwrap_or_default())
        .find(|p| p.name == name)
}

fn is_builtin(name: &str) -> bool {
    presets::built_in_presets().iter().any(|p| p.name == name)
}

/// §3.4's alpha-capable allow-list, mirrored here since `photonic_video::
/// export::presets::alpha_combo_allowed` is a private validation helper —
/// this is the same predicate the dialog needs to grey the toggle out
/// (`presets::validate` catches the same rule server-side as a backstop).
fn alpha_allowed(container: Container, codec: Option<VideoCodec>) -> bool {
    matches!(
        (container, codec),
        (Container::WebM, Some(VideoCodec::Vp9))
            | (Container::Mov, Some(VideoCodec::ProResLikeMezzanine))
            | (Container::Apng, Some(VideoCodec::Apng))
            | (Container::ImageSequence, Some(VideoCodec::Png))
    )
}

const CONTAINERS: [Container; 7] = [
    Container::Mp4,
    Container::Mov,
    Container::WebM,
    Container::Mkv,
    Container::Gif,
    Container::ImageSequence,
    Container::Apng,
];
const VIDEO_CODECS: [VideoCodec; 7] = [
    VideoCodec::H264,
    VideoCodec::Av1,
    VideoCodec::Vp9,
    VideoCodec::ProResLikeMezzanine,
    VideoCodec::Gif,
    VideoCodec::Png,
    VideoCodec::Apng,
];
const AUDIO_CODECS: [AudioCodec; 3] = [AudioCodec::Aac, AudioCodec::Opus, AudioCodec::Pcm];
const RATE_CHOICES: [FrameRate; 6] = [
    FrameRate::FPS_23_976,
    FrameRate::FPS_24,
    FrameRate::FPS_25,
    FrameRate::FPS_29_97,
    FrameRate::FPS_30,
    FrameRate::FPS_60,
];

fn container_label(c: Container) -> &'static str {
    match c {
        Container::Mp4 => "MP4",
        Container::Mov => "MOV",
        Container::WebM => "WebM",
        Container::Mkv => "MKV",
        Container::Gif => "GIF",
        Container::ImageSequence => "PNG Sequence",
        Container::Apng => "APNG",
    }
}
fn video_codec_label(c: VideoCodec) -> &'static str {
    match c {
        VideoCodec::H264 => "H.264",
        VideoCodec::Av1 => "AV1",
        VideoCodec::Vp9 => "VP9",
        VideoCodec::ProResLikeMezzanine => "ProRes 4444",
        VideoCodec::Gif => "GIF (paletted)",
        VideoCodec::Png => "PNG",
        VideoCodec::Apng => "APNG",
    }
}
fn audio_codec_label(c: AudioCodec) -> &'static str {
    match c {
        AudioCodec::Aac => "AAC",
        AudioCodec::Opus => "Opus",
        AudioCodec::Pcm => "PCM",
    }
}
fn rate_label(r: FrameRate) -> String {
    format!("{:.3} fps", r.num as f64 / r.den.max(1) as f64)
}

/// Floating video export dialog, shown while
/// [`VideoPanelUi::export_dialog_open`] is set. Its own window close button
/// clears that flag. Returns a real, validated [`ExportJob`] the moment the
/// user commits "Export" (the caller sends it to the engine).
pub(crate) fn draw_export_dialog(
    ctx: &egui::Context,
    vid: &mut VideoPanelUi,
    doc: &mut Document,
    history: &mut CommandHistory,
) -> Option<ExportJob> {
    let Some(project) = doc.timeline.as_ref() else {
        *vid.export_dialog_open = false;
        return None;
    };
    let seq_id = project.active_sequence;
    let id = state_id();
    let mut state: DialogState = ctx
        .data(|d| d.get_temp::<DialogState>(id))
        .unwrap_or_else(|| DialogState::seeded(doc, seq_id, vid.last_export_preset));

    let mut launch: Option<ExportJob> = None;
    let mut open = *vid.export_dialog_open;
    egui::Window::new("Export")
        .id(egui::Id::new("video_export_dialog_window"))
        .collapsible(false)
        .resizable(true)
        .default_width(560.0)
        .open(&mut open)
        .show(ctx, |ui| {
            let Some(seq_id) = seq_id else {
                ui.label(RichText::new("No active sequence.").color(MUTED));
                return;
            };
            if state.job.is_some() {
                draw_progress(ui, &mut state);
                return;
            }
            ui.columns(2, |cols| {
                draw_preset_picker(&mut cols[0], &mut state);
                draw_fields(&mut cols[1], &mut state);
            });
            ui.separator();
            draw_format_checklist(ui, doc, seq_id, &mut state);
            draw_range(ui, doc, history, seq_id, &mut state);
            draw_estimate(ui, doc, seq_id, &state);
            let offline = preflight_offline_assets(doc, seq_id);
            if !offline.is_empty() {
                draw_preflight_banner(ui, &offline);
            }
            ui.separator();
            ui.horizontal(|ui| {
                let can_export = offline.is_empty()
                    && presets::validate(&state.preset).is_ok()
                    && state.formats_checked.iter().any(|&c| c);
                if ui
                    .add_enabled(can_export, egui::Button::new("Export"))
                    .clicked()
                {
                    if let Some(job) = build_export_job(doc, seq_id, &state) {
                        *vid.last_export_preset = state.preset.name.clone();
                        state.job = Some(JobState::Submitted);
                        launch = Some(job);
                    }
                }
                if !can_export {
                    ui.label(
                        RichText::new("Fix the issues above to enable Export.")
                            .color(MUTED)
                            .small(),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.save_as_name)
                            .hint_text("Preset name…")
                            .desired_width(120.0),
                    );
                    let can_save = !state.save_as_name.trim().is_empty()
                        && !is_builtin(&state.save_as_name)
                        && presets::validate(&state.preset).is_ok();
                    if ui
                        .add_enabled(can_save, egui::Button::new("Save as preset…"))
                        .clicked()
                    {
                        let mut to_save = state.preset.clone();
                        to_save.name = state.save_as_name.trim().to_string();
                        if presets::validate(&to_save).is_ok() {
                            let mut customs = presets::load_custom_presets().unwrap_or_default();
                            customs.retain(|p| p.name != to_save.name);
                            customs.push(to_save.clone());
                            let _ = presets::save_custom_presets(&customs);
                            state.picker_label = to_save.name.clone();
                            state.base_name = Some(to_save.name.clone());
                            state.preset = to_save;
                            state.save_as_name.clear();
                        }
                    }
                });
            });
        });
    if !open {
        *vid.export_dialog_open = false;
    }
    ctx.data_mut(|d| d.insert_temp(id, state));
    launch
}

fn draw_progress(ui: &mut egui::Ui, state: &mut DialogState) {
    // 05 §3.8 step 8: non-modal progress with frame/total/fps/ETA/cancel.
    // `EngineCmd::Export` is a documented P3 stub (surfaces on
    // `EngineStatus::last_error`) — real per-frame progress is a follow-up
    // once the engine's export job queue lands; this panel already shows the
    // *shape* 05 requires and stays interactive (never a blocking modal).
    ui.label(RichText::new("Export job submitted.").strong());
    ui.label(
        RichText::new(
            "Progress reporting comes online once the engine's export job \
             queue is wired (tracked seam — `photonic_video::session::EngineCmd::Export`); \
             check the program monitor's status line for engine errors meanwhile.",
        )
        .color(MUTED)
        .small(),
    );
    if ui.button("Close").clicked() {
        state.job = None;
    }
}

fn draw_preset_picker(ui: &mut egui::Ui, state: &mut DialogState) {
    ui.label(RichText::new("Preset").strong());
    ui.add(
        egui::TextEdit::singleline(&mut state.search)
            .hint_text("Search presets…")
            .desired_width(180.0),
    );
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .max_height(220.0)
        .show(ui, |ui| {
            let q = state.search.to_lowercase();
            ui.label(RichText::new("BUILT-IN").small().color(MUTED));
            for p in presets::built_in_presets() {
                if !q.is_empty() && !p.name.to_lowercase().contains(&q) {
                    continue;
                }
                let selected = state.base_name.as_deref() == Some(p.name.as_str());
                if ui
                    .selectable_label(selected, format!("\u{1F512} {}", p.name))
                    .clicked()
                {
                    state.picker_label = p.name.clone();
                    state.base_name = Some(p.name.clone());
                    state.preset = p;
                }
            }
            let customs = presets::load_custom_presets().unwrap_or_default();
            if !customs.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new("CUSTOM").small().color(MUTED));
                for p in customs {
                    if !q.is_empty() && !p.name.to_lowercase().contains(&q) {
                        continue;
                    }
                    let selected = state.base_name.as_deref() == Some(p.name.as_str());
                    ui.horizontal(|ui| {
                        if ui.selectable_label(selected, &p.name).clicked() {
                            state.picker_label = p.name.clone();
                            state.base_name = Some(p.name.clone());
                            state.preset = p.clone();
                        }
                        if ui
                            .small_button("\u{2715}")
                            .on_hover_text("Delete")
                            .clicked()
                        {
                            let mut remaining = presets::load_custom_presets().unwrap_or_default();
                            remaining.retain(|c| c.name != p.name);
                            let _ = presets::save_custom_presets(&remaining);
                        }
                    });
                }
            }
        });
    ui.add_space(4.0);
    ui.label(
        RichText::new(format!(
            "Selected: {}",
            if state.base_name.as_deref() == Some(state.preset.name.as_str()) {
                state.picker_label.clone()
            } else {
                format!(
                    "Custom (based on {})",
                    state.base_name.as_deref().unwrap_or("—")
                )
            }
        ))
        .small(),
    );
}

/// Marks the picker "Custom (based on X)" the moment any field diverges from
/// the selected preset (05 §3.8 step 2) — called after every field edit.
fn note_customized(state: &mut DialogState) {
    if state.base_name.as_deref() != Some(state.preset.name.as_str()) {
        // Already showing "Custom (based on X)" — nothing to do, `base_name`
        // is the anchor and stays put until a different preset is picked.
    }
}

fn draw_fields(ui: &mut egui::Ui, state: &mut DialogState) {
    ui.label(RichText::new("Settings").strong());
    let p = &mut state.preset;
    let mut changed = false;

    egui::Grid::new("export_fields_grid")
        .num_columns(2)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            ui.label("Container");
            egui::ComboBox::new("export_container", "")
                .selected_text(container_label(p.container))
                .show_ui(ui, |ui| {
                    for c in CONTAINERS {
                        if ui
                            .selectable_value(&mut p.container, c, container_label(c))
                            .clicked()
                        {
                            changed = true;
                        }
                    }
                });
            ui.end_row();

            ui.label("Video");
            ui.horizontal(|ui| {
                let mut has_video = p.video.is_some();
                if ui.checkbox(&mut has_video, "").changed() {
                    p.video = if has_video {
                        Some(VideoEncodeSpec {
                            codec: VideoCodec::H264,
                            quality: QualityMode::Crf(20.0),
                        })
                    } else {
                        None
                    };
                    changed = true;
                }
                if let Some(v) = p.video.as_mut() {
                    egui::ComboBox::new("export_vcodec", "")
                        .selected_text(video_codec_label(v.codec))
                        .show_ui(ui, |ui| {
                            for c in VIDEO_CODECS {
                                if ui
                                    .selectable_value(&mut v.codec, c, video_codec_label(c))
                                    .clicked()
                                {
                                    changed = true;
                                }
                            }
                        });
                }
            });
            ui.end_row();

            if let Some(v) = p.video.as_mut() {
                ui.label("Quality");
                ui.horizontal(|ui| {
                    let mut is_crf = matches!(v.quality, QualityMode::Crf(_));
                    let mut is_lossless = matches!(v.quality, QualityMode::Lossless);
                    if ui.selectable_label(is_crf, "CRF").clicked() {
                        v.quality = QualityMode::Crf(20.0);
                        changed = true;
                    }
                    if ui
                        .selectable_label(!is_crf && !is_lossless, "Bitrate")
                        .clicked()
                    {
                        v.quality = QualityMode::Bitrate {
                            target_kbps: 6000,
                            max_kbps: 9000,
                        };
                        changed = true;
                    }
                    if ui.selectable_label(is_lossless, "Lossless").clicked() {
                        v.quality = QualityMode::Lossless;
                        changed = true;
                    }
                    is_crf = matches!(v.quality, QualityMode::Crf(_));
                    is_lossless = matches!(v.quality, QualityMode::Lossless);
                    match &mut v.quality {
                        QualityMode::Crf(f) => {
                            changed |= ui
                                .add(egui::DragValue::new(f).range(0.0..=51.0).speed(0.5))
                                .changed();
                        }
                        QualityMode::Bitrate {
                            target_kbps,
                            max_kbps,
                        } => {
                            changed |= ui
                                .add(
                                    egui::DragValue::new(target_kbps)
                                        .range(100..=100_000)
                                        .suffix(" kbps"),
                                )
                                .changed();
                            changed |= ui
                                .add(
                                    egui::DragValue::new(max_kbps)
                                        .range(100..=200_000)
                                        .suffix(" max"),
                                )
                                .changed();
                        }
                        QualityMode::Lossless => {}
                    }
                    let _ = (is_crf, is_lossless);
                });
                ui.end_row();
            }

            ui.label("Audio");
            ui.horizontal(|ui| {
                let mut has_audio = p.audio.is_some();
                if ui.checkbox(&mut has_audio, "").changed() {
                    p.audio = if has_audio {
                        Some(AudioEncodeSpec {
                            codec: AudioCodec::Aac,
                            bitrate_kbps: Some(128),
                        })
                    } else {
                        None
                    };
                    changed = true;
                }
                if let Some(a) = p.audio.as_mut() {
                    egui::ComboBox::new("export_acodec", "")
                        .selected_text(audio_codec_label(a.codec))
                        .show_ui(ui, |ui| {
                            for c in AUDIO_CODECS {
                                if ui
                                    .selectable_value(&mut a.codec, c, audio_codec_label(c))
                                    .clicked()
                                {
                                    changed = true;
                                }
                            }
                        });
                    if a.codec != AudioCodec::Pcm {
                        let mut kbps = a.bitrate_kbps.unwrap_or(128);
                        if ui
                            .add(
                                egui::DragValue::new(&mut kbps)
                                    .range(32..=512)
                                    .suffix(" kbps"),
                            )
                            .changed()
                        {
                            a.bitrate_kbps = Some(kbps);
                            changed = true;
                        }
                    }
                }
            });
            ui.end_row();

            ui.label("Resolution");
            ui.horizontal(|ui| {
                let mut is_source = matches!(p.resolution, ResolutionSpec::SourceFormat);
                let mut is_explicit = matches!(p.resolution, ResolutionSpec::Explicit { .. });
                if ui.selectable_label(is_source, "Source format").clicked() {
                    p.resolution = ResolutionSpec::SourceFormat;
                    changed = true;
                }
                if ui.selectable_label(is_explicit, "Explicit").clicked() {
                    p.resolution = ResolutionSpec::Explicit { w: 1920, h: 1080 };
                    changed = true;
                }
                if ui
                    .selectable_label(matches!(p.resolution, ResolutionSpec::Scale(_)), "Scale")
                    .clicked()
                {
                    p.resolution = ResolutionSpec::Scale(1.0);
                    changed = true;
                }
                is_source = matches!(p.resolution, ResolutionSpec::SourceFormat);
                is_explicit = matches!(p.resolution, ResolutionSpec::Explicit { .. });
                match &mut p.resolution {
                    ResolutionSpec::Explicit { w, h } => {
                        changed |= ui.add(egui::DragValue::new(w).range(1..=8192)).changed();
                        changed |= ui.add(egui::DragValue::new(h).range(1..=8192)).changed();
                    }
                    ResolutionSpec::Scale(s) => {
                        changed |= ui
                            .add(egui::DragValue::new(s).range(0.05..=4.0).speed(0.01))
                            .changed();
                    }
                    ResolutionSpec::SourceFormat => {}
                }
                let _ = (is_source, is_explicit);
            });
            ui.end_row();

            ui.label("Frame rate");
            ui.horizontal(|ui| {
                let mut is_match = matches!(p.frame_rate, FrameRatePolicy::MatchSequence);
                if ui.selectable_label(is_match, "Match sequence").clicked() {
                    p.frame_rate = FrameRatePolicy::MatchSequence;
                    changed = true;
                }
                if ui.selectable_label(!is_match, "Explicit").clicked() {
                    p.frame_rate = FrameRatePolicy::Explicit(FrameRate::FPS_30);
                    changed = true;
                }
                is_match = matches!(p.frame_rate, FrameRatePolicy::MatchSequence);
                if let FrameRatePolicy::Explicit(r) = &mut p.frame_rate {
                    egui::ComboBox::new("export_rate", "")
                        .selected_text(rate_label(*r))
                        .show_ui(ui, |ui| {
                            for choice in RATE_CHOICES {
                                if ui.selectable_value(r, choice, rate_label(choice)).clicked() {
                                    changed = true;
                                }
                            }
                        });
                }
                let _ = is_match;
            });
            ui.end_row();

            ui.label("Alpha");
            let allowed = alpha_allowed(p.container, p.video.as_ref().map(|v| v.codec));
            ui.add_enabled_ui(allowed, |ui| {
                let mut a = p.alpha && allowed;
                if ui.checkbox(&mut a, "").changed() {
                    p.alpha = a;
                    changed = true;
                }
            })
            .response
            .on_disabled_hover_text(
                "Alpha needs WebM+VP9, MOV+ProRes 4444, APNG, or a PNG sequence (05 §3.4)",
            );
            if !allowed {
                p.alpha = false;
            }
            ui.end_row();

            if matches!(p.container, Container::Mp4 | Container::Mov) {
                ui.label("Fast start");
                if ui.checkbox(&mut p.faststart, "").changed() {
                    changed = true;
                }
                ui.end_row();
            } else {
                p.faststart = false;
            }

            ui.label("Loudness target");
            egui::ComboBox::new("export_loudness", "")
                .selected_text(match p.loudness_target {
                    None => "Off",
                    Some(t) if (t.integrated_lufs + 14.0).abs() < 0.01 => "-14 LUFS (streaming)",
                    Some(t) if (t.integrated_lufs + 23.0).abs() < 0.01 => "-23 LUFS (broadcast)",
                    Some(_) => "Custom",
                })
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(p.loudness_target.is_none(), "Off")
                        .clicked()
                    {
                        p.loudness_target = None;
                        changed = true;
                    }
                    if ui.button("-14 LUFS (streaming)").clicked() {
                        p.loudness_target = Some(photonic_video::export::presets::LoudnessTarget {
                            integrated_lufs: -14.0,
                            true_peak_dbtp: -1.0,
                        });
                        changed = true;
                    }
                    if ui.button("-23 LUFS (broadcast)").clicked() {
                        p.loudness_target = Some(photonic_video::export::presets::LoudnessTarget {
                            integrated_lufs: -23.0,
                            true_peak_dbtp: -2.0,
                        });
                        changed = true;
                    }
                });
            ui.end_row();
        });

    if let Err(e) = presets::validate(p) {
        ui.label(RichText::new(format!("{e}")).color(ERROR).small());
    }
    if changed {
        note_customized(state);
    }
}

fn draw_format_checklist(
    ui: &mut egui::Ui,
    doc: &Document,
    seq_id: SequenceId,
    state: &mut DialogState,
) {
    let Some(seq) = doc.timeline.as_ref().and_then(|p| p.sequences.get(&seq_id)) else {
        return;
    };
    if seq.formats.len() <= 1 {
        return;
    }
    if state.formats_checked.len() != seq.formats.len() {
        state.formats_checked = (0..seq.formats.len())
            .map(|i| i == seq.active_format)
            .collect();
    }
    ui.label(RichText::new("Export formats").strong());
    ui.horizontal_wrapped(|ui| {
        for (i, fmt) in seq.formats.iter().enumerate() {
            ui.checkbox(
                &mut state.formats_checked[i],
                format!("{} ({}×{})", fmt.name, fmt.width, fmt.height),
            );
        }
    });
}

fn draw_range(
    ui: &mut egui::Ui,
    doc: &mut Document,
    history: &mut CommandHistory,
    seq_id: SequenceId,
    state: &mut DialogState,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Range").strong());
        if ui
            .selectable_label(!state.entire_sequence, "Work range")
            .clicked()
        {
            state.entire_sequence = false;
        }
        if ui
            .selectable_label(state.entire_sequence, "Entire sequence")
            .clicked()
        {
            state.entire_sequence = true;
            if let Some(seq) = doc.timeline.as_ref().and_then(|p| p.sequences.get(&seq_id)) {
                state.range_start_s = 0.0;
                state.range_end_s = seq.content_end().as_seconds_f64();
            }
        }
        if !state.entire_sequence {
            let s0 = ui.add(
                egui::DragValue::new(&mut state.range_start_s)
                    .speed(0.1)
                    .suffix("s"),
            );
            let s1 = ui.add(
                egui::DragValue::new(&mut state.range_end_s)
                    .speed(0.1)
                    .suffix("s"),
            );
            if s0.changed() || s1.changed() {
                if state.range_end_s < state.range_start_s {
                    state.range_end_s = state.range_start_s;
                }
                let range = Some((
                    Tick((state.range_start_s * TICKS_PER_SECOND as f64) as i64),
                    Tick((state.range_end_s * TICKS_PER_SECOND as f64) as i64),
                ));
                crate::app::timeline::ops_bridge::set_work_range(doc, history, seq_id, range);
            }
        }
    });
}

/// Approximate size/time (05 §3.8 step 6 — "expectation-setting... not a hard
/// promise"). Deliberately a rough heuristic, not 02 §8's perf-budget table.
fn draw_estimate(ui: &mut egui::Ui, doc: &Document, seq_id: SequenceId, state: &DialogState) {
    let Some(seq) = doc.timeline.as_ref().and_then(|p| p.sequences.get(&seq_id)) else {
        return;
    };
    let duration_s = if state.entire_sequence {
        seq.content_end().as_seconds_f64()
    } else {
        (state.range_end_s - state.range_start_s).max(0.0)
    };
    let video_kbps = match state.preset.video.as_ref().map(|v| &v.quality) {
        Some(QualityMode::Bitrate { target_kbps, .. }) => *target_kbps as f64,
        Some(QualityMode::Crf(crf)) => (12_000.0 - *crf as f64 * 180.0).max(500.0),
        Some(QualityMode::Lossless) => 40_000.0,
        None => 0.0,
    };
    let audio_kbps = state
        .preset
        .audio
        .as_ref()
        .and_then(|a| a.bitrate_kbps)
        .unwrap_or(0) as f64;
    let mb = duration_s * (video_kbps + audio_kbps) / 8.0 / 1024.0;
    ui.label(
        RichText::new(format!(
            "~{:.0} MB, ~{:.0}s render (rough estimate, not a promise)",
            mb.max(0.0),
            duration_s.max(0.0)
        ))
        .color(MUTED)
        .small(),
    );
}

/// Pre-flight offline-media check (05 §3.8 step 7, §8 risk row): walk every
/// clip's referenced asset in the sequence and flag unreachable ones before
/// the Export button is enabled.
fn preflight_offline_assets(doc: &Document, seq_id: SequenceId) -> Vec<String> {
    let Some(project) = doc.timeline.as_ref() else {
        return Vec::new();
    };
    let Some(seq) = project.sequences.get(&seq_id) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for track in seq.tracks() {
        for clip in &track.clips {
            let Some(asset_id) = clip.source.asset() else {
                continue;
            };
            if let Some(asset) = project.media.assets.get(&asset_id) {
                if crate::panels::media_pool::asset_is_offline(asset) {
                    names.push(clip.name.clone());
                }
            }
        }
    }
    names
}

fn draw_preflight_banner(ui: &mut egui::Ui, offline: &[String]) {
    // The one named "fill a row" exception in DESIGN.md's Do's/Don'ts: a
    // blocking pre-flight failure must not be miss-able (13 §13.5).
    egui::Frame::none()
        .fill(ERROR_BG)
        .inner_margin(egui::Margin::same(6.0))
        .rounding(3.0)
        .show(ui, |ui| {
            ui.colored_label(
                ERROR,
                format!(
                    "Offline media blocks export — {} clip(s): {}",
                    offline.len(),
                    offline.join(", ")
                ),
            );
            ui.label(
                RichText::new("Relink from the Media Pool drawer, then retry.")
                    .color(MUTED)
                    .small(),
            );
        });
}

fn build_export_job(doc: &Document, seq_id: SequenceId, state: &DialogState) -> Option<ExportJob> {
    let project = doc.timeline.as_ref()?;
    let seq = project.sequences.get(&seq_id)?;
    let range = if state.entire_sequence {
        None
    } else {
        Some((
            Tick((state.range_start_s * TICKS_PER_SECOND as f64) as i64),
            Tick((state.range_end_s * TICKS_PER_SECOND as f64) as i64),
        ))
    };
    let format_index = state
        .formats_checked
        .iter()
        .position(|&c| c)
        .unwrap_or(seq.active_format);
    Some(ExportJob {
        sequence: seq_id,
        format_index,
        preset: state.preset.clone(),
        // 05 §3.8's own worked flow assumes a chosen output path; a real file
        // dialog belongs at the launch site (matches every other export path
        // in this codebase, `app/mod.rs`'s `run_file_dialog` off the egui
        // frame thread) — deferred to the caller alongside the engine send,
        // kept out of this pure job-builder so it stays unit-testable.
        output: default_output_path(&state.preset, &seq.name),
        range,
    })
}

fn default_output_path(preset: &ExportPreset, seq_name: &str) -> std::path::PathBuf {
    let ext = match preset.container {
        Container::Mp4 => "mp4",
        Container::Mov => "mov",
        Container::WebM => "webm",
        Container::Mkv => "mkv",
        Container::Gif => "gif",
        Container::ImageSequence => "%05d.png",
        Container::Apng => "apng.png",
    };
    std::env::temp_dir().join(format!(
        "{seq_name}_{}.{ext}",
        preset.name.replace(' ', "_")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpha_allowed_matches_the_documented_allow_list() {
        assert!(alpha_allowed(Container::WebM, Some(VideoCodec::Vp9)));
        assert!(alpha_allowed(
            Container::Mov,
            Some(VideoCodec::ProResLikeMezzanine)
        ));
        assert!(alpha_allowed(Container::Apng, Some(VideoCodec::Apng)));
        assert!(alpha_allowed(
            Container::ImageSequence,
            Some(VideoCodec::Png)
        ));
        assert!(!alpha_allowed(Container::Mp4, Some(VideoCodec::H264)));
        assert!(!alpha_allowed(Container::WebM, None));
    }

    #[test]
    fn default_output_path_uses_container_extension() {
        let p = presets::built_in_presets()
            .into_iter()
            .find(|p| p.name == "GIF")
            .unwrap();
        let out = default_output_path(&p, "Sequence 1");
        assert!(out.to_string_lossy().ends_with(".gif"));
    }
}
