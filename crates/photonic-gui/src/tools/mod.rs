use egui_phosphor::regular as ph;
use photonic_core::PrimitiveKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Tool {
    #[default]
    Select,
    DirectSelect,
    Pan,
    Rectangle,
    RoundedRect,
    Ellipse,
    Polygon,
    Star,
    Spiral,
    Line,
    Arc,
    Grid,
    PolarGrid,
    Pen,
    ShapeBuilder,
    Text,
    Scissors,
    MagicWand,
    Lasso,
    Pencil,
    Smooth,
}

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::DirectSelect => "Direct Select",
            Tool::Pan => "Pan",
            Tool::Rectangle => "Rect",
            Tool::RoundedRect => "Rounded Rect",
            Tool::Ellipse => "Ellipse",
            Tool::Polygon => "Polygon",
            Tool::Star => "Star",
            Tool::Spiral => "Spiral",
            Tool::Line => "Line",
            Tool::Arc => "Arc",
            Tool::Grid => "Grid",
            Tool::PolarGrid => "Polar Grid",
            Tool::Pen => "Pen",
            Tool::ShapeBuilder => "Shape Builder",
            Tool::Text => "Text",
            Tool::Scissors => "Scissors",
            Tool::MagicWand => "Magic Wand",
            Tool::Lasso => "Lasso",
            Tool::Pencil => "Pencil",
            Tool::Smooth => "Smooth",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Tool::Select => ph::CURSOR,
            Tool::DirectSelect => ph::BEZIER_CURVE,
            Tool::Pan => ph::HAND,
            Tool::Rectangle => ph::RECTANGLE,
            Tool::RoundedRect => ph::RECTANGLE,
            Tool::Ellipse => ph::CIRCLE,
            Tool::Polygon => ph::POLYGON,
            Tool::Star => ph::STAR,
            Tool::Spiral => ph::SPIRAL,
            Tool::Line => ph::LINE_SEGMENT,
            Tool::Arc => ph::CIRCLE_HALF,
            Tool::Grid => ph::GRID_FOUR,
            Tool::PolarGrid => ph::CIRCLES_THREE,
            Tool::Pen => ph::PEN_NIB,
            Tool::ShapeBuilder => ph::UNITE,
            Tool::Text => ph::TEXT_T,
            Tool::Scissors => ph::SCISSORS,
            Tool::MagicWand => ph::MAGIC_WAND,
            Tool::Lasso => ph::LASSO,
            Tool::Pencil => ph::PENCIL,
            Tool::Smooth => ph::WAVE_SINE,
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Tool::Select => "Select and move objects",
            Tool::DirectSelect => "Edit individual anchor points",
            Tool::Pan => "Pan the canvas view",
            Tool::Rectangle => "Draw rectangles and squares",
            Tool::RoundedRect => "Draw rectangles with rounded corners",
            Tool::Ellipse => "Draw ellipses and circles",
            Tool::Polygon => "Draw regular polygons",
            Tool::Star => "Draw star shapes",
            Tool::Spiral => "Draw Archimedean spirals",
            Tool::Line => "Draw straight line segments with precise length and angle",
            Tool::Arc => "Draw open or closed arcs with configurable sweep angle",
            Tool::Grid => "Draw a rectangular grid with configurable rows and columns",
            Tool::PolarGrid => {
                "Draw a polar (radial) grid with concentric rings and radial sectors"
            }
            Tool::Pen => "Draw freeform paths with bezier curves",
            Tool::ShapeBuilder => "Combine or subtract overlapping shapes",
            Tool::Text => "Add text to the canvas",
            Tool::Scissors => "Cut a path at any point, splitting it into two open paths",
            Tool::MagicWand => {
                "Select all objects sharing a similar attribute (fill, stroke, opacity…)"
            }
            Tool::Lasso => "Drag a freehand lasso to select all objects within the enclosed region",
            Tool::Pencil => {
                "Draw freehand paths by dragging — anchor points auto-generated along the stroke"
            }
            Tool::Smooth => "Drag over a path to smooth its anchor points using corner-cutting",
        }
    }

    pub fn is_shape_creator(self) -> bool {
        !matches!(
            self,
            Tool::Select
                | Tool::DirectSelect
                | Tool::Pan
                | Tool::Pen
                | Tool::ShapeBuilder
                | Tool::Text
                | Tool::Scissors
                | Tool::MagicWand
                | Tool::Lasso
                | Tool::Pencil
                | Tool::Smooth
        )
    }

    pub fn from_primitive(p: PrimitiveKind) -> Self {
        match p {
            PrimitiveKind::Rectangle => Tool::Rectangle,
            PrimitiveKind::RoundedRect => Tool::RoundedRect,
            PrimitiveKind::Ellipse => Tool::Ellipse,
            PrimitiveKind::Polygon => Tool::Polygon,
            PrimitiveKind::Star => Tool::Star,
            PrimitiveKind::Line => Tool::Line,
            PrimitiveKind::Arc => Tool::Arc,
        }
    }
}
