//! Global search catalog + matching for the app's command palette.
//!
//! Searchable items are the editor tools plus a handful of commands. The UI
//! (in `app.rs`) ranks **direct** matches (title contains the query) first, then
//! **semantic** matches — items whose keywords or a fuzzy subsequence of the
//! title relate to the query — shown under a "Related" header.

use crate::Tool;
use egui_phosphor::regular as ph;

/// What activating a search result does.
#[derive(Clone, Copy)]
pub enum SearchAction {
    Tool(Tool),
    ToggleGrid,
    ToggleGuides,
    ToggleAudit,
    FileMenu,
    EditMenu,
    ToolsMenu,
    Undo,
    Redo,
    FitView,
    OutlineMode,
}

pub struct SearchItem {
    pub title: String,
    pub icon: &'static str,
    /// One-line description — shown in results and fed into semantic matching.
    pub description: String,
    pub keywords: &'static [&'static str],
    pub action: SearchAction,
}

const TOOLS: &[Tool] = &[
    Tool::Select,
    Tool::DirectSelect,
    Tool::Pan,
    Tool::Rectangle,
    Tool::RoundedRect,
    Tool::Ellipse,
    Tool::Polygon,
    Tool::Star,
    Tool::Spiral,
    Tool::Line,
    Tool::Arc,
    Tool::Grid,
    Tool::PolarGrid,
    Tool::Pen,
    Tool::ShapeBuilder,
    Tool::Text,
    Tool::Scissors,
    Tool::MagicWand,
    Tool::Lasso,
    Tool::Pencil,
    Tool::Smooth,
    Tool::RasterBrush,
    Tool::RasterEraser,
];

fn tool_keywords(t: Tool) -> &'static [&'static str] {
    match t {
        Tool::Select => &["move", "arrow", "pointer"],
        Tool::DirectSelect => &["node", "anchor", "point", "edit"],
        Tool::Pan => &["hand", "scroll", "navigate"],
        Tool::Rectangle => &["square", "box", "rect"],
        Tool::RoundedRect => &["rounded", "corner", "square"],
        Tool::Ellipse => &["circle", "oval", "round", "dot"],
        Tool::Polygon => &["hexagon", "pentagon", "sides", "shape"],
        Tool::Star => &["burst", "sparkle", "points"],
        Tool::Spiral => &["swirl", "coil", "helix"],
        Tool::Line => &["segment", "stroke", "straight"],
        Tool::Arc => &["curve", "semicircle"],
        Tool::Grid => &["table", "mesh", "rows", "columns"],
        Tool::PolarGrid => &["radial", "circular", "concentric"],
        Tool::Pen => &["bezier", "path", "curve", "draw"],
        Tool::ShapeBuilder => &["merge", "combine", "boolean", "union"],
        Tool::Text => &["type", "label", "font", "write"],
        Tool::Scissors => &["cut", "split", "snip"],
        Tool::MagicWand => &["select color", "wand", "fill select"],
        Tool::Lasso => &["freehand select", "loop"],
        Tool::Pencil => &["freehand", "sketch", "draw"],
        Tool::Smooth => &["simplify", "relax", "clean"],
        Tool::RasterBrush => &["paint", "pixel", "draw", "raster"],
        Tool::RasterEraser => &["erase", "rubber", "raster", "delete pixels"],
    }
}

/// Build the searchable catalog (tools + commands).
pub fn items() -> Vec<SearchItem> {
    let mut v: Vec<SearchItem> = TOOLS
        .iter()
        .map(|&t| SearchItem {
            title: t.label().to_string(),
            icon: t.icon(),
            description: t.description().to_string(),
            keywords: tool_keywords(t),
            action: SearchAction::Tool(t),
        })
        .collect();

    let cmd =
        |title: &str, icon: &'static str, description: &str, keywords: &'static [&'static str], action: SearchAction| SearchItem {
            title: title.to_string(),
            icon,
            description: description.to_string(),
            keywords,
            action,
        };
    v.extend([
        cmd("Toggle Grid", ph::GRID_FOUR, "Show or hide the canvas grid overlay", &["snap", "layout", "squares"], SearchAction::ToggleGrid),
        cmd("Toggle Guides", ph::RULER, "Show or hide ruler guides on the canvas", &["rulers", "alignment", "lines"], SearchAction::ToggleGuides),
        cmd("Audit Log", ph::MAGNIFYING_GLASS, "Open the MCP audit log of recent AI tool calls", &["mcp", "history", "tool calls", "log"], SearchAction::ToggleAudit),
        cmd("File Menu", ph::FILE, "New, open, save, and export documents", &["new", "open", "save", "export"], SearchAction::FileMenu),
        cmd("Preferences", ph::GEAR, "Edit application settings, theme, and options", &["settings", "edit", "options", "theme"], SearchAction::EditMenu),
        cmd("Tools Menu", ph::SQUARES_FOUR, "Browse all tools and pin them to the sidebar", &["pin", "toolbar", "all tools"], SearchAction::ToolsMenu),
        cmd("Undo", ph::ARROW_COUNTER_CLOCKWISE, "Undo the last action", &["back", "revert", "ctrl z"], SearchAction::Undo),
        cmd("Redo", ph::ARROW_CLOCKWISE, "Redo the last undone action", &["forward", "ctrl y"], SearchAction::Redo),
        cmd("Fit to View", ph::FRAME_CORNERS, "Zoom and center the artboards in the viewport", &["zoom", "frame", "center", "fit artboard"], SearchAction::FitView),
        cmd("Outline Mode", ph::SQUARES_FOUR, "Toggle wireframe outline preview of all shapes", &["wireframe", "skeleton", "preview"], SearchAction::OutlineMode),
    ]);
    v
}

/// True if every (non-space) char of `q` appears in `s` in order.
pub fn fuzzy_subseq(q: &str, s: &str) -> bool {
    let mut chars = s.chars();
    for qc in q.chars().filter(|c| !c.is_whitespace()) {
        loop {
            match chars.next() {
                Some(c) if c == qc => break,
                Some(_) => continue,
                None => return false,
            }
        }
    }
    true
}
