//! `RightDrawerGroup::AudioMixer` panel (04 §4.1) — track fader strips, master
//! bus meters, and per-track EQ/comp/automation entry points. Interior owned by
//! 09-audio-mixer.md. Expanded strips live on
//! [`super::VideoPanelUi::mixer_expanded_tracks`].
//!
//! Stub — filled by the audio-mixer (09) panel story.

use super::VideoPanelUi;
use egui::Ui;

/// Right-rail Audio Mixer drawer. Called directly from `app/mod.rs`'s
/// right-drawer match (no `PropPanelCtx`).
pub(crate) fn draw_audio_mixer(_ui: &mut Ui, _vid: &mut VideoPanelUi) {
    // Audio-mixer (09) panel story fills this.
}
