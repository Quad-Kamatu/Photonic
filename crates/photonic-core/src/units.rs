//! Document measurement units and conversion helpers.
//!
//! Canvas geometry is always stored in pixels (the document's native unit).
//! Rulers, readouts, and numeric inputs can present those pixel values in a
//! user-chosen unit. All conversions go through pixels as the common base and
//! are parameterised by the document DPI (pixels per inch).

use serde::{Deserialize, Serialize};

/// A measurement unit for ruler labels and numeric position inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DocumentUnit {
    /// Pixels — the document's native storage unit.
    #[default]
    Px,
    /// Millimetres.
    Mm,
    /// Inches.
    In,
    /// Points (1/72 inch).
    Pt,
}

impl DocumentUnit {
    /// Short label shown in the UI (ruler corner, readouts).
    pub fn label(self) -> &'static str {
        match self {
            DocumentUnit::Px => "px",
            DocumentUnit::Mm => "mm",
            DocumentUnit::In => "in",
            DocumentUnit::Pt => "pt",
        }
    }

    /// All units, in selector order.
    pub fn all() -> [DocumentUnit; 4] {
        [
            DocumentUnit::Px,
            DocumentUnit::Mm,
            DocumentUnit::In,
            DocumentUnit::Pt,
        ]
    }

    /// Next unit in the cycle (Px → Mm → In → Pt → Px). Useful for a
    /// click-to-cycle selector.
    pub fn next(self) -> DocumentUnit {
        match self {
            DocumentUnit::Px => DocumentUnit::Mm,
            DocumentUnit::Mm => DocumentUnit::In,
            DocumentUnit::In => DocumentUnit::Pt,
            DocumentUnit::Pt => DocumentUnit::Px,
        }
    }
}

/// Convert a value expressed in `unit` into document pixels, given `dpi`
/// (pixels per inch).
pub fn to_px(value: f64, unit: DocumentUnit, dpi: f64) -> f64 {
    match unit {
        DocumentUnit::Px => value,
        DocumentUnit::In => value * dpi,
        DocumentUnit::Mm => value * dpi / 25.4,
        DocumentUnit::Pt => value * dpi / 72.0,
    }
}

/// Convert a pixel value into `unit`, given `dpi` (pixels per inch).
pub fn from_px(px: f64, unit: DocumentUnit, dpi: f64) -> f64 {
    match unit {
        DocumentUnit::Px => px,
        DocumentUnit::In => px / dpi,
        DocumentUnit::Mm => px * 25.4 / dpi,
        DocumentUnit::Pt => px * 72.0 / dpi,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DPI: f64 = 96.0;

    #[test]
    fn px_is_identity() {
        assert_eq!(to_px(42.0, DocumentUnit::Px, DPI), 42.0);
        assert_eq!(from_px(42.0, DocumentUnit::Px, DPI), 42.0);
    }

    #[test]
    fn inch_conversion() {
        // 1 inch == dpi pixels.
        assert!((to_px(1.0, DocumentUnit::In, DPI) - 96.0).abs() < 1e-9);
        assert!((from_px(96.0, DocumentUnit::In, DPI) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn mm_conversion() {
        // 25.4 mm == 1 inch == dpi pixels.
        assert!((to_px(25.4, DocumentUnit::Mm, DPI) - 96.0).abs() < 1e-9);
    }

    #[test]
    fn pt_conversion() {
        // 72 pt == 1 inch == dpi pixels.
        assert!((to_px(72.0, DocumentUnit::Pt, DPI) - 96.0).abs() < 1e-9);
    }

    #[test]
    fn round_trip_all_units() {
        for unit in DocumentUnit::all() {
            for &dpi in &[72.0, 96.0, 150.0, 300.0] {
                for &px in &[0.0, 1.0, 37.5, 1024.0, -19.25] {
                    let unit_val = from_px(px, unit, dpi);
                    let back = to_px(unit_val, unit, dpi);
                    assert!(
                        (back - px).abs() < 1e-9,
                        "round-trip failed for {px} px, {unit:?}, dpi {dpi}: got {back}"
                    );
                }
            }
        }
    }

    #[test]
    fn next_cycles_through_all() {
        let mut u = DocumentUnit::Px;
        u = u.next();
        assert_eq!(u, DocumentUnit::Mm);
        u = u.next();
        assert_eq!(u, DocumentUnit::In);
        u = u.next();
        assert_eq!(u, DocumentUnit::Pt);
        u = u.next();
        assert_eq!(u, DocumentUnit::Px);
    }
}
