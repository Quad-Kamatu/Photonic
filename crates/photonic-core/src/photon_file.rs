//! `.photon` file container.
//!
//! Historically a `.photon` file was a `Document` serialized as JSON. To let a
//! project's full edit history (undo/redo, named checkpoints, branches) travel
//! with the file, the history is now written **alongside** the document as two
//! extra sibling keys rather than wrapping it:
//!
//! ```json
//! { "<all the document fields…>": …, "photon_format": 1, "photon_history": { … } }
//! ```
//!
//! The document stays the top-level object, which buys real backward AND forward
//! compatibility:
//! - **Older Photonic builds** (which read a `.photon` as a bare `Document`) open
//!   new-format files unchanged — `serde` ignores the two unknown keys. Saving
//!   is therefore *not* a one-way door.
//! - **History is best-effort and version-gated**: it is restored only when the
//!   document's own `format_version` matches this build's. A file written by an
//!   older/newer schema keeps opening (its document migrates as usual) but its
//!   embedded history is dropped rather than risk loading un-migrated nested
//!   documents. A malformed history payload is likewise dropped, never fatal.

use serde::Serialize;

use crate::document::{Document, CURRENT_FORMAT_VERSION};
use crate::history::HistorySnapshot;

/// Version of the `.photon` history-container format (independent of the
/// document's own `format_version`). Bump only on breaking changes to how the
/// `photon_history` payload is laid out.
pub const PHOTON_FORMAT_VERSION: u32 = 1;

/// Serialize a document plus its history snapshot to a pretty-printed `.photon`
/// JSON string. Pass `None` for `history` to write a document-only file (the
/// `photon_history` key is then omitted entirely).
pub fn save_photon(
    document: &Document,
    history: Option<&HistorySnapshot>,
) -> Result<String, serde_json::Error> {
    // Flatten the document's fields to the top level and append our two sibling
    // keys. Borrowing avoids cloning the (potentially large) document/history.
    #[derive(Serialize)]
    struct PhotonOut<'a> {
        #[serde(flatten)]
        document: &'a Document,
        photon_format: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        photon_history: Option<&'a HistorySnapshot>,
    }
    serde_json::to_string_pretty(&PhotonOut {
        document,
        photon_format: PHOTON_FORMAT_VERSION,
        photon_history: history,
    })
}

/// Parse a `.photon` JSON string into a migrated [`Document`] and, when present
/// and schema-compatible, its [`HistorySnapshot`].
///
/// Accepts both the new format (document fields + `photon_history`) and legacy
/// bare-`Document` files. A document that fails to parse is a hard error; the
/// history is always best-effort — a missing, malformed, or schema-mismatched
/// payload yields `None` history while the document still opens.
pub fn load_photon(json: &str) -> Result<(Document, Option<HistorySnapshot>), serde_json::Error> {
    let mut value: serde_json::Value = serde_json::from_str(json)?;

    // Lift the two sibling keys out before the value is consumed by document
    // migration. (Document deserialization ignores unknown keys regardless, but
    // removing them keeps the migrated tree clean.)
    let (history_value, photon_format) = match value.as_object_mut() {
        Some(obj) => (
            obj.remove("photon_history"),
            obj.remove("photon_format").and_then(|v| v.as_u64()),
        ),
        None => (None, None),
    };

    // The document's own format version, read BEFORE migration. The embedded
    // history was written against this same version, so it is only safe to
    // restore when that version matches what this build serializes today.
    let doc_version = crate::migration::detect_version(&value);

    let document = Document::from_value(value)?;

    let history = if doc_version == CURRENT_FORMAT_VERSION
        && photon_format.map_or(true, |f| f <= PHOTON_FORMAT_VERSION as u64)
    {
        history_value
            .and_then(|h| match h {
                serde_json::Value::Null => None,
                v => serde_json::from_value::<HistorySnapshot>(v).ok(),
            })
            .map(|mut s| {
                s.normalize_nested();
                s
            })
    } else {
        None
    };

    Ok((document, history))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_doc() -> Document {
        Document::new("doc", 64.0, 64.0)
    }

    #[test]
    fn legacy_bare_document_still_loads() {
        // A pre-history file is just the document's own JSON.
        let bare = sample_doc().to_json().unwrap();
        let (doc, history) = load_photon(&bare).unwrap();
        assert_eq!(doc.name, "doc");
        assert!(history.is_none(), "legacy file must yield no history");
    }

    #[test]
    fn new_format_round_trips_document_and_history() {
        let doc = sample_doc();
        let mut snap = HistorySnapshot::default();
        snap.branches.insert("main".into(), sample_doc());
        let json = save_photon(&doc, Some(&snap)).unwrap();

        // New-format files carry the two sibling keys…
        assert!(json.contains("photon_format"));
        assert!(json.contains("photon_history"));
        // …with the document still flattened at the top level (back-compat).
        assert!(json.contains("\"name\""));

        let (rdoc, rhist) = load_photon(&json).unwrap();
        assert_eq!(rdoc.name, "doc");
        let rhist = rhist.expect("history should round-trip at the current version");
        assert!(rhist.branches.contains_key("main"));
    }

    #[test]
    fn new_format_opens_as_bare_document_in_old_loader() {
        // Simulates an older build / the thumbnailer: from_json must still parse
        // a new-format file, ignoring the extra keys.
        let json = save_photon(&sample_doc(), Some(&HistorySnapshot::default())).unwrap();
        let doc = Document::from_json(&json).expect("old loader must accept new format");
        assert_eq!(doc.name, "doc");
    }

    #[test]
    fn document_only_save_omits_history_key() {
        let json = save_photon(&sample_doc(), None).unwrap();
        assert!(
            !json.contains("photon_history"),
            "None history must be omitted"
        );
        let (_, hist) = load_photon(&json).unwrap();
        assert!(hist.is_none());
    }

    #[test]
    fn malformed_history_degrades_but_document_opens() {
        let mut v = serde_json::to_value(sample_doc()).unwrap();
        let obj = v.as_object_mut().unwrap();
        obj.insert(
            "photon_format".into(),
            serde_json::json!(PHOTON_FORMAT_VERSION),
        );
        obj.insert(
            "photon_history".into(),
            serde_json::json!("not-a-history-object"),
        );
        let (doc, hist) = load_photon(&serde_json::to_string(&v).unwrap()).unwrap();
        assert_eq!(doc.name, "doc");
        assert!(
            hist.is_none(),
            "malformed history must not block the document"
        );
    }
}
