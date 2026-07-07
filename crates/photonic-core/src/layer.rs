use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type LayerId = Uuid;

/// A layer in the document — an ordered, named collection of scene nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub opacity: f32,
    pub blend_mode: BlendMode,
    /// Optional color tag used for identification in the layers panel (RGBA 0–1).
    #[serde(default)]
    pub color: Option<[f32; 4]>,
    /// Template layers are locked, dimmed reference layers for tracing over.
    #[serde(default)]
    pub is_template: bool,
    /// Whether the layer is included in print/export output. Non-print layers
    /// stay visible on the canvas but are excluded from exports (Illustrator's
    /// "Print" layer option). Defaults to true for back-compat.
    #[serde(default = "default_true")]
    pub print: bool,
    /// Granular locks (Photoshop's lock cluster). `locked` remains the "lock all"
    /// master; these add finer control. Defaults false.
    #[serde(default)]
    pub lock_transparency: bool,
    #[serde(default)]
    pub lock_pixels: bool,
    #[serde(default)]
    pub lock_position: bool,
    /// Ordered list of node IDs (bottom to top).
    pub node_ids: Vec<uuid::Uuid>,
}

fn default_true() -> bool {
    true
}

impl Layer {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            visible: true,
            locked: false,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            color: None,
            is_template: false,
            print: true,
            lock_transparency: false,
            lock_pixels: false,
            lock_position: false,
            node_ids: vec![],
        }
    }

    pub fn with_id(mut self, id: LayerId) -> Self {
        self.id = id;
        self
    }
}

/// Compositing blend mode for a layer or node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
    // ── Photoshop extras: no CSS `mix-blend-mode` keyword. They render correctly
    // on the raster/canvas paths (see `raster::blend`); in SVG/CSS they degrade to
    // Normal in browsers but round-trip within Photonic via their hyphenated names.
    LinearDodge,
    LinearBurn,
    Subtract,
    Divide,
    VividLight,
    LinearLight,
    PinLight,
    HardMix,
    DarkerColor,
    LighterColor,
}

impl BlendMode {
    /// The CSS `mix-blend-mode` keyword. The first 16 are 1:1 with the CSS spec;
    /// the Photoshop extras use hyphenated names browsers don't recognise (so they
    /// fall back to Normal there) but which [`from_css`](BlendMode::from_css)
    /// parses, keeping Photonic SVG round-trips lossless.
    pub fn to_css(self) -> &'static str {
        match self {
            BlendMode::Normal => "normal",
            BlendMode::Multiply => "multiply",
            BlendMode::Screen => "screen",
            BlendMode::Overlay => "overlay",
            BlendMode::Darken => "darken",
            BlendMode::Lighten => "lighten",
            BlendMode::ColorDodge => "color-dodge",
            BlendMode::ColorBurn => "color-burn",
            BlendMode::HardLight => "hard-light",
            BlendMode::SoftLight => "soft-light",
            BlendMode::Difference => "difference",
            BlendMode::Exclusion => "exclusion",
            BlendMode::Hue => "hue",
            BlendMode::Saturation => "saturation",
            BlendMode::Color => "color",
            BlendMode::Luminosity => "luminosity",
            BlendMode::LinearDodge => "linear-dodge",
            BlendMode::LinearBurn => "linear-burn",
            BlendMode::Subtract => "subtract",
            BlendMode::Divide => "divide",
            BlendMode::VividLight => "vivid-light",
            BlendMode::LinearLight => "linear-light",
            BlendMode::PinLight => "pin-light",
            BlendMode::HardMix => "hard-mix",
            BlendMode::DarkerColor => "darker-color",
            BlendMode::LighterColor => "lighter-color",
        }
    }

    /// Parse a CSS `mix-blend-mode` keyword (or a Photonic extra name).
    /// Case-insensitive; returns `None` for unrecognized values.
    pub fn from_css(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "normal" => BlendMode::Normal,
            "multiply" => BlendMode::Multiply,
            "screen" => BlendMode::Screen,
            "overlay" => BlendMode::Overlay,
            "darken" => BlendMode::Darken,
            "lighten" => BlendMode::Lighten,
            "color-dodge" => BlendMode::ColorDodge,
            "color-burn" => BlendMode::ColorBurn,
            "hard-light" => BlendMode::HardLight,
            "soft-light" => BlendMode::SoftLight,
            "difference" => BlendMode::Difference,
            "exclusion" => BlendMode::Exclusion,
            "hue" => BlendMode::Hue,
            "saturation" => BlendMode::Saturation,
            "color" => BlendMode::Color,
            "luminosity" => BlendMode::Luminosity,
            "linear-dodge" => BlendMode::LinearDodge,
            "linear-burn" => BlendMode::LinearBurn,
            "subtract" => BlendMode::Subtract,
            "divide" => BlendMode::Divide,
            "vivid-light" => BlendMode::VividLight,
            "linear-light" => BlendMode::LinearLight,
            "pin-light" => BlendMode::PinLight,
            "hard-mix" => BlendMode::HardMix,
            "darker-color" => BlendMode::DarkerColor,
            "lighter-color" => BlendMode::LighterColor,
            _ => return None,
        })
    }
}
