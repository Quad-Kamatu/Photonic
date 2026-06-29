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
- **Newer files**: a document saved by a newer build loads leniently (unknown
  fields dropped) while within `migration::COMPAT_WINDOW` versions ahead; beyond
  that window the loader refuses it.
- **Downgrade** (saving as an older version) is unsupported.

## Versions

### v2 — current

Added the `Raster` scene-node kind (`SceneNodeKind::Raster`) for Photoshop-style
pixel layers — see [`raster-editing.md`](raster-editing.md). The change is purely
additive: v1 documents contain no raster nodes and load unchanged, so the v1→v2
migration is a no-op version bump. A raster node serializes its pixels as a
base64 PNG (`{ width, height, png }`) plus an optional layer `mask` and
`source_uri`.

### v1

Initial versioned format. Documents that predate the `format_version` field are
treated as v1.
