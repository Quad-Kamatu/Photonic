//! Sequences, tracks, and the top-level project container (01 §2, §4).

use super::audio::{MasterBus, TrackAudio};
use super::captions::CaptionTrack;
use super::clip::Clip;
use super::graph::NodeGraph;
use super::ids::{ClipId, CueId, GraphId, MarkerId, SequenceId, TrackId};
use super::media::MediaPool;
use super::time::{FrameRate, Tick};
use crate::Color;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The document-level timeline container (01 §2). `Document::timeline` is
/// `Option<TimelineProject>`; the first video-mode action creates it undoably.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimelineProject {
    pub media: MediaPool,
    pub sequences: HashMap<SequenceId, Sequence>,
    /// UI ordering of sequences.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sequence_order: Vec<SequenceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_sequence: Option<SequenceId>,
    /// All user node graphs (per-clip + project), one arena (§8).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub graphs: HashMap<GraphId, NodeGraph>,
    /// Spliced after the active sequence output (02 §2 step 6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_graph: Option<GraphId>,
    #[serde(default)]
    pub settings: ProjectVideoSettings,
}

impl TimelineProject {
    /// An empty project (no sequences yet).
    pub fn new() -> Self {
        TimelineProject {
            media: MediaPool::new(),
            sequences: HashMap::new(),
            sequence_order: Vec::new(),
            active_sequence: None,
            graphs: HashMap::new(),
            project_graph: None,
            settings: ProjectVideoSettings::default(),
        }
    }

    /// Insert a sequence, appending it to the UI order and making it active if
    /// none was.
    pub fn insert_sequence(&mut self, seq: Sequence) -> SequenceId {
        let id = seq.id;
        self.sequences.insert(id, seq);
        if !self.sequence_order.contains(&id) {
            self.sequence_order.push(id);
        }
        if self.active_sequence.is_none() {
            self.active_sequence = Some(id);
        }
        id
    }
}

impl Default for TimelineProject {
    fn default() -> Self {
        TimelineProject::new()
    }
}

/// Project-wide video settings (proxy prefs, cache limits, default rates).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectVideoSettings {
    /// Whether proxy media is generated on import.
    #[serde(default)]
    pub generate_proxies: bool,
    /// Sidecar cache budget in megabytes; `None` = unbounded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_limit_mb: Option<u64>,
    /// Default frame rate for a new sequence.
    #[serde(default = "default_frame_rate")]
    pub default_frame_rate: FrameRate,
    /// Export/mix audio sample rate (Hz).
    #[serde(default = "default_audio_rate")]
    pub audio_sample_rate: u32,
}

fn default_frame_rate() -> FrameRate {
    FrameRate::FPS_30
}
fn default_audio_rate() -> u32 {
    48_000
}

impl Default for ProjectVideoSettings {
    fn default() -> Self {
        ProjectVideoSettings {
            generate_proxies: false,
            cache_limit_mb: None,
            default_frame_rate: default_frame_rate(),
            audio_sample_rate: default_audio_rate(),
        }
    }
}

/// A sequence: a stack of tracks with a frame rate and one or more aspect
/// formats.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sequence {
    pub id: SequenceId,
    pub name: String,
    pub frame_rate: FrameRate,
    /// Multi-aspect (CAP-012): ≥1 entries.
    pub formats: Vec<SequenceFormat>,
    #[serde(default)]
    pub active_format: usize,
    /// Index 0 = bottom of the composite stack.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub video_tracks: Vec<Track>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audio_tracks: Vec<Track>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub caption_tracks: Vec<CaptionTrack>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<Marker>,
    #[serde(default)]
    pub audio_master: MasterBus,
    /// In/out for preview + export.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_range: Option<(Tick, Tick)>,
}

impl Sequence {
    /// A new sequence with one format and empty tracks.
    pub fn new(name: impl Into<String>, frame_rate: FrameRate, width: u32, height: u32) -> Self {
        Sequence {
            id: SequenceId::new(),
            name: name.into(),
            frame_rate,
            formats: vec![SequenceFormat::new("16:9", width, height)],
            active_format: 0,
            video_tracks: Vec::new(),
            audio_tracks: Vec::new(),
            caption_tracks: Vec::new(),
            markers: Vec::new(),
            audio_master: MasterBus::new(),
            work_range: None,
        }
    }

    /// The active format (clamped to a valid index).
    pub fn format(&self) -> &SequenceFormat {
        let i = self.active_format.min(self.formats.len().saturating_sub(1));
        &self.formats[i]
    }

    /// All video + audio tracks (captions handled separately).
    pub fn tracks(&self) -> impl Iterator<Item = &Track> {
        self.video_tracks.iter().chain(self.audio_tracks.iter())
    }

    /// Find a track by id across video and audio lanes.
    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.tracks().find(|t| t.id == id)
    }

    pub fn track_mut(&mut self, id: TrackId) -> Option<&mut Track> {
        self.video_tracks
            .iter_mut()
            .chain(self.audio_tracks.iter_mut())
            .find(|t| t.id == id)
    }

    /// A structural copy of this sequence with brand-new identity — a fresh
    /// [`SequenceId`] plus fresh ids for every track, clip, marker, caption
    /// track and cue — so the duplicate addresses cleanly alongside the
    /// original (the command layer resolves clips/tracks by id across *all*
    /// sequences and would otherwise hit the first match). Link groups are
    /// remapped to fresh ids so a linked pair stays linked *within* the copy
    /// without tying it back to the original. Referenced assets, composition
    /// graphs and nested sequences are shared (their ids are left untouched).
    /// The caller renames the copy (see `ops::duplicate_sequence`).
    pub fn duplicate_with_fresh_ids(&self) -> Sequence {
        use super::clip::LinkGroupId;
        let mut dup = self.clone();
        dup.id = SequenceId::new();
        let mut link_remap: HashMap<LinkGroupId, LinkGroupId> = HashMap::new();
        for track in dup
            .video_tracks
            .iter_mut()
            .chain(dup.audio_tracks.iter_mut())
        {
            track.id = TrackId::new();
            for clip in &mut track.clips {
                clip.id = ClipId::new();
                if let Some(old) = clip.link_group {
                    let fresh = *link_remap.entry(old).or_default();
                    clip.link_group = Some(fresh);
                }
            }
        }
        for marker in &mut dup.markers {
            marker.id = MarkerId::new();
        }
        for ct in &mut dup.caption_tracks {
            ct.id = TrackId::new();
            for cue in &mut ct.cues {
                cue.id = CueId::new();
            }
        }
        dup
    }

    /// The end tick of the last clip across all tracks (sequence content length).
    pub fn content_end(&self) -> Tick {
        self.tracks()
            .flat_map(|t| t.clips.iter())
            .map(|c| c.end())
            .max()
            .unwrap_or(Tick::ZERO)
    }

    /// Enforce the per-track invariants (01 §4): every clip has `duration > 0`,
    /// clips are sorted by `start`, and no two clips on one track overlap.
    /// Debug-asserted after every edit op and checked on load.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.formats.is_empty() {
            return Err(ValidationError::NoFormats(self.id));
        }
        for track in self.tracks() {
            let mut prev_end: Option<Tick> = None;
            for clip in &track.clips {
                if clip.duration.0 <= 0 {
                    return Err(ValidationError::NonPositiveDuration {
                        track: track.id,
                        clip_start: clip.start,
                    });
                }
                if let Some(pe) = prev_end {
                    if clip.start < pe {
                        return Err(ValidationError::OverlapOrUnsorted {
                            track: track.id,
                            at: clip.start,
                        });
                    }
                }
                prev_end = Some(clip.end());
            }
        }
        Ok(())
    }
}

/// A validation failure for a sequence's track invariants.
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationError {
    NoFormats(SequenceId),
    NonPositiveDuration { track: TrackId, clip_start: Tick },
    OverlapOrUnsorted { track: TrackId, at: Tick },
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidationError::NoFormats(s) => write!(f, "sequence {s} has no formats"),
            ValidationError::NonPositiveDuration { track, clip_start } => {
                write!(
                    f,
                    "clip at {clip_start:?} on track {track} has duration <= 0"
                )
            }
            ValidationError::OverlapOrUnsorted { track, at } => {
                write!(f, "clip at {at:?} on track {track} overlaps or is unsorted")
            }
        }
    }
}

impl std::error::Error for ValidationError {}

/// One aspect-ratio variant of a sequence (CAP-012).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SequenceFormat {
    pub name: String,
    pub width: u32,
    pub height: u32,
}

impl SequenceFormat {
    pub fn new(name: impl Into<String>, width: u32, height: u32) -> Self {
        SequenceFormat {
            name: name.into(),
            width,
            height,
        }
    }
}

/// A video or audio track. Clip vectors are the z/mix order — no separate order
/// table (01 §4).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub name: String,
    pub kind: TrackKind,
    /// Sorted by start, non-overlapping (invariant, enforced by edit ops).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clips: Vec<Clip>,
    /// Video: hidden; audio: muted.
    #[serde(default = "super::grade::default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub locked: bool,
    /// Sync lock (14 §M-9): when set, ripple/insert edits are meant to shift
    /// this track in lock-step with the other sync-locked tracks. This is the
    /// data + toggle half only; the ripple-propagation wiring across sync-locked
    /// tracks is a later GUI concern.
    #[serde(default)]
    pub sync_lock: bool,
    /// volume/pan/solo, fx chain, automation (09).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<TrackAudio>,
    /// UI-only but persisted (like existing panel prefs).
    #[serde(default = "default_track_height")]
    pub height_px: f32,
}

fn default_track_height() -> f32 {
    64.0
}

impl Track {
    pub fn new(kind: TrackKind, name: impl Into<String>) -> Self {
        Track {
            id: TrackId::new(),
            name: name.into(),
            kind,
            clips: Vec::new(),
            enabled: true,
            locked: false,
            sync_lock: false,
            audio: if kind == TrackKind::Audio {
                Some(TrackAudio::new())
            } else {
                None
            },
            height_px: default_track_height(),
        }
    }

    /// Insertion index that keeps `clips` sorted by `start`.
    pub fn insert_index_for(&self, start: Tick) -> usize {
        self.clips.partition_point(|c| c.start < start)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Video,
    Audio,
}

/// A sequence marker.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub id: MarkerId,
    pub at: Tick,
    pub name: String,
    #[serde(default)]
    pub color: Option<Color>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

impl Marker {
    pub fn new(at: Tick, name: impl Into<String>) -> Self {
        Marker {
            id: MarkerId::new(),
            at,
            name: name.into(),
            color: None,
            note: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::clip::{Clip, ClipSource};
    use super::*;

    fn vid_track_with(clips: Vec<(i64, i64)>) -> Track {
        let mut t = Track::new(TrackKind::Video, "V1");
        for (start, dur) in clips {
            t.clips
                .push(Clip::new(ClipSource::Adjustment, Tick(start), Tick(dur)));
        }
        t
    }

    fn seq_with(track: Track) -> Sequence {
        let mut s = Sequence::new("seq", FrameRate::FPS_30, 1920, 1080);
        s.video_tracks.push(track);
        s
    }

    #[test]
    fn validate_accepts_sorted_non_overlapping() {
        let s = seq_with(vid_track_with(vec![(0, 100), (100, 50), (200, 10)]));
        assert!(s.validate().is_ok());
    }

    #[test]
    fn validate_rejects_overlap() {
        let s = seq_with(vid_track_with(vec![(0, 100), (50, 100)]));
        assert!(matches!(
            s.validate(),
            Err(ValidationError::OverlapOrUnsorted { .. })
        ));
    }

    #[test]
    fn validate_rejects_zero_duration() {
        let s = seq_with(vid_track_with(vec![(0, 0)]));
        assert!(matches!(
            s.validate(),
            Err(ValidationError::NonPositiveDuration { .. })
        ));
    }

    #[test]
    fn content_end_is_max_clip_end() {
        let s = seq_with(vid_track_with(vec![(0, 100), (100, 50)]));
        assert_eq!(s.content_end(), Tick(150));
    }

    #[test]
    fn track_serde_round_trips_including_sync_lock() {
        let mut t = Track::new(TrackKind::Video, "V1");
        t.sync_lock = true;
        t.clips
            .push(Clip::new(ClipSource::Adjustment, Tick(0), Tick(100)));
        let json = serde_json::to_string(&t).unwrap();
        let back: Track = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
        assert!(back.sync_lock);
    }

    #[test]
    fn track_sync_lock_defaults_false_when_absent_in_json() {
        // A pre-sync-lock serialized track (no `sync_lock` key) still loads.
        let json = r#"{"id":"00000000-0000-0000-0000-000000000001","name":"V1","kind":"video"}"#;
        let t: Track = serde_json::from_str(json).unwrap();
        assert!(!t.sync_lock);
    }

    #[test]
    fn duplicate_with_fresh_ids_reassigns_all_structural_ids() {
        let mut s = seq_with(vid_track_with(vec![(0, 100), (100, 50)]));
        s.markers.push(Marker::new(Tick(10), "m"));
        let dup = s.duplicate_with_fresh_ids();

        // Fresh identity everywhere so the copy addresses cleanly alongside
        // the original.
        assert_ne!(dup.id, s.id);
        assert_ne!(dup.video_tracks[0].id, s.video_tracks[0].id);
        for (a, b) in dup.video_tracks[0]
            .clips
            .iter()
            .zip(&s.video_tracks[0].clips)
        {
            assert_ne!(a.id, b.id);
        }
        assert_ne!(dup.markers[0].id, s.markers[0].id);
        // Content (timing) is otherwise identical.
        let spans = |seq: &Sequence| {
            seq.video_tracks[0]
                .clips
                .iter()
                .map(|c| (c.start, c.duration))
                .collect::<Vec<_>>()
        };
        assert_eq!(spans(&dup), spans(&s));
    }

    #[test]
    fn project_insert_sequence_sets_active_and_order() {
        let mut p = TimelineProject::new();
        let id = p.insert_sequence(Sequence::new("s", FrameRate::FPS_30, 1920, 1080));
        assert_eq!(p.active_sequence, Some(id));
        assert_eq!(p.sequence_order, vec![id]);
    }
}
