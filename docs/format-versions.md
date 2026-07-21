# `.photonic` Format Version Changelog

The `.photonic` file is JSON with a top-level `format_version` integer. On open,
the loader migrates the raw JSON forward to the current version through an
ordered chain of migrations (`crates/photonic-core/src/migration.rs`) before
deserializing into `Document`.

## Policy

- **Bump `CURRENT_FORMAT_VERSION`** (`crates/photonic-core/src/document.rs`) on
  every structural change or field addition that an older build could not
  otherwise interpret.
- **Add a migration** for each bump: implement `FormatMigration` (from N → N+1)
  and append it to `migration::migrations()`. Migrations operate on the raw
  `serde_json::Value`, so they add new fields with defaults or rename moved
  fields without depending on the in-memory types.
- **Add a changelog entry** below for each version.
- **Newer files, unknown struct _fields_**: a document saved by a newer build
  loads leniently — unknown object fields are dropped (serde's default) — while
  within `migration::COMPAT_WINDOW` versions ahead; beyond that window the loader
  refuses it.
- **Newer files, unknown enum _variants_**: a new value in an open-ended
  persisted enum is *preserved*, not dropped (39 §2.2). The unknown variant's
  serialized form is retained verbatim and re-emitted unchanged on save, so a
  round-trip through an older build is lossless. The open-ended enums are
  `EffectKind`, `TransitionKind`, `AudioFxKind`, `GradeOpKind`, `ClipSource`,
  `GraphOp`, and `GradeOpParams`; each has an `Unknown` fallback. An unknown
  effect renders passthrough, an unknown transition renders as a cut, and an
  unknown source shows a placeholder frame — inert, never guessed.
- **New enum variants do NOT require a `CURRENT_FORMAT_VERSION` bump**: because
  any build with the `Unknown` fallback can read (and losslessly re-write) a file
  that uses a newer variant of one of the open-ended enums above, adding such a
  variant is not by itself a format-breaking change. (A new *field* on a struct,
  or a change an older build could not round-trip, still needs a bump.)
- **Downgrade** (saving as an older version) is unsupported.

## Versions

### v4 — current

Added an explicit `anchor_space` to video `ClipTransform`. Newly authored
transforms use center-relative anchor offsets. The v3→v4 migration preserves
legacy rendering by tagging every base and per-format reframe transform as
`absolute` without changing anchor numbers or animation keyframes.

Forward-compatible enum variants (39 §2.2) landed within v4 and deliberately did
**not** bump the version or add a migration: they only widen what the loader
accepts (unknown variants of the open-ended enums are now preserved instead of
rejected) and do not change what current builds emit for known data. Do not add
a spurious v4→v5 no-op migration for it.

### v3

Added the video-editor `timeline` field on `Document`
(`Option<timeline::TimelineProject>`, docs/specs/video-editor/01-data-model.md
§2) — the media pool, sequences/tracks/clips, node graphs, grades, captions, and
audio mixer for the video editor. The change is purely additive: `timeline` is
`Option` + `#[serde(default)]` + `skip_serializing_if = "Option::is_none"`, so
v2 documents (which have no timeline) load unchanged and v3 documents without
video features omit the key entirely. The v2→v3 migration is a no-op version
bump (`migration::V2ToV3`).

### v2

Added the `Raster` scene-node kind (`SceneNodeKind::Raster`) for Photoshop-style
pixel layers — see [`raster-editing.md`](raster-editing.md). The change is purely
additive: v1 documents contain no raster nodes and load unchanged, so the v1→v2
migration is a no-op version bump. A raster node serializes its pixels as a
base64 PNG (`{ width, height, png }`) plus an optional layer `mask` and
`source_uri`.

### v1

Initial versioned format. Documents that predate the `format_version` field are
treated as v1.
