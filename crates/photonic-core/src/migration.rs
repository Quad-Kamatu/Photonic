//! `.photonic` file-format versioning and forward migration.
//!
//! Documents carry a `format_version` (see [`crate::document::CURRENT_FORMAT_VERSION`]).
//! Rather than deserialize straight into [`Document`](crate::document::Document) and
//! reject anything unexpected, the loader first migrates the raw JSON
//! [`serde_json::Value`] up to the current version through an ordered chain of
//! [`FormatMigration`] steps, then deserializes. This lets older documents open
//! cleanly after the model grows, and lets slightly-newer documents load
//! leniently (unknown fields dropped) within a compatibility window.

use serde_json::Value;

/// How many versions ahead of the current one a file may be and still load
/// (with unknown fields dropped) before the loader refuses it outright.
pub const COMPAT_WINDOW: u32 = 1;

/// An error raised while migrating a document forward.
#[derive(Debug, Clone)]
pub enum MigrationError {
    /// A migration step failed.
    Failed { from: u32, to: u32, reason: String },
    /// The file is too far ahead of this build to load safely.
    TooNew { file: u32, supported: u32 },
}

impl std::fmt::Display for MigrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MigrationError::Failed { from, to, reason } => {
                write!(f, "format migration {from}→{to} failed: {reason}")
            }
            MigrationError::TooNew { file, supported } => write!(
                f,
                "unsupported format version {file} (this build supports up to {supported})"
            ),
        }
    }
}

impl std::error::Error for MigrationError {}

/// Upgrade a raw document [`Value`] from one format version to the next.
///
/// Implementations operate on the JSON tree directly (adding new fields with
/// defaults, renaming moved fields, etc.) before struct deserialization, so a
/// migration never has to know about the in-memory types.
pub trait FormatMigration: Send + Sync {
    /// The version this migration upgrades *from*.
    fn from_version(&self) -> u32;
    /// The version this migration upgrades *to* (must be `from_version() + 1`).
    fn to_version(&self) -> u32;
    /// Mutate `value` in place to the target version.
    fn migrate(&self, value: &mut Value) -> Result<(), String>;
}

/// The ordered migration chain. Each entry upgrades version N → N+1.
pub fn migrations() -> Vec<Box<dyn FormatMigration>> {
    vec![Box::new(V1ToV2), Box::new(V2ToV3), Box::new(V3ToV4)]
}

/// v1 → v2: the `Raster` node kind was added. The change is purely additive —
/// existing v1 documents contain no raster nodes — so this only stamps the new
/// version number; serde defaults supply any missing fields on load.
struct V1ToV2;
impl FormatMigration for V1ToV2 {
    fn from_version(&self) -> u32 {
        1
    }
    fn to_version(&self) -> u32 {
        2
    }
    fn migrate(&self, _value: &mut Value) -> Result<(), String> {
        Ok(())
    }
}

/// v2 → v3: introduced the video-editor `timeline` field (01 §2). Purely
/// additive — `timeline` is `Option` + `#[serde(default)]`, so existing v2
/// documents contain no timeline and load unchanged; this only stamps the new
/// version number.
struct V2ToV3;
impl FormatMigration for V2ToV3 {
    fn from_version(&self) -> u32 {
        2
    }
    fn to_version(&self) -> u32 {
        3
    }
    fn migrate(&self, _value: &mut Value) -> Result<(), String> {
        Ok(())
    }
}

/// v3 → v4: clip anchors became explicitly center-relative. Existing v3
/// values were authored as absolute output-frame pixels, so preserve their
/// numeric values and tag both base and per-format reframe transforms.
struct V3ToV4;
impl FormatMigration for V3ToV4 {
    fn from_version(&self) -> u32 {
        3
    }
    fn to_version(&self) -> u32 {
        4
    }
    fn migrate(&self, value: &mut Value) -> Result<(), String> {
        let root = value.as_object_mut().ok_or("document is not an object")?;
        let Some(sequences) = root
            .get_mut("timeline")
            .and_then(Value::as_object_mut)
            .and_then(|timeline| timeline.get_mut("sequences"))
            .and_then(Value::as_object_mut)
        else {
            return Ok(());
        };

        for sequence in sequences.values_mut().filter_map(Value::as_object_mut) {
            for track_key in ["video_tracks", "audio_tracks"] {
                let Some(tracks) = sequence.get_mut(track_key).and_then(Value::as_array_mut) else {
                    continue;
                };
                for clip in tracks
                    .iter_mut()
                    .filter_map(Value::as_object_mut)
                    .filter_map(|track| track.get_mut("clips"))
                    .filter_map(Value::as_array_mut)
                    .flatten()
                    .filter_map(Value::as_object_mut)
                {
                    if let Some(base) = clip
                        .get_mut("transform")
                        .and_then(Value::as_object_mut)
                        .and_then(|transform| transform.get_mut("base"))
                        .and_then(Value::as_object_mut)
                    {
                        base.entry("anchor_space")
                            .or_insert_with(|| Value::from("absolute"));
                    }
                    if let Some(reframes) = clip.get_mut("reframe").and_then(Value::as_object_mut) {
                        for transform in reframes.values_mut().filter_map(Value::as_object_mut) {
                            transform
                                .entry("anchor_space")
                                .or_insert_with(|| Value::from("absolute"));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// Read `format_version` from a raw document value, defaulting to 1 when absent
/// (documents predating the field).
pub fn detect_version(value: &Value) -> u32 {
    value
        .get("format_version")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(1)
}

/// Apply `chain` to bring `value` up to `target`, returning the resulting
/// version. Stops early (without error) once no migration advances further —
/// remaining gaps are filled by serde defaults at deserialization time.
pub fn run_migrations_with(
    value: &mut Value,
    chain: &[Box<dyn FormatMigration>],
    target: u32,
) -> Result<u32, MigrationError> {
    let mut version = detect_version(value);
    while version < target {
        let Some(m) = chain.iter().find(|m| m.from_version() == version) else {
            break;
        };
        m.migrate(value).map_err(|reason| MigrationError::Failed {
            from: m.from_version(),
            to: m.to_version(),
            reason,
        })?;
        version = m.to_version();
        if let Some(obj) = value.as_object_mut() {
            obj.insert("format_version".into(), Value::from(version));
        }
    }
    Ok(version)
}

/// Apply the built-in [`migrations`] chain up to `target`.
pub fn run_migrations(value: &mut Value, target: u32) -> Result<u32, MigrationError> {
    run_migrations_with(value, &migrations(), target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct AddField {
        from: u32,
        key: &'static str,
    }
    impl FormatMigration for AddField {
        fn from_version(&self) -> u32 {
            self.from
        }
        fn to_version(&self) -> u32 {
            self.from + 1
        }
        fn migrate(&self, value: &mut Value) -> Result<(), String> {
            value
                .as_object_mut()
                .ok_or("not an object")?
                .insert(self.key.into(), json!(true));
            Ok(())
        }
    }

    #[test]
    fn detect_version_defaults_to_one() {
        assert_eq!(detect_version(&json!({})), 1);
        assert_eq!(detect_version(&json!({"format_version": 3})), 3);
    }

    #[test]
    fn v3_to_v4_tags_all_clip_transforms_without_changing_values() {
        let clip = |anchor_x, keyed| {
            json!({
                "transform": {
                    "base": { "anchor_x": anchor_x, "anchor_y": -7.0 },
                    "tracks": [{ "path": "transform.anchor_x", "keys": keyed }]
                },
                "reframe": {
                    "0": { "anchor_x": anchor_x + 1.0, "anchor_y": 9.0 }
                }
            })
        };
        let keys = json!([{ "time": 12, "value": { "t": "float", "v": 42.0 } }]);
        let mut value = json!({
            "format_version": 3,
            "timeline": { "sequences": {
                "outer": {
                    "video_tracks": [{ "clips": [clip(10.0, keys.clone())] }],
                    "audio_tracks": [{ "clips": [clip(20.0, keys.clone())] }]
                },
                "nested": {
                    "video_tracks": [{ "clips": [clip(30.0, keys.clone())] }]
                }
            }}
        });

        let before_keys = value["timeline"]["sequences"]["outer"]["video_tracks"][0]["clips"][0]
            ["transform"]["tracks"]
            .clone();
        assert_eq!(run_migrations(&mut value, 4).unwrap(), 4);

        for (sequence, track, expected) in [
            ("outer", "video_tracks", 10.0),
            ("outer", "audio_tracks", 20.0),
            ("nested", "video_tracks", 30.0),
        ] {
            let clip = &value["timeline"]["sequences"][sequence][track][0]["clips"][0];
            assert_eq!(clip["transform"]["base"]["anchor_space"], "absolute");
            assert_eq!(clip["transform"]["base"]["anchor_x"], expected);
            assert_eq!(clip["reframe"]["0"]["anchor_space"], "absolute");
            assert_eq!(clip["reframe"]["0"]["anchor_x"], expected + 1.0);
        }
        assert_eq!(
            value["timeline"]["sequences"]["outer"]["video_tracks"][0]["clips"][0]["transform"]
                ["tracks"],
            before_keys
        );
    }

    #[test]
    fn v3_to_v4_preserves_explicit_anchor_space_and_tolerates_optional_shapes() {
        let mut value = json!({
            "format_version": 3,
            "timeline": { "sequences": {
                "valid": { "video_tracks": [{ "clips": [{
                    "transform": { "base": { "anchor_space": "center_offset" } },
                    "reframe": { "0": { "anchor_space": "center_offset" } }
                }]}]},
                "malformed_optional": { "video_tracks": "not-an-array" }
            }}
        });
        assert_eq!(run_migrations(&mut value, 4).unwrap(), 4);
        let clip = &value["timeline"]["sequences"]["valid"]["video_tracks"][0]["clips"][0];
        assert_eq!(clip["transform"]["base"]["anchor_space"], "center_offset");
        assert_eq!(clip["reframe"]["0"]["anchor_space"], "center_offset");
    }

    #[test]
    fn chain_applies_in_order_and_bumps_version() {
        let chain: Vec<Box<dyn FormatMigration>> = vec![
            Box::new(AddField { from: 1, key: "a" }),
            Box::new(AddField { from: 2, key: "b" }),
        ];
        let mut v = json!({ "format_version": 1 });
        let out = run_migrations_with(&mut v, &chain, 3).unwrap();
        assert_eq!(out, 3);
        assert_eq!(v["format_version"], 3);
        assert_eq!(v["a"], json!(true));
        assert_eq!(v["b"], json!(true));
    }

    #[test]
    fn stops_when_no_migration_advances() {
        // Chain can only reach v2; target is v3. Should stop at v2, no error.
        let chain: Vec<Box<dyn FormatMigration>> = vec![Box::new(AddField { from: 1, key: "a" })];
        let mut v = json!({ "format_version": 1 });
        let out = run_migrations_with(&mut v, &chain, 3).unwrap();
        assert_eq!(out, 2);
    }

    #[test]
    fn failing_migration_reports_error() {
        struct Boom;
        impl FormatMigration for Boom {
            fn from_version(&self) -> u32 {
                1
            }
            fn to_version(&self) -> u32 {
                2
            }
            fn migrate(&self, _v: &mut Value) -> Result<(), String> {
                Err("kaboom".into())
            }
        }
        let chain: Vec<Box<dyn FormatMigration>> = vec![Box::new(Boom)];
        let mut v = json!({ "format_version": 1 });
        let err = run_migrations_with(&mut v, &chain, 2).unwrap_err();
        assert!(matches!(err, MigrationError::Failed { from: 1, to: 2, .. }));
    }

    #[test]
    fn current_chain_is_a_noop_for_v1() {
        let mut v = json!({ "format_version": 1 });
        assert_eq!(run_migrations(&mut v, 1).unwrap(), 1);
    }
}
