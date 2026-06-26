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

### v1 — current

Initial versioned format. Documents that predate the `format_version` field are
treated as v1.
