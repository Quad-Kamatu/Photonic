//! Media pool (01 §3): assets referenced, never embedded.
//!
//! Contrast with `RasterImage`'s base64-PNG embedding — video files are orders
//! of magnitude too large to inline. Offline/missing media is a first-class
//! state: an asset with no reachable file renders a placeholder; relink matches
//! `content_hash` first, then filename (§9). This module is also the canonical
//! home for `VectorRef`/`VectorStateKey` (relocated from `photonic-video`'s
//! `contract.rs`, which now re-exports them).

use super::clip::ClipEffect;
use super::grade::Grade;
use super::ids::{AssetId, BinId};
use super::time::{FrameRate, Tick};
use crate::node::NodeId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// The asset pool plus its folder (bin) hierarchy.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct MediaPool {
    pub assets: HashMap<AssetId, MediaAsset>,
    /// Folders; a flat list with parent refs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bins: Vec<MediaBin>,
}

impl MediaPool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, asset: MediaAsset) -> AssetId {
        let id = asset.id;
        self.assets.insert(id, asset);
        id
    }
}

/// A media-pool asset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MediaAsset {
    pub id: AssetId,
    pub kind: AssetKind,
    pub source: AssetSource,
    /// Filled by the engine after ffprobe; cached in-file (small, needed for
    /// offline layout).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe: Option<MediaProbe>,
    /// Engine-managed proxy (path + status).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxyRef>,
    /// xxh3 of file head+tail+len — the relink identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// Containing bin (folder), or `None` for the pool root/unfiled (01 §3).
    /// Additive field: v3 files written before bins load with this absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin: Option<BinId>,
    /// Asset-level effect stack (35 §2): applied in the asset's source colour
    /// space beneath every clip that references it. Empty = neutral (§2.6).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<ClipEffect>,
    /// Asset-level grade (35 §2): applied after `effects`, still in source space.
    /// `None` = neutral.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grade: Option<Grade>,
}

impl MediaAsset {
    pub fn new(kind: AssetKind, source: AssetSource) -> Self {
        MediaAsset {
            id: AssetId::new(),
            kind,
            source,
            probe: None,
            proxy: None,
            content_hash: None,
            bin: None,
            effects: Vec::new(),
            grade: None,
        }
    }

    /// A file-backed asset from an absolute path.
    pub fn from_file(kind: AssetKind, path: impl Into<PathBuf>) -> Self {
        MediaAsset::new(
            kind,
            AssetSource::File {
                path: path.into(),
                rel_path: None,
            },
        )
    }
}

/// Asset media kind. `VectorDoc` is this document or an external `.photon`/`.svg`;
/// `Lut3d` is a `.cube` file, referenced not embedded (07 §1) — gets the same
/// offline/relink handling as media.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Video,
    Audio,
    Image,
    VectorDoc,
    Lut3d,
}

/// Where an asset's pixels/samples come from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum AssetSource {
    /// Absolute path plus an optional project-relative fallback. The loader
    /// tries `rel_path` first (project moves survive), then `path`, then
    /// relink-by-hash (§9).
    File {
        path: PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rel_path: Option<PathBuf>,
    },
    /// Lives inside this `Document` (artboard or node subtree).
    EmbeddedVector { root: VectorRef },
}

/// Reference to vector content inside (or associated with) the document (01 §3).
/// Canonical home; `photonic-video::contract` re-exports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "vref", rename_all = "snake_case")]
pub enum VectorRef {
    Artboard(usize),
    Node(NodeId),
    WholeDocument,
}

/// Cache key for a rasterized vector frame: a hash combining the referenced
/// nodes' state, evaluated animated props, and output size (02 §3, 03 §2.5).
/// Relocated from `contract.rs`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VectorStateKey(pub u128);

/// The subset of ffprobe output we persist in-file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MediaProbe {
    pub duration: Tick,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<VideoStreamInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioStreamInfo>,
    pub container: String,
    pub codec: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VideoStreamInfo {
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    #[serde(default = "default_pixel_aspect")]
    pub pixel_aspect: f32,
    #[serde(default)]
    pub color: ProbedColor,
    /// Whether a keyframe (GOP) index has been built and cached in the sidecar.
    #[serde(default)]
    pub keyframe_index_cached: bool,
}

fn default_pixel_aspect() -> f32 {
    1.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioStreamInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub codec: String,
}

/// Probed color characteristics (primaries/transfer/range), as reported by
/// ffprobe. Strings kept verbatim; the render pipeline (03) interprets them.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct ProbedColor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primaries: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matrix: Option<String>,
    /// `true` = full/PC range, `false` = limited/TV range, `None` = unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_range: Option<bool>,
}

/// How a proxy file entered the pool (G-15A).
///
/// `Generated` is the serde default so projects written before origin existed
/// load as engine-managed cache files. `Attached` marks user-owned files that
/// must never be deleted on detach.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProxyOrigin {
    #[default]
    Generated,
    Attached,
}

/// Engine-managed proxy media reference.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProxyRef {
    pub path: PathBuf,
    pub status: ProxyStatus,
    /// Provenance; defaults to [`ProxyOrigin::Generated`] for old project JSON.
    #[serde(default)]
    pub origin: ProxyOrigin,
}

impl ProxyRef {
    /// Ready proxy produced by the engine (cache-owned path).
    pub fn ready_generated(path: impl Into<PathBuf>) -> Self {
        ProxyRef {
            path: path.into(),
            status: ProxyStatus::Ready,
            origin: ProxyOrigin::Generated,
        }
    }

    /// Ready proxy linked from a user-supplied file (G-15A attach).
    pub fn ready_attached(path: impl Into<PathBuf>) -> Self {
        ProxyRef {
            path: path.into(),
            status: ProxyStatus::Ready,
            origin: ProxyOrigin::Attached,
        }
    }

    /// Pending/Failed intermediate for engine generation jobs.
    pub fn with_status(path: impl Into<PathBuf>, status: ProxyStatus) -> Self {
        ProxyRef {
            path: path.into(),
            status,
            origin: ProxyOrigin::Generated,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyStatus {
    Pending,
    Ready,
    Failed,
}

/// A media bin (folder). Flat list with parent refs (01 §3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MediaBin {
    pub id: BinId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<BinId>,
}

impl MediaBin {
    pub fn new(name: impl Into<String>, parent: Option<BinId>) -> Self {
        MediaBin {
            id: BinId::new(),
            name: name.into(),
            parent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_pool_insert_and_roundtrip() {
        let mut pool = MediaPool::new();
        let id = pool.insert(MediaAsset::from_file(AssetKind::Video, "/tmp/clip.mp4"));
        assert!(pool.assets.contains_key(&id));
        let j = serde_json::to_string(&pool).unwrap();
        let back: MediaPool = serde_json::from_str(&j).unwrap();
        assert_eq!(pool, back);
    }

    #[test]
    fn asset_effects_absent_from_json_when_empty() {
        // Additive discipline (35 §2 / §2.6): an asset with no effect/grade scope
        // omits both keys, so pre-scope media loads shape-identical.
        let a = MediaAsset::from_file(AssetKind::Video, "/tmp/clip.mp4");
        assert!(a.effects.is_empty());
        assert!(a.grade.is_none());
        let json = serde_json::to_string(&a).unwrap();
        assert!(!json.contains("effects"));
        assert!(!json.contains("grade"));
        // A pre-scope asset JSON (no keys) still loads with neutral defaults.
        let back: MediaAsset = serde_json::from_str(&json).unwrap();
        assert!(back.effects.is_empty());
        assert!(back.grade.is_none());
    }

    #[test]
    fn lut3d_is_a_media_asset_kind() {
        let a = MediaAsset::from_file(AssetKind::Lut3d, "/luts/kodak.cube");
        assert_eq!(a.kind, AssetKind::Lut3d);
    }

    #[test]
    fn asset_source_serde_tags() {
        let f = AssetSource::File {
            path: "/a/b.mp4".into(),
            rel_path: Some("b.mp4".into()),
        };
        let j = serde_json::to_string(&f).unwrap();
        assert!(j.contains("\"source\":\"file\""));
        let back: AssetSource = serde_json::from_str(&j).unwrap();
        assert_eq!(f, back);
    }

    /// G-15A: projects written before `origin` existed deserialize as Generated.
    #[test]
    fn proxy_ref_serde_defaults_origin_to_generated() {
        let old = r#"{"path":"/cache/abc.proxy.mp4","status":"ready"}"#;
        let pref: ProxyRef = serde_json::from_str(old).unwrap();
        assert_eq!(pref.status, ProxyStatus::Ready);
        assert_eq!(pref.origin, ProxyOrigin::Generated);
        assert_eq!(pref.path, PathBuf::from("/cache/abc.proxy.mp4"));
    }

    #[test]
    fn proxy_ref_attached_roundtrips() {
        let pref = ProxyRef::ready_attached("/user/cam_proxy.mp4");
        let j = serde_json::to_string(&pref).unwrap();
        let back: ProxyRef = serde_json::from_str(&j).unwrap();
        assert_eq!(pref, back);
        assert_eq!(back.origin, ProxyOrigin::Attached);
    }
}
