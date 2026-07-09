//! Load-time timeline finalization (01 §4 invariant enforcement, §6.2 orphaned
//! paths).
//!
//! `Document` derives `Deserialize`, so raw serde cannot run the timeline
//! invariants. Instead [`Document::from_value`](crate::document::Document::from_value)
//! — the single load seam shared by `from_json` and the `.photon` wrapper —
//! calls [`finalize_load`] on the deserialized project. Two passes run there:
//!
//! 1. **Orphan flagging (repair).** Every [`PropertyTrack`] whose
//!    [`PropPath`](super::anim::PropPath) does not resolve in
//!    [`prop_registry`](super::prop_registry) for its owning target kind is
//!    flagged [`orphaned`](super::anim::PropertyTrack::orphaned) rather than
//!    dropped (01 §6.2): an asset/plugin may register the path later, and eval
//!    falls back to `base`. This is idempotent and re-derived on every load, so a
//!    path that becomes known un-orphans automatically.
//!
//! 2. **Sequence validation (reject).** Each loaded [`Sequence`] must satisfy
//!    [`Sequence::validate`] (sorted, non-overlapping, `duration > 0`). A file
//!    whose on-disk clips overlap is *rejected* with a load error rather than
//!    silently repaired: closing a gap requires an editorial decision (which clip
//!    moves, by how much) that the loader cannot make without risking data loss,
//!    so failing loudly is safer than guessing.
//!
//! The effect/grade/audio-fx param bags all declare
//! [`PropSet::TARGET_KIND`](super::anim::PropSet::TARGET_KIND) `= GraphNode`
//! (their generic surface is intentionally lenient), so orphan resolution here
//! keys off the *concrete* kind field (`EffectKind`, `GradeOpKind`, `AudioFxKind`)
//! rather than the generic bound — that is where the real registry blocks live.

use super::anim::PropertyTrack;
use super::prop_registry::{self, PropTargetKind};
use super::sequence::{TimelineProject, ValidationError};

/// Flag every track in `tracks` orphaned iff its path is unknown for `kind`.
/// Re-derived from scratch each call (a previously-orphaned path that is now
/// registered clears its flag, and vice versa).
fn flag_tracks(tracks: &mut [PropertyTrack], kind: PropTargetKind) {
    for t in tracks {
        t.orphaned = prop_registry::resolve(kind, t.property.as_str()).is_none();
    }
}

/// Run the load-time passes over a freshly deserialized project: flag orphaned
/// property tracks (repair) and validate every sequence (reject on violation).
pub fn finalize_load(project: &mut TimelineProject) -> Result<(), ValidationError> {
    flag_orphaned_property_tracks(project);
    for seq in project.sequences.values() {
        seq.validate()?;
    }
    Ok(())
}

/// The orphan-flagging pass in isolation (see module docs). Walks every
/// animatable `AnimProps` lane the project owns and flags unresolved paths.
pub fn flag_orphaned_property_tracks(project: &mut TimelineProject) {
    for seq in project.sequences.values_mut() {
        for track in seq
            .video_tracks
            .iter_mut()
            .chain(seq.audio_tracks.iter_mut())
        {
            if let Some(audio) = track.audio.as_mut() {
                flag_tracks(&mut audio.params.tracks, PropTargetKind::TrackAudioParams);
                for unit in &mut audio.fx_chain {
                    flag_tracks(&mut unit.params.tracks, PropTargetKind::AudioFx(unit.kind));
                }
            }
            for clip in &mut track.clips {
                flag_tracks(&mut clip.transform.tracks, PropTargetKind::ClipTransform);
                for effect in &mut clip.effects {
                    flag_tracks(
                        &mut effect.params.tracks,
                        PropTargetKind::Effect(effect.kind),
                    );
                }
                if let Some(grade) = clip.grade.as_mut() {
                    for op in &mut grade.ops {
                        flag_tracks(&mut op.params.tracks, PropTargetKind::GradeOp(op.kind));
                    }
                }
                if let Some(audio) = clip.audio.as_mut() {
                    flag_tracks(&mut audio.params.tracks, PropTargetKind::ClipAudioParams);
                }
            }
        }
        // Master bus params + its fx chain.
        flag_tracks(
            &mut seq.audio_master.params.tracks,
            PropTargetKind::MasterBusParams,
        );
        for unit in &mut seq.audio_master.fx_chain {
            flag_tracks(&mut unit.params.tracks, PropTargetKind::AudioFx(unit.kind));
        }
    }
    // Graph-node params resolve leniently (`PropTargetKind::GraphNode` accepts
    // any path), so they are never orphaned — no pass needed for `project.graphs`.
}
