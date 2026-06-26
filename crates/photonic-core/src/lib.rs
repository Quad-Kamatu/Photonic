pub mod annotation;
pub mod audit;
pub mod color;
pub mod document;
pub mod export;
pub mod history;
pub mod import;
pub mod layer;
pub mod migration;
pub mod node;
pub mod ops;
pub mod path;
pub mod selection;
pub mod style;
pub mod transform;

// Re-export the most commonly used types at the crate root
pub use annotation::{Annotation, AnnotationId};
pub use audit::{audit_timestamp, AuditEntry, AuditLog};
pub use color::Color;
pub use document::{
    ActionSet, CharacterStyle, ColorSwatch, DimensionAnnotation, Document, DocumentId,
    DocumentVariable, EventTrigger, ExportProfile, GradientSwatch, GrammarRule, GraphicStyle,
    Guide, GuideOrientation, Page, ParagraphStyle, SpotColor, Symbol, WidthProfile, Workspace,
};
pub use history::{CheckpointInfo, Command, CommandHistory};
pub use import::{import_svg, ImportError};
pub use layer::{BlendMode, Layer, LayerId};
pub use node::{
    AssetExportSpec, FontStyle, GaussianGlow, GlowEffect, NodeId, PrimitiveKind, SceneNode,
    SceneNodeKind,
};
pub use path::PathData;
pub use selection::Selection;
pub use style::{
    interpolate_stops, ArrowheadStyle, Fill, FillKind, FluidGradient, FluidGradientPoint, Gradient,
    GradientKind, GradientStop, MeshGradient, MeshGradientVertex, Stroke,
};
pub use transform::Transform;
