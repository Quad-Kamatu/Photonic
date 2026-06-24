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
    /// Ordered list of node IDs (bottom to top).
    pub node_ids: Vec<uuid::Uuid>,
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
}
