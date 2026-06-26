use egui::{Color32, RichText};
use egui_phosphor::regular as ph;
use kurbo::{BezPath, PathEl, Point};
use photonic_core::{
    history::{Command, CommandHistory},
    node::{GroupNode, NodeId, PathNode},
    Color, Document, Fill, Layer, PathData, SceneNode, SceneNodeKind, Selection, Stroke,
};
use photonic_render::{CanvasView, ExportBackground, ExportOptions, PhotonicRenderer};
use std::path::Path;
use std::sync::Arc;

use crate::{
    panels::{self, EyedropperTarget, PanelAction, SelectSameAttr, ShapeKind, ZOrderOp},
    preferences::AppPreferences,
    radial_wheel::{WheelContext, WheelNodeKind, WheelState},
    tools::Tool,
};

// ─── Eyedropper ───────────────────────────────────────────────────────────────

/// Raw pixel data captured from the screen when eyedropper mode starts.
struct EyedropperCapture {
    /// Screen origin in logical (OS) coords.
    origin_x: i32,
    origin_y: i32,
    /// Physical pixels per logical pixel for this display.
    scale: f32,
    /// Dimensions of the captured image in physical pixels.
    width: u32,
    height: u32,
    /// Row-major RGBA pixel data (physical resolution).
    pixels: Vec<u8>,
}

impl EyedropperCapture {
    /// Sample the pixel at a logical screen coordinate.
    fn sample_logical(&self, lx: f32, ly: f32) -> Option<[u8; 4]> {
        let px = ((lx - self.origin_x as f32) * self.scale) as i32;
        let py = ((ly - self.origin_y as f32) * self.scale) as i32;
        if px < 0 || py < 0 || px >= self.width as i32 || py >= self.height as i32 {
            return None;
        }
        let base = ((py as u32 * self.width + px as u32) * 4) as usize;
        if base + 3 >= self.pixels.len() {
            return None;
        }
        Some([
            self.pixels[base],
            self.pixels[base + 1],
            self.pixels[base + 2],
            self.pixels[base + 3],
        ])
    }
}

/// State for the screen eyedropper tool.
#[derive(Default)]
pub struct EyedropperState {
    pub target: Option<EyedropperTarget>,
    capture: Option<EyedropperCapture>,
    /// Skip the very first `primary_clicked` after activation so the button's
    /// own release doesn't immediately trigger a sample.
    skip_click: bool,
}

impl EyedropperState {
    pub fn active(&self) -> bool {
        self.target.is_some()
    }

    fn sample_at_screen_logical(&self, lx: f32, ly: f32) -> Option<[u8; 4]> {
        self.capture.as_ref()?.sample_logical(lx, ly)
    }

    fn cancel(&mut self) {
        self.target = None;
        self.capture = None;
        self.skip_click = false;
    }
}

/// Capture the screen that contains the given logical window position.
/// Returns `None` if the screen cannot be captured or is unavailable.
fn capture_screen(window_logical_x: i32, window_logical_y: i32) -> Option<EyedropperCapture> {
    use screenshots::Screen;
    let screen = Screen::from_point(window_logical_x, window_logical_y).ok()?;
    let img = screen.capture().ok()?;
    let w = img.width();
    let h = img.height();
    let pixels = img.into_raw();
    Some(EyedropperCapture {
        origin_x: screen.display_info.x,
        origin_y: screen.display_info.y,
        scale: screen.display_info.scale_factor,
        width: w,
        height: h,
        pixels,
    })
}

// ─── Drawer kind ──────────────────────────────────────────────────────────────

/// Which top-bar drawer is currently open (replaces floating popover menus).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DrawerKind {
    File,
    Edit,
    Tools,
}

/// Which corner handle is being dragged during a resize operation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResizeHandle {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

// ─── Diff highlight ────────────────────────────────────────────────────────────

/// Category of a node in a checkpoint diff, used to colour canvas highlights.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffCategory {
    /// Present in current doc but not in the baseline checkpoint — shown green.
    Added,
    /// Present in the baseline checkpoint but not in the current doc — shown red.
    Removed,
    /// Present in both but with changed properties — shown yellow.
    Modified,
}

const FILE_OPTIONS: &[&str] = &["Document", "Save", "Export"];
const EDIT_OPTIONS: &[&str] = &["Appearance", "Canvas", "Tool Defaults", "Behavior"];

// ─── Export dialog ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExportFormat {
    Png,
    Jpeg,
    WebP,
    Gif,
    Tiff,
    Ico,
    Svg,
}

pub struct ExportDialog {
    pub format: ExportFormat,
    pub background: ExportBackground,
    pub crop_to_content: bool,
    pub png_width: u32,
    pub png_height: u32,
    pub ico_size_16: bool,
    pub ico_size_32: bool,
    pub ico_size_48: bool,
    pub ico_size_256: bool,
    /// JPEG quality (1–100).
    pub jpeg_quality: u8,
    /// Aspect ratio of the document at the time the dialog was opened.
    aspect: f64,
}

impl ExportDialog {
    pub fn new(doc: &Document) -> Self {
        Self {
            format: ExportFormat::Png,
            background: ExportBackground::Transparent,
            crop_to_content: true,
            png_width: doc.width as u32,
            png_height: doc.height as u32,
            ico_size_16: true,
            ico_size_32: true,
            ico_size_48: true,
            ico_size_256: true,
            jpeg_quality: 90,
            aspect: doc.width / doc.height,
        }
    }

    pub fn export_opts(&self) -> ExportOptions {
        let ico_sizes = [
            self.ico_size_16.then_some(16u32),
            self.ico_size_32.then_some(32),
            self.ico_size_48.then_some(48),
            self.ico_size_256.then_some(256),
        ]
        .into_iter()
        .flatten()
        .collect();
        ExportOptions {
            background: self.background,
            crop_to_content: self.crop_to_content,
            ico_sizes,
            jpeg_quality: self.jpeg_quality,
        }
    }
}

/// Which tab is active in the console panel.
#[derive(PartialEq, Clone, Copy, Default)]
pub enum ConsoleTab {
    #[default]
    Lua,
    Claude,
}

// ─── Simplify dialog ─────────────────────────────────────────────────────────

struct SimplifyDialog {
    node_id: NodeId,
    node_name: String,
    tolerance: f64,
}

struct FindReplaceTextDialog {
    find: String,
    replace: String,
    regex: bool,
    case_sensitive: bool,
    selection_only: bool,
}

// ── Extracted sub-structs ─────────────────────────────────────────────────────

/// State for the Lua REPL console panel.
#[derive(Default)]
pub struct LuaConsoleState {
    pub visible: bool,
    pub expanded: bool,
    pub tab: ConsoleTab,
    pub input: String,
    pub log: Vec<(bool, String)>,
    /// Lua code queued for execution by main.rs after the draw lock is released.
    pub pending: Option<String>,
}

/// State for the in-app Claude chat panel.
#[derive(Default)]
pub struct ClaudeChatState {
    /// Chat history: (is_user, message_text).
    pub messages: Vec<(bool, String)>,
    pub input: String,
    pub busy: bool,
    /// Message queued for main.rs to dispatch to the Claude subprocess.
    pub pending: Option<String>,
}

/// State for the floating MCP audit log panel.
pub struct AuditPanelState {
    /// Shared MCP audit log (set by main.rs after construction).
    pub log: Option<Arc<std::sync::Mutex<photonic_core::AuditLog>>>,
    pub panel_open: bool,
    pub filter: String,
}
impl Default for AuditPanelState {
    fn default() -> Self {
        Self {
            log: None,
            panel_open: false,
            filter: String::new(),
        }
    }
}

/// State for the diff highlight overlay shown after AI edits.
#[derive(Default)]
pub struct DiffOverlayState {
    /// Added/modified nodes to highlight on the canvas (node_id, category).
    pub highlights: Vec<(NodeId, DiffCategory)>,
    /// Pre-computed canvas-space bounding boxes for removed nodes (not in doc).
    pub removed_boxes: Vec<egui::Rect>,
    pub overlay_active: bool,
}

pub struct PhotonicApp {
    pub active_tool: Tool,
    pub fill_color: [f32; 4],
    pub polygon_sides: u32,
    pub star_points: u32,
    pub star_inner_ratio: f32,
    pub rounded_rect_radius: f64,
    pub spiral_turns: f32,
    pub spiral_inner_radius: f32,
    pub spiral_segs_per_turn: u32,
    /// Pending shear values typed into the Properties panel (applied on "Apply Shear" click).
    pub shear_x: f64,
    pub shear_y: f64,
    /// Line tool: snap endpoint to the nearest 45° angle from the start point.
    pub line_snap_45: bool,
    /// Currently selected harmony rule in the Color Guide panel.
    pub color_guide_rule: String,
    /// Arc tool: start angle in degrees (0 = 3 o'clock).
    pub arc_start_angle: f64,
    /// Arc tool: end angle in degrees.
    pub arc_end_angle: f64,
    /// Arc tool: if true, draw open arc; if false, close the arc (pie sector).
    pub arc_open: bool,
    /// Grid tool: number of columns.
    pub grid_cols: u32,
    /// Grid tool: number of rows.
    pub grid_rows: u32,
    /// Polar Grid tool: number of concentric rings.
    pub polar_grid_rings: u32,
    /// Polar Grid tool: number of radial sectors.
    pub polar_grid_sectors: u32,
    /// Polar Grid tool: inner radius fraction (0 = full disk).
    pub polar_grid_inner_ratio: f32,
    /// Layer IDs checked in the layers panel for multi-layer operations (e.g. Merge).
    pub selected_layer_ids: Vec<photonic_core::layer::LayerId>,

    /// Currently selected node (Select tool).
    pub selected_id: Option<NodeId>,

    /// Canvas-space position where the current drag began (shape creation).
    drag_start_canvas: Option<(f64, f64)>,

    /// Accumulated anchor points for the in-progress pen path (canvas space).
    pen_points: Vec<(f64, f64)>,

    /// Whether we are currently dragging a selected node to move it.
    moving: bool,

    /// Which corner handle is being dragged (None = not resizing).
    resizing: Option<ResizeHandle>,
    /// Canvas-space bounding box captured at the start of a resize drag.
    resize_origin_bounds: Option<(f64, f64, f64, f64)>,
    /// Node transform matrix captured at the start of a resize drag.
    resize_origin_transform: Option<[f64; 6]>,
    /// Font size captured at resize-drag start (TextNode only).
    resize_origin_font_size: Option<f64>,

    /// Transforms of all selected nodes captured at the start of a multi-node resize.
    resize_multi_origins: Vec<(NodeId, [f64; 6])>,

    /// Screen-space position where a marquee (drag-select) began; None when inactive.
    marquee_start: Option<egui::Pos2>,

    // ── Direct Selection (point edit) tool state ─────────────────────────────
    /// The node whose anchor points are currently being edited.
    point_edit_node: Option<NodeId>,
    /// Indices into the BezPath element array that are currently selected.
    point_selected: Vec<usize>,
    /// Snapshot of the node captured at drag-start (None when not dragging).
    /// Used to build the UpdateNode undo command on drag release.
    point_drag_origin: Option<SceneNode>,

    // ── Shape Builder tool state ──────────────────────────────────────────────
    /// Node under cursor in Shape Builder mode (for highlight preview).
    shape_builder_hovered: Option<NodeId>,
    /// Nodes touched during the current Shape Builder drag (in touch order).
    shape_builder_drag_ids: Vec<NodeId>,
    /// True when Alt was held at the start of the current drag (subtract mode).
    shape_builder_subtract_mode: bool,

    // ── Console / REPL ────────────────────────────────────────────────────────
    pub lua_console: LuaConsoleState,

    /// Actions queued by panel widgets (z-order, boolean ops) to be processed
    /// after all panels have finished drawing, with access to doc + history.
    pub pending_panel_actions: Vec<PanelAction>,

    // ── Claude chat ───────────────────────────────────────────────────────────
    pub claude_chat: ClaudeChatState,

    // ── File I/O ──────────────────────────────────────────────────────────────
    /// Path of the currently open .photon file (None = unsaved new document).
    pub current_file: Option<std::path::PathBuf>,
    /// One-shot status message shown in the toolbar after save/load.
    file_status: Option<String>,
    /// Export settings modal — Some while open.
    export_dialog: Option<ExportDialog>,
    /// Simplify Path dialog — Some while open.
    simplify_dialog: Option<SimplifyDialog>,
    /// Find / Replace Text dialog — Some while open.
    find_replace_text_dialog: Option<FindReplaceTextDialog>,

    // ── Welcome screen ────────────────────────────────────────────────────────
    /// Show the welcome/new-document screen instead of the editor.
    pub show_welcome: bool,
    /// State for the welcome screen (form fields + recent docs list).
    pub welcome: crate::welcome::WelcomeState,

    // ── Smooth viewport animation ─────────────────────────────────────────────
    smooth: SmoothViewState,

    // ── Preferences ───────────────────────────────────────────────────────────
    pub prefs: AppPreferences,
    /// Which top-bar drawer is open, if any.
    pub active_drawer: Option<DrawerKind>,
    /// Which option is selected in the currently open drawer (index into the options list).
    /// Resets to None whenever active_drawer changes.
    selected_drawer_option: Option<usize>,

    // ── Radial wheel ──────────────────────────────────────────────────────────
    /// Open right-click selection wheel, or None when closed.
    radial_wheel: Option<WheelState>,

    // ── Audit panel ───────────────────────────────────────────────────────────
    pub audit: AuditPanelState,

    // ── Diff highlight overlay ────────────────────────────────────────────────
    pub diff: DiffOverlayState,

    // ── Outline Mode ─────────────────────────────────────────────────────────
    /// When true, the canvas shows path wireframes only (no fills or strokes).
    pub outline_mode: bool,

    // ── Guides ────────────────────────────────────────────────────────────────
    /// When true, ruler guides are rendered on the canvas (toggle with Ctrl+;).
    pub guides_visible: bool,

    // ── Isolation Mode ───────────────────────────────────────────────────────
    /// When set, only children of this group are selectable/editable.
    /// None = normal editing mode.
    pub isolated_group: Option<NodeId>,

    // ── Pencil tool state ────────────────────────────────────────────────────
    /// Canvas-space points collected during an active pencil drag.
    pencil_points: Vec<(f64, f64)>,

    // ── Lasso tool state ─────────────────────────────────────────────────────
    /// Screen-space points collected during an active lasso drag.
    lasso_points: Vec<egui::Pos2>,

    // ── Magic Wand tool options ───────────────────────────────────────────────
    /// Which attribute the Magic Wand matches when clicked.
    pub magic_wand_attribute: SelectSameAttr,
    /// Tolerance for the Magic Wand tool (color/numeric difference threshold).
    pub magic_wand_tolerance: f64,

    // ── GUI Clipboard ─────────────────────────────────────────────────────────
    /// Nodes copied with Ctrl+C, stored in-process for Ctrl+V / Ctrl+Shift+V.
    /// Each entry is a deep-clone of the node at copy time with its original transform.
    pub gui_clipboard: Vec<SceneNode>,

    // ── Composition Analysis ──────────────────────────────────────────────────
    /// Latest findings from the composition analyzer (shown in the GUI panel).
    pub composition_findings: Vec<String>,
    /// Latest rhythm patterns from the rhythm detector (shown in the GUI panel).
    pub rhythm_findings: Vec<String>,

    // ── Branches ─────────────────────────────────────────────────────────────
    /// Cached list of branch names from CommandHistory (refreshed on branch actions).
    pub branch_names: Vec<String>,
    /// Text input for naming a new branch in the Branches panel.
    pub branch_name_input: String,

    /// Selected swatch library name for the Color Swatches panel dropdown.
    pub swatch_library_selected: String,
    /// Text input for naming a new graphic style in the Graphic Styles panel.
    pub graphic_style_name_input: String,
    /// Text input for naming a new width profile in the Width Profiles panel.
    pub width_profile_name_input: String,
    /// Cached grammar rule list: (name, rule_type).
    pub grammar_rules: Vec<(String, String)>,
    /// Text input for the new grammar rule name.
    pub grammar_rule_name_input: String,
    /// Selected rule type for the grammar rule form.
    pub grammar_rule_type_selected: String,
    /// JSON params text for the grammar rule form.
    pub grammar_rule_params_input: String,
    /// Latest grammar check results: (rule_name, passed, message).
    pub grammar_check_results: Vec<(String, bool, String)>,
    /// Latest distance measurements: (from_name, to_name, h_gap, v_gap, center_dist).
    pub distance_results: Vec<(String, String, f64, f64, f64)>,
    /// Cached action set names: (name, step_count).
    pub action_names: Vec<(String, usize)>,
    /// Cached history entries: (step_index, description) newest first.
    pub history_entries: Vec<(usize, String)>,
    /// Bleed input (mm) for print settings panel.
    pub bleed_mm_input: f64,
    /// Slug input (mm) for print settings panel.
    pub slug_mm_input: f64,
    /// Construction line angle input (degrees).
    pub construction_angle: f64,
    /// Construction line origin X.
    pub construction_x: f64,
    /// Construction line origin Y.
    pub construction_y: f64,
    /// Artboard margin top input (document units).
    pub margin_top_input: f64,
    /// Artboard margin right input (document units).
    pub margin_right_input: f64,
    /// Artboard margin bottom input (document units).
    pub margin_bottom_input: f64,
    /// Artboard margin left input (document units).
    pub margin_left_input: f64,
    /// Selected event name for event trigger panel.
    pub event_trigger_event: String,
    /// Selected action name for event trigger panel.
    pub event_trigger_action: String,
    /// Input field for workspace name in the workspaces panel.
    pub workspace_name_input: String,

    // ── Properties panel ─────────────────────────────────────────────────────
    /// Live search query that filters which property accordions are visible.
    pub prop_search: String,
    /// Recolor panel: comma-separated hex palette input.
    pub recolor_palette_input: String,

    // ── Eyedropper ────────────────────────────────────────────────────────────
    pub eyedropper: EyedropperState,
    /// Window top-left in logical screen coordinates (updated by main.rs each frame).
    pub window_logical_pos: (i32, i32),
    /// DPI scale factor of the main window (updated by main.rs each frame).
    pub window_scale_factor: f32,
}

/// Interpolation state for smooth zoom and WASD pan.
struct SmoothViewState {
    /// Target zoom in log-space; actual zoom lerps toward `exp(log_zoom_target)`.
    log_zoom_target: f64,
    /// Screen-space pivot used when lerping zoom (last scroll position).
    zoom_pivot: (f64, f64),
    /// Current pan velocity (px/s) applied by WASD keys.
    pan_vel_x: f64,
    pan_vel_y: f64,
}

impl Default for SmoothViewState {
    fn default() -> Self {
        Self {
            log_zoom_target: 0.0, // ln(1.0)
            zoom_pivot: (640.0, 400.0),
            pan_vel_x: 0.0,
            pan_vel_y: 0.0,
        }
    }
}

// ─── Two-column drawer helper ─────────────────────────────────────────────────

/// Renders a two-column menu: fixed-width nav on the left, content on the right.
/// Returns the (possibly updated) selected option index.
fn draw_two_column_menu(
    ui: &mut egui::Ui,
    left_col_width: f32,
    content_height: f32,
    options: &[&str],
    selected: Option<usize>,
    content: impl FnOnce(&mut egui::Ui, Option<usize>),
) -> Option<usize> {
    let mut new_selection = selected;
    let right_col_width = (ui.available_width() - left_col_width - 16.0).max(120.0);

    ui.horizontal(|ui| {
        // ── Left nav ─────────────────────────────────────────────────────
        ui.allocate_ui_with_layout(
            egui::vec2(left_col_width, content_height),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("drawer_nav_scroll")
                    .max_height(content_height)
                    .show(ui, |ui| {
                        ui.set_width(left_col_width); // prevents NaN from infinite available_width
                        for (i, label) in options.iter().enumerate() {
                            if ui
                                .selectable_label(new_selection == Some(i), *label)
                                .clicked()
                            {
                                new_selection = Some(i);
                            }
                        }
                    });
            },
        );

        ui.separator();

        // ── Right content ─────────────────────────────────────────────────
        ui.allocate_ui_with_layout(
            egui::vec2(right_col_width, content_height),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("drawer_content_scroll")
                    .max_height(content_height)
                    .show(ui, |ui| {
                        ui.set_min_width(right_col_width); // prevents collapse
                        content(ui, new_selection);
                    });
            },
        );
    });

    new_selection
}

impl Default for PhotonicApp {
    fn default() -> Self {
        Self {
            active_tool: Tool::Select,
            fill_color: [0.22, 0.47, 0.87, 1.0],
            polygon_sides: 6,
            star_points: 5,
            star_inner_ratio: 0.45,
            rounded_rect_radius: 10.0,
            spiral_turns: 3.0,
            spiral_inner_radius: 0.0,
            spiral_segs_per_turn: 16,
            shear_x: 0.0,
            shear_y: 0.0,
            line_snap_45: false,
            color_guide_rule: "complementary".to_string(),
            arc_start_angle: 0.0,
            arc_end_angle: 270.0,
            arc_open: false,
            grid_cols: 4,
            grid_rows: 4,
            polar_grid_rings: 4,
            polar_grid_sectors: 8,
            polar_grid_inner_ratio: 0.0,
            selected_layer_ids: Vec::new(),
            selected_id: None,
            drag_start_canvas: None,
            pen_points: Vec::new(),
            moving: false,
            resizing: None,
            resize_origin_bounds: None,
            resize_origin_transform: None,
            resize_origin_font_size: None,
            resize_multi_origins: Vec::new(),
            marquee_start: None,
            point_edit_node: None,
            point_selected: Vec::new(),
            point_drag_origin: None,
            shape_builder_hovered: None,
            shape_builder_drag_ids: Vec::new(),
            shape_builder_subtract_mode: false,
            lua_console: LuaConsoleState {
                log: vec![(
                    false,
                    "Photonic Lua REPL — type `photonic` to see the API".into(),
                )],
                ..LuaConsoleState::default()
            },

            pending_panel_actions: Vec::new(),

            claude_chat: ClaudeChatState::default(),

            current_file: None,
            file_status: None,
            export_dialog: None,
            simplify_dialog: None,
            find_replace_text_dialog: None,
            smooth: SmoothViewState::default(),
            prefs: AppPreferences::default(),
            active_drawer: None,
            selected_drawer_option: None,

            show_welcome: false,
            welcome: crate::welcome::WelcomeState::new(),

            radial_wheel: None,

            audit: AuditPanelState::default(),

            diff: DiffOverlayState::default(),

            composition_findings: Vec::new(),
            rhythm_findings: Vec::new(),
            branch_names: Vec::new(),
            branch_name_input: String::new(),
            swatch_library_selected: String::new(),
            graphic_style_name_input: String::new(),
            width_profile_name_input: String::new(),
            grammar_rules: Vec::new(),
            grammar_rule_name_input: String::new(),
            grammar_rule_type_selected: String::new(),
            grammar_rule_params_input: String::new(),
            grammar_check_results: Vec::new(),
            distance_results: Vec::new(),
            action_names: Vec::new(),
            history_entries: Vec::new(),
            bleed_mm_input: 0.0,
            slug_mm_input: 0.0,
            construction_angle: 45.0,
            construction_x: 0.0,
            construction_y: 0.0,
            margin_top_input: 0.0,
            margin_right_input: 0.0,
            margin_bottom_input: 0.0,
            margin_left_input: 0.0,
            event_trigger_event: String::new(),
            event_trigger_action: String::new(),
            workspace_name_input: String::new(),

            prop_search: String::new(),
            recolor_palette_input: String::new(),

            eyedropper: EyedropperState::default(),
            window_logical_pos: (0, 0),
            window_scale_factor: 1.0,
            outline_mode: false,
            guides_visible: true,
            isolated_group: None,
            pencil_points: Vec::new(),
            lasso_points: Vec::new(),
            magic_wand_attribute: SelectSameAttr::FillColor,
            magic_wand_tolerance: 0.05,
            gui_clipboard: Vec::new(),
        }
    }
}

/// Load a document from disk, supporting `.photon` and `.svg` files.
/// Run a blocking `rfd` file dialog OFF the winit/Wayland event-loop thread.
///
/// `rfd`'s portal-backed dialogs (`pick_file`/`save_file`) internally
/// `pollster::block_on` an async XDG-desktop-portal call on the *calling*
/// thread. When that caller is the egui draw closure — which runs inside
/// winit's Wayland calloop event-loop callback — the portal's D-Bus events get
/// delivered back into winit's calloop re-entrantly (`calloop: Received an
/// event for non-existence source`) and the process aborts with SIGABRT
/// (`org.freedesktop.DBus.Error.UnknownMethod: Object does not exist at
/// .../request/...ashpd_...`). Spawning the dialog on a dedicated thread gives
/// the portal its own context and avoids the re-entrancy. The UI thread blocks
/// on `join()` while the dialog is open, which is the expected modal behaviour.
fn run_file_dialog<F>(f: F) -> Option<std::path::PathBuf>
where
    F: FnOnce() -> Option<std::path::PathBuf> + Send + 'static,
{
    std::thread::spawn(f).join().unwrap_or(None)
}

fn load_document(path: &Path) -> Result<Document, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    if ext == "svg" {
        photonic_core::import_svg(&content).map_err(|e| e.to_string())
    } else {
        Document::from_json(&content).map_err(|e| e.to_string())
    }
}

impl PhotonicApp {
    pub fn new() -> Self {
        let prefs = AppPreferences::load();
        let fill_color = prefs.default_fill_color;
        let console_visible = prefs.console_open_on_start;
        let mut s = Self::default();
        s.prefs = prefs;
        s.fill_color = fill_color;
        s.lua_console.visible = console_visible;
        s
    }

    /// Start with the welcome screen shown (used when no file is given on the CLI).
    pub fn new_with_welcome() -> Self {
        let prefs = AppPreferences::load();
        let fill_color = prefs.default_fill_color;
        let console_visible = prefs.console_open_on_start;
        let mut s = Self {
            show_welcome: true,
            ..Self::default()
        };
        s.prefs = prefs;
        s.fill_color = fill_color;
        s.lua_console.visible = console_visible;
        s
    }

    /// Draw the full UI for one frame.
    ///
    /// Returns `true` if the document was modified this frame.
    pub fn draw(
        &mut self,
        ctx: &egui::Context,
        doc: &mut Document,
        view: &mut CanvasView,
        renderer: &mut PhotonicRenderer,
        mcp_running: bool,
        history: &mut CommandHistory,
    ) -> bool {
        let mut doc_modified = false;

        // ── Apply theme ───────────────────────────────────────────────────────
        if self.prefs.dark_mode {
            ctx.set_visuals(crate::theme::build_dark_theme());
        } else {
            ctx.set_visuals(crate::theme::build_light_theme());
        }
        ctx.set_pixels_per_point(self.prefs.ui_scale);

        // ── Welcome screen (shown before the editor on first launch) ─────────
        if self.show_welcome {
            if let Some(action) = self.welcome.draw(ctx) {
                use crate::welcome::WelcomeAction;
                match action {
                    WelcomeAction::CreateNew {
                        name,
                        width,
                        height,
                    } => {
                        *doc = photonic_core::Document::new(name, width, height);
                        self.current_file = None;
                        self.selected_id = None;
                        self.show_welcome = false;
                        doc_modified = true;
                    }
                    WelcomeAction::OpenFile(path) => match load_document(&path) {
                        Ok(loaded) => {
                            self.welcome.add_recent(path.clone(), loaded.name.clone());
                            *doc = loaded;
                            self.current_file = Some(path);
                            self.selected_id = None;
                            self.show_welcome = false;
                            doc_modified = true;
                        }
                        Err(e) => {
                            self.file_status = Some(format!("Open failed: {e}"));
                        }
                    },
                    WelcomeAction::OpenBrowse => {
                        if let Some(path) = run_file_dialog(|| {
                            rfd::FileDialog::new()
                                .add_filter("Photonic", &["photon"])
                                .add_filter("SVG", &["svg"])
                                .add_filter("All supported", &["photon", "svg"])
                                .pick_file()
                        }) {
                            match load_document(&path) {
                                Ok(loaded) => {
                                    self.welcome.add_recent(path.clone(), loaded.name.clone());
                                    *doc = loaded;
                                    self.current_file = Some(path);
                                    self.selected_id = None;
                                    self.show_welcome = false;
                                    doc_modified = true;
                                }
                                Err(e) => {
                                    self.file_status = Some(format!("Open failed: {e}"));
                                }
                            }
                        }
                    }
                }
            }
            return doc_modified;
        }

        // ── Export modal ─────────────────────────────────────────────────────
        self.draw_export_modal(ctx, doc);

        // ── Simplify Path dialog ──────────────────────────────────────────────
        self.draw_simplify_dialog(ctx, doc, history);

        // ── Find / Replace Text dialog ────────────────────────────────────────
        self.draw_find_replace_text_dialog(ctx, doc, history);

        // ── Top toolbar ──────────────────────────────────────────────────────
        let toolbar_resp = egui::TopBottomPanel::top("toolbar")
            .frame(
                egui::Frame::side_top_panel(&ctx.style())
                    .inner_margin(egui::Margin::symmetric(8.0, 6.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // File toggle button — opens/closes the File drawer
                    let file_active = self.active_drawer == Some(DrawerKind::File);
                    if ui.selectable_label(file_active, "File").clicked() {
                        self.active_drawer = if file_active {
                            None
                        } else {
                            Some(DrawerKind::File)
                        };
                        self.selected_drawer_option = None;
                    }

                    // Edit toggle button — opens/closes the Preferences drawer
                    let edit_active = self.active_drawer == Some(DrawerKind::Edit);
                    if ui.selectable_label(edit_active, "Edit").clicked() {
                        if edit_active {
                            self.prefs.save();
                            self.active_drawer = None;
                        } else {
                            self.active_drawer = Some(DrawerKind::Edit);
                        }
                        self.selected_drawer_option = None;
                    }

                    // Tools menu — lists all tools, lets user pin them to the sidebar
                    let tools_active = self.active_drawer == Some(DrawerKind::Tools);
                    if ui.selectable_label(tools_active, "Tools").clicked() {
                        self.active_drawer = if tools_active {
                            None
                        } else {
                            Some(DrawerKind::Tools)
                        };
                        self.selected_drawer_option = None;
                    }

                    // Audit log toggle
                    if ui
                        .selectable_label(self.audit.panel_open, "Audit")
                        .clicked()
                    {
                        self.audit.panel_open = !self.audit.panel_open;
                    }

                    // Diff overlay clear button (only visible when a diff is active)
                    if self.diff.overlay_active {
                        ui.separator();
                        if ui
                            .button(
                                RichText::new("✕ Clear Diff")
                                    .small()
                                    .color(Color32::from_rgb(239, 68, 68)),
                            )
                            .on_hover_text("Clear diff highlights")
                            .clicked()
                        {
                            self.pending_panel_actions.push(PanelAction::ClearDiff);
                        }
                    }

                    ui.separator();
                    panels::draw_toolbar(ui, &doc.name, view.zoom);

                    // Show file status message on the right
                    if let Some(status) = &self.file_status {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new(status).weak().italics());
                        });
                    }
                });
            });

        // Close drawer on Escape
        if viewport_kb(ctx) && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.active_drawer == Some(DrawerKind::Edit) {
                self.prefs.save();
            }
            self.active_drawer = None;
            self.selected_drawer_option = None;
        }

        // ── Menu drawer (floating overlay) ────────────────────────────────────
        if let Some(drawer_kind) = self.active_drawer {
            let screen = ctx.screen_rect();
            let toolbar_bottom = toolbar_resp.response.rect.bottom();
            let drawer_height = (screen.height() * 0.6).max(300.0);
            let content_height = drawer_height - 24.0; // subtract Frame::popup inner_margin (12 * 2)
            let drawer_width = screen.width();

            let drawer_resp = egui::Area::new(egui::Id::new("menu_drawer"))
                .fixed_pos(egui::pos2(0.0, toolbar_bottom))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    // Bound the Area width so horizontal_wrapped has a wrap point.
                    ui.set_width(drawer_width);
                    egui::Frame::popup(&ctx.style())
                        .inner_margin(egui::Margin::same(12.0))
                        .show(ui, |ui| {
                    match drawer_kind {
                        DrawerKind::File => {
                            // ── File drawer ───────────────────────────────────
                            let new_sel = draw_two_column_menu(
                                ui, 160.0, content_height, FILE_OPTIONS,
                                self.selected_drawer_option,
                                |ui, selected| match selected {
                                    None => {
                                        ui.add_space(40.0);
                                        ui.vertical_centered(|ui| {
                                            ui.label(egui::RichText::new("Select an option").weak());
                                        });
                                    }
                                    Some(0) => {
                                        ui.label(RichText::new("Document").strong());
                                        ui.add_space(4.0);
                                        if ui.button("  New  ").clicked() {
                                            *doc = Document::default_artboard();
                                            self.current_file = None;
                                            self.selected_id = None;
                                            self.file_status = Some("New document".into());
                                            self.active_drawer = None;
                                            self.selected_drawer_option = None;
                                        }
                                        if ui.button("  Open…  ").clicked() {
                                            self.active_drawer = None;
                                            self.selected_drawer_option = None;
                                            if let Some(path) = run_file_dialog(|| {
                                                rfd::FileDialog::new()
                                                    .add_filter("Photonic", &["photon"])
                                                    .add_filter("SVG", &["svg"])
                                                    .add_filter("All supported", &["photon", "svg"])
                                                    .pick_file()
                                            }) {
                                                match load_document(&path) {
                                                    Ok(loaded) => {
                                                        self.welcome.add_recent(path.clone(), loaded.name.clone());
                                                        *doc = loaded;
                                                        self.selected_id = None;
                                                        doc_modified = true;
                                                        self.file_status = Some(format!("Opened {}", path.file_name().unwrap_or_default().to_string_lossy()));
                                                        self.current_file = Some(path);
                                                    }
                                                    Err(e) => self.file_status = Some(format!("Open failed: {e}")),
                                                }
                                            }
                                        }
                                    }
                                    Some(1) => {
                                        ui.label(RichText::new("Save").strong());
                                        ui.add_space(4.0);
                                        let can_save = self.current_file.is_some();
                                        if ui.add_enabled(can_save, egui::Button::new("  Save  ")).clicked() {
                                            self.active_drawer = None;
                                            self.selected_drawer_option = None;
                                            if let Some(path) = &self.current_file.clone() {
                                                match doc.to_json() {
                                                    Ok(json) => match std::fs::write(path, &json) {
                                                        Ok(_) => {
                                                            self.welcome.add_recent(path.clone(), doc.name.clone());
                                                            self.file_status = Some(format!("Saved {}", path.file_name().unwrap_or_default().to_string_lossy()));
                                                        }
                                                        Err(e) => self.file_status = Some(format!("Save failed: {e}")),
                                                    },
                                                    Err(e) => self.file_status = Some(format!("Serialize failed: {e}")),
                                                }
                                            }
                                        }
                                        if ui.button("  Save As…  ").clicked() {
                                            self.active_drawer = None;
                                            self.selected_drawer_option = None;
                                            let default_name = self.current_file.as_ref()
                                                .and_then(|p| p.file_name())
                                                .map(|n| n.to_string_lossy().into_owned())
                                                .unwrap_or_else(|| format!("{}.photon", doc.name));
                                            let start_dir = self.current_file.as_ref()
                                                .and_then(|p| p.parent())
                                                .map(|p| p.to_path_buf());
                                            let mut dialog = rfd::FileDialog::new()
                                                .add_filter("Photonic", &["photon"])
                                                .set_file_name(&default_name);
                                            if let Some(dir) = start_dir {
                                                dialog = dialog.set_directory(dir);
                                            }
                                            if let Some(path) = run_file_dialog(move || dialog.save_file()) {
                                                let path = if path.extension().is_none() {
                                                    path.with_extension("photon")
                                                } else { path };
                                                match doc.to_json() {
                                                    Ok(json) => match std::fs::write(&path, &json) {
                                                        Ok(_) => {
                                                            self.welcome.add_recent(path.clone(), doc.name.clone());
                                                            self.file_status = Some(format!("Saved {}", path.file_name().unwrap_or_default().to_string_lossy()));
                                                            self.current_file = Some(path);
                                                        }
                                                        Err(e) => self.file_status = Some(format!("Save failed: {e}")),
                                                    },
                                                    Err(e) => self.file_status = Some(format!("Serialize failed: {e}")),
                                                }
                                            }
                                        }
                                    }
                                    Some(2) => {
                                        ui.label(RichText::new("Export").strong());
                                        ui.add_space(4.0);
                                        if ui.button("  Export…  ").clicked() {
                                            self.active_drawer = None;
                                            self.selected_drawer_option = None;
                                            self.export_dialog = Some(ExportDialog::new(doc));
                                        }
                                    }
                                    _ => {}
                                },
                            );
                            self.selected_drawer_option = new_sel;
                        }

                        DrawerKind::Edit => {
                            // ── Preferences drawer ────────────────────────────
                            let new_sel = draw_two_column_menu(
                                ui, 160.0, content_height, EDIT_OPTIONS,
                                self.selected_drawer_option,
                                |ui, selected| match selected {
                                    None => {
                                        ui.add_space(40.0);
                                        ui.vertical_centered(|ui| {
                                            ui.label(egui::RichText::new("Select an option").weak());
                                        });
                                    }
                                    Some(0) => {
                                        ui.label(RichText::new("Appearance").strong());
                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Theme");
                                            ui.add_space(4.0);
                                            if ui.selectable_label(self.prefs.dark_mode, format!("{} Dark", ph::MOON)).clicked() {
                                                self.prefs.dark_mode = true;
                                            }
                                            if ui.selectable_label(!self.prefs.dark_mode, format!("{} Light", ph::SUN)).clicked() {
                                                self.prefs.dark_mode = false;
                                            }
                                        });
                                        ui.horizontal(|ui| {
                                            ui.label("UI Scale");
                                            egui::ComboBox::from_id_salt("ui_scale")
                                                .selected_text(format!("{}%", (self.prefs.ui_scale * 100.0) as u32))
                                                .show_ui(ui, |ui| {
                                                    for &scale in &[0.75f32, 1.0, 1.25, 1.5, 2.0] {
                                                        ui.selectable_value(
                                                            &mut self.prefs.ui_scale,
                                                            scale,
                                                            format!("{}%", (scale * 100.0) as u32),
                                                        );
                                                    }
                                                });
                                        });
                                    }
                                    Some(1) => {
                                        ui.label(RichText::new("Canvas").strong());
                                        ui.add_space(4.0);
                                        ui.checkbox(&mut self.prefs.show_grid, "Show Grid");
                                        ui.add_enabled_ui(self.prefs.show_grid, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.label("Grid Size");
                                                egui::ComboBox::from_id_salt("grid_size")
                                                    .selected_text(format!("{}px", self.prefs.grid_size))
                                                    .show_ui(ui, |ui| {
                                                        for size in [8u32, 16, 32, 64] {
                                                            ui.selectable_value(
                                                                &mut self.prefs.grid_size,
                                                                size,
                                                                format!("{}px", size),
                                                            );
                                                        }
                                                    });
                                            });
                                            ui.horizontal(|ui| {
                                                ui.label("Grid Color");
                                                ui.color_edit_button_rgba_unmultiplied(&mut self.prefs.grid_color);
                                            });
                                            ui.checkbox(&mut self.prefs.snap_to_grid, "Snap to Grid");
                                        });
                                        ui.checkbox(&mut self.prefs.show_rulers, "Show Rulers");
                                        ui.checkbox(&mut self.outline_mode, "Outline Mode")
                                            .on_hover_text("Show path wireframes only (no fills or strokes). Shortcut: Ctrl+Y");
                                        ui.separator();
                                        ui.label(egui::RichText::new("Guides").strong());
                                        ui.checkbox(&mut self.guides_visible, "Show Guides")
                                            .on_hover_text("Show/hide ruler guides on the canvas. Shortcut: Ctrl+;");
                                        let guide_count = doc.guides.len();
                                        ui.add_enabled_ui(guide_count > 0, |ui| {
                                            if ui.button(format!("Clear All Guides ({})", guide_count)).clicked() {
                                                self.pending_panel_actions.push(panels::PanelAction::ClearGuides);
                                            }
                                        });
                                    }
                                    Some(2) => {
                                        ui.label(RichText::new("Tool Defaults").strong());
                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Default Fill");
                                            if ui.color_edit_button_rgba_unmultiplied(&mut self.prefs.default_fill_color).changed() {
                                                self.fill_color = self.prefs.default_fill_color;
                                            }
                                        });
                                        ui.checkbox(&mut self.prefs.default_stroke_enabled, "Default Stroke");
                                        ui.add_enabled_ui(self.prefs.default_stroke_enabled, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.label("Stroke Color");
                                                ui.color_edit_button_rgba_unmultiplied(&mut self.prefs.default_stroke_color);
                                            });
                                            ui.horizontal(|ui| {
                                                ui.label("Stroke Width");
                                                ui.add(
                                                    egui::Slider::new(&mut self.prefs.default_stroke_width, 0.5..=32.0)
                                                        .suffix(" px"),
                                                );
                                            });
                                        });
                                    }
                                    Some(3) => {
                                        ui.label(RichText::new("Behavior").strong());
                                        ui.add_space(4.0);
                                        ui.checkbox(&mut self.prefs.console_open_on_start, "Open Console on Start");
                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Arrow nudge (px):");
                                            ui.add(egui::DragValue::new(&mut self.prefs.nudge_distance)
                                                .speed(0.1)
                                                .range(0.1..=100.0)
                                                .fixed_decimals(1))
                                                .on_hover_text("Distance moved per arrow key press (Shift×10)");
                                        });
                                    }
                                    _ => {}
                                },
                            );
                            self.selected_drawer_option = new_sel;
                        }

                        DrawerKind::Tools => {
                            // ── Tools drawer ─────────────────────────────────
                            ui.label(
                                RichText::new("TOOLS")
                                    .small()
                                    .color(Color32::from_rgb(80, 80, 110)),
                            );
                            ui.add_space(4.0);

                            const TOOL_CATEGORIES: &[(&str, &[Tool])] = &[
                                ("Selection & Navigation", &[Tool::Select, Tool::DirectSelect, Tool::Pan]),
                                ("Shapes", &[Tool::Rectangle, Tool::RoundedRect, Tool::Ellipse, Tool::Arc, Tool::Polygon, Tool::Star, Tool::Line, Tool::Grid, Tool::PolarGrid]),
                                ("Drawing & Text", &[Tool::Pen, Tool::ShapeBuilder, Tool::Text]),
                                ("Path Editing", &[Tool::Scissors, Tool::MagicWand, Tool::Lasso, Tool::Pencil, Tool::Smooth]),
                            ];

                            let mut tool_to_activate: Option<Tool> = None;
                            let mut pin_toggle: Option<Tool> = None;

                            egui::ScrollArea::vertical()
                                .id_salt("tools_drawer_scroll")
                                .max_height(content_height)
                                .show(ui, |ui| {
                                    ui.set_min_width(360.0);
                                    for (category, tools) in TOOL_CATEGORIES {
                                        ui.label(
                                            RichText::new(*category)
                                                .small()
                                                .color(Color32::from_rgb(110, 110, 150)),
                                        );
                                        ui.add_space(2.0);
                                        for tool in *tools {
                                            ui.horizontal(|ui| {
                                                let is_active = self.active_tool == *tool;
                                                let pinned = self.prefs.pinned_tools.contains(tool);

                                                let pin_color = if pinned {
                                                    Color32::from_rgb(110, 86, 207)
                                                } else {
                                                    Color32::from_gray(90)
                                                };
                                                let pin_hint = if pinned {
                                                    "Remove from sidebar hotbar"
                                                } else {
                                                    "Pin to sidebar hotbar"
                                                };
                                                if ui
                                                    .button(
                                                        RichText::new(egui_phosphor::regular::PUSH_PIN)
                                                            .color(pin_color),
                                                    )
                                                    .on_hover_text(pin_hint)
                                                    .clicked()
                                                {
                                                    pin_toggle = Some(*tool);
                                                }

                                                let tool_label = format!(
                                                    "{}  {}  —  {}",
                                                    tool.icon(),
                                                    tool.label(),
                                                    tool.description()
                                                );
                                                if ui.selectable_label(is_active, tool_label).clicked() {
                                                    tool_to_activate = Some(*tool);
                                                }
                                            });
                                        }
                                        ui.add_space(6.0);
                                    }

                                    if !self.prefs.pinned_tools.is_empty() {
                                        ui.separator();
                                        let pinned_names = self.prefs.pinned_tools
                                            .iter()
                                            .map(|t| t.label())
                                            .collect::<Vec<_>>()
                                            .join(", ");
                                        ui.label(
                                            RichText::new(format!(
                                                "{} Sidebar hotbar: {}",
                                                egui_phosphor::regular::PUSH_PIN,
                                                pinned_names
                                            ))
                                            .weak()
                                            .small(),
                                        );
                                    }
                                });

                            if let Some(tool) = pin_toggle {
                                if self.prefs.pinned_tools.contains(&tool) {
                                    self.prefs.pinned_tools.retain(|t| *t != tool);
                                } else {
                                    self.prefs.pinned_tools.push(tool);
                                }
                                self.prefs.save();
                            }
                            if let Some(tool) = tool_to_activate {
                                self.pen_points.clear();
                                self.pencil_points.clear();
                                self.lasso_points.clear();
                                self.isolated_group = None;
                                self.point_edit_node = None;
                                self.point_selected.clear();
                                self.point_drag_origin = None;
                                self.active_tool = tool;
                                self.active_drawer = None;
                                self.selected_drawer_option = None;
                            }
                        }
                    } // match
                        }); // Frame::popup
                }); // Area inner

            // Close when the user clicks outside the drawer.
            // Also exclude the toolbar so the toggle buttons can handle their own state.
            if ctx.input(|i| i.pointer.any_click()) {
                let clicked_inside = ctx
                    .input(|i| i.pointer.interact_pos())
                    .map(|pos| {
                        drawer_resp.response.rect.contains(pos)
                            || toolbar_resp.response.rect.contains(pos)
                    })
                    .unwrap_or(false);
                if !clicked_inside {
                    if self.active_drawer == Some(DrawerKind::Edit) {
                        self.prefs.save();
                    }
                    self.active_drawer = None;
                    self.selected_drawer_option = None;
                }
            }
        }

        // ── Bottom status bar ────────────────────────────────────────────────
        egui::TopBottomPanel::bottom("statusbar")
            .frame(egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin::symmetric(8.0, 4.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Photonic v0.1").weak());
                    // Isolation Mode indicator.
                    if let Some(iso_id) = self.isolated_group {
                        ui.separator();
                        let name = doc.nodes.get(&iso_id).map(|n| n.name.as_str()).unwrap_or("Group");
                        ui.label(RichText::new(format!("Isolation: {}  (Esc to exit)", name))
                            .color(egui::Color32::from_rgb(80, 160, 255))
                            .strong());
                    }
                    ui.separator();
                    let sel_info = self.selected_id
                        .and_then(|id| doc.nodes.get(&id))
                        .map(|n| format!("  •  \"{}\" selected", n.name))
                        .unwrap_or_default();
                    ui.label(format!(
                        "{} {}  •  {} objects{}  •  {:.0}%",
                        self.active_tool.icon(),
                        self.active_tool.label(),
                        doc.node_count(),
                        sel_info,
                        view.zoom * 100.0,
                    ));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if mcp_running {
                            ui.label(RichText::new("MCP :7842 ✓").color(Color32::from_rgb(52, 211, 153)));
                        } else {
                            ui.label(RichText::new("MCP offline ✗").color(Color32::from_rgb(248, 113, 113)))
                                .on_hover_text("MCP server failed to bind — another Photonic instance may be running on port 7842");
                        }
                        ui.separator();
                        // Console toggle
                        let label = if self.lua_console.visible {
                            format!("{} Hide Console", ph::TERMINAL)
                        } else {
                            format!("{} Console", ph::TERMINAL)
                        };
                        if ui.selectable_label(self.lua_console.visible, label).clicked() {
                            self.lua_console.visible = !self.lua_console.visible;
                        }
                    });
                });
            });

        // ── Left tools panel ─────────────────────────────────────────────────
        egui::SidePanel::left("tools")
            .default_width(180.0)
            .min_width(140.0)
            .show(ctx, |ui| {
                if let Some(tool) =
                    panels::draw_tools_panel(ui, self.active_tool, &self.prefs.pinned_tools)
                {
                    self.pen_points.clear();
                    self.pencil_points.clear();
                    self.lasso_points.clear();
                    self.isolated_group = None;
                    self.point_edit_node = None;
                    self.point_selected.clear();
                    self.point_drag_origin = None;
                    self.active_tool = tool;
                    if tool != Tool::Select && tool != Tool::DirectSelect {
                        self.selected_id = None;
                        doc.selection.clear();
                    }
                }
            });

        // ── Properties panel (below tools, separate panel) ────────────────────
        egui::SidePanel::left("properties")
            .default_width(220.0)
            .min_width(160.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let selected_node = self.selected_id.and_then(|id| doc.nodes.get(&id));
                    let selection_count = doc.selection.node_ids.len();
                    if let Some(action) = panels::draw_properties_panel(
                        ui,
                        doc,
                        self.active_tool,
                        &mut self.fill_color,
                        &mut self.polygon_sides,
                        &mut self.star_points,
                        &mut self.star_inner_ratio,
                        &mut self.rounded_rect_radius,
                        &mut self.spiral_turns,
                        &mut self.spiral_inner_radius,
                        &mut self.spiral_segs_per_turn,
                        selected_node,
                        self.selected_id,
                        selection_count,
                        &doc.selection.node_ids.iter().cloned().collect::<Vec<_>>(),
                        &mut self.prop_search,
                        &mut self.shear_x,
                        &mut self.shear_y,
                        &mut self.line_snap_45,
                        &mut self.color_guide_rule,
                        &mut self.arc_start_angle,
                        &mut self.arc_end_angle,
                        &mut self.arc_open,
                        &mut self.grid_cols,
                        &mut self.grid_rows,
                        &mut self.polar_grid_rings,
                        &mut self.polar_grid_sectors,
                        &mut self.polar_grid_inner_ratio,
                        &mut self.recolor_palette_input,
                        &mut self.magic_wand_attribute,
                        &mut self.magic_wand_tolerance,
                        &self.composition_findings,
                        &self.rhythm_findings,
                        &self.branch_names.clone(),
                        &mut self.branch_name_input,
                        &mut self.swatch_library_selected,
                        &mut self.graphic_style_name_input,
                        &mut self.width_profile_name_input,
                        &self.grammar_rules,
                        &mut self.grammar_rule_name_input,
                        &mut self.grammar_rule_type_selected,
                        &mut self.grammar_rule_params_input,
                        &self.grammar_check_results,
                        &self.distance_results,
                        &self.action_names,
                        &self.history_entries,
                        history.undo_depth(),
                        &mut self.bleed_mm_input,
                        &mut self.slug_mm_input,
                        &mut self.construction_angle,
                        &mut self.construction_x,
                        &mut self.construction_y,
                        &mut self.margin_top_input,
                        &mut self.margin_right_input,
                        &mut self.margin_bottom_input,
                        &mut self.margin_left_input,
                        &mut self.event_trigger_event,
                        &mut self.event_trigger_action,
                        &mut self.workspace_name_input,
                    ) {
                        self.pending_panel_actions.push(action);
                    }
                });
            });

        // ── Right panel: layers + change log + AI chat ──────────────────────
        egui::SidePanel::right("right_panel")
            .default_width(280.0)
            .min_width(220.0)
            .max_width(400.0)
            .show(ctx, |ui| {
                let total_h = ui.available_height();
                let changelog_h = (total_h * 0.38).max(120.0).min(total_h - 330.0);

                // ── Layers panel (top) ────────────────────────────────────────
                egui::ScrollArea::vertical()
                    .id_salt("layers_scroll")
                    .max_height(150.0)
                    .show(ui, |ui| {
                        if let Some(action) =
                            panels::draw_layers_panel(ui, doc, &mut self.selected_layer_ids)
                        {
                            self.pending_panel_actions.push(action);
                        }
                    });

                ui.separator();

                // ── Change log (middle) ───────────────────────────────────────
                let checkpoints = history.list_checkpoints();
                if let Some(action) = panels::draw_changelog_panel(ui, &checkpoints, changelog_h) {
                    self.pending_panel_actions.push(action);
                }

                ui.separator();

                // ── AI chat (bottom) ─────────────────────────────────────────
                self.draw_claude_tab(ui);
            });

        // ── Console panel ────────────────────────────────────────────────────
        // Changing the panel ID when toggling expanded forces egui to reset
        // its stored height to the new default_height.
        let (panel_id, default_h, min_h) = if self.lua_console.expanded {
            ("console_expanded", 480.0, 300.0)
        } else {
            ("console", 220.0, 120.0)
        };
        egui::TopBottomPanel::bottom(panel_id)
            .resizable(true)
            .default_height(default_h)
            .min_height(min_h)
            .show_animated(ctx, self.lua_console.visible, |ui| {
                self.draw_console(ui);
            });

        // ── Audit panel (floating window) ────────────────────────────────────
        if self.audit.panel_open {
            panels::draw_audit_panel(
                ctx,
                &self.audit.log,
                &mut self.audit.panel_open,
                &mut self.audit.filter,
            );
        }

        // ── Central canvas area ──────────────────────────────────────────────
        egui::CentralPanel::default()
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                let rect = ui.available_rect_before_wrap();
                let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());

                // ── Cursor coordinate overlay (Info Panel) ───────────────────
                if let Some(cursor_screen) = ui.input(|i| i.pointer.hover_pos()) {
                    if rect.contains(cursor_screen) {
                        let (cx, cy) =
                            view.screen_to_canvas(cursor_screen.x as f64, cursor_screen.y as f64);
                        let coord_text = format!("  X: {:.1}  Y: {:.1}  ", cx, cy);
                        let fg_painter = ctx.layer_painter(egui::LayerId::new(
                            egui::Order::Foreground,
                            egui::Id::new("cursor_coords_overlay"),
                        ));
                        let text_color = if self.prefs.dark_mode {
                            egui::Color32::from_rgba_unmultiplied(220, 220, 220, 200)
                        } else {
                            egui::Color32::from_rgba_unmultiplied(30, 30, 30, 200)
                        };
                        let bg_color = if self.prefs.dark_mode {
                            egui::Color32::from_rgba_unmultiplied(20, 20, 30, 160)
                        } else {
                            egui::Color32::from_rgba_unmultiplied(240, 240, 250, 160)
                        };
                        let font = egui::FontId::monospace(11.0);
                        let text_pos = rect.min + egui::vec2(4.0, rect.height() - 20.0);
                        let galley = ctx.fonts(|f| f.layout_no_wrap(coord_text, font, text_color));
                        let text_rect = egui::Rect::from_min_size(text_pos, galley.size());
                        fg_painter.rect_filled(text_rect.expand(2.0), 2.0, bg_color);
                        fg_painter.galley(text_pos, galley, text_color);
                    }
                }

                // ── Outline Mode overlay ──────────────────────────────────────
                // Cover GPU-rendered geometry with a flat background, then draw
                // all visible path nodes as 1 px wireframe strokes.
                if self.outline_mode {
                    let painter = ui.painter_at(rect);
                    let bg = if self.prefs.dark_mode {
                        egui::Color32::from_rgb(28, 28, 40)
                    } else {
                        egui::Color32::WHITE
                    };
                    painter.rect_filled(rect, 0.0, bg);

                    // Draw artboard boundary.
                    let (ax0, ay0) = view.canvas_to_screen(0.0, 0.0);
                    let (ax1, ay1) = view.canvas_to_screen(doc.width, doc.height);
                    painter.rect_stroke(
                        egui::Rect::from_min_max(
                            egui::pos2(ax0 as f32, ay0 as f32),
                            egui::pos2(ax1 as f32, ay1 as f32),
                        ),
                        0.0,
                        egui::Stroke::new(1.0, egui::Color32::from_gray(128)),
                    );

                    // Draw each visible path node as a 1 px wireframe.
                    let outline_color = if self.prefs.dark_mode {
                        egui::Color32::from_rgb(180, 180, 210)
                    } else {
                        egui::Color32::from_rgb(30, 30, 60)
                    };
                    let outline_stroke = egui::Stroke::new(1.0, outline_color);
                    for node in doc.nodes.values() {
                        if !node.visible {
                            continue;
                        }
                        if let SceneNodeKind::Path(pn) = &node.kind {
                            let pts = bez_to_screen_points_xf(
                                &pn.path_data.to_bez_path(),
                                view,
                                &node.transform,
                            );
                            if pts.len() >= 2 {
                                painter.add(egui::Shape::line(pts, outline_stroke));
                            }
                        }
                    }
                }

                // ── Grid overlay ─────────────────────────────────────────────
                if self.prefs.show_grid {
                    let grid_screen_size = self.prefs.grid_size as f64 * view.zoom;
                    if grid_screen_size >= 4.0 {
                        let painter = ui.painter_at(rect);
                        let [gr, gg, gb, ga] = self.prefs.grid_color;
                        let color =
                            egui::Color32::from(egui::Rgba::from_rgba_unmultiplied(gr, gg, gb, ga));
                        let stroke = egui::Stroke::new(1.0, color);
                        let g = self.prefs.grid_size as f64;
                        let (cx0, cy0) =
                            view.screen_to_canvas(rect.min.x as f64, rect.min.y as f64);
                        let (cx1, cy1) =
                            view.screen_to_canvas(rect.max.x as f64, rect.max.y as f64);
                        // Vertical lines
                        let mut cx = (cx0 / g).floor() * g;
                        while cx <= cx1 {
                            cx += g;
                            let (sx, _) = view.canvas_to_screen(cx, 0.0);
                            painter.line_segment(
                                [
                                    egui::pos2(sx as f32, rect.min.y),
                                    egui::pos2(sx as f32, rect.max.y),
                                ],
                                stroke,
                            );
                        }
                        // Horizontal lines
                        let mut cy = (cy0 / g).floor() * g;
                        while cy <= cy1 {
                            cy += g;
                            let (_, sy) = view.canvas_to_screen(0.0, cy);
                            painter.line_segment(
                                [
                                    egui::pos2(rect.min.x, sy as f32),
                                    egui::pos2(rect.max.x, sy as f32),
                                ],
                                stroke,
                            );
                        }
                    }
                }

                // ── Ruler strips ─────────────────────────────────────────────
                if self.prefs.show_rulers {
                    let painter = ui.painter_at(rect);
                    let ruler_h = 18.0f32;
                    let bg = if self.prefs.dark_mode {
                        egui::Color32::from_rgb(19, 19, 31)
                    } else {
                        egui::Color32::from_rgb(234, 228, 255)
                    };
                    let tick_col = egui::Color32::from_gray(140);
                    // Ruler backgrounds
                    painter.rect_filled(
                        egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), ruler_h)),
                        0.0,
                        bg,
                    );
                    painter.rect_filled(
                        egui::Rect::from_min_size(rect.min, egui::vec2(ruler_h, rect.height())),
                        0.0,
                        bg,
                    );
                    // Choose tick interval to keep ticks ~50px apart on screen
                    let raw = 50.0 / view.zoom;
                    let mag = 10.0f64.powf(raw.log10().floor());
                    let n = raw / mag;
                    let tick = if n < 2.0 {
                        mag
                    } else if n < 5.0 {
                        2.0 * mag
                    } else {
                        5.0 * mag
                    };
                    // Horizontal ruler ticks
                    let (cx0, _) = view.screen_to_canvas(rect.min.x as f64, 0.0);
                    let (cx1, _) = view.screen_to_canvas(rect.max.x as f64, 0.0);
                    let mut c = (cx0 / tick).floor() * tick;
                    while c <= cx1 {
                        let (sx, _) = view.canvas_to_screen(c, 0.0);
                        let sx = sx as f32;
                        if sx > rect.min.x + ruler_h {
                            painter.line_segment(
                                [
                                    egui::pos2(sx, rect.min.y + ruler_h - 5.0),
                                    egui::pos2(sx, rect.min.y + ruler_h),
                                ],
                                egui::Stroke::new(1.0, tick_col),
                            );
                            painter.text(
                                egui::pos2(sx + 2.0, rect.min.y + 2.0),
                                egui::Align2::LEFT_TOP,
                                format!("{}", c as i64),
                                egui::FontId::proportional(9.0),
                                tick_col,
                            );
                        }
                        c += tick;
                    }
                    // Vertical ruler ticks
                    let (_, cy0) = view.screen_to_canvas(0.0, rect.min.y as f64);
                    let (_, cy1) = view.screen_to_canvas(0.0, rect.max.y as f64);
                    let mut c = (cy0 / tick).floor() * tick;
                    while c <= cy1 {
                        let (_, sy) = view.canvas_to_screen(0.0, c);
                        let sy = sy as f32;
                        if sy > rect.min.y + ruler_h {
                            painter.line_segment(
                                [
                                    egui::pos2(rect.min.x + ruler_h - 5.0, sy),
                                    egui::pos2(rect.min.x + ruler_h, sy),
                                ],
                                egui::Stroke::new(1.0, tick_col),
                            );
                        }
                        c += tick;
                    }
                }

                // ── Guide overlay ─────────────────────────────────────────────
                // Render horizontal/vertical guide lines across the canvas.
                if self.guides_visible && !doc.guides.is_empty() {
                    let painter = ui.painter_at(rect);
                    for guide in &doc.guides {
                        let default_color = egui::Color32::from_rgba_unmultiplied(0, 200, 200, 180);
                        let color = guide
                            .color
                            .map(|[r, g, b, a]| {
                                egui::Color32::from_rgba_unmultiplied(
                                    (r * 255.0) as u8,
                                    (g * 255.0) as u8,
                                    (b * 255.0) as u8,
                                    (a * 255.0) as u8,
                                )
                            })
                            .unwrap_or(default_color);
                        let stroke = egui::Stroke::new(1.0, color);
                        if let Some(angle_deg) = guide.angle_degrees {
                            // Angled construction line: draw through (position_x, position_y) at given angle.
                            let (ox, oy) =
                                view.canvas_to_screen(guide.position_x, guide.position_y);
                            let angle_rad = angle_deg.to_radians();
                            let cos_a = angle_rad.cos() as f32;
                            let sin_a = angle_rad.sin() as f32;
                            // Extend far enough to reach any screen edge.
                            let ext = (rect.width() + rect.height()) * 2.0;
                            let p1 = egui::pos2(ox as f32 - cos_a * ext, oy as f32 - sin_a * ext);
                            let p2 = egui::pos2(ox as f32 + cos_a * ext, oy as f32 + sin_a * ext);
                            painter.line_segment([p1, p2], stroke);
                        } else {
                            match guide.orientation {
                                photonic_core::GuideOrientation::Horizontal => {
                                    let (_, sy) = view.canvas_to_screen(0.0, guide.position);
                                    let sy = sy as f32;
                                    if sy >= rect.min.y && sy <= rect.max.y {
                                        painter.line_segment(
                                            [
                                                egui::pos2(rect.min.x, sy),
                                                egui::pos2(rect.max.x, sy),
                                            ],
                                            stroke,
                                        );
                                    }
                                }
                                photonic_core::GuideOrientation::Vertical => {
                                    let (sx, _) = view.canvas_to_screen(guide.position, 0.0);
                                    let sx = sx as f32;
                                    if sx >= rect.min.x && sx <= rect.max.x {
                                        painter.line_segment(
                                            [
                                                egui::pos2(sx, rect.min.y),
                                                egui::pos2(sx, rect.max.y),
                                            ],
                                            stroke,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }

                // ── Artboard margin overlay ───────────────────────────────────
                if self.guides_visible
                    && (doc.margin_top > 0.0
                        || doc.margin_right > 0.0
                        || doc.margin_bottom > 0.0
                        || doc.margin_left > 0.0)
                {
                    let margin_painter = ui.painter_at(rect);
                    let margin_stroke = egui::Stroke::new(
                        1.0,
                        egui::Color32::from_rgba_unmultiplied(100, 180, 255, 120),
                    );
                    let (ax0, ay0) = view.canvas_to_screen(0.0, 0.0);
                    let (ax1, ay1) = view.canvas_to_screen(doc.width, doc.height);
                    let mx0 = (ax0 + doc.margin_left * view.zoom) as f32;
                    let mx1 = (ax1 - doc.margin_right * view.zoom) as f32;
                    let my0 = (ay0 + doc.margin_top * view.zoom) as f32;
                    let my1 = (ay1 - doc.margin_bottom * view.zoom) as f32;
                    if mx0 < mx1 && my0 < my1 {
                        margin_painter.rect_stroke(
                            egui::Rect::from_min_max(egui::pos2(mx0, my0), egui::pos2(mx1, my1)),
                            0.0,
                            margin_stroke,
                        );
                    }
                }

                // ── Dimension annotation overlay ──────────────────────────────
                if self.guides_visible && !doc.dimensions.is_empty() {
                    let dim_painter = ui.painter_at(rect);
                    let dim_color = egui::Color32::from_rgba_unmultiplied(255, 160, 40, 220);
                    let dim_stroke = egui::Stroke::new(1.5, dim_color);
                    for dim in &doc.dimensions {
                        let (sx0, sy0) = view.canvas_to_screen(dim.from_x, dim.from_y);
                        let (sx1, sy1) = view.canvas_to_screen(dim.to_x, dim.to_y);

                        // Perpendicular unit for offset
                        let dx = (sx1 - sx0) as f32;
                        let dy = (sy1 - sy0) as f32;
                        let len = (dx * dx + dy * dy).sqrt().max(1.0);
                        let offset_px = (dim.label_offset * view.zoom) as f32;
                        let perp_x = -dy / len * offset_px;
                        let perp_y = dx / len * offset_px;

                        let p0 = egui::pos2(sx0 as f32 + perp_x, sy0 as f32 + perp_y);
                        let p1 = egui::pos2(sx1 as f32 + perp_x, sy1 as f32 + perp_y);

                        // Main dimension line
                        dim_painter.line_segment([p0, p1], dim_stroke);

                        // Extension tick marks
                        let tick = 6.0_f32;
                        let ux = dx / len;
                        let uy = dy / len;
                        dim_painter.line_segment(
                            [
                                egui::pos2(p0.x - uy * tick, p0.y + ux * tick),
                                egui::pos2(p0.x + uy * tick, p0.y - ux * tick),
                            ],
                            dim_stroke,
                        );
                        dim_painter.line_segment(
                            [
                                egui::pos2(p1.x - uy * tick, p1.y + ux * tick),
                                egui::pos2(p1.x + uy * tick, p1.y - ux * tick),
                            ],
                            dim_stroke,
                        );

                        // Distance label at midpoint
                        let mid = egui::pos2((p0.x + p1.x) / 2.0, (p0.y + p1.y) / 2.0);
                        let dist_text = format!("{:.1}", dim.distance());
                        dim_painter.text(
                            mid + egui::vec2(-perp_x * 0.5 - 4.0, -perp_y * 0.5 - 8.0),
                            egui::Align2::CENTER_CENTER,
                            &dist_text,
                            egui::FontId::proportional(11.0),
                            dim_color,
                        );
                    }
                }

                // ── Right-click radial wheel ──────────────────────────────────
                if response.secondary_clicked() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let (cx, cy) = view.screen_to_canvas(pos.x as f64, pos.y as f64);
                        let hit = hit_test(doc, cx, cy, renderer);
                        let wheel_ctx = match hit {
                            Some(id)
                                if doc.selection.contains(&id) && doc.selection.count() > 1 =>
                            {
                                WheelContext::MultiNode {
                                    node_ids: doc.selection.ids().copied().collect(),
                                }
                            }
                            Some(id) => {
                                let kind = match doc.get_node(&id).map(|n| &n.kind) {
                                    Some(SceneNodeKind::Group(_)) => WheelNodeKind::Group,
                                    Some(SceneNodeKind::Text(_)) => WheelNodeKind::Text,
                                    _ => WheelNodeKind::Path,
                                };
                                WheelContext::SingleNode {
                                    node_id: id,
                                    node_kind: kind,
                                }
                            }
                            None if doc.selection.count() > 1 => WheelContext::MultiNode {
                                node_ids: doc.selection.ids().copied().collect(),
                            },
                            _ => WheelContext::EmptyCanvas {
                                canvas_x: cx,
                                canvas_y: cy,
                            },
                        };
                        self.radial_wheel = Some(WheelState::new(pos, (cx, cy), &wheel_ctx));
                    }
                }

                // Update wheel hover, paint overlay, and handle interaction.
                // This block runs before any early-return tool handlers so the
                // wheel is always rendered while open.
                if self.radial_wheel.is_some() {
                    // Scroll wheel cycles pages
                    let scroll_y = ui.input(|i| i.raw_scroll_delta.y);
                    if scroll_y != 0.0 {
                        if let Some(ref mut wheel) = self.radial_wheel {
                            if scroll_y > 0.0 {
                                wheel.prev_page();
                            } else {
                                wheel.next_page();
                            }
                        }
                    }

                    // Update hover position
                    if let Some(cursor) = ui.input(|i| i.pointer.hover_pos()) {
                        if let Some(ref mut wheel) = self.radial_wheel {
                            wheel.update_hover(cursor);
                        }
                    }

                    // Paint the overlay now — before any `return` can skip it
                    if let Some(ref wheel) = self.radial_wheel {
                        wheel.draw(ui.painter());
                    }

                    // Escape closes without selecting
                    if viewport_kb(ui.ctx()) && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        self.radial_wheel = None;
                        return;
                    }

                    // Primary click: select item or dismiss
                    if response.clicked_by(egui::PointerButton::Primary) {
                        if let Some(wheel) = self.radial_wheel.take() {
                            if let Some(idx) = wheel.hovered {
                                // item index is relative to the current page
                                if let Some(item) = wheel.current_page_items().get(idx) {
                                    let pa = PanelAction::from_wheel_action(
                                        item.action.clone(),
                                        wheel.canvas_pos,
                                        self.fill_color,
                                    );
                                    self.pending_panel_actions.push(pa);
                                }
                            }
                        }
                        return; // consume click — don't pass to tool handler
                    }

                    // Keep the wheel open during non-click interactions (pan, zoom)
                    return;
                }

                // ── Canvas pan: middle mouse drag ────────────────────────────
                if response.dragged_by(egui::PointerButton::Middle) {
                    let delta = response.drag_delta();
                    view.pan_x += delta.x as f64;
                    view.pan_y += delta.y as f64;
                }

                // ── Canvas pan: alt + left drag (skip when Shape Builder uses alt) ──
                if response.dragged_by(egui::PointerButton::Primary)
                    && ui.input(|i| i.modifiers.alt)
                    && self.active_tool != Tool::ShapeBuilder
                {
                    let delta = response.drag_delta();
                    view.pan_x += delta.x as f64;
                    view.pan_y += delta.y as f64;
                    return;
                }

                // ── Canvas pan: space + left drag ────────────────────────────────────
                let space_held = ui.input(|i| i.key_down(egui::Key::Space));
                if space_held {
                    let cursor = if response.dragged_by(egui::PointerButton::Primary) {
                        egui::CursorIcon::Grabbing
                    } else {
                        egui::CursorIcon::Grab
                    };
                    ui.ctx().set_cursor_icon(cursor);
                    if response.dragged_by(egui::PointerButton::Primary) {
                        let delta = response.drag_delta();
                        view.pan_x += delta.x as f64;
                        view.pan_y += delta.y as f64;
                    }
                    return;
                }

                // ── Arrow-key nudge ───────────────────────────────────────────
                if viewport_kb(ctx) {
                    let shift = ctx.input(|i| i.modifiers.shift);
                    let dist = self.prefs.nudge_distance * if shift { 10.0 } else { 1.0 };
                    let (dx, dy) = ctx.input(|i| {
                        let mut dx = 0.0_f64;
                        let mut dy = 0.0_f64;
                        if i.key_pressed(egui::Key::ArrowLeft) {
                            dx -= dist;
                        }
                        if i.key_pressed(egui::Key::ArrowRight) {
                            dx += dist;
                        }
                        if i.key_pressed(egui::Key::ArrowUp) {
                            dy -= dist;
                        }
                        if i.key_pressed(egui::Key::ArrowDown) {
                            dy += dist;
                        }
                        (dx, dy)
                    });
                    if (dx.abs() > 1e-12 || dy.abs() > 1e-12) && !doc.selection.is_empty() {
                        use photonic_core::transform::Transform;
                        let sel_ids: Vec<NodeId> = doc.selection.ids().copied().collect();
                        let cmds: Vec<Command> = sel_ids
                            .iter()
                            .filter_map(|id| {
                                let node = doc.nodes.get(id)?;
                                let mut new_node = node.clone();
                                new_node.transform =
                                    new_node.transform.then(&Transform::translate(dx, dy));
                                Some(Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                })
                            })
                            .collect();
                        if !cmds.is_empty() {
                            history.execute(Command::Batch(cmds), doc);
                            doc_modified = true;
                        }
                    }
                }

                let dt = ctx.input(|i| i.unstable_dt as f64).min(0.1);

                // ── Smooth zoom: lerp actual zoom toward log-space target ─────
                {
                    let target = self.smooth.log_zoom_target.exp();
                    if (view.zoom - target).abs() > 1e-5 {
                        let rate = 1.0 - (-22.0 * dt).exp();
                        let new_zoom = view.zoom + (target - view.zoom) * rate;
                        let factor = new_zoom / view.zoom;
                        let (px, py) = self.smooth.zoom_pivot;
                        view.zoom_at(factor, px, py);
                        ctx.request_repaint();
                    }
                }

                // ── Zoom: scroll accumulates into log-space target ────────────
                let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0 && response.hovered() {
                    let pivot = ui
                        .input(|i| i.pointer.hover_pos())
                        .unwrap_or(response.rect.center());
                    self.smooth.zoom_pivot = (pivot.x as f64, pivot.y as f64);
                    self.smooth.log_zoom_target += scroll as f64 * 0.001;
                    self.smooth.log_zoom_target = self
                        .smooth
                        .log_zoom_target
                        .clamp(0.01_f64.ln(), 64.0_f64.ln());
                }

                // ── WASD pan: velocity + exponential friction ─────────────────
                if viewport_kb(ctx) {
                    let (w, a, s, d) = ctx.input(|i| {
                        (
                            i.key_down(egui::Key::W),
                            i.key_down(egui::Key::A),
                            i.key_down(egui::Key::S),
                            i.key_down(egui::Key::D),
                        )
                    });
                    let accel = 2800.0 * dt;
                    let max_v = 900.0_f64;
                    if a {
                        self.smooth.pan_vel_x = (self.smooth.pan_vel_x + accel).min(max_v);
                    }
                    if d {
                        self.smooth.pan_vel_x = (self.smooth.pan_vel_x - accel).max(-max_v);
                    }
                    if w {
                        self.smooth.pan_vel_y = (self.smooth.pan_vel_y + accel).min(max_v);
                    }
                    if s {
                        self.smooth.pan_vel_y = (self.smooth.pan_vel_y - accel).max(-max_v);
                    }
                }
                let friction = (-10.0_f64 * dt).exp();
                self.smooth.pan_vel_x *= friction;
                self.smooth.pan_vel_y *= friction;
                if self.smooth.pan_vel_x.abs() > 0.5 || self.smooth.pan_vel_y.abs() > 0.5 {
                    view.pan_x += self.smooth.pan_vel_x * dt;
                    view.pan_y += self.smooth.pan_vel_y * dt;
                    ctx.request_repaint();
                }

                // ── Fit artboard: middle-click double-click ──────────────────
                if response.double_clicked_by(egui::PointerButton::Middle) {
                    view.fit_to_rect(
                        0.0,
                        0.0,
                        rect.width() as f64 * 0.8,
                        rect.height() as f64 * 0.8,
                    );
                    self.smooth.log_zoom_target = view.zoom.ln();
                }

                // ── Diff highlight overlay ────────────────────────────────────
                if self.diff.overlay_active {
                    for (node_id, category) in &self.diff.highlights {
                        if let Some(node) = doc.nodes.get(node_id) {
                            if let Some((cx0, cy0, cx1, cy1)) =
                                text_aware_canvas_bounds(node, renderer)
                            {
                                let (sx0, sy0) = view.canvas_to_screen(cx0, cy0);
                                let (sx1, sy1) = view.canvas_to_screen(cx1, cy1);
                                let rect = egui::Rect::from_min_max(
                                    egui::pos2(sx0 as f32, sy0 as f32),
                                    egui::pos2(sx1 as f32, sy1 as f32),
                                );
                                let (stroke_col, fill_col) = match category {
                                    DiffCategory::Added => (
                                        Color32::from_rgb(34, 197, 94),
                                        Color32::from_rgba_unmultiplied(34, 197, 94, 25),
                                    ),
                                    DiffCategory::Modified => (
                                        Color32::from_rgb(234, 179, 8),
                                        Color32::from_rgba_unmultiplied(234, 179, 8, 25),
                                    ),
                                    DiffCategory::Removed => unreachable!(),
                                };
                                ui.painter().rect_filled(rect, 2.0, fill_col);
                                ui.painter().rect_stroke(
                                    rect,
                                    2.0,
                                    egui::Stroke::new(2.0, stroke_col),
                                );
                            }
                        }
                    }
                    // Removed nodes use pre-computed canvas-space boxes.
                    let red_stroke = Color32::from_rgb(239, 68, 68);
                    let red_fill = Color32::from_rgba_unmultiplied(239, 68, 68, 25);
                    for &canvas_rect in &self.diff.removed_boxes {
                        let (sx0, sy0) = view
                            .canvas_to_screen(canvas_rect.min.x as f64, canvas_rect.min.y as f64);
                        let (sx1, sy1) = view
                            .canvas_to_screen(canvas_rect.max.x as f64, canvas_rect.max.y as f64);
                        let screen_rect = egui::Rect::from_min_max(
                            egui::pos2(sx0 as f32, sy0 as f32),
                            egui::pos2(sx1 as f32, sy1 as f32),
                        );
                        ui.painter().rect_filled(screen_rect, 2.0, red_fill);
                        ui.painter().rect_stroke(
                            screen_rect,
                            2.0,
                            egui::Stroke::new(2.0, red_stroke),
                        );
                    }
                }

                // ── Select tool ──────────────────────────────────────────────
                if self.active_tool == Tool::Select {
                    self.handle_select_tool(
                        ui,
                        &response,
                        doc,
                        view,
                        renderer,
                        &mut doc_modified,
                        history,
                    );
                    return;
                }

                // ── Direct Selection (point edit) tool ────────────────────────
                if self.active_tool == Tool::DirectSelect {
                    self.handle_direct_select_tool(
                        ui,
                        &response,
                        doc,
                        view,
                        renderer,
                        &mut doc_modified,
                        history,
                    );
                    return;
                }

                // ── Pan tool ──────────────────────────────────────────────────
                if self.active_tool == Tool::Pan {
                    let cursor = if response.dragged_by(egui::PointerButton::Primary) {
                        egui::CursorIcon::Grabbing
                    } else {
                        egui::CursorIcon::Grab
                    };
                    ui.ctx().set_cursor_icon(cursor);
                    if response.dragged_by(egui::PointerButton::Primary) {
                        let delta = response.drag_delta();
                        view.pan_x += delta.x as f64;
                        view.pan_y += delta.y as f64;
                    }
                    return;
                }

                // ── Pen tool ─────────────────────────────────────────────────
                if self.active_tool == Tool::Pen {
                    self.handle_pen_tool(ui, &response, doc, view, &mut doc_modified);
                    return;
                }

                // ── Shape Builder tool ────────────────────────────────────────
                if self.active_tool == Tool::ShapeBuilder {
                    self.handle_shape_builder_tool(
                        ui,
                        &response,
                        doc,
                        view,
                        renderer,
                        &mut doc_modified,
                        history,
                    );
                    return;
                }

                // ── Scissors tool ─────────────────────────────────────────────
                // Hover: show a blue dot at the nearest point on any path.
                // Click: split the nearest path at that point.
                if self.active_tool == Tool::Scissors {
                    ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
                    if let Some(cursor) = ui.input(|i| i.pointer.hover_pos()) {
                        if rect.contains(cursor) {
                            let (cx, cy) = view.screen_to_canvas(cursor.x as f64, cursor.y as f64);

                            // Find the path node nearest to the cursor.
                            let mut best_node_id = None;
                            let mut best_dist = 20.0f64 / view.zoom; // 20px snap radius in canvas units
                            let mut best_cut = (cx, cy);

                            for node in doc.nodes.values() {
                                if !node.visible {
                                    continue;
                                }
                                let pn = match &node.kind {
                                    SceneNodeKind::Path(p) => p,
                                    _ => continue,
                                };
                                if pn.path_data.is_empty() {
                                    continue;
                                }

                                // Transform cursor to local space.
                                let inv = node.transform.to_kurbo().inverse();
                                let lpt = inv * kurbo::Point::new(cx, cy);

                                // Sample points on the path to find closest.
                                let samples = pn.path_data.sample_positions(64);
                                for &(sx, sy, _) in &samples {
                                    let d = ((sx - lpt.x).powi(2) + (sy - lpt.y).powi(2)).sqrt();
                                    if d < best_dist {
                                        // Transform the sample back to canvas space.
                                        let fwd = node.transform.to_kurbo();
                                        let sp = fwd * kurbo::Point::new(sx, sy);
                                        best_dist = d;
                                        best_node_id = Some(node.id);
                                        best_cut = (sp.x, sp.y);
                                    }
                                }
                            }

                            // Draw indicator dot at cut point.
                            if let Some(_nid) = best_node_id {
                                let painter = ctx.layer_painter(egui::LayerId::new(
                                    egui::Order::Foreground,
                                    egui::Id::new("scissors_indicator"),
                                ));
                                let (sx, sy) = view.canvas_to_screen(best_cut.0, best_cut.1);
                                painter.circle_filled(
                                    egui::pos2(sx as f32, sy as f32),
                                    5.0,
                                    egui::Color32::from_rgb(0, 140, 255),
                                );
                                painter.circle_stroke(
                                    egui::pos2(sx as f32, sy as f32),
                                    5.0,
                                    egui::Stroke::new(1.5, egui::Color32::WHITE),
                                );
                            }

                            // Click: split the path.
                            if response.clicked_by(egui::PointerButton::Primary) {
                                if let Some(nid) = best_node_id {
                                    if let Some(node) = doc.nodes.get(&nid) {
                                        let pn = match &node.kind {
                                            SceneNodeKind::Path(p) => p.clone(),
                                            _ => {
                                                return;
                                            }
                                        };
                                        let inv = node.transform.to_kurbo().inverse();
                                        let lpt = inv * kurbo::Point::new(cx, cy);

                                        if let Some((path_a, path_b)) =
                                            pn.path_data.split_at_point(lpt.x, lpt.y)
                                        {
                                            let layer_id = node.layer_id;
                                            let t = node.transform.clone();
                                            let opacity = node.opacity;
                                            let blend_mode = node.blend_mode;
                                            let name_base = node.name.clone();

                                            let mut na = SceneNode::new(
                                                format!("{} (1/2)", name_base),
                                                layer_id,
                                                SceneNodeKind::Path(
                                                    photonic_core::node::PathNode {
                                                        path_data: path_a,
                                                        ..pn.clone()
                                                    },
                                                ),
                                            );
                                            na.transform = t.clone();
                                            na.opacity = opacity;
                                            na.blend_mode = blend_mode;

                                            let mut nb = SceneNode::new(
                                                format!("{} (2/2)", name_base),
                                                layer_id,
                                                SceneNodeKind::Path(
                                                    photonic_core::node::PathNode {
                                                        path_data: path_b,
                                                        ..pn.clone()
                                                    },
                                                ),
                                            );
                                            nb.transform = t;
                                            nb.opacity = opacity;
                                            nb.blend_mode = blend_mode;

                                            let sel_a = na.id;
                                            let sel_b = nb.id;

                                            history.execute(
                                                Command::Batch(vec![
                                                    Command::RemoveNode { node_id: nid },
                                                    Command::AddNode {
                                                        node: na,
                                                        layer_id: Some(layer_id),
                                                    },
                                                    Command::AddNode {
                                                        node: nb,
                                                        layer_id: Some(layer_id),
                                                    },
                                                ]),
                                                doc,
                                            );
                                            doc.selection = photonic_core::Selection::from_ids(
                                                [sel_a, sel_b].iter().copied(),
                                            );
                                            doc_modified = true;
                                        }
                                    }
                                }
                                return;
                            }
                        }
                    }
                    return;
                }

                // ── Magic Wand tool ───────────────────────────────────────────
                if self.active_tool == Tool::MagicWand {
                    ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
                    if response.clicked_by(egui::PointerButton::Primary) {
                        if let Some(pos) = response.interact_pointer_pos() {
                            let (cx, cy) = view.screen_to_canvas(pos.x as f64, pos.y as f64);
                            // Find topmost visible unlocked node whose AABB contains click.
                            let hit = hit_test(doc, cx, cy, renderer);
                            if let Some(ref_id) = hit {
                                if let Some(ref_node) = doc.nodes.get(&ref_id).cloned() {
                                    let tolerance = self.magic_wand_tolerance as f32;
                                    let attr = self.magic_wand_attribute;
                                    let mut matched: Vec<NodeId> = Vec::new();
                                    for (nid, node) in &doc.nodes {
                                        let ok = match attr {
                                            SelectSameAttr::FillColor => {
                                                let ref_c = magic_wand_solid_fill(&ref_node);
                                                let cand_c = magic_wand_solid_fill(node);
                                                match (ref_c, cand_c) {
                                                    (Some(rc), Some(cc)) => {
                                                        magic_wand_color_dist(rc, cc) <= tolerance
                                                    }
                                                    (None, None) => true,
                                                    _ => false,
                                                }
                                            }
                                            SelectSameAttr::StrokeColor => {
                                                if let (
                                                    SceneNodeKind::Path(rp),
                                                    SceneNodeKind::Path(cp),
                                                ) = (&ref_node.kind, &node.kind)
                                                {
                                                    match (rp.stroke.enabled, cp.stroke.enabled) {
                                                        (true, true) => {
                                                            magic_wand_color_dist(
                                                                rp.stroke.color,
                                                                cp.stroke.color,
                                                            ) <= tolerance
                                                        }
                                                        (false, false) => true,
                                                        _ => false,
                                                    }
                                                } else {
                                                    false
                                                }
                                            }
                                            SelectSameAttr::StrokeWeight => {
                                                if let (
                                                    SceneNodeKind::Path(rp),
                                                    SceneNodeKind::Path(cp),
                                                ) = (&ref_node.kind, &node.kind)
                                                {
                                                    (rp.stroke.width - cp.stroke.width).abs()
                                                        <= tolerance as f64
                                                } else {
                                                    false
                                                }
                                            }
                                            SelectSameAttr::Opacity => {
                                                (ref_node.opacity - node.opacity).abs() <= tolerance
                                            }
                                            SelectSameAttr::BlendMode => {
                                                ref_node.blend_mode == node.blend_mode
                                            }
                                            SelectSameAttr::ObjectType => {
                                                std::mem::discriminant(&ref_node.kind)
                                                    == std::mem::discriminant(&node.kind)
                                            }
                                        };
                                        if ok {
                                            matched.push(*nid);
                                        }
                                    }
                                    doc.selection.clear();
                                    for nid in &matched {
                                        doc.selection.add(*nid);
                                    }
                                    self.selected_id = Some(ref_id);
                                    doc_modified = true;
                                }
                            }
                        }
                        return;
                    }
                }

                // ── Lasso tool ────────────────────────────────────────────────
                if self.active_tool == Tool::Lasso {
                    ctx.set_cursor_icon(egui::CursorIcon::Crosshair);

                    // Collect points while dragging.
                    if response.dragged_by(egui::PointerButton::Primary) {
                        if let Some(pos) = response.interact_pointer_pos() {
                            self.lasso_points.push(pos);
                        }
                    }

                    // Draw the lasso overlay while dragging.
                    if !self.lasso_points.is_empty() {
                        let painter = ctx.layer_painter(egui::LayerId::new(
                            egui::Order::Tooltip,
                            egui::Id::new("lasso_overlay"),
                        ));
                        let stroke = egui::Stroke::new(1.5, egui::Color32::from_rgb(30, 120, 255));
                        let pts: Vec<egui::Pos2> = self.lasso_points.clone();
                        for w in pts.windows(2) {
                            painter.line_segment([w[0], w[1]], stroke);
                        }
                        // Close the lasso visually.
                        if pts.len() >= 2 {
                            painter.line_segment(
                                [*pts.last().unwrap(), pts[0]],
                                egui::Stroke::new(
                                    1.0,
                                    egui::Color32::from_rgba_premultiplied(30, 120, 255, 80),
                                ),
                            );
                        }
                    }

                    // On release: compute selection.
                    if response.drag_stopped() {
                        let pts = std::mem::take(&mut self.lasso_points);
                        if pts.len() >= 3 {
                            // Convert screen polygon to canvas coordinates.
                            let poly: Vec<[f64; 2]> = pts
                                .iter()
                                .map(|p| {
                                    let (cx, cy) = view.screen_to_canvas(p.x as f64, p.y as f64);
                                    [cx, cy]
                                })
                                .collect();

                            let additive = ui.input(|i| i.modifiers.shift);
                            if !additive {
                                doc.selection.clear();
                                self.selected_id = None;
                            }

                            // Collect matching IDs before mutating selection.
                            let to_select: Vec<NodeId> = doc
                                .nodes_in_draw_order()
                                .into_iter()
                                .filter(|n| !n.locked)
                                .filter_map(|node| {
                                    node_world_aabb_opt(node).and_then(|aabb| {
                                        let cx = (aabb.0 + aabb.2) / 2.0;
                                        let cy = (aabb.1 + aabb.3) / 2.0;
                                        if lasso_point_in_polygon(cx, cy, &poly) {
                                            Some(node.id)
                                        } else {
                                            None
                                        }
                                    })
                                })
                                .collect();
                            for nid in to_select {
                                doc.selection.add(nid);
                                if self.selected_id.is_none() {
                                    self.selected_id = Some(nid);
                                }
                            }
                            doc_modified = true;
                        }
                        return;
                    }

                    if response.dragged_by(egui::PointerButton::Primary) {
                        return;
                    }
                }

                // ── Pencil tool ───────────────────────────────────────────────
                if self.active_tool == Tool::Pencil {
                    ctx.set_cursor_icon(egui::CursorIcon::Crosshair);

                    // Collect canvas points while dragging, throttled to ~5 units apart.
                    if response.dragged_by(egui::PointerButton::Primary) {
                        if let Some(pos) = response.interact_pointer_pos() {
                            let (cx, cy) = view.screen_to_canvas(pos.x as f64, pos.y as f64);
                            let should_add = match self.pencil_points.last() {
                                Some(&(lx, ly)) => {
                                    let dx = cx - lx;
                                    let dy = cy - ly;
                                    dx * dx + dy * dy >= 25.0 // 5 unit threshold
                                }
                                None => true,
                            };
                            if should_add {
                                self.pencil_points.push((cx, cy));
                            }
                        }
                    }

                    // Draw the preview stroke.
                    if self.pencil_points.len() >= 2 {
                        let painter = ctx.layer_painter(egui::LayerId::new(
                            egui::Order::Tooltip,
                            egui::Id::new("pencil_preview"),
                        ));
                        let stroke = egui::Stroke::new(1.5, egui::Color32::from_rgb(80, 80, 200));
                        let screen_pts: Vec<egui::Pos2> = self
                            .pencil_points
                            .iter()
                            .map(|&(cx, cy)| {
                                let (sx, sy) = view.canvas_to_screen(cx, cy);
                                egui::pos2(sx as f32, sy as f32)
                            })
                            .collect();
                        for w in screen_pts.windows(2) {
                            painter.line_segment([w[0], w[1]], stroke);
                        }
                    }

                    // On release: build the path node.
                    if response.drag_stopped() {
                        let pts = std::mem::take(&mut self.pencil_points);
                        if pts.len() >= 2 {
                            // Build SVG path string from collected points.
                            let mut svg = format!("M {:.3} {:.3}", pts[0].0, pts[0].1);
                            for &(x, y) in &pts[1..] {
                                svg.push_str(&format!(" L {:.3} {:.3}", x, y));
                            }
                            if let Ok(path) = PathData::from_svg(&svg) {
                                let num = doc.node_count() + 1;
                                let stroke_arg = self.prefs.default_stroke_enabled.then(|| {
                                    (
                                        self.prefs.default_stroke_color,
                                        self.prefs.default_stroke_width,
                                    )
                                });
                                let node =
                                    make_node(path, self.fill_color, stroke_arg, "Pencil", num);
                                let cmd = Command::AddNode {
                                    node,
                                    layer_id: None,
                                };
                                history.execute(cmd, doc);
                                doc_modified = true;
                            }
                        }
                        return;
                    }

                    if response.dragged_by(egui::PointerButton::Primary) {
                        return;
                    }
                }

                // ── Smooth tool ───────────────────────────────────────────────
                if self.active_tool == Tool::Smooth {
                    ctx.set_cursor_icon(egui::CursorIcon::Crosshair);

                    // On click (or drag end): smooth the hit-tested path node.
                    let should_smooth = response.clicked_by(egui::PointerButton::Primary)
                        || response.drag_stopped();

                    if should_smooth {
                        if let Some(pos) = response.interact_pointer_pos() {
                            let (cx, cy) = view.screen_to_canvas(pos.x as f64, pos.y as f64);
                            if let Some(hit_id) = hit_test(doc, cx, cy, renderer) {
                                if let Some(node) = doc.nodes.get(&hit_id) {
                                    if let SceneNodeKind::Path(pn) = &node.kind {
                                        let smoothed = pn.path_data.smooth(0.25, 2);
                                        let mut new_node = node.clone();
                                        if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
                                            new_pn.path_data = smoothed;
                                        }
                                        history.execute(
                                            Command::UpdateNode {
                                                old: node.clone(),
                                                new: new_node,
                                            },
                                            doc,
                                        );
                                        doc_modified = true;
                                    }
                                }
                            }
                        }
                        return;
                    }
                    if response.dragged_by(egui::PointerButton::Primary) {
                        return;
                    }
                }

                // ── Text tool ─────────────────────────────────────────────────
                if self.active_tool == Tool::Text {
                    if response.clicked_by(egui::PointerButton::Primary) {
                        if let Some(pos) = response.interact_pointer_pos() {
                            use photonic_core::node::TextNode;
                            let (cx, cy) = view.screen_to_canvas(pos.x as f64, pos.y as f64);
                            let (cx, cy) = (self.snap(cx), self.snap(cy));
                            let [r, g, b, a] = self.fill_color;
                            let mut text_node = TextNode::new("Text");
                            text_node.fill = Fill::solid(Color { r, g, b, a });
                            let kind = SceneNodeKind::Text(text_node);
                            let num = doc.node_count() + 1;
                            let mut node =
                                SceneNode::new(format!("Text {}", num), Default::default(), kind);
                            node.transform = photonic_core::transform::Transform::translate(cx, cy);
                            doc.add_node(node, None);
                            doc_modified = true;
                        }
                    }
                    return;
                }

                // ── Shape creation tools ─────────────────────────────────────
                if !self.active_tool.is_shape_creator() {
                    return;
                }
                if ui.input(|i| i.modifiers.alt) {
                    return;
                }

                if response.drag_started_by(egui::PointerButton::Primary) {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let (cx, cy) = view.screen_to_canvas(pos.x as f64, pos.y as f64);
                        self.drag_start_canvas = Some((self.snap(cx), self.snap(cy)));
                    }
                }

                if response.drag_stopped_by(egui::PointerButton::Primary) {
                    if let (Some((sx, sy)), Some(end_pos)) = (
                        self.drag_start_canvas.take(),
                        response.interact_pointer_pos(),
                    ) {
                        let (ex_raw, ey_raw) =
                            view.screen_to_canvas(end_pos.x as f64, end_pos.y as f64);
                        let (mut ex, mut ey) = (self.snap(ex_raw), self.snap(ey_raw));
                        // Line tool: snap endpoint to nearest 45° angle when Snap45 is on or Shift held.
                        if self.active_tool == Tool::Line {
                            let shift_held = ui.input(|i| i.modifiers.shift);
                            if self.line_snap_45 || shift_held {
                                let (snapped_ex, snapped_ey) = snap_line_to_45(sx, sy, ex, ey);
                                ex = snapped_ex;
                                ey = snapped_ey;
                            }
                        }
                        if (ex - sx).abs() > 2.0 || (ey - sy).abs() > 2.0 {
                            if let Some(path) = self.build_shape(sx, sy, ex, ey) {
                                let stroke_arg = self.prefs.default_stroke_enabled.then(|| {
                                    (
                                        self.prefs.default_stroke_color,
                                        self.prefs.default_stroke_width,
                                    )
                                });
                                let node = make_node(
                                    path,
                                    self.fill_color,
                                    stroke_arg,
                                    self.active_tool.label(),
                                    doc.node_count() + 1,
                                );
                                doc.add_node(node, None);
                                doc_modified = true;
                            }
                        }
                    }
                } else if self.drag_start_canvas.is_none()
                    && response.clicked_by(egui::PointerButton::Primary)
                {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let (cx, cy) = view.screen_to_canvas(pos.x as f64, pos.y as f64);
                        if let Some(path) =
                            self.build_shape(cx - 50.0, cy - 50.0, cx + 50.0, cy + 50.0)
                        {
                            let stroke_arg = self.prefs.default_stroke_enabled.then(|| {
                                (
                                    self.prefs.default_stroke_color,
                                    self.prefs.default_stroke_width,
                                )
                            });
                            let node = make_node(
                                path,
                                self.fill_color,
                                stroke_arg,
                                self.active_tool.label(),
                                doc.node_count() + 1,
                            );
                            doc.add_node(node, None);
                            doc_modified = true;
                        }
                    }
                }

                // ── Shape preview while dragging ─────────────────────────────
                if let Some((sx, sy)) = self.drag_start_canvas {
                    let cursor = response
                        .interact_pointer_pos()
                        .or_else(|| ui.input(|i| i.pointer.hover_pos()));
                    if let Some(cursor) = cursor {
                        let (ex_raw, ey_raw) =
                            view.screen_to_canvas(cursor.x as f64, cursor.y as f64);
                        let (ex, ey) = if self.active_tool == Tool::Line {
                            let shift_held = ui.input(|i| i.modifiers.shift);
                            if self.line_snap_45 || shift_held {
                                snap_line_to_45(sx, sy, ex_raw, ey_raw)
                            } else {
                                (ex_raw, ey_raw)
                            }
                        } else {
                            (ex_raw, ey_raw)
                        };
                        if let Some(path) = self.build_shape(sx, sy, ex, ey) {
                            let pts = bez_to_screen_points(&path.to_bez_path(), view);
                            if pts.len() >= 2 {
                                let [fr, fg, fb, _] = self.fill_color;
                                let fill = Color32::from_rgba_unmultiplied(
                                    (fr * 255.0) as u8,
                                    (fg * 255.0) as u8,
                                    (fb * 255.0) as u8,
                                    40,
                                );
                                let stroke_color = Color32::from_rgb(110, 86, 207);
                                ui.painter().add(egui::Shape::Path(egui::epaint::PathShape {
                                    points: pts,
                                    closed: true,
                                    fill,
                                    stroke: egui::epaint::PathStroke::new(1.5, stroke_color),
                                }));
                            }
                        }
                    }
                }
            });

        // ── Drain panel actions (z-order, boolean ops) ───────────────────────
        // Use take() so `self` is not borrowed during the loop, allowing calls
        // to &self/&mut self methods (build_shape_with_tool, do_group_selected).
        'actions: for action in std::mem::take(&mut self.pending_panel_actions) {
            match action {
                PanelAction::ReorderNode { node_id, op } => {
                    if let Some((layer_id, cur_idx)) = doc.node_layer_and_index(&node_id) {
                        let layer_len = doc
                            .layers
                            .get(&layer_id)
                            .map(|l| l.node_ids.len())
                            .unwrap_or(0);
                        if layer_len > 0 {
                            let new_index = match op {
                                ZOrderOp::SendToBack => 0,
                                ZOrderOp::BringToFront => layer_len - 1,
                                ZOrderOp::SendBackward => cur_idx.saturating_sub(1),
                                ZOrderOp::BringForward => (cur_idx + 1).min(layer_len - 1),
                            };
                            if new_index != cur_idx {
                                let cmd = Command::ReorderNode {
                                    layer_id,
                                    node_id,
                                    old_index: cur_idx,
                                    new_index,
                                };
                                history.execute(cmd, doc);
                                doc_modified = true;
                            }
                        }
                    }
                }
                PanelAction::BooleanOp(bool_op) => {
                    // Determine target (lower z) and tool (upper z) from selection
                    let sel_ids: Vec<_> = doc.selection.ids().copied().collect();
                    if sel_ids.len() == 2 {
                        if let (Some((lid_a, idx_a)), Some((lid_b, idx_b))) = (
                            doc.node_layer_and_index(&sel_ids[0]),
                            doc.node_layer_and_index(&sel_ids[1]),
                        ) {
                            if lid_a == lid_b {
                                let (target_id, tool_id) = if idx_a <= idx_b {
                                    (sel_ids[0], sel_ids[1])
                                } else {
                                    (sel_ids[1], sel_ids[0])
                                };
                                let (target_idx, tool_idx) = if idx_a <= idx_b {
                                    (idx_a, idx_b)
                                } else {
                                    (idx_b, idx_a)
                                };

                                if let (Some(tn), Some(on)) =
                                    (doc.get_node(&target_id), doc.get_node(&tool_id))
                                {
                                    if let (SceneNodeKind::Path(tp), SceneNodeKind::Path(op_node)) =
                                        (&tn.kind, &on.kind)
                                    {
                                        use photonic_core::ops::boolean::{boolean_op, BooleanOp};
                                        let target_baked = gui_apply_affine_to_path(
                                            &tp.path_data,
                                            tn.transform.to_kurbo(),
                                        );
                                        let tool_baked = gui_apply_affine_to_path(
                                            &op_node.path_data,
                                            on.transform.to_kurbo(),
                                        );
                                        if let Ok(result_path) =
                                            boolean_op(&target_baked, &tool_baked, bool_op)
                                        {
                                            let mut result_pn = PathNode::new(result_path);
                                            result_pn.fill = tp.fill.clone();
                                            result_pn.stroke = tp.stroke.clone();
                                            let op_name = match bool_op {
                                                BooleanOp::Union => "union",
                                                BooleanOp::Subtract => "subtract",
                                                BooleanOp::Intersect => "intersect",
                                                BooleanOp::Exclude => "exclude",
                                                BooleanOp::Divide => "divide",
                                            };
                                            let result_name =
                                                format!("{} {} {}", tn.name, op_name, on.name);
                                            let result_node = SceneNode::new(
                                                &result_name,
                                                lid_a,
                                                SceneNodeKind::Path(result_pn),
                                            );
                                            let result_id = result_node.id;
                                            let orig_len = doc
                                                .layers
                                                .get(&lid_a)
                                                .map(|l| l.node_ids.len())
                                                .unwrap_or(2);
                                            let tool_below = tool_idx < target_idx;
                                            let adj_target = if tool_below {
                                                target_idx.saturating_sub(1)
                                            } else {
                                                target_idx
                                            };
                                            let result_pos = orig_len.saturating_sub(2);
                                            let cmd = Command::Batch(vec![
                                                Command::RemoveNode { node_id: tool_id },
                                                Command::RemoveNode { node_id: target_id },
                                                Command::AddNode {
                                                    node: result_node,
                                                    layer_id: Some(lid_a),
                                                },
                                                Command::ReorderNode {
                                                    layer_id: lid_a,
                                                    node_id: result_id,
                                                    old_index: result_pos,
                                                    new_index: adj_target,
                                                },
                                            ]);
                                            history.execute(cmd, doc);
                                            self.selected_id = Some(result_id);
                                            doc.selection = Selection::single(result_id);
                                            doc_modified = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                PanelAction::RestoreCheckpoint(id) => {
                    if let Some(snapshot) = history.restore_checkpoint(id) {
                        *doc = snapshot;
                        self.selected_id = None;
                        doc.selection.clear();
                        doc_modified = true;
                    }
                }
                PanelAction::UpdateNodeFill { node_id, fill } => {
                    // Record solid fill color in recent-colors list.
                    if let photonic_core::style::FillKind::Solid(c) = &fill.kind {
                        doc.record_recent_color(*c);
                    }
                    if let Some(node) = doc.nodes.get(&node_id) {
                        let mut new_node = node.clone();
                        if let SceneNodeKind::Path(pn) = &mut new_node.kind {
                            pn.fill = fill;
                        }
                        let cmd = Command::UpdateNode {
                            old: node.clone(),
                            new: new_node,
                        };
                        history.execute(cmd, doc);
                        doc_modified = true;
                    }
                }
                PanelAction::UpdateNodeStroke { node_id, stroke } => {
                    // Record stroke color in recent-colors list.
                    if stroke.enabled {
                        doc.record_recent_color(stroke.color);
                    }
                    if let Some(node) = doc.nodes.get(&node_id) {
                        let mut new_node = node.clone();
                        if let SceneNodeKind::Path(pn) = &mut new_node.kind {
                            pn.stroke = stroke;
                        }
                        let cmd = Command::UpdateNode {
                            old: node.clone(),
                            new: new_node,
                        };
                        history.execute(cmd, doc);
                        doc_modified = true;
                    }
                }

                PanelAction::UpdateNodeOuterGlow { node_id, glow } => {
                    if let Some(node) = doc.nodes.get(&node_id) {
                        let mut new_node = node.clone();
                        new_node.outer_glow = glow;
                        let cmd = Command::UpdateNode {
                            old: node.clone(),
                            new: new_node,
                        };
                        history.execute(cmd, doc);
                        doc_modified = true;
                    }
                }

                PanelAction::UpdateNodeInnerGlow { node_id, glow } => {
                    if let Some(node) = doc.nodes.get(&node_id) {
                        let mut new_node = node.clone();
                        new_node.inner_glow = glow;
                        let cmd = Command::UpdateNode {
                            old: node.clone(),
                            new: new_node,
                        };
                        history.execute(cmd, doc);
                        doc_modified = true;
                    }
                }

                PanelAction::UpdateNodeGaussianGlow { node_id, glow } => {
                    if let Some(node) = doc.nodes.get(&node_id) {
                        let mut new_node = node.clone();
                        new_node.gaussian_glow = glow;
                        let cmd = Command::UpdateNode {
                            old: node.clone(),
                            new: new_node,
                        };
                        history.execute(cmd, doc);
                        doc_modified = true;
                    }
                }

                PanelAction::SetLocked { node_id, locked } => {
                    if let Some(node) = doc.nodes.get(&node_id) {
                        let mut new_node = node.clone();
                        new_node.locked = locked;
                        let cmd = Command::UpdateNode {
                            old: node.clone(),
                            new: new_node,
                        };
                        history.execute(cmd, doc);
                        doc_modified = true;
                    }
                }

                PanelAction::SetVisible { node_id, visible } => {
                    if let Some(node) = doc.nodes.get(&node_id) {
                        let mut new_node = node.clone();
                        new_node.visible = visible;
                        let cmd = Command::UpdateNode {
                            old: node.clone(),
                            new: new_node,
                        };
                        history.execute(cmd, doc);
                        doc_modified = true;
                    }
                }

                PanelAction::SetNodePosition { node_id, x, y } => {
                    if let Some(node) = doc.nodes.get(&node_id) {
                        let mut new_node = node.clone();
                        new_node.transform.matrix[4] = x;
                        new_node.transform.matrix[5] = y;
                        let cmd = Command::UpdateNode {
                            old: node.clone(),
                            new: new_node,
                        };
                        history.execute(cmd, doc);
                        doc_modified = true;
                    }
                }

                PanelAction::SetNodeSize {
                    node_id,
                    width,
                    height,
                } => {
                    if width > 1e-6 && height > 1e-6 {
                        if let Some(node) = doc.nodes.get(&node_id).cloned() {
                            if let Some(local) = node.local_bounds() {
                                let affine = node.transform.to_kurbo();
                                let corners_x = [local.x0, local.x1, local.x1, local.x0];
                                let corners_y = [local.y0, local.y0, local.y1, local.y1];
                                let (mut ax, mut max_x) = (f64::INFINITY, f64::NEG_INFINITY);
                                let (mut ay, mut max_y) = (f64::INFINITY, f64::NEG_INFINITY);
                                for i in 0..4 {
                                    let p = affine * Point::new(corners_x[i], corners_y[i]);
                                    if p.x < ax {
                                        ax = p.x;
                                    }
                                    if p.x > max_x {
                                        max_x = p.x;
                                    }
                                    if p.y < ay {
                                        ay = p.y;
                                    }
                                    if p.y > max_y {
                                        max_y = p.y;
                                    }
                                }
                                let cur_w = max_x - ax;
                                let cur_h = max_y - ay;
                                if cur_w > 1e-9 && cur_h > 1e-9 {
                                    let sx = width / cur_w;
                                    let sy = height / cur_h;
                                    let scale_t = photonic_core::transform::Transform::scale_around(
                                        sx, sy, ax, ay,
                                    );
                                    let mut new_node = node.clone();
                                    new_node.transform = node.transform.then(&scale_t);
                                    history.execute(
                                        Command::UpdateNode {
                                            old: node,
                                            new: new_node,
                                        },
                                        doc,
                                    );
                                    doc_modified = true;
                                }
                            }
                        }
                    }
                }

                PanelAction::RotateNode {
                    node_ids,
                    angle_deg,
                } => {
                    // node_ids[0] is the primary: its current angle defines the delta.
                    if let Some(&primary_id) = node_ids.first() {
                        if let Some(primary) = doc.nodes.get(&primary_id).cloned() {
                            let [a, b, _c, _d, _e, _f] = primary.transform.matrix;
                            let current_rad = b.atan2(a);
                            let delta_rad = angle_deg.to_radians() - current_rad;
                            // Shared pivot: center of the selection's world bounds when
                            // multiple are selected; the node's own center otherwise.
                            let (cx, cy) = if node_ids.len() > 1 {
                                selection_canvas_bounds(doc, &node_ids, renderer)
                                    .map(|(x0, y0, x1, y1)| ((x0 + x1) / 2.0, (y0 + y1) / 2.0))
                                    .unwrap_or((
                                        primary.transform.matrix[4],
                                        primary.transform.matrix[5],
                                    ))
                            } else {
                                match primary.local_bounds() {
                                    Some(local) => {
                                        let c = primary.transform.to_kurbo()
                                            * Point::new(
                                                (local.x0 + local.x1) / 2.0,
                                                (local.y0 + local.y1) / 2.0,
                                            );
                                        (c.x, c.y)
                                    }
                                    None => {
                                        (primary.transform.matrix[4], primary.transform.matrix[5])
                                    }
                                }
                            };
                            let rot_t = photonic_core::transform::Transform::rotate_around(
                                delta_rad, cx, cy,
                            );
                            let mut cmds = Vec::new();
                            for nid in &node_ids {
                                if let Some(node) = doc.nodes.get(nid) {
                                    let mut new_node = node.clone();
                                    // Apply in WORLD space: node transform first, then
                                    // the rotation about the shared pivot.
                                    new_node.transform = rot_t.then(&node.transform);
                                    cmds.push(Command::UpdateNode {
                                        old: node.clone(),
                                        new: new_node,
                                    });
                                }
                            }
                            if !cmds.is_empty() {
                                history.execute(Command::Batch(cmds), doc);
                                doc_modified = true;
                            }
                        }
                    }
                }

                PanelAction::DuplicateNode { node_id } => {
                    if let Some(node) = doc.nodes.get(&node_id).cloned() {
                        let mut copy = node.clone();
                        copy.id = uuid::Uuid::new_v4();
                        copy.name = format!("{} copy", copy.name);
                        copy.transform.matrix[4] += 10.0;
                        copy.transform.matrix[5] += 10.0;
                        let lid = copy.layer_id;
                        let copy_id = copy.id;
                        let cmd = Command::AddNode {
                            node: copy,
                            layer_id: Some(lid),
                        };
                        history.execute(cmd, doc);
                        doc.selection = Selection::single(copy_id);
                        self.selected_id = Some(copy_id);
                        doc_modified = true;
                    }
                }

                PanelAction::DeleteNode { node_id } => {
                    history.execute(Command::RemoveNode { node_id }, doc);
                    if self.selected_id == Some(node_id) {
                        self.selected_id = None;
                        doc.selection.clear();
                    }
                    doc_modified = true;
                }

                PanelAction::AddAnchorPoints { node_id } => {
                    if let Some(node) = doc.nodes.get(&node_id) {
                        if let SceneNodeKind::Path(pn) = &node.kind {
                            let new_path = pn.path_data.subdivide(1);
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
                                new_pn.path_data = new_path;
                            }
                            let cmd = Command::UpdateNode {
                                old: node.clone(),
                                new: new_node,
                            };
                            history.execute(cmd, doc);
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::JoinPaths { node_ids } => {
                    use photonic_core::ops::join::{close_open_paths, join_two_paths};
                    let ids: Vec<NodeId> = if node_ids.is_empty() {
                        doc.selection.ids().copied().collect()
                    } else {
                        node_ids
                    };

                    if ids.len() == 1 {
                        let nid = ids[0];
                        if let Some(node) = doc.nodes.get(&nid) {
                            if let SceneNodeKind::Path(pn) = &node.kind {
                                let closed = close_open_paths(&pn.path_data);
                                let mut new_node = node.clone();
                                if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
                                    new_pn.path_data = closed;
                                }
                                history.execute(
                                    Command::UpdateNode {
                                        old: node.clone(),
                                        new: new_node,
                                    },
                                    doc,
                                );
                                doc_modified = true;
                            }
                        }
                    } else if ids.len() == 2 {
                        let id_a = ids[0];
                        let id_b = ids[1];
                        if let (Some(node_a), Some(node_b)) =
                            (doc.nodes.get(&id_a).cloned(), doc.nodes.get(&id_b).cloned())
                        {
                            if let (SceneNodeKind::Path(pn_a), SceneNodeKind::Path(pn_b)) =
                                (&node_a.kind, &node_b.kind)
                            {
                                let merged = join_two_paths(&pn_a.path_data, &pn_b.path_data);
                                let mut result = node_a.clone();
                                if let SceneNodeKind::Path(ref mut rp) = result.kind {
                                    rp.path_data = merged;
                                }
                                history.execute(
                                    Command::Batch(vec![
                                        Command::UpdateNode {
                                            old: node_a,
                                            new: result.clone(),
                                        },
                                        Command::RemoveNode { node_id: id_b },
                                    ]),
                                    doc,
                                );
                                doc.selection.clear();
                                doc.selection.add(result.id);
                                doc_modified = true;
                            }
                        }
                    }
                }

                PanelAction::PathfinderCrop { node_ids } => {
                    use photonic_core::ops::boolean::{boolean_op, BooleanOp};
                    use photonic_core::transform::Transform;

                    let ids: Vec<NodeId> = if node_ids.is_empty() {
                        doc.selection.ids().copied().collect()
                    } else {
                        node_ids
                    };

                    if ids.len() >= 2 {
                        // Find the frontmost node by z-order.
                        let frontmost_id = ids
                            .iter()
                            .max_by_key(|nid| {
                                doc.node_layer_and_index(nid)
                                    .map(|(lid, pos)| {
                                        let layer_pos = doc
                                            .layer_order
                                            .iter()
                                            .position(|id| id == &lid)
                                            .unwrap_or(0);
                                        (layer_pos, pos)
                                    })
                                    .unwrap_or((0, 0))
                            })
                            .copied();

                        if let Some(front_id) = frontmost_id {
                            if let Some(front_node) = doc.nodes.get(&front_id).cloned() {
                                if let SceneNodeKind::Path(front_pn) = &front_node.kind {
                                    let front_path = gui_apply_affine_to_path(
                                        &front_pn.path_data,
                                        front_node.transform.to_kurbo(),
                                    );
                                    let mut cmds: Vec<Command> = Vec::new();
                                    let mut had_error = false;

                                    for nid in &ids {
                                        if *nid == front_id {
                                            continue;
                                        }
                                        if let Some(node) = doc.nodes.get(nid).cloned() {
                                            if let SceneNodeKind::Path(pn) = &node.kind {
                                                let baked = gui_apply_affine_to_path(
                                                    &pn.path_data,
                                                    node.transform.to_kurbo(),
                                                );
                                                if let Ok(cropped) = boolean_op(
                                                    &baked,
                                                    &front_path,
                                                    BooleanOp::Intersect,
                                                ) {
                                                    let mut new_node = node.clone();
                                                    if let SceneNodeKind::Path(ref mut new_pn) =
                                                        new_node.kind
                                                    {
                                                        new_pn.path_data = cropped;
                                                    }
                                                    new_node.transform = Transform::IDENTITY;
                                                    cmds.push(Command::UpdateNode {
                                                        old: node,
                                                        new: new_node,
                                                    });
                                                } else {
                                                    had_error = true;
                                                }
                                            }
                                        }
                                    }

                                    if !had_error && !cmds.is_empty() {
                                        cmds.push(Command::RemoveNode { node_id: front_id });
                                        history.execute(Command::Batch(cmds), doc);
                                        doc.selection.clear();
                                        doc_modified = true;
                                    }
                                }
                            }
                        }
                    }
                }

                PanelAction::PathfinderMinusBack { node_ids } => {
                    use photonic_core::ops::boolean::{boolean_op, BooleanOp};
                    use photonic_core::transform::Transform;

                    let ids: Vec<NodeId> = if node_ids.is_empty() {
                        doc.selection.ids().copied().collect()
                    } else {
                        node_ids
                    };

                    if ids.len() >= 2 {
                        // Find the frontmost node by z-order.
                        let frontmost_id = ids
                            .iter()
                            .max_by_key(|nid| {
                                doc.node_layer_and_index(nid)
                                    .map(|(lid, pos)| {
                                        let layer_pos = doc
                                            .layer_order
                                            .iter()
                                            .position(|id| id == &lid)
                                            .unwrap_or(0);
                                        (layer_pos, pos)
                                    })
                                    .unwrap_or((0, 0))
                            })
                            .copied();

                        if let Some(front_id) = frontmost_id {
                            if let Some(front_node) = doc.nodes.get(&front_id).cloned() {
                                if let SceneNodeKind::Path(front_pn) = &front_node.kind {
                                    let mut result_path = gui_apply_affine_to_path(
                                        &front_pn.path_data,
                                        front_node.transform.to_kurbo(),
                                    );
                                    let mut cmds: Vec<Command> = Vec::new();
                                    let mut had_error = false;

                                    for nid in &ids {
                                        if *nid == front_id {
                                            continue;
                                        }
                                        if let Some(node) = doc.nodes.get(nid).cloned() {
                                            if let SceneNodeKind::Path(pn) = &node.kind {
                                                let baked = gui_apply_affine_to_path(
                                                    &pn.path_data,
                                                    node.transform.to_kurbo(),
                                                );
                                                match boolean_op(
                                                    &result_path,
                                                    &baked,
                                                    BooleanOp::Subtract,
                                                ) {
                                                    Ok(p) => result_path = p,
                                                    Err(_) => {
                                                        had_error = true;
                                                        break;
                                                    }
                                                }
                                                cmds.push(Command::RemoveNode { node_id: *nid });
                                            }
                                        }
                                    }

                                    if !had_error {
                                        let mut new_front = front_node.clone();
                                        if let SceneNodeKind::Path(ref mut new_pn) = new_front.kind
                                        {
                                            new_pn.path_data = result_path;
                                        }
                                        new_front.transform = Transform::IDENTITY;
                                        let update = Command::UpdateNode {
                                            old: front_node,
                                            new: new_front,
                                        };
                                        // UpdateNode first, then removes, so undo order is correct.
                                        let mut all_cmds = vec![update];
                                        all_cmds.extend(cmds);
                                        history.execute(Command::Batch(all_cmds), doc);
                                        doc.selection.clear();
                                        doc.selection.add(front_id);
                                        doc_modified = true;
                                    }
                                }
                            }
                        }
                    }
                }

                PanelAction::PathfinderMinusFront { node_ids } => {
                    use photonic_core::ops::boolean::{boolean_op, BooleanOp};
                    use photonic_core::transform::Transform;

                    let ids: Vec<NodeId> = if node_ids.is_empty() {
                        doc.selection.ids().copied().collect()
                    } else {
                        node_ids
                    };

                    if ids.len() >= 2 {
                        // Find the frontmost node by z-order.
                        let frontmost_id = ids
                            .iter()
                            .max_by_key(|nid| {
                                doc.node_layer_and_index(nid)
                                    .map(|(lid, pos)| {
                                        let layer_pos = doc
                                            .layer_order
                                            .iter()
                                            .position(|id| id == &lid)
                                            .unwrap_or(0);
                                        (layer_pos, pos)
                                    })
                                    .unwrap_or((0, 0))
                            })
                            .copied();

                        if let Some(front_id) = frontmost_id {
                            if let Some(front_node) = doc.nodes.get(&front_id).cloned() {
                                if let SceneNodeKind::Path(front_pn) = &front_node.kind {
                                    let front_path = gui_apply_affine_to_path(
                                        &front_pn.path_data,
                                        front_node.transform.to_kurbo(),
                                    );
                                    let mut cmds: Vec<Command> = Vec::new();
                                    let mut had_error = false;

                                    for nid in &ids {
                                        if *nid == front_id {
                                            continue;
                                        }
                                        if let Some(node) = doc.nodes.get(nid).cloned() {
                                            if let SceneNodeKind::Path(pn) = &node.kind {
                                                let baked = gui_apply_affine_to_path(
                                                    &pn.path_data,
                                                    node.transform.to_kurbo(),
                                                );
                                                match boolean_op(
                                                    &baked,
                                                    &front_path,
                                                    BooleanOp::Subtract,
                                                ) {
                                                    Ok(result) => {
                                                        let mut new_node = node.clone();
                                                        if let SceneNodeKind::Path(ref mut new_pn) =
                                                            new_node.kind
                                                        {
                                                            new_pn.path_data = result;
                                                        }
                                                        new_node.transform = Transform::IDENTITY;
                                                        cmds.push(Command::UpdateNode {
                                                            old: node,
                                                            new: new_node,
                                                        });
                                                    }
                                                    Err(_) => {
                                                        had_error = true;
                                                        break;
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    if !had_error && !cmds.is_empty() {
                                        cmds.push(Command::RemoveNode { node_id: front_id });
                                        history.execute(Command::Batch(cmds), doc);
                                        doc.selection.clear();
                                        doc_modified = true;
                                    }
                                }
                            }
                        }
                    }
                }

                PanelAction::PathfinderTrim { node_ids } => {
                    use photonic_core::ops::boolean::{boolean_op, BooleanOp};
                    use photonic_core::transform::Transform;

                    let ids: Vec<NodeId> = if node_ids.is_empty() {
                        doc.selection.ids().copied().collect()
                    } else {
                        node_ids
                    };

                    if ids.len() >= 2 {
                        // Sort back-to-front by z-order.
                        let mut sorted_ids = ids.clone();
                        sorted_ids.sort_by_key(|nid| {
                            doc.node_layer_and_index(nid)
                                .map(|(lid, pos)| {
                                    let layer_pos = doc
                                        .layer_order
                                        .iter()
                                        .position(|id| id == &lid)
                                        .unwrap_or(0);
                                    (layer_pos, pos)
                                })
                                .unwrap_or((0, 0))
                        });

                        // Bake all paths.
                        let baked: Vec<(NodeId, photonic_core::path::PathData)> = sorted_ids
                            .iter()
                            .filter_map(|nid| {
                                let node = doc.nodes.get(nid)?;
                                if let SceneNodeKind::Path(pn) = &node.kind {
                                    Some((
                                        *nid,
                                        gui_apply_affine_to_path(
                                            &pn.path_data,
                                            node.transform.to_kurbo(),
                                        ),
                                    ))
                                } else {
                                    None
                                }
                            })
                            .collect();

                        if baked.len() >= 2 {
                            let mut cmds: Vec<Command> = Vec::new();
                            let mut had_error = false;

                            for i in 0..baked.len() {
                                let (nid, ref path) = baked[i];
                                let mut trimmed = path.clone();
                                for j in (i + 1)..baked.len() {
                                    match boolean_op(&trimmed, &baked[j].1, BooleanOp::Subtract) {
                                        Ok(p) => trimmed = p,
                                        Err(_) => {
                                            had_error = true;
                                            break;
                                        }
                                    }
                                }
                                if had_error {
                                    break;
                                }
                                if let Some(node) = doc.nodes.get(&nid).cloned() {
                                    let mut new_node = node.clone();
                                    if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
                                        new_pn.path_data = trimmed;
                                        new_pn.stroke.enabled = false;
                                    }
                                    new_node.transform = Transform::IDENTITY;
                                    cmds.push(Command::UpdateNode {
                                        old: node,
                                        new: new_node,
                                    });
                                }
                            }

                            if !had_error && !cmds.is_empty() {
                                history.execute(Command::Batch(cmds), doc);
                                doc_modified = true;
                            }
                        }
                    }
                }

                PanelAction::PathfinderOutline { node_ids } => {
                    use photonic_core::style::{Fill, FillKind};

                    let ids: Vec<NodeId> = if node_ids.is_empty() {
                        doc.selection.ids().copied().collect()
                    } else {
                        node_ids
                    };

                    let mut cmds: Vec<Command> = Vec::new();
                    for nid in &ids {
                        if let Some(node) = doc.nodes.get(nid).cloned() {
                            if let SceneNodeKind::Path(ref pn) = node.kind {
                                let stroke_color = match &pn.fill.kind {
                                    FillKind::Solid(c) => *c,
                                    FillKind::Gradient(g) => g
                                        .stops
                                        .first()
                                        .map(|s| s.color)
                                        .unwrap_or(photonic_core::color::Color::BLACK),
                                    FillKind::FluidGradient(fg) => fg
                                        .points
                                        .first()
                                        .map(|p| p.color)
                                        .unwrap_or(photonic_core::color::Color::BLACK),
                                    _ => photonic_core::color::Color::BLACK,
                                };
                                let stroke_width = if pn.stroke.enabled {
                                    pn.stroke.width
                                } else {
                                    1.0
                                };
                                let mut new_node = node.clone();
                                if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
                                    new_pn.fill = Fill::none();
                                    new_pn.stroke.color = stroke_color;
                                    new_pn.stroke.width = stroke_width;
                                    new_pn.stroke.enabled = true;
                                }
                                cmds.push(Command::UpdateNode {
                                    old: node,
                                    new: new_node,
                                });
                            }
                        }
                    }
                    if !cmds.is_empty() {
                        history.execute(Command::Batch(cmds), doc);
                        doc_modified = true;
                    }
                }

                PanelAction::PathfinderDivide { node_ids } => {
                    use photonic_core::node::PathNode;
                    use photonic_core::ops::boolean::divide_paths;
                    use photonic_core::ops::transform_ops::apply_affine_to_path;

                    let ids: Vec<NodeId> = if node_ids.is_empty() {
                        doc.selection.ids().copied().collect()
                    } else {
                        node_ids
                    };
                    if ids.len() == 2 {
                        let back_id = ids[0];
                        let front_id = ids[1];
                        if let (Some(back_node), Some(front_node)) = (
                            doc.nodes.get(&back_id).cloned(),
                            doc.nodes.get(&front_id).cloned(),
                        ) {
                            if let (
                                SceneNodeKind::Path(ref back_pn),
                                SceneNodeKind::Path(ref front_pn),
                            ) = (&back_node.kind, &front_node.kind)
                            {
                                let back_baked = apply_affine_to_path(
                                    &back_pn.path_data,
                                    back_node.transform.to_kurbo(),
                                );
                                let front_baked = apply_affine_to_path(
                                    &front_pn.path_data,
                                    front_node.transform.to_kurbo(),
                                );
                                let faces = divide_paths(&back_baked, &front_baked);
                                if !faces.is_empty() {
                                    let target_layer = back_node.layer_id;
                                    let source_pns: [&PathNode; 2] = [back_pn, front_pn];
                                    let source_nodes: [&SceneNode; 2] = [&back_node, &front_node];
                                    let mut cmds: Vec<Command> = Vec::new();
                                    cmds.push(Command::RemoveNode { node_id: back_id });
                                    cmds.push(Command::RemoveNode { node_id: front_id });
                                    for (i, (path_data, source_idx)) in
                                        faces.into_iter().enumerate()
                                    {
                                        let src_pn = source_pns[source_idx];
                                        let src_node = source_nodes[source_idx];
                                        let mut new_pn = src_pn.clone();
                                        new_pn.path_data = path_data;
                                        let mut new_node = SceneNode::new(
                                            format!("{} face {}", src_node.name, i + 1),
                                            target_layer,
                                            SceneNodeKind::Path(new_pn),
                                        );
                                        new_node.opacity = src_node.opacity;
                                        new_node.blend_mode = src_node.blend_mode;
                                        new_node.tags = src_node.tags.clone();
                                        cmds.push(Command::AddNode {
                                            node: new_node,
                                            layer_id: Some(target_layer),
                                        });
                                    }
                                    history.execute(Command::Batch(cmds), doc);
                                    doc_modified = true;
                                }
                            }
                        }
                    }
                }

                PanelAction::PathfinderMerge { node_ids } => {
                    use photonic_core::ops::boolean::{boolean_op, BooleanOp};
                    use photonic_core::style::FillKind;
                    use std::collections::HashMap;

                    let ids: Vec<NodeId> = if node_ids.is_empty() {
                        doc.selection.ids().copied().collect()
                    } else {
                        node_ids
                    };

                    if ids.len() >= 2 {
                        // Sort back-to-front.
                        let mut sorted_ids = ids.clone();
                        sorted_ids.sort_by_key(|nid| {
                            doc.node_layer_and_index(nid)
                                .map(|(lid, pos)| {
                                    let lp = doc
                                        .layer_order
                                        .iter()
                                        .position(|id| id == &lid)
                                        .unwrap_or(0);
                                    (lp, pos)
                                })
                                .unwrap_or((0, 0))
                        });

                        let target_layer = doc
                            .nodes
                            .get(&sorted_ids[0])
                            .map(|n| n.layer_id)
                            .unwrap_or_else(|| doc.layer_order[0]);

                        // Collect only path nodes.
                        let baked: Vec<(NodeId, photonic_core::path::PathData)> = sorted_ids
                            .iter()
                            .filter_map(|nid| {
                                let node = doc.nodes.get(nid)?;
                                if let SceneNodeKind::Path(pn) = &node.kind {
                                    Some((
                                        *nid,
                                        gui_apply_affine_to_path(
                                            &pn.path_data,
                                            node.transform.to_kurbo(),
                                        ),
                                    ))
                                } else {
                                    None
                                }
                            })
                            .collect();

                        if baked.len() >= 2 {
                            // Trim each path: subtract all paths above it.
                            let mut trimmed: Vec<(NodeId, photonic_core::path::PathData)> =
                                Vec::new();
                            let mut had_error = false;
                            for i in 0..baked.len() {
                                let (nid, ref path) = baked[i];
                                let mut t = path.clone();
                                for j in (i + 1)..baked.len() {
                                    match boolean_op(&t, &baked[j].1, BooleanOp::Subtract) {
                                        Ok(p) => t = p,
                                        Err(_) => {
                                            had_error = true;
                                            break;
                                        }
                                    }
                                }
                                if had_error {
                                    break;
                                }
                                trimmed.push((nid, t));
                            }

                            if !had_error {
                                // Group trimmed faces by fill color key.
                                let mut groups: Vec<(String, Vec<photonic_core::path::PathData>)> =
                                    Vec::new();
                                let mut key_idx: HashMap<String, usize> = HashMap::new();
                                let mut key_rep: HashMap<String, NodeId> = HashMap::new();
                                for (nid, t_path) in &trimmed {
                                    let k = match doc.nodes.get(nid).map(|n| &n.kind) {
                                        Some(SceneNodeKind::Path(pn)) => match &pn.fill.kind {
                                            FillKind::Solid(c) => format!(
                                                "solid:{:.2},{:.2},{:.2},{:.2}",
                                                c.r, c.g, c.b, c.a
                                            ),
                                            _ => format!("other:{}", nid),
                                        },
                                        _ => format!("other:{}", nid),
                                    };
                                    if let Some(&idx) = key_idx.get(&k) {
                                        groups[idx].1.push(t_path.clone());
                                    } else {
                                        let idx = groups.len();
                                        key_idx.insert(k.clone(), idx);
                                        key_rep.insert(k.clone(), *nid);
                                        groups.push((k, vec![t_path.clone()]));
                                    }
                                }

                                // Union each group and build result nodes.
                                let mut cmds: Vec<Command> = Vec::new();
                                for nid in &sorted_ids {
                                    cmds.push(Command::RemoveNode { node_id: *nid });
                                }
                                let mut union_err = false;
                                for (key, paths) in &groups {
                                    let mut merged = paths[0].clone();
                                    for path in &paths[1..] {
                                        match boolean_op(&merged, path, BooleanOp::Union) {
                                            Ok(p) => merged = p,
                                            Err(_) => {
                                                union_err = true;
                                                break;
                                            }
                                        }
                                    }
                                    if union_err {
                                        break;
                                    }
                                    if let Some(rep_id) = key_rep.get(key) {
                                        if let Some(rep_node) = doc.nodes.get(rep_id).cloned() {
                                            if let SceneNodeKind::Path(ref rep_pn) = rep_node.kind {
                                                let mut new_pn = rep_pn.clone();
                                                new_pn.path_data = merged;
                                                new_pn.stroke.enabled = false;
                                                let label = if paths.len() > 1 {
                                                    format!("{} merged", rep_node.name)
                                                } else {
                                                    rep_node.name.clone()
                                                };
                                                let mut new_node = SceneNode::new(
                                                    label,
                                                    target_layer,
                                                    SceneNodeKind::Path(new_pn),
                                                );
                                                new_node.opacity = rep_node.opacity;
                                                new_node.blend_mode = rep_node.blend_mode;
                                                cmds.push(Command::AddNode {
                                                    node: new_node,
                                                    layer_id: Some(target_layer),
                                                });
                                            }
                                        }
                                    }
                                }

                                if !union_err && cmds.len() > sorted_ids.len() {
                                    history.execute(Command::Batch(cmds), doc);
                                    doc_modified = true;
                                }
                            }
                        }
                    }
                }

                PanelAction::DivideObjectsBelow { node_id } => {
                    use photonic_core::ops::boolean::{boolean_op, divide_paths, BooleanOp};
                    use photonic_core::ops::transform_ops::apply_affine_to_path;

                    if let Some(cutter_node) = doc.nodes.get(&node_id).cloned() {
                        if let SceneNodeKind::Path(ref cutter_pn) = cutter_node.kind {
                            let cutter_baked = apply_affine_to_path(
                                &cutter_pn.path_data,
                                cutter_node.transform.to_kurbo(),
                            );
                            if let Some((cutter_layer_id, cutter_z)) =
                                doc.node_layer_and_index(&node_id)
                            {
                                let below_ids: Vec<NodeId> = doc
                                    .layers
                                    .get(&cutter_layer_id)
                                    .map(|l| l.node_ids[..cutter_z].to_vec())
                                    .unwrap_or_default();

                                let mut cmds: Vec<Command> = Vec::new();
                                for target_id in &below_ids {
                                    if let Some(target_node) = doc.nodes.get(target_id).cloned() {
                                        if let SceneNodeKind::Path(ref target_pn) = target_node.kind
                                        {
                                            let target_baked = apply_affine_to_path(
                                                &target_pn.path_data,
                                                target_node.transform.to_kurbo(),
                                            );
                                            let overlap = boolean_op(
                                                &target_baked,
                                                &cutter_baked,
                                                BooleanOp::Intersect,
                                            )
                                            .unwrap_or_else(|_| {
                                                photonic_core::PathData::from_bez_path(
                                                    &kurbo::BezPath::new(),
                                                )
                                            });
                                            if overlap.is_empty() {
                                                continue;
                                            }
                                            let faces = divide_paths(&target_baked, &cutter_baked);
                                            cmds.push(Command::RemoveNode {
                                                node_id: *target_id,
                                            });
                                            for (i, (path_data, _)) in faces.into_iter().enumerate()
                                            {
                                                let mut new_pn = target_pn.clone();
                                                new_pn.path_data = path_data;
                                                let mut new_node = SceneNode::new(
                                                    format!("{} face {}", target_node.name, i + 1),
                                                    cutter_layer_id,
                                                    SceneNodeKind::Path(new_pn),
                                                );
                                                new_node.opacity = target_node.opacity;
                                                new_node.blend_mode = target_node.blend_mode;
                                                new_node.tags = target_node.tags.clone();
                                                cmds.push(Command::AddNode {
                                                    node: new_node,
                                                    layer_id: Some(cutter_layer_id),
                                                });
                                            }
                                        }
                                    }
                                }
                                cmds.push(Command::RemoveNode { node_id });
                                if !cmds.is_empty() {
                                    history.execute(Command::Batch(cmds), doc);
                                    doc_modified = true;
                                }
                            }
                        }
                    }
                }

                PanelAction::MakeCompoundPath { node_ids } => {
                    let ids: Vec<NodeId> = if node_ids.is_empty() {
                        doc.selection.ids().copied().collect()
                    } else {
                        node_ids
                    };
                    if ids.len() >= 2 {
                        // Find bottommost node (lowest z-order) as style source.
                        let bottom_id = ids
                            .iter()
                            .min_by_key(|nid| {
                                doc.node_layer_and_index(nid)
                                    .map(|(lid, pos)| {
                                        let layer_pos = doc
                                            .layer_order
                                            .iter()
                                            .position(|id| id == &lid)
                                            .unwrap_or(0);
                                        (layer_pos, pos)
                                    })
                                    .unwrap_or((0, 0))
                            })
                            .copied();

                        if let Some(base_id) = bottom_id {
                            // Delegate to MCP handler by collecting paths.
                            // We need to do it inline here since MCP handler is async and mutexed.
                            // Use the same logic: merge all baked paths into one PathData.
                            let base_node = doc.nodes.get(&base_id).cloned();
                            if let Some(base_node) = base_node {
                                if let SceneNodeKind::Path(ref base_pn) = base_node.kind {
                                    // Build merged path by appending all subpaths.
                                    let mut merged_bez = base_pn.path_data.to_bez_path();
                                    for nid in &ids {
                                        if *nid == base_id {
                                            continue;
                                        }
                                        if let Some(n) = doc.nodes.get(nid) {
                                            if let SceneNodeKind::Path(pn) = &n.kind {
                                                let baked = gui_apply_affine_to_path(
                                                    &pn.path_data,
                                                    n.transform.to_kurbo(),
                                                );
                                                for el in baked.to_bez_path().elements() {
                                                    merged_bez.push(*el);
                                                }
                                            }
                                        }
                                    }
                                    let compound_path =
                                        photonic_core::path::PathData::from_bez_path(&merged_bez);
                                    let (base_layer_id, base_index) =
                                        doc.node_layer_and_index(&base_id).unwrap_or_default();
                                    let mut compound_pn =
                                        photonic_core::node::PathNode::new(compound_path);
                                    compound_pn.fill = base_pn.fill.clone();
                                    compound_pn.stroke = base_pn.stroke.clone();
                                    compound_pn.is_compound = true;
                                    let compound_node = SceneNode::new(
                                        format!("{} (compound)", base_node.name),
                                        base_layer_id,
                                        SceneNodeKind::Path(compound_pn),
                                    );
                                    let compound_id = compound_node.id;
                                    let mut cmds = vec![Command::AddNode {
                                        node: compound_node,
                                        layer_id: Some(base_layer_id),
                                    }];
                                    cmds.push(Command::ReorderNode {
                                        layer_id: base_layer_id,
                                        node_id: compound_id,
                                        old_index: doc.layers[&base_layer_id].node_ids.len(),
                                        new_index: base_index,
                                    });
                                    for nid in &ids {
                                        cmds.push(Command::RemoveNode { node_id: *nid });
                                    }
                                    history.execute(Command::Batch(cmds), doc);
                                    doc.selection.clear();
                                    doc.selection.add(compound_id);
                                    doc_modified = true;
                                }
                            }
                        }
                    }
                }

                PanelAction::ReleaseCompoundPath { node_id } => {
                    if let Some(node) = doc.nodes.get(&node_id).cloned() {
                        if let SceneNodeKind::Path(ref pn) = node.kind {
                            // Split compound path into subpaths.
                            let bez = pn.path_data.to_bez_path();
                            let mut subpaths: Vec<kurbo::BezPath> = Vec::new();
                            let mut current = kurbo::BezPath::new();
                            for el in bez.elements() {
                                match el {
                                    kurbo::PathEl::MoveTo(_) => {
                                        if !current.elements().is_empty() {
                                            subpaths.push(current.clone());
                                            current = kurbo::BezPath::new();
                                        }
                                        current.push(*el);
                                    }
                                    _ => current.push(*el),
                                }
                            }
                            if !current.elements().is_empty() {
                                subpaths.push(current);
                            }

                            if subpaths.len() <= 1 {
                                // Nothing to release.
                            } else {
                                let (layer_id, _base_index) =
                                    doc.node_layer_and_index(&node_id).unwrap_or_default();
                                let mut cmds = vec![Command::RemoveNode { node_id }];
                                for (i, sub_bez) in subpaths.iter().enumerate() {
                                    let mut sub_pn = photonic_core::node::PathNode::new(
                                        photonic_core::path::PathData::from_bez_path(sub_bez),
                                    );
                                    sub_pn.fill = pn.fill.clone();
                                    sub_pn.stroke = pn.stroke.clone();
                                    sub_pn.is_compound = false;
                                    let sub_node = SceneNode::new(
                                        format!(
                                            "{} {}",
                                            node.name.trim_end_matches(" (compound)"),
                                            i + 1
                                        ),
                                        layer_id,
                                        SceneNodeKind::Path(sub_pn),
                                    );
                                    cmds.push(Command::AddNode {
                                        node: sub_node,
                                        layer_id: Some(layer_id),
                                    });
                                }
                                history.execute(Command::Batch(cmds), doc);
                                doc.selection.clear();
                                doc_modified = true;
                            }
                        }
                    }
                }

                PanelAction::ShearNode {
                    node_ids,
                    shear_x,
                    shear_y,
                } => {
                    if node_ids.len() <= 1 {
                        // Single node: shear about its own local center (unchanged).
                        if let Some(old_node) =
                            node_ids.first().and_then(|id| doc.nodes.get(id).cloned())
                        {
                            let mut new_node = old_node.clone();
                            let (cx, cy) = new_node
                                .local_bounds()
                                .map(|b| (b.x0 + b.width() / 2.0, b.y0 + b.height() / 2.0))
                                .unwrap_or((0.0, 0.0));
                            use photonic_core::ops::transform_ops;
                            transform_ops::shear(&mut new_node, shear_x, shear_y, cx, cy);
                            history.execute(
                                Command::UpdateNode {
                                    old: old_node,
                                    new: new_node,
                                },
                                doc,
                            );
                            doc_modified = true;
                        }
                    } else {
                        // Multi: shear every node about the shared world center.
                        let (cx, cy) = selection_canvas_bounds(doc, &node_ids, renderer)
                            .map(|(x0, y0, x1, y1)| ((x0 + x1) / 2.0, (y0 + y1) / 2.0))
                            .unwrap_or((0.0, 0.0));
                        let m = photonic_core::transform::Transform::shear_around(
                            shear_x, shear_y, cx, cy,
                        );
                        let mut cmds = Vec::new();
                        for nid in &node_ids {
                            if let Some(node) = doc.nodes.get(nid) {
                                let mut new_node = node.clone();
                                // Apply in WORLD space: node transform first, then the
                                // mirror/shear about the shared pivot (correct after moves).
                                new_node.transform = m.then(&node.transform);
                                cmds.push(Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                });
                            }
                        }
                        if !cmds.is_empty() {
                            history.execute(Command::Batch(cmds), doc);
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::DistributeOnPath {
                    path_node_id,
                    node_ids,
                    align,
                } => {
                    // Resolve from selection if path_node_id is nil.
                    let (guide_id, source_ids) = if path_node_id == uuid::Uuid::nil() {
                        let sel: Vec<NodeId> = doc.selection.node_ids.iter().cloned().collect();
                        if sel.len() < 2 {
                            continue;
                        }
                        // The "guide" is the frontmost path node in the selection.
                        // Find the node with the highest position in the document's node order.
                        // Use the first path node from the selection (selection ordering).
                        let guide = sel
                            .iter()
                            .find(|id| {
                                matches!(
                                    doc.nodes.get(id).map(|n| &n.kind),
                                    Some(SceneNodeKind::Path(_))
                                )
                            })
                            .copied();
                        let guide = match guide {
                            Some(g) => g,
                            None => continue,
                        };
                        let sources: Vec<NodeId> =
                            sel.iter().filter(|&&id| id != guide).copied().collect();
                        (guide, sources)
                    } else {
                        (path_node_id, node_ids)
                    };
                    if source_ids.is_empty() {
                        continue;
                    }

                    let path_data = match doc.nodes.get(&guide_id) {
                        Some(n) => match &n.kind {
                            SceneNodeKind::Path(p) => p.path_data.clone(),
                            _ => continue,
                        },
                        None => continue,
                    };
                    let positions = path_data.sample_positions(source_ids.len());
                    if positions.is_empty() {
                        continue;
                    }

                    let mut commands: Vec<Command> = Vec::new();
                    for (k, (px, py, angle_deg)) in positions.iter().enumerate() {
                        let src_id = source_ids[k % source_ids.len()];
                        if let Some(src) = doc.nodes.get(&src_id).cloned() {
                            let mut new_node = src.clone();
                            new_node.id = uuid::Uuid::new_v4();
                            new_node.name = format!("{} {}", src.name, k + 1);
                            new_node.transform.matrix[4] = px + src.transform.matrix[4];
                            new_node.transform.matrix[5] = py + src.transform.matrix[5];
                            if align {
                                use std::f64::consts::PI;
                                let rad = angle_deg * PI / 180.0;
                                let (cos_r, sin_r) = (rad.cos(), rad.sin());
                                let m = &src.transform.matrix;
                                new_node.transform.matrix[0] = m[0] * cos_r + m[2] * sin_r;
                                new_node.transform.matrix[1] = m[1] * cos_r + m[3] * sin_r;
                                new_node.transform.matrix[2] = -m[0] * sin_r + m[2] * cos_r;
                                new_node.transform.matrix[3] = -m[1] * sin_r + m[3] * cos_r;
                            }
                            let lid = new_node.layer_id;
                            commands.push(Command::AddNode {
                                node: new_node,
                                layer_id: Some(lid),
                            });
                        }
                    }
                    if !commands.is_empty() {
                        history.execute(Command::Batch(commands), doc);
                        doc_modified = true;
                    }
                }

                PanelAction::SnapToPixel { node_ids } => {
                    let mut commands: Vec<Command> = Vec::new();
                    for id in node_ids {
                        if let Some(old_node) = doc.nodes.get(&id).cloned() {
                            let mut new_node = old_node.clone();
                            new_node.transform.matrix[4] = new_node.transform.matrix[4].round();
                            new_node.transform.matrix[5] = new_node.transform.matrix[5].round();
                            let dx =
                                (old_node.transform.matrix[4] - new_node.transform.matrix[4]).abs();
                            let dy =
                                (old_node.transform.matrix[5] - new_node.transform.matrix[5]).abs();
                            if dx > 1e-9 || dy > 1e-9 {
                                commands.push(Command::UpdateNode {
                                    old: old_node,
                                    new: new_node,
                                });
                            }
                        }
                    }
                    if !commands.is_empty() {
                        history.execute(Command::Batch(commands), doc);
                        doc_modified = true;
                    }
                }

                PanelAction::SelectSame { node_id, attribute } => {
                    let ref_node = doc.nodes.get(&node_id).cloned();
                    if let Some(ref_node) = ref_node {
                        let tolerance: f32 = 0.01;
                        let mut matched: Vec<NodeId> = Vec::new();
                        for (nid, node) in &doc.nodes {
                            let hits = match attribute {
                                SelectSameAttr::FillColor => {
                                    let rc = gui_solid_fill_color(&ref_node);
                                    let cc = gui_solid_fill_color(node);
                                    match (rc, cc) {
                                        (Some(rc), Some(cc)) => gui_color_dist(rc, cc) <= tolerance,
                                        (None, None) => true,
                                        _ => false,
                                    }
                                }
                                SelectSameAttr::StrokeColor => {
                                    if let (SceneNodeKind::Path(rp), SceneNodeKind::Path(cp)) =
                                        (&ref_node.kind, &node.kind)
                                    {
                                        match (rp.stroke.enabled, cp.stroke.enabled) {
                                            (true, true) => {
                                                gui_color_dist(rp.stroke.color, cp.stroke.color)
                                                    <= tolerance
                                            }
                                            (false, false) => true,
                                            _ => false,
                                        }
                                    } else {
                                        false
                                    }
                                }
                                SelectSameAttr::StrokeWeight => {
                                    if let (SceneNodeKind::Path(rp), SceneNodeKind::Path(cp)) =
                                        (&ref_node.kind, &node.kind)
                                    {
                                        (rp.stroke.width - cp.stroke.width).abs()
                                            <= tolerance as f64
                                    } else {
                                        false
                                    }
                                }
                                SelectSameAttr::Opacity => {
                                    (ref_node.opacity - node.opacity).abs() <= tolerance
                                }
                                SelectSameAttr::BlendMode => ref_node.blend_mode == node.blend_mode,
                                SelectSameAttr::ObjectType => {
                                    std::mem::discriminant(&ref_node.kind)
                                        == std::mem::discriminant(&node.kind)
                                }
                            };
                            if hits {
                                matched.push(*nid);
                            }
                        }
                        doc.selection.clear();
                        for nid in matched {
                            doc.selection.add(nid);
                        }
                        doc_modified = true;
                    }
                }

                PanelAction::ReversePathDirection { node_id } => {
                    if let Some(node) = doc.nodes.get(&node_id) {
                        if let SceneNodeKind::Path(pn) = &node.kind {
                            let reversed = pn.path_data.reverse();
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
                                new_pn.path_data = reversed;
                            }
                            let cmd = Command::UpdateNode {
                                old: node.clone(),
                                new: new_node,
                            };
                            history.execute(cmd, doc);
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::AverageAnchorPoints { node_id } => {
                    if let Some(node) = doc.nodes.get(&node_id) {
                        if let SceneNodeKind::Path(pn) = &node.kind {
                            let averaged = pn.path_data.average_anchor_points(true, true);
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
                                new_pn.path_data = averaged;
                            }
                            let cmd = Command::UpdateNode {
                                old: node.clone(),
                                new: new_node,
                            };
                            history.execute(cmd, doc);
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::OpenSimplifyDialog { node_id } => {
                    let name = doc
                        .nodes
                        .get(&node_id)
                        .map(|n| n.name.clone())
                        .unwrap_or_else(|| node_id.to_string());
                    self.simplify_dialog = Some(SimplifyDialog {
                        node_id,
                        node_name: name,
                        tolerance: 1.0,
                    });
                }

                PanelAction::OpenFindReplaceTextDialog => {
                    self.find_replace_text_dialog = Some(FindReplaceTextDialog {
                        find: String::new(),
                        replace: String::new(),
                        regex: false,
                        case_sensitive: true,
                        selection_only: false,
                    });
                }

                PanelAction::OutlineStroke { node_id } => {
                    use photonic_core::ops::stroke_outline::outline_stroke as do_outline;
                    use photonic_core::style::{Fill, FillKind};
                    if let Some(node) = doc.nodes.get(&node_id).cloned() {
                        if let SceneNodeKind::Path(ref pn) = node.kind {
                            if pn.stroke.enabled {
                                if let Ok(outline_data) = do_outline(&pn.path_data, &pn.stroke) {
                                    let layer_id = node.layer_id;
                                    let stroke_color = pn.stroke.color;
                                    let stroke_opacity = pn.stroke.opacity;
                                    let mut outline_pn = PathNode::new(outline_data);
                                    outline_pn.fill = Fill {
                                        kind: FillKind::Solid(stroke_color),
                                        opacity: stroke_opacity,
                                        enabled: true,
                                    };
                                    outline_pn.stroke = photonic_core::style::Stroke::none();
                                    let outline_node = SceneNode::new(
                                        &format!("{} outline", node.name),
                                        layer_id,
                                        SceneNodeKind::Path(outline_pn),
                                    );
                                    let mut updated_orig = node.clone();
                                    if let SceneNodeKind::Path(ref mut op) = updated_orig.kind {
                                        op.stroke.enabled = false;
                                    }
                                    let batch = Command::Batch(vec![
                                        Command::AddNode {
                                            node: outline_node,
                                            layer_id: Some(layer_id),
                                        },
                                        Command::UpdateNode {
                                            old: node,
                                            new: updated_orig,
                                        },
                                    ]);
                                    history.execute(batch, doc);
                                    doc_modified = true;
                                }
                            }
                        }
                    }
                }

                PanelAction::InvertColors { node_ids } => {
                    use photonic_core::style::FillKind;
                    let ids: Vec<NodeId> = if node_ids.is_empty() {
                        doc.selection.ids().copied().collect()
                    } else {
                        node_ids
                    };
                    let mut cmds: Vec<Command> = Vec::new();
                    for id in &ids {
                        if let Some(node) = doc.nodes.get(id) {
                            if let SceneNodeKind::Path(_) = &node.kind {
                                let mut new_node = node.clone();
                                if let SceneNodeKind::Path(ref mut np) = new_node.kind {
                                    match &mut np.fill.kind {
                                        FillKind::Solid(c) => *c = c.invert(),
                                        FillKind::Gradient(g) => {
                                            for stop in &mut g.stops {
                                                stop.color = stop.color.invert();
                                            }
                                        }
                                        FillKind::FluidGradient(fg) => {
                                            for pt in &mut fg.points {
                                                pt.color = pt.color.invert();
                                            }
                                        }
                                        FillKind::MeshGradient(mg) => {
                                            for v in &mut mg.vertices {
                                                v.color = v.color.invert();
                                            }
                                        }
                                        FillKind::None => {}
                                    }
                                    if np.stroke.enabled {
                                        np.stroke.color = np.stroke.color.invert();
                                    }
                                }
                                cmds.push(Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                });
                            }
                        }
                    }
                    if !cmds.is_empty() {
                        history.execute(Command::Batch(cmds), doc);
                        doc_modified = true;
                    }
                }

                PanelAction::ConvertToGrayscale { node_ids } => {
                    use photonic_core::style::FillKind;
                    let ids: Vec<NodeId> = if node_ids.is_empty() {
                        doc.selection.ids().copied().collect()
                    } else {
                        node_ids
                    };
                    let mut cmds: Vec<Command> = Vec::new();
                    for id in &ids {
                        if let Some(node) = doc.nodes.get(id) {
                            if let SceneNodeKind::Path(_) = &node.kind {
                                let mut new_node = node.clone();
                                if let SceneNodeKind::Path(ref mut np) = new_node.kind {
                                    match &mut np.fill.kind {
                                        FillKind::Solid(c) => *c = c.to_grayscale(),
                                        FillKind::Gradient(g) => {
                                            for stop in &mut g.stops {
                                                stop.color = stop.color.to_grayscale();
                                            }
                                        }
                                        FillKind::FluidGradient(fg) => {
                                            for pt in &mut fg.points {
                                                pt.color = pt.color.to_grayscale();
                                            }
                                        }
                                        FillKind::MeshGradient(mg) => {
                                            for v in &mut mg.vertices {
                                                v.color = v.color.to_grayscale();
                                            }
                                        }
                                        FillKind::None => {}
                                    }
                                    if np.stroke.enabled {
                                        np.stroke.color = np.stroke.color.to_grayscale();
                                    }
                                }
                                cmds.push(Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                });
                            }
                        }
                    }
                    if !cmds.is_empty() {
                        history.execute(Command::Batch(cmds), doc);
                        doc_modified = true;
                    }
                }

                PanelAction::AdjustColors {
                    node_ids,
                    delta_r,
                    delta_g,
                    delta_b,
                    delta_a,
                } => {
                    use photonic_core::style::FillKind;
                    let ids: Vec<NodeId> = if node_ids.is_empty() {
                        doc.selection.ids().copied().collect()
                    } else {
                        node_ids
                    };
                    let shift = |c: Color| -> Color {
                        Color {
                            r: (c.r + delta_r).clamp(0.0, 1.0),
                            g: (c.g + delta_g).clamp(0.0, 1.0),
                            b: (c.b + delta_b).clamp(0.0, 1.0),
                            a: (c.a + delta_a).clamp(0.0, 1.0),
                        }
                    };
                    let mut cmds: Vec<Command> = Vec::new();
                    for id in &ids {
                        if let Some(node) = doc.nodes.get(id) {
                            if let SceneNodeKind::Path(_) = &node.kind {
                                let mut new_node = node.clone();
                                if let SceneNodeKind::Path(ref mut np) = new_node.kind {
                                    match &mut np.fill.kind {
                                        FillKind::Solid(c) => *c = shift(*c),
                                        FillKind::Gradient(g) => {
                                            for stop in &mut g.stops {
                                                stop.color = shift(stop.color);
                                            }
                                        }
                                        FillKind::FluidGradient(fg) => {
                                            for pt in &mut fg.points {
                                                pt.color = shift(pt.color);
                                            }
                                        }
                                        FillKind::MeshGradient(mg) => {
                                            for v in &mut mg.vertices {
                                                v.color = shift(v.color);
                                            }
                                        }
                                        FillKind::None => {}
                                    }
                                    if np.stroke.enabled {
                                        np.stroke.color = shift(np.stroke.color);
                                    }
                                }
                                cmds.push(Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                });
                            }
                        }
                    }
                    if !cmds.is_empty() {
                        history.execute(Command::Batch(cmds), doc);
                        doc_modified = true;
                    }
                }

                PanelAction::RecolorArtwork { node_ids, palette } => {
                    use photonic_core::style::FillKind;
                    fn color_dist(a: [f32; 4], b: [f32; 4]) -> f32 {
                        let dr = a[0] - b[0];
                        let dg = a[1] - b[1];
                        let db = a[2] - b[2];
                        dr * dr + dg * dg + db * db
                    }
                    fn nearest(c: [f32; 4], pal: &[[f32; 4]]) -> [f32; 4] {
                        *pal.iter()
                            .min_by(|a, b| {
                                color_dist(c, **a)
                                    .partial_cmp(&color_dist(c, **b))
                                    .unwrap_or(std::cmp::Ordering::Equal)
                            })
                            .unwrap()
                    }
                    let ids: Vec<NodeId> = if node_ids.is_empty() {
                        doc.selection.ids().copied().collect()
                    } else {
                        node_ids
                    };
                    let mut cmds: Vec<Command> = Vec::new();
                    for id in &ids {
                        if let Some(node) = doc.nodes.get(id) {
                            if let SceneNodeKind::Path(pn) = &node.kind {
                                if pn.fill.enabled {
                                    if let FillKind::Solid(c) = &pn.fill.kind {
                                        let orig = [c.r, c.g, c.b, c.a];
                                        let tgt = nearest(orig, &palette);
                                        if (orig[0] - tgt[0]).abs() > 1e-6
                                            || (orig[1] - tgt[1]).abs() > 1e-6
                                            || (orig[2] - tgt[2]).abs() > 1e-6
                                        {
                                            let mut new_node = node.clone();
                                            if let SceneNodeKind::Path(ref mut np) = new_node.kind {
                                                np.fill.kind = FillKind::Solid(Color {
                                                    r: tgt[0],
                                                    g: tgt[1],
                                                    b: tgt[2],
                                                    a: tgt[3],
                                                });
                                            }
                                            cmds.push(Command::UpdateNode {
                                                old: node.clone(),
                                                new: new_node,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if !cmds.is_empty() {
                        history.execute(Command::Batch(cmds), doc);
                        doc_modified = true;
                    }
                }

                PanelAction::RecolorPreview { ids, to } => {
                    // Live preview — mutate the captured nodes directly, no history.
                    use photonic_core::style::FillKind;
                    let new_color = Color {
                        r: to[0],
                        g: to[1],
                        b: to[2],
                        a: to[3],
                    };
                    for id in &ids {
                        if let Some(node) = doc.nodes.get_mut(id) {
                            match &mut node.kind {
                                SceneNodeKind::Path(p) => p.fill.kind = FillKind::Solid(new_color),
                                SceneNodeKind::Text(t) => t.fill.kind = FillKind::Solid(new_color),
                                _ => {}
                            }
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::RecolorCommit { ids, from, to } => {
                    // Commit as a single undoable step: old=`from`, new=`to`.
                    use photonic_core::style::FillKind;
                    let from_color = Color {
                        r: from[0],
                        g: from[1],
                        b: from[2],
                        a: from[3],
                    };
                    let to_color = Color {
                        r: to[0],
                        g: to[1],
                        b: to[2],
                        a: to[3],
                    };
                    if (from[0] - to[0]).abs() > 1e-6
                        || (from[1] - to[1]).abs() > 1e-6
                        || (from[2] - to[2]).abs() > 1e-6
                        || (from[3] - to[3]).abs() > 1e-6
                    {
                        let mut cmds: Vec<Command> = Vec::new();
                        for id in &ids {
                            if let Some(node) = doc.nodes.get(id) {
                                // Fabricate old (fill=from) and new (fill=to) from the
                                // current node so undo restores the original color.
                                let mut old_node = node.clone();
                                let mut new_node = node.clone();
                                match &mut old_node.kind {
                                    SceneNodeKind::Path(p) => {
                                        p.fill.kind = FillKind::Solid(from_color)
                                    }
                                    SceneNodeKind::Text(t) => {
                                        t.fill.kind = FillKind::Solid(from_color)
                                    }
                                    _ => {}
                                }
                                match &mut new_node.kind {
                                    SceneNodeKind::Path(p) => {
                                        p.fill.kind = FillKind::Solid(to_color)
                                    }
                                    SceneNodeKind::Text(t) => {
                                        t.fill.kind = FillKind::Solid(to_color)
                                    }
                                    _ => {}
                                }
                                cmds.push(Command::UpdateNode {
                                    old: old_node,
                                    new: new_node,
                                });
                            }
                        }
                        if !cmds.is_empty() {
                            history.execute(Command::Batch(cmds), doc);
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::UngroupNode { node_id } => {
                    if let Some(node) = doc.nodes.get(&node_id) {
                        if let SceneNodeKind::Group(g) = &node.kind {
                            let children = g.children.clone();
                            let node_clone = node.clone();
                            if let Some((layer_id, group_index)) =
                                doc.node_layer_and_index(&node_id)
                            {
                                let first_child = children.first().copied();
                                let cmd = Command::UngroupNodes {
                                    group: node_clone,
                                    layer_id,
                                    group_index,
                                    children,
                                };
                                history.execute(cmd, doc);
                                self.selected_id = first_child;
                                if let Some(fc) = first_child {
                                    doc.selection = Selection::single(fc);
                                } else {
                                    doc.selection.clear();
                                }
                                doc_modified = true;
                            }
                        }
                    }
                }

                PanelAction::DeleteSelected => {
                    let ids: Vec<_> = doc.selection.ids().copied().collect();
                    if !ids.is_empty() {
                        let cmds = ids
                            .iter()
                            .map(|&id| Command::RemoveNode { node_id: id })
                            .collect();
                        history.execute(Command::Batch(cmds), doc);
                        self.selected_id = None;
                        doc.selection.clear();
                        doc_modified = true;
                    }
                }

                PanelAction::CreateShapeAtPos {
                    shape,
                    canvas_x,
                    canvas_y,
                    fill,
                } => {
                    let half = 50.0_f64;
                    let (sx, sy, ex, ey) = (
                        canvas_x - half,
                        canvas_y - half,
                        canvas_x + half,
                        canvas_y + half,
                    );
                    if shape == ShapeKind::Text {
                        use photonic_core::node::TextNode;
                        let [r, g, b, a] = fill;
                        let mut text_node = TextNode::new("Text");
                        text_node.fill = Fill::solid(Color { r, g, b, a });
                        let num = doc.node_count() + 1;
                        let mut node = SceneNode::new(
                            format!("Text {}", num),
                            Default::default(),
                            SceneNodeKind::Text(text_node),
                        );
                        node.transform =
                            photonic_core::transform::Transform::translate(canvas_x, canvas_y);
                        doc.add_node(node, None);
                        doc_modified = true;
                    } else {
                        let tool = match shape {
                            ShapeKind::Shape(p) => Tool::from_primitive(p),
                            ShapeKind::Text => unreachable!(),
                        };
                        if let Some(path) = self.build_shape_with_tool(tool, sx, sy, ex, ey) {
                            let stroke_arg = self.prefs.default_stroke_enabled.then(|| {
                                (
                                    self.prefs.default_stroke_color,
                                    self.prefs.default_stroke_width,
                                )
                            });
                            let node = make_node(
                                path,
                                fill,
                                stroke_arg,
                                shape.label(),
                                doc.node_count() + 1,
                            );
                            history.execute(
                                Command::AddNode {
                                    node,
                                    layer_id: None,
                                },
                                doc,
                            );
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::GroupSelected => {
                    self.do_group_selected(doc, history, &mut doc_modified);
                }

                PanelAction::CopyAsSvg { node_ids } => {
                    let ids: Vec<_> = if node_ids.is_empty() {
                        doc.selection.ids().copied().collect()
                    } else {
                        node_ids
                    };
                    if !ids.is_empty() {
                        let svg = photonic_core::export::export_nodes_as_svg(doc, &ids);
                        ctx.output_mut(|o| o.copied_text = svg);
                        self.file_status = Some("Copied SVG to clipboard".to_string());
                    }
                }

                PanelAction::DiffWithCheckpoint { checkpoint_id } => {
                    if let Some(snapshot) = history.get_checkpoint_snapshot(checkpoint_id) {
                        let mut highlights = Vec::new();
                        let mut removed_boxes = Vec::new();

                        // Added: in current doc but not in snapshot
                        // Modified: in both but different
                        for (id, node) in &doc.nodes {
                            if !snapshot.nodes.contains_key(id) {
                                highlights.push((*id, DiffCategory::Added));
                            } else if let Some(old) = snapshot.nodes.get(id) {
                                let from_val = serde_json::to_value(old).unwrap_or_default();
                                let to_val = serde_json::to_value(node).unwrap_or_default();
                                if from_val != to_val {
                                    highlights.push((*id, DiffCategory::Modified));
                                }
                            }
                        }

                        // Removed: in snapshot but not in current doc
                        for (id, old_node) in &snapshot.nodes {
                            if !doc.nodes.contains_key(id) {
                                if let Some((cx0, cy0, cx1, cy1)) =
                                    text_aware_canvas_bounds(old_node, renderer)
                                {
                                    removed_boxes.push(egui::Rect::from_min_max(
                                        egui::pos2(cx0 as f32, cy0 as f32),
                                        egui::pos2(cx1 as f32, cy1 as f32),
                                    ));
                                }
                            }
                        }

                        let total = highlights.len() + removed_boxes.len();
                        self.diff.highlights = highlights;
                        self.diff.removed_boxes = removed_boxes;
                        self.diff.overlay_active = true;
                        self.file_status = Some(format!("{} diff change(s) highlighted", total));
                    }
                }

                PanelAction::ClearDiff => {
                    self.diff.highlights.clear();
                    self.diff.removed_boxes.clear();
                    self.diff.overlay_active = false;
                    self.file_status = Some("Diff cleared".to_string());
                }

                PanelAction::StartEyedropper(target) => {
                    self.eyedropper.capture =
                        capture_screen(self.window_logical_pos.0, self.window_logical_pos.1);
                    self.eyedropper.target = Some(target);
                    self.eyedropper.skip_click = true;
                }

                PanelAction::CollectInNewLayer { node_ids } => {
                    self.do_collect_in_new_layer(node_ids, doc, history, &mut doc_modified);
                }

                PanelAction::ReleaseToLayers { node_ids } => {
                    self.do_release_to_layers(node_ids, doc, history, &mut doc_modified);
                }

                PanelAction::MergeLayers { layer_ids } => {
                    self.do_merge_layers(layer_ids, doc, history, &mut doc_modified);
                }

                PanelAction::FlattenArtwork => {
                    let all_ids: Vec<_> = doc.layer_order.clone();
                    if all_ids.len() >= 2 {
                        self.do_merge_layers(all_ids, doc, history, &mut doc_modified);
                    }
                }

                PanelAction::SetLayerColor { layer_id, color } => {
                    if let Some(layer) = doc.layers.get(&layer_id) {
                        let cmd = Command::UpdateLayer {
                            layer_id,
                            old_name: layer.name.clone(),
                            new_name: layer.name.clone(),
                            old_visible: layer.visible,
                            new_visible: layer.visible,
                            old_locked: layer.locked,
                            new_locked: layer.locked,
                            old_color: layer.color,
                            new_color: color,
                            old_is_template: layer.is_template,
                            new_is_template: layer.is_template,
                        };
                        history.execute(cmd, doc);
                        doc_modified = true;
                    }
                }

                PanelAction::SetLayerTemplate {
                    layer_id,
                    is_template,
                } => {
                    if let Some(layer) = doc.layers.get(&layer_id) {
                        let cmd = Command::UpdateLayer {
                            layer_id,
                            old_name: layer.name.clone(),
                            new_name: layer.name.clone(),
                            old_visible: layer.visible,
                            new_visible: layer.visible,
                            old_locked: layer.locked,
                            // Template layers are implicitly locked.
                            new_locked: if is_template { true } else { layer.locked },
                            old_color: layer.color,
                            new_color: layer.color,
                            old_is_template: layer.is_template,
                            new_is_template: is_template,
                        };
                        history.execute(cmd, doc);
                        doc_modified = true;
                    }
                }

                PanelAction::RenameLayer { layer_id, name } => {
                    if let Some(layer) = doc.layers.get(&layer_id) {
                        let cmd = Command::UpdateLayer {
                            layer_id,
                            old_name: layer.name.clone(),
                            new_name: name.clone(),
                            old_visible: layer.visible,
                            new_visible: layer.visible,
                            old_locked: layer.locked,
                            new_locked: layer.locked,
                            old_color: layer.color,
                            new_color: layer.color,
                            old_is_template: layer.is_template,
                            new_is_template: layer.is_template,
                        };
                        history.execute(cmd, doc);
                        doc_modified = true;
                    }
                }

                PanelAction::AlignNodes {
                    operation,
                    key_object_id,
                } => {
                    use photonic_core::transform::Transform;

                    let sel_ids: Vec<NodeId> = doc.selection.ids().copied().collect();
                    if sel_ids.len() >= 2 {
                        let world_bounds = |node: &SceneNode| -> Option<(f64, f64, f64, f64)> {
                            let local = node.local_bounds()?;
                            let corners = [
                                (local.x0, local.y0),
                                (local.x1, local.y0),
                                (local.x1, local.y1),
                                (local.x0, local.y1),
                            ];
                            let pts: Vec<(f64, f64)> = corners
                                .iter()
                                .map(|(x, y)| node.transform.apply(*x, *y))
                                .collect();
                            let min_x = pts.iter().map(|(x, _)| *x).fold(f64::INFINITY, f64::min);
                            let min_y = pts.iter().map(|(_, y)| *y).fold(f64::INFINITY, f64::min);
                            let max_x = pts
                                .iter()
                                .map(|(x, _)| *x)
                                .fold(f64::NEG_INFINITY, f64::max);
                            let max_y = pts
                                .iter()
                                .map(|(_, y)| *y)
                                .fold(f64::NEG_INFINITY, f64::max);
                            Some((min_x, min_y, max_x, max_y))
                        };
                        let node_bounds: Vec<(SceneNode, (f64, f64, f64, f64))> = sel_ids
                            .iter()
                            .filter_map(|id| {
                                doc.nodes
                                    .get(id)
                                    .and_then(|n| world_bounds(n).map(|b| (n.clone(), b)))
                            })
                            .collect();
                        if node_bounds.len() >= 2 {
                            let (ref_x0, ref_y0, ref_x1, ref_y1) =
                                if let Some(key_id) = key_object_id {
                                    node_bounds
                                        .iter()
                                        .find(|(n, _)| n.id == key_id)
                                        .map(|(_, b)| *b)
                                        .unwrap_or_else(|| {
                                            let x0 = node_bounds
                                                .iter()
                                                .map(|(_, b)| b.0)
                                                .fold(f64::INFINITY, f64::min);
                                            let y0 = node_bounds
                                                .iter()
                                                .map(|(_, b)| b.1)
                                                .fold(f64::INFINITY, f64::min);
                                            let x1 = node_bounds
                                                .iter()
                                                .map(|(_, b)| b.2)
                                                .fold(f64::NEG_INFINITY, f64::max);
                                            let y1 = node_bounds
                                                .iter()
                                                .map(|(_, b)| b.3)
                                                .fold(f64::NEG_INFINITY, f64::max);
                                            (x0, y0, x1, y1)
                                        })
                                } else {
                                    let x0 = node_bounds
                                        .iter()
                                        .map(|(_, b)| b.0)
                                        .fold(f64::INFINITY, f64::min);
                                    let y0 = node_bounds
                                        .iter()
                                        .map(|(_, b)| b.1)
                                        .fold(f64::INFINITY, f64::min);
                                    let x1 = node_bounds
                                        .iter()
                                        .map(|(_, b)| b.2)
                                        .fold(f64::NEG_INFINITY, f64::max);
                                    let y1 = node_bounds
                                        .iter()
                                        .map(|(_, b)| b.3)
                                        .fold(f64::NEG_INFINITY, f64::max);
                                    (x0, y0, x1, y1)
                                };
                            let ref_cx = (ref_x0 + ref_x1) / 2.0;
                            let ref_cy = (ref_y0 + ref_y1) / 2.0;
                            let mut cmds: Vec<Command> = Vec::new();
                            for (node, bounds) in &node_bounds {
                                // Skip the key object — it is the reference, not moved.
                                if key_object_id.map(|k| k == node.id).unwrap_or(false) {
                                    continue;
                                }
                                let (nx0, ny0, nx1, ny1) = *bounds;
                                let ncx = (nx0 + nx1) / 2.0;
                                let ncy = (ny0 + ny1) / 2.0;
                                let (dx, dy) = match operation.as_str() {
                                    "left" => (ref_x0 - nx0, 0.0),
                                    "center_horizontal" => (ref_cx - ncx, 0.0),
                                    "right" => (ref_x1 - nx1, 0.0),
                                    "top" => (0.0, ref_y0 - ny0),
                                    "center_vertical" => (0.0, ref_cy - ncy),
                                    "bottom" => (0.0, ref_y1 - ny1),
                                    _ => (0.0, 0.0),
                                };
                                if dx.abs() > 1e-9 || dy.abs() > 1e-9 {
                                    let old = node.clone();
                                    let mut new = old.clone();
                                    new.transform =
                                        new.transform.then(&Transform::translate(dx, dy));
                                    cmds.push(Command::UpdateNode { old, new });
                                }
                            }
                            if !cmds.is_empty() {
                                history.execute(Command::Batch(cmds), doc);
                                doc_modified = true;
                            }
                        }
                    }
                }
                PanelAction::AlignToArtboard { operation } => {
                    use photonic_core::transform::Transform;

                    let sel_ids: Vec<NodeId> = doc.selection.ids().copied().collect();
                    if !sel_ids.is_empty() {
                        let ref_x0 = 0.0_f64;
                        let ref_y0 = 0.0_f64;
                        let ref_x1 = doc.width;
                        let ref_y1 = doc.height;
                        let ref_cx = ref_x1 / 2.0;
                        let ref_cy = ref_y1 / 2.0;

                        let world_bounds = |node: &SceneNode| -> Option<(f64, f64, f64, f64)> {
                            let local = node.local_bounds()?;
                            let corners = [
                                (local.x0, local.y0),
                                (local.x1, local.y0),
                                (local.x1, local.y1),
                                (local.x0, local.y1),
                            ];
                            let pts: Vec<(f64, f64)> = corners
                                .iter()
                                .map(|(x, y)| node.transform.apply(*x, *y))
                                .collect();
                            let min_x = pts.iter().map(|(x, _)| *x).fold(f64::INFINITY, f64::min);
                            let min_y = pts.iter().map(|(_, y)| *y).fold(f64::INFINITY, f64::min);
                            let max_x = pts
                                .iter()
                                .map(|(x, _)| *x)
                                .fold(f64::NEG_INFINITY, f64::max);
                            let max_y = pts
                                .iter()
                                .map(|(_, y)| *y)
                                .fold(f64::NEG_INFINITY, f64::max);
                            Some((min_x, min_y, max_x, max_y))
                        };

                        let mut cmds: Vec<Command> = Vec::new();
                        for id in &sel_ids {
                            if let Some(node) = doc.nodes.get(id) {
                                if let Some((nx0, ny0, nx1, ny1)) = world_bounds(node) {
                                    let ncx = (nx0 + nx1) / 2.0;
                                    let ncy = (ny0 + ny1) / 2.0;
                                    let (dx, dy) = match operation.as_str() {
                                        "left" => (ref_x0 - nx0, 0.0),
                                        "center_horizontal" => (ref_cx - ncx, 0.0),
                                        "right" => (ref_x1 - nx1, 0.0),
                                        "top" => (0.0, ref_y0 - ny0),
                                        "center_vertical" => (0.0, ref_cy - ncy),
                                        "bottom" => (0.0, ref_y1 - ny1),
                                        _ => (0.0, 0.0),
                                    };
                                    if dx.abs() > 1e-9 || dy.abs() > 1e-9 {
                                        let old = node.clone();
                                        let mut new = old.clone();
                                        new.transform =
                                            new.transform.then(&Transform::translate(dx, dy));
                                        cmds.push(Command::UpdateNode { old, new });
                                    }
                                }
                            }
                        }
                        if !cmds.is_empty() {
                            history.execute(Command::Batch(cmds), doc);
                            doc_modified = true;
                        }
                    }
                }
                PanelAction::ClearGuides => {
                    let old_guides = doc.guides.clone();
                    let new_guides: Vec<_> =
                        old_guides.iter().filter(|g| g.locked).cloned().collect();
                    let removed = old_guides.len() - new_guides.len();
                    if removed > 0 {
                        history.execute(
                            Command::SetGuides {
                                old: old_guides,
                                new: new_guides,
                            },
                            doc,
                        );
                        doc_modified = true;
                    }
                }

                PanelAction::ConvertToSmooth { node_ids } => {
                    convert_anchor_points_gui(true, node_ids, doc, history, &mut doc_modified);
                }

                PanelAction::ConvertToCorner { node_ids } => {
                    convert_anchor_points_gui(false, node_ids, doc, history, &mut doc_modified);
                }

                PanelAction::BlendColors {
                    node_ids,
                    direction,
                } => {
                    use photonic_core::style::FillKind;
                    use photonic_core::Color;

                    // Resolve node list: empty vec means "use current selection".
                    let ids: Vec<NodeId> = if node_ids.is_empty() {
                        doc.selection.ids().copied().collect()
                    } else {
                        node_ids
                    };

                    if ids.len() < 2 {
                        // Not enough nodes — silently ignore.
                    } else {
                        // Collect path nodes, filtering non-path kinds.
                        let mut nodes: Vec<SceneNode> = ids
                            .iter()
                            .filter_map(|id| doc.nodes.get(id))
                            .filter(|n| matches!(n.kind, SceneNodeKind::Path(_)))
                            .cloned()
                            .collect();

                        // Sort by the requested direction.
                        match direction.as_str() {
                            "horizontal" => {
                                nodes.sort_by(|a, b| {
                                    gui_path_center_x(a)
                                        .partial_cmp(&gui_path_center_x(b))
                                        .unwrap_or(std::cmp::Ordering::Equal)
                                });
                            }
                            "vertical" => {
                                nodes.sort_by(|a, b| {
                                    gui_path_center_y(a)
                                        .partial_cmp(&gui_path_center_y(b))
                                        .unwrap_or(std::cmp::Ordering::Equal)
                                });
                            }
                            "depth" => {
                                let mut z_index: std::collections::HashMap<NodeId, usize> =
                                    std::collections::HashMap::new();
                                let mut z = 0usize;
                                for layer_id in &doc.layer_order {
                                    if let Some(layer) = doc.layers.get(layer_id) {
                                        for &nid in &layer.node_ids {
                                            z_index.insert(nid, z);
                                            z += 1;
                                        }
                                    }
                                }
                                nodes.sort_by_key(|n| z_index.get(&n.id).copied().unwrap_or(0));
                            }
                            _ => {} // no sort — use provided order
                        }

                        let n = nodes.len();
                        if n >= 2 {
                            // Extract endpoint solid fill colors.
                            let start_opt = match &nodes[0].kind {
                                SceneNodeKind::Path(p) => match &p.fill.kind {
                                    FillKind::Solid(c) => Some(*c),
                                    _ => None,
                                },
                                _ => None,
                            };
                            let end_opt = match &nodes[n - 1].kind {
                                SceneNodeKind::Path(p) => match &p.fill.kind {
                                    FillKind::Solid(c) => Some(*c),
                                    _ => None,
                                },
                                _ => None,
                            };

                            if let (Some(start_color), Some(end_color)) = (start_opt, end_opt) {
                                let mut cmds: Vec<Command> = Vec::new();
                                for (i, node) in nodes.iter().enumerate() {
                                    if i == 0 || i == n - 1 {
                                        continue;
                                    }
                                    let t = i as f32 / (n - 1) as f32;
                                    let blended = Color {
                                        r: start_color.r + t * (end_color.r - start_color.r),
                                        g: start_color.g + t * (end_color.g - start_color.g),
                                        b: start_color.b + t * (end_color.b - start_color.b),
                                        a: start_color.a + t * (end_color.a - start_color.a),
                                    };
                                    let mut new_node = node.clone();
                                    if let SceneNodeKind::Path(ref mut p) = new_node.kind {
                                        p.fill.kind = FillKind::Solid(blended);
                                    }
                                    cmds.push(Command::UpdateNode {
                                        old: node.clone(),
                                        new: new_node,
                                    });
                                }
                                if !cmds.is_empty() {
                                    history.execute(Command::Batch(cmds), doc);
                                    doc_modified = true;
                                }
                            }
                        }
                    }
                }

                PanelAction::ZigZagPath {
                    node_ids,
                    size,
                    ridges,
                    smooth,
                } => {
                    let mut commands = Vec::new();
                    for nid in &node_ids {
                        if let Some(node) = doc.nodes.get(nid) {
                            if let SceneNodeKind::Path(pn) = &node.kind {
                                let bez = pn.path_data.to_bez_path();
                                let new_bez = gui_zig_zag(&bez, size, ridges, smooth);
                                let mut new_node = node.clone();
                                if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
                                    new_pn.path_data = PathData::from_bez_path(&new_bez);
                                }
                                commands.push(Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                });
                            }
                        }
                    }
                    if !commands.is_empty() {
                        for cmd in commands {
                            history.execute(cmd, doc);
                        }
                        doc_modified = true;
                    }
                }

                PanelAction::PuckerBloat { node_ids, strength } => {
                    let mut commands = Vec::new();
                    for nid in &node_ids {
                        if let Some(node) = doc.nodes.get(nid) {
                            if let SceneNodeKind::Path(pn) = &node.kind {
                                let bez = pn.path_data.to_bez_path();
                                let center = gui_path_centroid(&bez);
                                let new_bez = gui_pucker_bloat(&bez, strength, center);
                                let mut new_node = node.clone();
                                if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
                                    new_pn.path_data = PathData::from_bez_path(&new_bez);
                                }
                                commands.push(Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                });
                            }
                        }
                    }
                    if !commands.is_empty() {
                        for cmd in commands {
                            history.execute(cmd, doc);
                        }
                        doc_modified = true;
                    }
                }

                PanelAction::AddDropShadow { node_id } => {
                    if let Some(node) = doc.nodes.get(&node_id) {
                        let mut shadow = node.clone();
                        shadow.id = uuid::Uuid::new_v4();
                        shadow.name = format!("{} Shadow", node.name);
                        shadow.opacity = 0.4;
                        shadow.transform.matrix[4] += 5.0;
                        shadow.transform.matrix[5] += 5.0;
                        match &mut shadow.kind {
                            SceneNodeKind::Path(pn) => {
                                pn.fill = Fill::solid(photonic_core::color::Color::new(
                                    0.0, 0.0, 0.0, 1.0,
                                ));
                                pn.stroke = Stroke::none();
                            }
                            SceneNodeKind::Text(tn) => {
                                tn.fill = Fill::solid(photonic_core::color::Color::new(
                                    0.0, 0.0, 0.0, 1.0,
                                ));
                                tn.stroke = Stroke::none();
                            }
                            SceneNodeKind::Group(_) => {}
                        }
                        history.execute(
                            Command::AddNode {
                                node: shadow,
                                layer_id: Some(node.layer_id),
                            },
                            doc,
                        );
                        doc_modified = true;
                    }
                }

                PanelAction::SetTextTypography {
                    node_id,
                    line_height,
                    letter_spacing,
                } => {
                    if let Some(node) = doc.nodes.get(&node_id) {
                        if let SceneNodeKind::Text(_tn) = &node.kind {
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Text(ref mut new_tn) = new_node.kind {
                                if let Some(lh) = line_height {
                                    new_tn.line_height = lh;
                                }
                                if let Some(ls) = letter_spacing {
                                    new_tn.letter_spacing = ls;
                                }
                            }
                            history.execute(
                                Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                },
                                doc,
                            );
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::FlipNodes {
                    node_ids,
                    horizontal,
                } => {
                    if node_ids.len() <= 1 {
                        // Single node: mirror the path geometry in place (unchanged).
                        let mut commands = Vec::new();
                        for nid in &node_ids {
                            if let Some(node) = doc.nodes.get(nid) {
                                if let SceneNodeKind::Path(pn) = &node.kind {
                                    use kurbo::Shape;
                                    let bez = pn.path_data.to_bez_path();
                                    let bbox = bez.bounding_box();
                                    let cx = bbox.x0 + bbox.width() / 2.0;
                                    let cy = bbox.y0 + bbox.height() / 2.0;
                                    let flip = |p: kurbo::Point| -> kurbo::Point {
                                        kurbo::Point::new(
                                            if horizontal { 2.0 * cx - p.x } else { p.x },
                                            if !horizontal { 2.0 * cy - p.y } else { p.y },
                                        )
                                    };
                                    let mut new_bez = BezPath::new();
                                    for el in bez.elements() {
                                        match *el {
                                            PathEl::MoveTo(p) => new_bez.move_to(flip(p)),
                                            PathEl::LineTo(p) => new_bez.line_to(flip(p)),
                                            PathEl::CurveTo(c1, c2, p) => {
                                                new_bez.curve_to(flip(c1), flip(c2), flip(p))
                                            }
                                            PathEl::QuadTo(c, p) => {
                                                new_bez.quad_to(flip(c), flip(p))
                                            }
                                            PathEl::ClosePath => new_bez.close_path(),
                                        }
                                    }
                                    let mut new_node = node.clone();
                                    if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
                                        new_pn.path_data = PathData::from_bez_path(&new_bez);
                                    }
                                    commands.push(Command::UpdateNode {
                                        old: node.clone(),
                                        new: new_node,
                                    });
                                }
                            }
                        }
                        if !commands.is_empty() {
                            for cmd in commands {
                                history.execute(cmd, doc);
                            }
                            doc_modified = true;
                        }
                    } else {
                        // Multi: mirror the whole selection about its shared center
                        // (any node kind), as one undoable step.
                        let (cx, cy) = selection_canvas_bounds(doc, &node_ids, renderer)
                            .map(|(x0, y0, x1, y1)| ((x0 + x1) / 2.0, (y0 + y1) / 2.0))
                            .unwrap_or((0.0, 0.0));
                        let (sx, sy) = if horizontal { (-1.0, 1.0) } else { (1.0, -1.0) };
                        let m = photonic_core::transform::Transform::scale_around(sx, sy, cx, cy);
                        let mut cmds = Vec::new();
                        for nid in &node_ids {
                            if let Some(node) = doc.nodes.get(nid) {
                                let mut new_node = node.clone();
                                // Apply in WORLD space: node transform first, then the
                                // mirror/shear about the shared pivot (correct after moves).
                                new_node.transform = m.then(&node.transform);
                                cmds.push(Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                });
                            }
                        }
                        if !cmds.is_empty() {
                            history.execute(Command::Batch(cmds), doc);
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::MirrorCopy { node_ids, axis } => {
                    let flip_h = axis != "vertical";
                    let mut commands = Vec::new();
                    for nid in &node_ids {
                        if let Some(node) = doc.nodes.get(nid).cloned() {
                            let layer_id = node.layer_id;
                            let mut cloned = node.clone();
                            cloned.id = uuid::Uuid::new_v4();
                            cloned.name = if cloned.name.is_empty() {
                                "mirror".to_string()
                            } else {
                                format!("{} mirror", cloned.name)
                            };

                            if let SceneNodeKind::Path(ref pn) = node.kind {
                                use kurbo::Shape;
                                let bez = pn.path_data.to_bez_path();
                                let bbox = bez.bounding_box();
                                let cx = bbox.x0 + bbox.width() / 2.0;
                                let cy = bbox.y0 + bbox.height() / 2.0;
                                let flip = |p: kurbo::Point| -> kurbo::Point {
                                    kurbo::Point::new(
                                        if flip_h { 2.0 * cx - p.x } else { p.x },
                                        if !flip_h { 2.0 * cy - p.y } else { p.y },
                                    )
                                };
                                let mut new_bez = BezPath::new();
                                for el in bez.elements() {
                                    match *el {
                                        PathEl::MoveTo(p) => new_bez.move_to(flip(p)),
                                        PathEl::LineTo(p) => new_bez.line_to(flip(p)),
                                        PathEl::CurveTo(c1, c2, p) => {
                                            new_bez.curve_to(flip(c1), flip(c2), flip(p))
                                        }
                                        PathEl::QuadTo(c, p) => new_bez.quad_to(flip(c), flip(p)),
                                        PathEl::ClosePath => new_bez.close_path(),
                                    }
                                }
                                if let SceneNodeKind::Path(ref mut new_pn) = cloned.kind {
                                    new_pn.path_data = PathData::from_bez_path(&new_bez);
                                }
                            } else if flip_h {
                                cloned.transform.matrix[0] *= -1.0;
                                cloned.transform.matrix[2] *= -1.0;
                            } else {
                                cloned.transform.matrix[1] *= -1.0;
                                cloned.transform.matrix[3] *= -1.0;
                            }
                            commands.push(Command::AddNode {
                                layer_id: Some(layer_id),
                                node: cloned,
                            });
                        }
                    }
                    if !commands.is_empty() {
                        let batch = if commands.len() == 1 {
                            commands.remove(0)
                        } else {
                            Command::Batch(commands)
                        };
                        history.execute(batch, doc);
                        doc_modified = true;
                    }
                }

                PanelAction::RotateCopies { node_id, count } => {
                    use photonic_core::transform::Transform;
                    if count >= 2 {
                        if let Some(src) = doc.nodes.get(&node_id).cloned() {
                            let layer_id = src.layer_id;
                            let (cx, cy) = if let Some(lb) = src.local_bounds() {
                                let (x0, y0) = src.transform.apply(lb.x0, lb.y0);
                                let (x1, y1) = src.transform.apply(lb.x1, lb.y1);
                                ((x0 + x1) / 2.0, (y0 + y1) / 2.0)
                            } else {
                                src.transform.apply(0.0, 0.0)
                            };
                            let angle_step = std::f64::consts::TAU / count as f64;
                            let orig_tx = src.transform.matrix[4];
                            let orig_ty = src.transform.matrix[5];
                            let mut cmds: Vec<Command> = Vec::new();
                            for i in 1..count {
                                let angle = angle_step * i as f64;
                                let rot = Transform::rotate_around(angle, cx, cy);
                                let mut copy = src.clone();
                                copy.id = uuid::Uuid::new_v4();
                                copy.name = format!("{} copy {}", src.name, i);
                                copy.transform = src.transform.then(&rot);
                                let (rot_tx, rot_ty) = rot.apply(orig_tx, orig_ty);
                                copy.transform.matrix[4] = rot_tx;
                                copy.transform.matrix[5] = rot_ty;
                                cmds.push(Command::AddNode {
                                    node: copy,
                                    layer_id: Some(layer_id),
                                });
                            }
                            if !cmds.is_empty() {
                                history.execute(Command::Batch(cmds), doc);
                                doc_modified = true;
                            }
                        }
                    }
                }

                PanelAction::CopyAppearance {
                    source_id,
                    target_ids,
                    copy_fill,
                    copy_stroke,
                    copy_opacity,
                } => {
                    if let Some(src) = doc.nodes.get(&source_id).cloned() {
                        let src_fill = if let SceneNodeKind::Path(ref p) = src.kind {
                            Some(p.fill.clone())
                        } else {
                            None
                        };
                        let src_stroke = if let SceneNodeKind::Path(ref p) = src.kind {
                            Some(p.stroke.clone())
                        } else {
                            None
                        };
                        let src_opacity = src.opacity;
                        let mut cmds: Vec<Command> = Vec::new();
                        for tid in target_ids {
                            if let Some(tgt) = doc.nodes.get(&tid).cloned() {
                                let mut new_node = tgt.clone();
                                if copy_opacity {
                                    new_node.opacity = src_opacity;
                                }
                                if let SceneNodeKind::Path(ref mut p) = new_node.kind {
                                    if copy_fill {
                                        if let Some(ref f) = src_fill {
                                            p.fill = f.clone();
                                        }
                                    }
                                    if copy_stroke {
                                        if let Some(ref s) = src_stroke {
                                            p.stroke = s.clone();
                                        }
                                    }
                                }
                                cmds.push(Command::UpdateNode {
                                    old: tgt,
                                    new: new_node,
                                });
                            }
                        }
                        if !cmds.is_empty() {
                            let batch = if cmds.len() == 1 {
                                cmds.remove(0)
                            } else {
                                Command::Batch(cmds)
                            };
                            history.execute(batch, doc);
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::RemoveExportProfile { name } => {
                    doc.export_profiles.retain(|p| p.name != name);
                    doc_modified = true;
                }

                PanelAction::PinObjectGuides { node_ids } => {
                    let tolerance = 0.5_f64;
                    let mut new_guides: Vec<photonic_core::Guide> = Vec::new();

                    let add_h =
                        |pos: f64,
                         new_guides: &mut Vec<photonic_core::Guide>,
                         doc_guides: &[photonic_core::Guide]| {
                            let exists = doc_guides.iter().chain(new_guides.iter()).any(|g| {
                                g.orientation == photonic_core::GuideOrientation::Horizontal
                                    && (g.position - pos).abs() < tolerance
                            });
                            if !exists {
                                new_guides.push(photonic_core::Guide::new(
                                    photonic_core::GuideOrientation::Horizontal,
                                    pos,
                                ));
                            }
                        };
                    let add_v =
                        |pos: f64,
                         new_guides: &mut Vec<photonic_core::Guide>,
                         doc_guides: &[photonic_core::Guide]| {
                            let exists = doc_guides.iter().chain(new_guides.iter()).any(|g| {
                                g.orientation == photonic_core::GuideOrientation::Vertical
                                    && (g.position - pos).abs() < tolerance
                            });
                            if !exists {
                                new_guides.push(photonic_core::Guide::new(
                                    photonic_core::GuideOrientation::Vertical,
                                    pos,
                                ));
                            }
                        };

                    for nid in &node_ids {
                        if let Some(node) = doc.nodes.get(nid) {
                            let tx = node.transform.matrix[4];
                            let ty = node.transform.matrix[5];
                            if let SceneNodeKind::Path(pn) = &node.kind {
                                use kurbo::Shape;
                                let bez = pn.path_data.to_bez_path();
                                let bb = bez.bounding_box();
                                let (x0, y0, x1, y1) =
                                    (bb.x0 + tx, bb.y0 + ty, bb.x1 + tx, bb.y1 + ty);
                                add_h(y0, &mut new_guides, &doc.guides);
                                add_h(y1, &mut new_guides, &doc.guides);
                                add_h((y0 + y1) / 2.0, &mut new_guides, &doc.guides);
                                add_v(x0, &mut new_guides, &doc.guides);
                                add_v(x1, &mut new_guides, &doc.guides);
                                add_v((x0 + x1) / 2.0, &mut new_guides, &doc.guides);
                            }
                        }
                    }
                    if !new_guides.is_empty() {
                        doc.guides.extend(new_guides);
                        doc_modified = true;
                    }
                }

                PanelAction::ReverseNodeOrder { node_ids } => {
                    let mut commands = Vec::new();
                    for nid in &node_ids {
                        if let Some(node) = doc.nodes.get(nid).cloned() {
                            if let SceneNodeKind::Group(ref g) = node.kind {
                                if g.children.len() > 1 {
                                    let mut new_node = node.clone();
                                    if let SceneNodeKind::Group(ref mut ng) = new_node.kind {
                                        ng.children.reverse();
                                    }
                                    commands.push(Command::UpdateNode {
                                        old: node,
                                        new: new_node,
                                    });
                                }
                            }
                        }
                    }
                    if !commands.is_empty() {
                        let batch = if commands.len() == 1 {
                            commands.remove(0)
                        } else {
                            Command::Batch(commands)
                        };
                        history.execute(batch, doc);
                        doc_modified = true;
                    }
                }

                PanelAction::ApplyParagraphStyle {
                    node_id,
                    style_name,
                } => {
                    use photonic_core::node::TextAlign;
                    let style = doc
                        .paragraph_styles
                        .iter()
                        .find(|s| s.name == style_name)
                        .cloned();
                    if let (Some(style), Some(node)) = (style, doc.nodes.get(&node_id).cloned()) {
                        if let SceneNodeKind::Text(_) = &node.kind {
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Text(ref mut t) = new_node.kind {
                                if let Some(align_str) = &style.align {
                                    t.align = match align_str.as_str() {
                                        "center" => TextAlign::Center,
                                        "right" => TextAlign::Right,
                                        _ => TextAlign::Left,
                                    };
                                }
                                if let Some(lh) = style.line_height {
                                    t.line_height = lh;
                                }
                                if let Some(ls) = style.letter_spacing {
                                    t.letter_spacing = ls;
                                }
                                if let Some(fs) = style.font_size {
                                    t.font_size = fs;
                                }
                                if let Some(ff) = &style.font_family {
                                    t.font_family = ff.clone();
                                }
                            }
                            history.execute(
                                Command::UpdateNode {
                                    old: node,
                                    new: new_node,
                                },
                                doc,
                            );
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::DeleteParagraphStyle { name } => {
                    doc.paragraph_styles.retain(|s| s.name != name);
                    doc_modified = true;
                }

                PanelAction::ApplyCharacterStyle {
                    node_id,
                    style_name,
                } => {
                    use photonic_core::color::Color;
                    use photonic_core::style::Fill;
                    let style = doc
                        .character_styles
                        .iter()
                        .find(|s| s.name == style_name)
                        .cloned();
                    if let (Some(style), Some(node)) = (style, doc.nodes.get(&node_id).cloned()) {
                        if let SceneNodeKind::Text(_) = &node.kind {
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Text(ref mut t) = new_node.kind {
                                if let Some(ff) = &style.font_family {
                                    t.font_family = ff.clone();
                                }
                                if let Some(fs) = style.font_size {
                                    t.font_size = fs;
                                }
                                if let Some(fw) = style.font_weight {
                                    t.font_weight = fw;
                                }
                                if let Some(ls) = style.letter_spacing {
                                    t.letter_spacing = ls;
                                }
                                if let Some(lh) = style.line_height {
                                    t.line_height = lh;
                                }
                                if let Some(hex) = &style.fill_hex {
                                    if let Some(color) = Color::from_hex(hex) {
                                        t.fill = Fill::solid(color);
                                    }
                                }
                            }
                            history.execute(
                                Command::UpdateNode {
                                    old: node,
                                    new: new_node,
                                },
                                doc,
                            );
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::DeleteCharacterStyle { name } => {
                    doc.character_styles.retain(|s| s.name != name);
                    doc_modified = true;
                }

                PanelAction::TagNodeForExport {
                    node_id,
                    name,
                    format,
                } => {
                    use photonic_core::AssetExportSpec;
                    if let Some(node) = doc.nodes.get(&node_id).cloned() {
                        let mut new_node = node.clone();
                        new_node.export_spec = Some(AssetExportSpec {
                            name: name.clone(),
                            format: format.clone(),
                            scales: vec![1.0],
                        });
                        history.execute(
                            Command::UpdateNode {
                                old: node,
                                new: new_node,
                            },
                            doc,
                        );
                        doc_modified = true;
                    }
                }

                PanelAction::RemoveExportTag { node_id } => {
                    if let Some(node) = doc.nodes.get(&node_id).cloned() {
                        let mut new_node = node.clone();
                        new_node.export_spec = None;
                        history.execute(
                            Command::UpdateNode {
                                old: node,
                                new: new_node,
                            },
                            doc,
                        );
                        doc_modified = true;
                    }
                }

                PanelAction::SelectSimilar { node_ids, match_by } => {
                    use photonic_core::style::FillKind;
                    let tol_f = 5.0_f32 / 255.0_f32;
                    let criteria: Vec<&str> = match_by.split(',').map(|s| s.trim()).collect();

                    // Collect reference attributes.
                    let mut ref_fills: Vec<[f32; 3]> = Vec::new();
                    for nid in &node_ids {
                        if let Some(node) = doc.nodes.get(nid) {
                            if let SceneNodeKind::Path(p) = &node.kind {
                                if p.fill.enabled {
                                    if let FillKind::Solid(c) = &p.fill.kind {
                                        ref_fills.push([c.r, c.g, c.b]);
                                    }
                                }
                            }
                        }
                    }

                    let color_matches = |a: [f32; 3]| -> bool {
                        ref_fills.iter().any(|rc| {
                            (a[0] - rc[0]).abs() <= tol_f
                                && (a[1] - rc[1]).abs() <= tol_f
                                && (a[2] - rc[2]).abs() <= tol_f
                        })
                    };

                    let matched: Vec<NodeId> = doc
                        .nodes
                        .iter()
                        .filter(|(id, node)| {
                            if node_ids.contains(id) {
                                return false;
                            }
                            for crit in &criteria {
                                let ok = match *crit {
                                    "fill_color" => match &node.kind {
                                        SceneNodeKind::Path(p) => {
                                            if p.fill.enabled {
                                                if let FillKind::Solid(c) = &p.fill.kind {
                                                    color_matches([c.r, c.g, c.b])
                                                } else {
                                                    false
                                                }
                                            } else {
                                                false
                                            }
                                        }
                                        _ => false,
                                    },
                                    "kind" => {
                                        let ref_kind = node_ids
                                            .first()
                                            .and_then(|rid| doc.nodes.get(rid))
                                            .map(|rn| match &rn.kind {
                                                SceneNodeKind::Path(_) => "path",
                                                SceneNodeKind::Text(_) => "text",
                                                SceneNodeKind::Group(_) => "group",
                                            })
                                            .unwrap_or("");
                                        let this_kind = match &node.kind {
                                            SceneNodeKind::Path(_) => "path",
                                            SceneNodeKind::Text(_) => "text",
                                            SceneNodeKind::Group(_) => "group",
                                        };
                                        this_kind == ref_kind
                                    }
                                    _ => true,
                                };
                                if !ok {
                                    return false;
                                }
                            }
                            true
                        })
                        .map(|(id, _)| *id)
                        .collect();

                    doc.selection.node_ids.clear();
                    for nid in node_ids.iter().chain(matched.iter()) {
                        doc.selection.node_ids.insert(*nid);
                    }
                    doc_modified = true;
                }

                PanelAction::CopyDocumentTemplate => {
                    // Build a node-stripped template and copy the JSON to the OS clipboard.
                    let mut template = doc.clone();
                    template.nodes.clear();
                    template.selection = Default::default();
                    for layer in template.layers.values_mut() {
                        layer.node_ids.clear();
                    }
                    if let Ok(json_str) = template.to_json() {
                        ctx.copy_text(json_str);
                    }
                }

                PanelAction::ApplyColorSwatch {
                    node_id,
                    swatch_name,
                } => {
                    if let Some(swatch) = doc.color_swatches.iter().find(|s| s.name == swatch_name)
                    {
                        if let Some(color) =
                            photonic_core::Color::from_hex(&swatch.color_hex.clone())
                        {
                            if let Some(node) = doc.nodes.get(&node_id) {
                                let mut new_node = node.clone();
                                if let SceneNodeKind::Path(ref mut pn) = new_node.kind {
                                    pn.fill = Fill::solid(color);
                                }
                                history.execute(
                                    Command::UpdateNode {
                                        old: node.clone(),
                                        new: new_node,
                                    },
                                    doc,
                                );
                                doc_modified = true;
                            }
                        }
                    }
                }

                PanelAction::DeleteColorSwatch { name } => {
                    doc.color_swatches.retain(|s| s.name != name);
                    doc_modified = true;
                }

                PanelAction::LoadSwatchLibrary {
                    library,
                    clear_existing,
                } => {
                    use photonic_core::ColorSwatch;
                    let palette: &[(&str, &str)] = match library.as_str() {
                        "web" => &[
                            ("White", "#ffffff"),
                            ("Silver", "#c0c0c0"),
                            ("Gray", "#808080"),
                            ("Black", "#000000"),
                            ("Red", "#ff0000"),
                            ("Maroon", "#800000"),
                            ("Yellow", "#ffff00"),
                            ("Olive", "#808000"),
                            ("Lime", "#00ff00"),
                            ("Green", "#008000"),
                            ("Aqua", "#00ffff"),
                            ("Teal", "#008080"),
                            ("Blue", "#0000ff"),
                            ("Navy", "#000080"),
                            ("Fuchsia", "#ff00ff"),
                            ("Purple", "#800080"),
                        ],
                        "material" => &[
                            ("Red 500", "#f44336"),
                            ("Pink 500", "#e91e63"),
                            ("Purple 500", "#9c27b0"),
                            ("Deep Purple 500", "#673ab7"),
                            ("Indigo 500", "#3f51b5"),
                            ("Blue 500", "#2196f3"),
                            ("Cyan 500", "#00bcd4"),
                            ("Teal 500", "#009688"),
                            ("Green 500", "#4caf50"),
                            ("Yellow 500", "#ffeb3b"),
                            ("Orange 500", "#ff9800"),
                            ("Deep Orange 500", "#ff5722"),
                            ("Brown 500", "#795548"),
                            ("Grey 500", "#9e9e9e"),
                            ("Blue Grey 500", "#607d8b"),
                            ("White", "#ffffff"),
                        ],
                        "pastels" => &[
                            ("Pastel Pink", "#ffb3ba"),
                            ("Pastel Peach", "#ffdfba"),
                            ("Pastel Yellow", "#ffffba"),
                            ("Pastel Green", "#baffc9"),
                            ("Pastel Blue", "#bae1ff"),
                            ("Pastel Lavender", "#d4baff"),
                            ("Pastel Mint", "#b5ead7"),
                            ("Pastel Lilac", "#c7ceea"),
                            ("Pastel Coral", "#ffd7be"),
                            ("Pastel Sky", "#aec6cf"),
                            ("Pastel Lemon", "#fffacd"),
                            ("Pastel Rose", "#f2c6c2"),
                        ],
                        "earth_tones" => &[
                            ("Terracotta", "#c65d3c"),
                            ("Rust", "#b7410e"),
                            ("Burnt Sienna", "#e97451"),
                            ("Sandy Brown", "#daa06d"),
                            ("Khaki", "#c3a882"),
                            ("Tan", "#d2b48c"),
                            ("Warm Taupe", "#b09080"),
                            ("Driftwood", "#9a7b4f"),
                            ("Saddle Brown", "#8b4513"),
                            ("Dark Chocolate", "#5c3317"),
                            ("Forest Floor", "#4a3728"),
                            ("Moss", "#8a9a5b"),
                        ],
                        "neon" => &[
                            ("Neon Pink", "#ff006e"),
                            ("Neon Orange", "#fb5607"),
                            ("Neon Yellow", "#ffbe0b"),
                            ("Neon Green", "#8338ec"),
                            ("Neon Cyan", "#00f5d4"),
                            ("Neon Blue", "#3a86ff"),
                            ("Electric Lime", "#ccff00"),
                            ("Hot Magenta", "#ff00ff"),
                            ("Laser Lemon", "#ffff66"),
                            ("Neon Red", "#ff073a"),
                            ("Electric Blue", "#00b0ff"),
                            ("UV Purple", "#9400d3"),
                        ],
                        "grayscale" => &[
                            ("White", "#ffffff"),
                            ("Gray 10", "#e6e6e6"),
                            ("Gray 20", "#cccccc"),
                            ("Gray 30", "#b3b3b3"),
                            ("Gray 40", "#999999"),
                            ("Gray 50", "#808080"),
                            ("Gray 60", "#666666"),
                            ("Gray 70", "#4d4d4d"),
                            ("Gray 80", "#333333"),
                            ("Gray 90", "#1a1a1a"),
                            ("Black", "#000000"),
                        ],
                        _ => &[],
                    };
                    if clear_existing {
                        doc.color_swatches.clear();
                    }
                    for (name, hex) in palette {
                        if !doc.color_swatches.iter().any(|s| s.name == *name) {
                            doc.color_swatches.push(ColorSwatch::new(*name, *hex));
                        }
                    }
                    doc_modified = true;
                }

                PanelAction::SaveWidthProfile { stroke_width, name } => {
                    use photonic_core::WidthProfile;
                    // Uniform 2-point profile — same width at both ends
                    let widths = vec![stroke_width, stroke_width];
                    let profile = WidthProfile::new(&name, widths);
                    if let Some(existing) = doc.width_profiles.iter_mut().find(|p| p.name == name) {
                        *existing = profile;
                    } else {
                        doc.width_profiles.push(profile);
                    }
                    self.width_profile_name_input.clear();
                    doc_modified = true;
                }

                PanelAction::ApplyWidthProfile {
                    node_id,
                    profile_name,
                } => {
                    let avg = doc
                        .width_profiles
                        .iter()
                        .find(|p| p.name == profile_name)
                        .map(|p| p.average_width());
                    if let Some(avg_width) = avg {
                        if let Some(node) = doc.nodes.get(&node_id).cloned() {
                            if let SceneNodeKind::Path(_) = &node.kind {
                                let mut new_node = node.clone();
                                if let SceneNodeKind::Path(ref mut pn) = new_node.kind {
                                    pn.stroke.width = avg_width;
                                }
                                history.execute(
                                    Command::UpdateNode {
                                        old: node,
                                        new: new_node,
                                    },
                                    doc,
                                );
                            }
                        }
                    }
                }

                PanelAction::DeleteWidthProfile { name } => {
                    doc.width_profiles.retain(|p| p.name != name);
                    doc_modified = true;
                }

                PanelAction::SaveGraphicStyle { node_id, name } => {
                    use photonic_core::GraphicStyle;
                    if let Some(node) = doc.nodes.get(&node_id) {
                        let (fill, stroke) = match &node.kind {
                            SceneNodeKind::Path(pn) => (pn.fill.clone(), pn.stroke.clone()),
                            SceneNodeKind::Text(tn) => {
                                use photonic_core::style::Stroke;
                                (tn.fill.clone(), Stroke::none())
                            }
                            SceneNodeKind::Group(_) => {
                                use photonic_core::style::{Fill, Stroke};
                                (Fill::default(), Stroke::none())
                            }
                        };
                        let fill_json = serde_json::to_string(&fill).unwrap_or_default();
                        let stroke_json = serde_json::to_string(&stroke).unwrap_or_default();
                        let style = GraphicStyle::new(&name, fill_json, stroke_json, node.opacity);
                        if let Some(existing) =
                            doc.graphic_styles.iter_mut().find(|s| s.name == name)
                        {
                            *existing = style;
                        } else {
                            doc.graphic_styles.push(style);
                        }
                        self.graphic_style_name_input.clear();
                        doc_modified = true;
                    }
                }

                PanelAction::ApplyGraphicStyle {
                    node_id,
                    style_name,
                } => {
                    use photonic_core::style::{Fill, Stroke};
                    let style_data = doc
                        .graphic_styles
                        .iter()
                        .find(|s| s.name == style_name)
                        .cloned();
                    if let Some(style) = style_data {
                        let fill: Fill = serde_json::from_str(&style.fill_json).unwrap_or_default();
                        let stroke: Stroke =
                            serde_json::from_str(&style.stroke_json).unwrap_or_default();
                        if let Some(node) = doc.nodes.get(&node_id).cloned() {
                            let mut new_node = node.clone();
                            new_node.opacity = style.opacity;
                            match &mut new_node.kind {
                                SceneNodeKind::Path(pn) => {
                                    pn.fill = fill;
                                    pn.stroke = stroke;
                                }
                                SceneNodeKind::Text(tn) => {
                                    tn.fill = fill;
                                }
                                SceneNodeKind::Group(_) => {}
                            }
                            history.execute(
                                Command::UpdateNode {
                                    old: node,
                                    new: new_node,
                                },
                                doc,
                            );
                        }
                    }
                }

                PanelAction::DeleteGraphicStyle { name } => {
                    doc.graphic_styles.retain(|s| s.name != name);
                    doc_modified = true;
                }

                PanelAction::FlattenTransparency => {
                    use photonic_core::style::{Fill, FillKind};
                    let ids: Vec<NodeId> = doc.selection.ids().copied().collect();
                    let target: Vec<NodeId> = if ids.is_empty() {
                        doc.nodes.keys().cloned().collect()
                    } else {
                        ids
                    };

                    fn bake_fill(fill: &Fill, combined: f32) -> Fill {
                        let kind = match &fill.kind {
                            FillKind::Solid(c) => FillKind::Solid(photonic_core::color::Color {
                                r: c.r,
                                g: c.g,
                                b: c.b,
                                a: c.a * combined,
                            }),
                            FillKind::Gradient(g) => {
                                let mut g2 = g.clone();
                                for stop in g2.stops.iter_mut() {
                                    stop.color.a *= combined;
                                }
                                FillKind::Gradient(g2)
                            }
                            other => other.clone(),
                        };
                        Fill {
                            kind,
                            opacity: 1.0,
                            enabled: fill.enabled,
                        }
                    }

                    let mut cmds: Vec<Command> = Vec::new();
                    for nid in target {
                        if let Some(node) = doc.nodes.get(&nid) {
                            let node_opacity = node.opacity as f32;
                            if node_opacity >= 1.0 - f32::EPSILON
                                && match &node.kind {
                                    SceneNodeKind::Path(pn) => pn.fill.opacity >= 1.0 - 1e-6,
                                    SceneNodeKind::Text(tn) => tn.fill.opacity >= 1.0 - 1e-6,
                                    _ => true,
                                }
                            {
                                continue;
                            }
                            let mut new_node = node.clone();
                            new_node.opacity = 1.0;
                            match &mut new_node.kind {
                                SceneNodeKind::Path(pn) => {
                                    let combined = (pn.fill.opacity as f32) * node_opacity;
                                    pn.fill = bake_fill(&pn.fill, combined);
                                    pn.stroke.color.a *= node_opacity;
                                    pn.stroke.opacity = 1.0;
                                }
                                SceneNodeKind::Text(tn) => {
                                    let combined = (tn.fill.opacity as f32) * node_opacity;
                                    tn.fill = bake_fill(&tn.fill, combined);
                                }
                                SceneNodeKind::Group(_) => {}
                            }
                            cmds.push(Command::UpdateNode {
                                old: node.clone(),
                                new: new_node,
                            });
                        }
                    }
                    if !cmds.is_empty() {
                        history.execute(Command::Batch(cmds), doc);
                        doc_modified = true;
                    }
                }

                PanelAction::UndoNode { node_id, steps } => {
                    history.revert_node_steps(node_id, steps, doc);
                    doc_modified = true;
                }

                PanelAction::RefreshHistory => {
                    self.history_entries = history.history_entries(20);
                }

                PanelAction::SetDocumentBleed { bleed_mm, slug_mm } => {
                    doc.bleed_mm = bleed_mm;
                    doc.slug_mm = slug_mm;
                    doc_modified = true;
                }

                PanelAction::SetArtboardMargins {
                    top,
                    right,
                    bottom,
                    left,
                } => {
                    doc.margin_top = top;
                    doc.margin_right = right;
                    doc.margin_bottom = bottom;
                    doc.margin_left = left;
                    doc_modified = true;
                }

                PanelAction::RegisterEventTrigger { event, action_name } => {
                    let already = doc
                        .event_triggers
                        .iter()
                        .any(|t| t.event == event && t.action_name == action_name);
                    let action_exists = doc.action_sets.iter().any(|a| a.name == action_name);
                    if !already && action_exists {
                        doc.event_triggers
                            .push(photonic_core::EventTrigger { event, action_name });
                        doc_modified = true;
                    }
                }

                PanelAction::RemoveEventTrigger { event, action_name } => {
                    if let Some(ref aname) = action_name {
                        doc.event_triggers
                            .retain(|t| !(t.event == event && t.action_name == *aname));
                    } else {
                        doc.event_triggers.retain(|t| t.event != event);
                    }
                    doc_modified = true;
                }

                PanelAction::AddConstructionLine {
                    x,
                    y,
                    angle_degrees,
                } => {
                    use photonic_core::document::{Guide, GuideOrientation};
                    let mut guide = Guide::new(GuideOrientation::Horizontal, 0.0);
                    guide.color = Some([1.0, 0.5, 0.0, 0.85]); // orange
                    guide.angle_degrees = Some(angle_degrees);
                    guide.position_x = x;
                    guide.position_y = y;
                    doc.guides.push(guide);
                    doc_modified = true;
                }

                PanelAction::ApplyGridLayout {
                    group_id,
                    columns,
                    gap_x,
                    gap_y,
                } => {
                    if let Some(group_node) = doc.nodes.get(&group_id) {
                        let child_ids = match &group_node.kind {
                            SceneNodeKind::Group(g) => g.children.clone(),
                            _ => vec![],
                        };
                        if child_ids.len() > 1 {
                            struct CB {
                                id: NodeId,
                                w: f64,
                                h: f64,
                            }
                            let mut children: Vec<CB> = Vec::new();
                            for cid in &child_ids {
                                if let Some(child) = doc.nodes.get(cid) {
                                    let (w, h) = match &child.kind {
                                        SceneNodeKind::Path(pn) => {
                                            if let Some(bb) = pn.path_data.bounding_box() {
                                                (
                                                    bb.width().abs().max(1.0),
                                                    bb.height().abs().max(1.0),
                                                )
                                            } else {
                                                (60.0, 30.0)
                                            }
                                        }
                                        _ => (60.0, 30.0),
                                    };
                                    children.push(CB { id: *cid, w, h });
                                }
                            }
                            let col_width = children.iter().map(|c| c.w).fold(0.0_f64, f64::max);
                            let row_height = children.iter().map(|c| c.h).fold(0.0_f64, f64::max);
                            let mut cmds: Vec<Command> = Vec::new();
                            for (i, child) in children.iter().enumerate() {
                                let col = i % columns;
                                let row = i / columns;
                                let new_tx = col as f64 * (col_width + gap_x);
                                let new_ty = row as f64 * (row_height + gap_y);
                                if let Some(old) = doc.nodes.get(&child.id) {
                                    let mut new_node = old.clone();
                                    new_node.transform.matrix[4] = new_tx;
                                    new_node.transform.matrix[5] = new_ty;
                                    cmds.push(Command::UpdateNode {
                                        old: old.clone(),
                                        new: new_node,
                                    });
                                }
                            }
                            if !cmds.is_empty() {
                                history.execute(Command::Batch(cmds), doc);
                                doc_modified = true;
                            }
                        }
                    }
                }

                PanelAction::ApplyStackLayout {
                    group_id,
                    align_h,
                    align_v,
                } => {
                    if let Some(group_node) = doc.nodes.get(&group_id) {
                        let child_ids = match &group_node.kind {
                            SceneNodeKind::Group(g) => g.children.clone(),
                            _ => vec![],
                        };
                        if !child_ids.is_empty() {
                            struct CB {
                                id: NodeId,
                                w: f64,
                                h: f64,
                            }
                            let mut children: Vec<CB> = Vec::new();
                            let mut min_x = f64::MAX;
                            let mut min_y = f64::MAX;
                            let mut max_x = f64::MIN;
                            let mut max_y = f64::MIN;
                            for cid in &child_ids {
                                if let Some(child) = doc.nodes.get(cid) {
                                    let (w, h) = match &child.kind {
                                        SceneNodeKind::Path(pn) => {
                                            if let Some(bb) = pn.path_data.bounding_box() {
                                                (
                                                    bb.width().abs().max(1.0),
                                                    bb.height().abs().max(1.0),
                                                )
                                            } else {
                                                (60.0, 30.0)
                                            }
                                        }
                                        _ => (60.0, 30.0),
                                    };
                                    let tx = child.transform.matrix[4];
                                    let ty = child.transform.matrix[5];
                                    min_x = min_x.min(tx);
                                    min_y = min_y.min(ty);
                                    max_x = max_x.max(tx + w);
                                    max_y = max_y.max(ty + h);
                                    children.push(CB { id: *cid, w, h });
                                }
                            }
                            let union_x = min_x;
                            let union_y = min_y;
                            let union_w = (max_x - min_x).max(1.0);
                            let union_h = (max_y - min_y).max(1.0);
                            let mut cmds: Vec<Command> = Vec::new();
                            for child in &children {
                                let new_tx = match align_h.as_str() {
                                    "left" => union_x,
                                    "right" => union_x + union_w - child.w,
                                    _ => union_x + (union_w - child.w) / 2.0,
                                };
                                let new_ty = match align_v.as_str() {
                                    "top" => union_y,
                                    "bottom" => union_y + union_h - child.h,
                                    _ => union_y + (union_h - child.h) / 2.0,
                                };
                                if let Some(old) = doc.nodes.get(&child.id) {
                                    let mut new_node = old.clone();
                                    new_node.transform.matrix[4] = new_tx;
                                    new_node.transform.matrix[5] = new_ty;
                                    cmds.push(Command::UpdateNode {
                                        old: old.clone(),
                                        new: new_node,
                                    });
                                }
                            }
                            if !cmds.is_empty() {
                                history.execute(Command::Batch(cmds), doc);
                                doc_modified = true;
                            }
                        }
                    }
                }

                PanelAction::ApplyFlexLayout {
                    group_id,
                    direction,
                    gap,
                    align,
                    padding,
                } => {
                    if let Some(group_node) = doc.nodes.get(&group_id) {
                        let child_ids = match &group_node.kind {
                            SceneNodeKind::Group(g) => g.children.clone(),
                            _ => vec![],
                        };
                        if child_ids.len() > 1 {
                            struct ChildBox {
                                id: NodeId,
                                tx: f64,
                                ty: f64,
                                w: f64,
                                h: f64,
                            }
                            let mut children: Vec<ChildBox> = Vec::new();
                            for cid in &child_ids {
                                if let Some(child) = doc.nodes.get(cid) {
                                    let (w, h) = match &child.kind {
                                        SceneNodeKind::Path(pn) => {
                                            if let Some(bb) = pn.path_data.bounding_box() {
                                                (
                                                    bb.width().abs().max(1.0),
                                                    bb.height().abs().max(1.0),
                                                )
                                            } else {
                                                (60.0, 30.0)
                                            }
                                        }
                                        _ => (60.0, 30.0),
                                    };
                                    children.push(ChildBox {
                                        id: *cid,
                                        tx: child.transform.matrix[4],
                                        ty: child.transform.matrix[5],
                                        w,
                                        h,
                                    });
                                }
                            }
                            match direction.as_str() {
                                "column" => children.sort_by(|a, b| {
                                    a.ty.partial_cmp(&b.ty).unwrap_or(std::cmp::Ordering::Equal)
                                }),
                                _ => children.sort_by(|a, b| {
                                    a.tx.partial_cmp(&b.tx).unwrap_or(std::cmp::Ordering::Equal)
                                }),
                            }
                            let cross_max: f64 = match direction.as_str() {
                                "column" => children.iter().map(|c| c.w).fold(0.0_f64, f64::max),
                                _ => children.iter().map(|c| c.h).fold(0.0_f64, f64::max),
                            };
                            let mut cursor = padding;
                            let mut cmds: Vec<Command> = Vec::new();
                            for child in &children {
                                let cross_size = match direction.as_str() {
                                    "column" => child.w,
                                    _ => child.h,
                                };
                                let cross_offset = match align.as_str() {
                                    "start" => padding,
                                    "end" => padding + cross_max - cross_size,
                                    _ => {
                                        padding
                                            + if cross_max > cross_size {
                                                (cross_max - cross_size) / 2.0
                                            } else {
                                                0.0
                                            }
                                    }
                                };
                                let (new_tx, new_ty) = match direction.as_str() {
                                    "column" => (cross_offset, cursor),
                                    _ => (cursor, cross_offset),
                                };
                                let main_size = match direction.as_str() {
                                    "column" => child.h,
                                    _ => child.w,
                                };
                                cursor += main_size + gap;
                                if let Some(old) = doc.nodes.get(&child.id) {
                                    let mut new_node = old.clone();
                                    new_node.transform.matrix[4] = new_tx;
                                    new_node.transform.matrix[5] = new_ty;
                                    cmds.push(Command::UpdateNode {
                                        old: old.clone(),
                                        new: new_node,
                                    });
                                }
                            }
                            if !cmds.is_empty() {
                                history.execute(Command::Batch(cmds), doc);
                                doc_modified = true;
                            }
                        }
                    }
                }

                PanelAction::DefineSpotColor {
                    name,
                    hex,
                    overprint,
                } => {
                    let hex_norm = if hex.starts_with('#') {
                        hex.clone()
                    } else {
                        format!("#{}", hex)
                    };
                    if let Some(existing) = doc.spot_colors.iter_mut().find(|s| s.name == name) {
                        existing.hex = hex_norm;
                        existing.overprint = overprint;
                    } else {
                        use photonic_core::SpotColor;
                        doc.spot_colors
                            .push(SpotColor::new(name, hex_norm, overprint));
                    }
                    doc_modified = true;
                }

                PanelAction::ApplySpotColor {
                    node_id,
                    color_name,
                } => {
                    let hex = doc
                        .spot_colors
                        .iter()
                        .find(|s| s.name == color_name)
                        .map(|s| s.hex.clone());
                    if let Some(hex) = hex {
                        if let Some(color) = photonic_core::Color::from_hex(&hex) {
                            use photonic_core::style::{Fill, FillKind};
                            let fill = Fill {
                                kind: FillKind::Solid(color),
                                opacity: 1.0,
                                enabled: true,
                            };
                            if let Some(node) = doc.nodes.get(&node_id) {
                                let mut new_node = node.clone();
                                match &mut new_node.kind {
                                    SceneNodeKind::Path(pn) => {
                                        pn.fill = fill;
                                    }
                                    SceneNodeKind::Text(tn) => {
                                        tn.fill = fill;
                                    }
                                    SceneNodeKind::Group(_) => {}
                                }
                                history.execute(
                                    Command::UpdateNode {
                                        old: node.clone(),
                                        new: new_node,
                                    },
                                    doc,
                                );
                                doc_modified = true;
                            }
                        }
                    }
                }

                PanelAction::DeleteSpotColor { name } => {
                    doc.spot_colors.retain(|s| s.name != name);
                    doc_modified = true;
                }

                PanelAction::SaveGradientSwatch { node_id, name } => {
                    if let Some(node) = doc.nodes.get(&node_id) {
                        let fill = match &node.kind {
                            SceneNodeKind::Path(pn) => Some(pn.fill.clone()),
                            _ => None,
                        };
                        if let Some(fill) = fill {
                            if let Ok(fill_json) = serde_json::to_string(&fill) {
                                use photonic_core::GradientSwatch;
                                if let Some(existing) =
                                    doc.gradient_swatches.iter_mut().find(|s| s.name == name)
                                {
                                    existing.fill_json = fill_json;
                                } else {
                                    doc.gradient_swatches
                                        .push(GradientSwatch::new(name, fill_json));
                                }
                                doc_modified = true;
                            }
                        }
                    }
                }

                PanelAction::ApplyGradientSwatch {
                    node_id,
                    swatch_name,
                } => {
                    let fill_json = doc
                        .gradient_swatches
                        .iter()
                        .find(|s| s.name == swatch_name)
                        .map(|s| s.fill_json.clone());
                    if let Some(fill_json) = fill_json {
                        if let Ok(fill) = serde_json::from_str::<Fill>(&fill_json) {
                            if let Some(node) = doc.nodes.get(&node_id) {
                                let mut new_node = node.clone();
                                if let SceneNodeKind::Path(ref mut pn) = new_node.kind {
                                    pn.fill = fill;
                                }
                                history.execute(
                                    Command::UpdateNode {
                                        old: node.clone(),
                                        new: new_node,
                                    },
                                    doc,
                                );
                                doc_modified = true;
                            }
                        }
                    }
                }

                PanelAction::DeleteGradientSwatch { name } => {
                    doc.gradient_swatches.retain(|s| s.name != name);
                    doc_modified = true;
                }

                PanelAction::AnalyzeComposition => {
                    // Run composition analysis inline using doc data
                    use photonic_core::node::SceneNodeKind;
                    use photonic_core::style::FillKind;
                    let mut findings: Vec<String> = Vec::new();

                    let canvas_w = doc.width as f64;
                    let canvas_h = doc.height as f64;
                    let mid_x = canvas_w / 2.0;
                    let mid_y = canvas_h / 2.0;
                    let (mut q_tl, mut q_tr, mut q_bl, mut q_br) = (0usize, 0usize, 0usize, 0usize);

                    struct Info {
                        bx: f64,
                        by: f64,
                        bw: f64,
                        bh: f64,
                        r: f32,
                        g: f32,
                        b: f32,
                        solid: bool,
                    }
                    let mut infos: Vec<Info> = Vec::new();

                    for node in doc.nodes_in_draw_order() {
                        if !node.visible {
                            continue;
                        }
                        let (wx, wy) = node.transform.apply(0.0, 0.0);
                        let (bx, by, bw, bh) = if let Some(lb) = node.local_bounds() {
                            let (x0, y0) = node.transform.apply(lb.x0, lb.y0);
                            let (x1, y1) = node.transform.apply(lb.x1, lb.y1);
                            (
                                x0.min(x1),
                                y0.min(y1),
                                (x1 - x0).abs().max(1.0),
                                (y1 - y0).abs().max(1.0),
                            )
                        } else {
                            (wx, wy, 1.0, 1.0)
                        };
                        let cx = bx + bw / 2.0;
                        let cy = by + bh / 2.0;
                        match (cx < mid_x, cy < mid_y) {
                            (true, true) => q_tl += 1,
                            (false, true) => q_tr += 1,
                            (true, false) => q_bl += 1,
                            (false, false) => q_br += 1,
                        }
                        let (r, g, b, solid) = match &node.kind {
                            SceneNodeKind::Path(pn) => match &pn.fill.kind {
                                FillKind::Solid(c) => (c.r, c.g, c.b, true),
                                _ => (0.5, 0.5, 0.5, false),
                            },
                            SceneNodeKind::Text(tn) => match &tn.fill.kind {
                                FillKind::Solid(c) => (c.r, c.g, c.b, true),
                                _ => (0.0, 0.0, 0.0, true),
                            },
                            SceneNodeKind::Group(_) => (0.5, 0.5, 0.5, false),
                        };
                        infos.push(Info {
                            bx,
                            by,
                            bw,
                            bh,
                            r,
                            g,
                            b,
                            solid,
                        });
                    }

                    if infos.is_empty() {
                        self.composition_findings =
                            vec!["No visible nodes to analyze.".to_string()];
                    } else {
                        let left = q_tl + q_bl;
                        let right = q_tr + q_br;
                        let top = q_tl + q_tr;
                        let bottom = q_bl + q_br;
                        let h_imb = if left + right > 0 {
                            ((left as f64 - right as f64).abs() / (left + right) as f64 * 100.0)
                                as u32
                        } else {
                            0
                        };
                        let v_imb = if top + bottom > 0 {
                            ((top as f64 - bottom as f64).abs() / (top + bottom) as f64 * 100.0)
                                as u32
                        } else {
                            0
                        };
                        if h_imb > 40 {
                            let side = if left > right { "left" } else { "right" };
                            findings.push(format!(
                                "⚠ Balance: {}% more objects on the {} ({} left, {} right).",
                                h_imb, side, left, right
                            ));
                        }
                        if v_imb > 40 {
                            let side = if top > bottom { "top" } else { "bottom" };
                            findings.push(format!(
                                "ℹ Balance: {}% more objects near the {} ({} top, {} bottom).",
                                v_imb, side, top, bottom
                            ));
                        }
                        if h_imb <= 20 && v_imb <= 20 {
                            findings.push(
                                "✓ Balance: objects distributed evenly across quadrants."
                                    .to_string(),
                            );
                        }
                        let total_area: f64 = infos.iter().map(|n| n.bw * n.bh).sum();
                        let canvas_area = (canvas_w * canvas_h).max(1.0);
                        let density = (total_area / canvas_area * 100.0).min(200.0);
                        if density < 5.0 {
                            findings.push(format!(
                                "ℹ Density: very sparse ({:.1}% canvas coverage).",
                                density
                            ));
                        } else if density > 120.0 {
                            findings.push(format!(
                                "⚠ Density: may be overcrowded ({:.1}% combined coverage).",
                                density
                            ));
                        }
                        let mut overlap_count = 0usize;
                        'ov: for i in 0..infos.len() {
                            for j in (i + 1)..infos.len() {
                                let a = &infos[i];
                                let b = &infos[j];
                                if a.bx < b.bx + b.bw
                                    && a.bx + a.bw > b.bx
                                    && a.by < b.by + b.bh
                                    && a.by + a.bh > b.by
                                {
                                    overlap_count += 1;
                                    if overlap_count >= 10 {
                                        break 'ov;
                                    }
                                }
                            }
                        }
                        if overlap_count > 0 {
                            findings.push(format!(
                                "ℹ Overlaps: {} overlapping object pair(s) detected.",
                                overlap_count
                            ));
                        }
                        let solid: Vec<_> = infos.iter().filter(|n| n.solid).collect();
                        let unique_colors: std::collections::HashSet<(u8, u8, u8)> = solid
                            .iter()
                            .map(|n| {
                                (
                                    (n.r * 255.0) as u8,
                                    (n.g * 255.0) as u8,
                                    (n.b * 255.0) as u8,
                                )
                            })
                            .collect();
                        if unique_colors.len() > 12 {
                            findings.push(format!("ℹ Colors: {} unique fill colors — consider reducing for visual cohesion.", unique_colors.len()));
                        }
                        let off_canvas = infos
                            .iter()
                            .filter(|n| {
                                n.bx + n.bw < 0.0
                                    || n.by + n.bh < 0.0
                                    || n.bx > canvas_w
                                    || n.by > canvas_h
                            })
                            .count();
                        if off_canvas > 0 {
                            findings.push(format!("⚠ Off-canvas: {} object(s) outside bounds — won't appear in exports.", off_canvas));
                        }
                        if findings
                            .iter()
                            .all(|f| f.starts_with('✓') || f.starts_with('ℹ'))
                        {
                            findings.push(format!(
                                "✓ {} node(s) analyzed. No critical issues.",
                                infos.len()
                            ));
                        }
                        self.composition_findings = findings;
                    }
                }

                PanelAction::DetectRhythms => {
                    use photonic_core::node::SceneNodeKind;
                    let tolerance = 4.0_f64;
                    let min_count = 3usize;

                    struct Metrics {
                        cx: f64,
                        cy: f64,
                        w: f64,
                        rot_deg: f64,
                    }
                    let mut metrics: Vec<Metrics> = Vec::new();
                    for node in doc.nodes_in_draw_order() {
                        if !node.visible {
                            continue;
                        }
                        if matches!(node.kind, SceneNodeKind::Group(_)) {
                            continue;
                        }
                        let (bx, by, bw, bh) = if let Some(lb) = node.local_bounds() {
                            let (x0, y0) = node.transform.apply(lb.x0, lb.y0);
                            let (x1, y1) = node.transform.apply(lb.x1, lb.y1);
                            let nx = x0.min(x1);
                            let ny = y0.min(y1);
                            let nw = (x1 - x0).abs().max(0.001);
                            let nh = (y1 - y0).abs().max(0.001);
                            (nx, ny, nw, nh)
                        } else {
                            let (wx, wy) = node.transform.apply(0.0, 0.0);
                            (wx, wy, 1.0, 1.0)
                        };
                        let rot = {
                            let r = node.transform.matrix[1]
                                .atan2(node.transform.matrix[0])
                                .to_degrees()
                                % 360.0;
                            if r < 0.0 {
                                r + 360.0
                            } else {
                                r
                            }
                        };
                        metrics.push(Metrics {
                            cx: bx + bw / 2.0,
                            cy: by + bh / 2.0,
                            w: bw,
                            rot_deg: rot,
                        });
                    }

                    if metrics.len() < min_count {
                        self.rhythm_findings = vec![format!(
                            "Need ≥{} leaf nodes to detect rhythms ({} found).",
                            min_count,
                            metrics.len()
                        )];
                    } else {
                        let mut findings: Vec<String> = Vec::new();

                        // Horizontal spacing
                        let mut xs: Vec<f64> = metrics.iter().map(|m| m.cx).collect();
                        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
                        let gaps: Vec<f64> = xs.windows(2).map(|w| w[1] - w[0]).collect();
                        if let Some(best) = gaps.iter().filter(|&&g| g >= 1.0).max_by_key(|&&g| {
                            gaps.iter().filter(|&&x| (x - g).abs() < tolerance).count()
                        }) {
                            let cnt = gaps
                                .iter()
                                .filter(|&&g| (g - best).abs() < tolerance)
                                .count();
                            if cnt + 1 >= min_count {
                                findings.push(format!(
                                    "↔ {} objects spaced ~{:.0}px horizontally",
                                    cnt + 1,
                                    best
                                ));
                            }
                        }

                        // Vertical spacing
                        let mut ys: Vec<f64> = metrics.iter().map(|m| m.cy).collect();
                        ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
                        let gaps_v: Vec<f64> = ys.windows(2).map(|w| w[1] - w[0]).collect();
                        if let Some(best) = gaps_v.iter().filter(|&&g| g >= 1.0).max_by_key(|&&g| {
                            gaps_v
                                .iter()
                                .filter(|&&x| (x - g).abs() < tolerance)
                                .count()
                        }) {
                            let cnt = gaps_v
                                .iter()
                                .filter(|&&g| (g - best).abs() < tolerance)
                                .count();
                            if cnt + 1 >= min_count {
                                findings.push(format!(
                                    "↕ {} objects spaced ~{:.0}px vertically",
                                    cnt + 1,
                                    best
                                ));
                            }
                        }

                        // Uniform width
                        let mut widths: Vec<f64> = metrics.iter().map(|m| m.w).collect();
                        widths.sort_by(|a, b| a.partial_cmp(b).unwrap());
                        if let Some(best) = widths.iter().filter(|&&w| w >= 1.0).max_by_key(|&&w| {
                            widths
                                .iter()
                                .filter(|&&x| (x - w).abs() < tolerance)
                                .count()
                        }) {
                            let cnt = widths
                                .iter()
                                .filter(|&&w| (w - best).abs() < tolerance)
                                .count();
                            if cnt >= min_count {
                                findings
                                    .push(format!("⇔ {} objects share width ~{:.0}px", cnt, best));
                            }
                        }

                        // Rotation rhythm
                        let mut rots: Vec<f64> = metrics.iter().map(|m| m.rot_deg).collect();
                        rots.sort_by(|a, b| a.partial_cmp(b).unwrap());
                        let rot_gaps: Vec<f64> = rots.windows(2).map(|w| w[1] - w[0]).collect();
                        if let Some(best) =
                            rot_gaps.iter().filter(|&&g| g >= 1.0).max_by_key(|&&g| {
                                rot_gaps.iter().filter(|&&x| (x - g).abs() < 3.0).count()
                            })
                        {
                            let cnt = rot_gaps.iter().filter(|&&g| (g - best).abs() < 3.0).count();
                            if cnt + 1 >= min_count && *best >= 5.0 {
                                let n = (360.0 / best).round() as u32;
                                let sym = if n >= 2 && n <= 12 {
                                    format!(" ({}× symmetry)", n)
                                } else {
                                    String::new()
                                };
                                findings.push(format!(
                                    "↻ {} objects rotated ~{:.0}°/step{}",
                                    cnt + 1,
                                    best,
                                    sym
                                ));
                            }
                        }

                        if findings.is_empty() {
                            findings
                                .push(format!("No rhythms detected in {} nodes.", metrics.len()));
                        }
                        self.rhythm_findings = findings;
                    }
                }

                PanelAction::PlayAction { name } => {
                    // GUI can't call async MCP handlers; refresh the actions list
                    // Actual playback is available via the MCP play_action tool
                    self.action_names = doc
                        .action_sets
                        .iter()
                        .map(|a| {
                            let cnt = serde_json::from_str::<serde_json::Value>(&a.steps_json)
                                .ok()
                                .and_then(|v| v.as_array().map(|arr| arr.len()))
                                .unwrap_or(0);
                            (a.name.clone(), cnt)
                        })
                        .collect();
                    let _ = name; // Playback requires MCP tool: play_action { "name": "..." }
                }

                PanelAction::DeleteAction { name } => {
                    doc.action_sets.retain(|a| a.name != name);
                    self.action_names = doc
                        .action_sets
                        .iter()
                        .map(|a| {
                            let cnt = serde_json::from_str::<serde_json::Value>(&a.steps_json)
                                .ok()
                                .and_then(|v| v.as_array().map(|arr| arr.len()))
                                .unwrap_or(0);
                            (a.name.clone(), cnt)
                        })
                        .collect();
                    doc_modified = true;
                }

                PanelAction::MeasureDistances { node_ids } => {
                    struct NBox {
                        name: String,
                        x0: f64,
                        y0: f64,
                        x1: f64,
                        y1: f64,
                    }
                    let mut boxes: Vec<NBox> = Vec::new();
                    for &id in &node_ids {
                        if let Some(node) = doc.nodes.get(&id) {
                            let (bx, by, bw, bh) = if let Some(lb) = node.local_bounds() {
                                let (x0, y0) = node.transform.apply(lb.x0, lb.y0);
                                let (x1, y1) = node.transform.apply(lb.x1, lb.y1);
                                let nx = x0.min(x1);
                                let ny = y0.min(y1);
                                let nw = (x1 - x0).abs();
                                let nh = (y1 - y0).abs();
                                (nx, ny, nw, nh)
                            } else {
                                let (wx, wy) = node.transform.apply(0.0, 0.0);
                                (wx, wy, 0.0, 0.0)
                            };
                            boxes.push(NBox {
                                name: if node.name.is_empty() {
                                    id.to_string()
                                } else {
                                    node.name.clone()
                                },
                                x0: bx,
                                y0: by,
                                x1: bx + bw,
                                y1: by + bh,
                            });
                        }
                    }
                    let n = boxes.len();
                    let mut results: Vec<(String, String, f64, f64, f64)> = Vec::new();
                    let pairs: Vec<(usize, usize)> = if n <= 6 {
                        let mut p = Vec::new();
                        for i in 0..n {
                            for j in (i + 1)..n {
                                p.push((i, j));
                            }
                        }
                        p
                    } else {
                        (0..n - 1).map(|i| (i, i + 1)).collect()
                    };
                    for (i, j) in pairs {
                        let a = &boxes[i];
                        let b = &boxes[j];
                        let acx = (a.x0 + a.x1) / 2.0;
                        let acy = (a.y0 + a.y1) / 2.0;
                        let bcx = (b.x0 + b.x1) / 2.0;
                        let bcy = (b.y0 + b.y1) / 2.0;
                        let center_dist = ((bcx - acx).powi(2) + (bcy - acy).powi(2)).sqrt();
                        let h_gap = if a.x1 <= b.x0 {
                            b.x0 - a.x1
                        } else if b.x1 <= a.x0 {
                            b.x1 - a.x0
                        } else {
                            -(a.x1.min(b.x1) - a.x0.max(b.x0))
                        };
                        let v_gap = if a.y1 <= b.y0 {
                            b.y0 - a.y1
                        } else if b.y1 <= a.y0 {
                            b.y1 - a.y0
                        } else {
                            -(a.y1.min(b.y1) - a.y0.max(b.y0))
                        };
                        results.push((
                            a.name.clone(),
                            b.name.clone(),
                            (h_gap * 10.0).round() / 10.0,
                            (v_gap * 10.0).round() / 10.0,
                            (center_dist * 10.0).round() / 10.0,
                        ));
                    }
                    self.distance_results = results;
                }

                PanelAction::DefineGrammarRule {
                    name,
                    rule_type,
                    params_json,
                } => {
                    use photonic_core::GrammarRule;
                    // Validate params as JSON
                    if serde_json::from_str::<serde_json::Value>(&params_json).is_ok() {
                        let rule = GrammarRule::new(&name, &rule_type, &params_json);
                        if let Some(idx) = doc.grammar_rules.iter().position(|r| r.name == name) {
                            doc.grammar_rules[idx] = rule;
                        } else {
                            doc.grammar_rules.push(rule);
                        }
                        self.grammar_rules = doc
                            .grammar_rules
                            .iter()
                            .map(|r| (r.name.clone(), r.rule_type.clone()))
                            .collect();
                        doc_modified = true;
                    }
                }

                PanelAction::DeleteGrammarRule { name } => {
                    doc.grammar_rules.retain(|r| r.name != name);
                    self.grammar_rules = doc
                        .grammar_rules
                        .iter()
                        .map(|r| (r.name.clone(), r.rule_type.clone()))
                        .collect();
                    doc_modified = true;
                }

                PanelAction::CheckGrammar => {
                    use photonic_core::node::SceneNodeKind;
                    use photonic_core::style::FillKind;
                    // Gather document metrics
                    let mut unique_colors: std::collections::HashSet<String> =
                        std::collections::HashSet::new();
                    let mut min_text_size: f64 = f64::MAX;
                    let mut total_nodes = 0usize;
                    for node in doc.nodes_in_draw_order() {
                        if !node.visible {
                            continue;
                        }
                        total_nodes += 1;
                        match &node.kind {
                            SceneNodeKind::Path(pn) => {
                                if let FillKind::Solid(c) = &pn.fill.kind {
                                    unique_colors
                                        .insert(format!("{:.3},{:.3},{:.3}", c.r, c.g, c.b));
                                }
                            }
                            SceneNodeKind::Text(tn) => {
                                if let FillKind::Solid(c) = &tn.fill.kind {
                                    unique_colors
                                        .insert(format!("{:.3},{:.3},{:.3}", c.r, c.g, c.b));
                                }
                                if tn.font_size < min_text_size {
                                    min_text_size = tn.font_size;
                                }
                            }
                            SceneNodeKind::Group(_) => {}
                        }
                    }
                    let layer_names: Vec<String> = doc
                        .layer_order
                        .iter()
                        .filter_map(|id| doc.layers.get(id))
                        .map(|l| l.name.clone())
                        .collect();

                    let mut results: Vec<(String, bool, String)> = Vec::new();
                    for rule in &doc.grammar_rules {
                        let params: serde_json::Value = serde_json::from_str(&rule.params_json)
                            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
                        let (passed, msg) = match rule.rule_type.as_str() {
                            "palette_includes" => {
                                let hex = params["color_hex"].as_str().unwrap_or("").to_lowercase();
                                let hex_trim = hex.trim_start_matches('#');
                                let found = if hex_trim.len() == 6 {
                                    if let (Ok(r), Ok(g), Ok(b)) = (
                                        u8::from_str_radix(&hex_trim[0..2], 16),
                                        u8::from_str_radix(&hex_trim[2..4], 16),
                                        u8::from_str_radix(&hex_trim[4..6], 16),
                                    ) {
                                        let (tr, tg, tb) =
                                            (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
                                        unique_colors.iter().any(|c| {
                                            let p: Vec<f32> = c
                                                .split(',')
                                                .filter_map(|x| x.parse().ok())
                                                .collect();
                                            p.len() == 3
                                                && (p[0] - tr).abs() < 0.02
                                                && (p[1] - tg).abs() < 0.02
                                                && (p[2] - tb).abs() < 0.02
                                        })
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                };
                                if found {
                                    (true, format!("{} present", hex))
                                } else {
                                    (false, format!("{} not found in any fill", hex))
                                }
                            }
                            "max_colors" => {
                                let limit = params["count"].as_u64().unwrap_or(10) as usize;
                                if unique_colors.len() <= limit {
                                    (true, format!("{} colors (≤{})", unique_colors.len(), limit))
                                } else {
                                    (
                                        false,
                                        format!(
                                            "{} colors exceeds limit {}",
                                            unique_colors.len(),
                                            limit
                                        ),
                                    )
                                }
                            }
                            "min_text_size" => {
                                let min_px = params["px"].as_f64().unwrap_or(12.0);
                                if min_text_size == f64::MAX {
                                    (true, "no text nodes (vacuously satisfied)".to_string())
                                } else if min_text_size >= min_px {
                                    (
                                        true,
                                        format!(
                                            "smallest text {:.0}px (≥{:.0})",
                                            min_text_size, min_px
                                        ),
                                    )
                                } else {
                                    (
                                        false,
                                        format!(
                                            "text as small as {:.0}px (min {:.0})",
                                            min_text_size, min_px
                                        ),
                                    )
                                }
                            }
                            "required_layer" => {
                                let target = params["name"].as_str().unwrap_or("");
                                let prefix = params["prefix"].as_str().unwrap_or("");
                                let found = if !target.is_empty() {
                                    layer_names.iter().any(|n| n == target)
                                } else {
                                    layer_names.iter().any(|n| n.starts_with(prefix))
                                };
                                if found {
                                    (true, "layer present".to_string())
                                } else {
                                    (
                                        false,
                                        format!(
                                            "layer not found (have: {})",
                                            layer_names.join(", ")
                                        ),
                                    )
                                }
                            }
                            "max_node_count" => {
                                let limit = params["count"].as_u64().unwrap_or(500) as usize;
                                if total_nodes <= limit {
                                    (true, format!("{} nodes (≤{})", total_nodes, limit))
                                } else {
                                    (
                                        false,
                                        format!("{} nodes exceeds limit {}", total_nodes, limit),
                                    )
                                }
                            }
                            _ => (false, format!("unknown rule type")),
                        };
                        results.push((rule.name.clone(), passed, msg));
                    }
                    self.grammar_check_results = results;
                }

                PanelAction::BranchCreate { name } => {
                    history.branch_create(name, doc);
                    self.branch_names = history.branch_list();
                }

                PanelAction::BranchSwitch { name } => {
                    if let Some(snapshot) = history.branch_switch(&name) {
                        *doc = snapshot;
                        self.selected_id = None;
                        doc.selection.clear();
                        self.branch_names = history.branch_list();
                        doc_modified = true;
                    }
                }

                PanelAction::BranchDelete { name } => {
                    history.branch_delete(&name);
                    self.branch_names = history.branch_list();
                }

                PanelAction::BindTextVariable {
                    node_id,
                    variable_name,
                } => {
                    if let Some(node) = doc.nodes.get(&node_id) {
                        if matches!(node.kind, SceneNodeKind::Text(_)) {
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
                                tn.variable_binding = Some(variable_name);
                            }
                            history.execute(
                                Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                },
                                doc,
                            );
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::UnbindTextVariable { node_id } => {
                    if let Some(node) = doc.nodes.get(&node_id) {
                        if matches!(node.kind, SceneNodeKind::Text(_)) {
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
                                tn.variable_binding = None;
                            }
                            history.execute(
                                Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                },
                                doc,
                            );
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::ApplyVariables => {
                    let var_map: std::collections::HashMap<String, String> = doc
                        .variables
                        .iter()
                        .map(|v| (v.name.clone(), v.value.clone()))
                        .collect();
                    let mut commands = Vec::new();
                    for node in doc.nodes.values() {
                        if let SceneNodeKind::Text(ref tn) = node.kind {
                            if let Some(ref binding) = tn.variable_binding {
                                if let Some(value) = var_map.get(binding.as_str()) {
                                    if tn.content != *value {
                                        let mut new_node = node.clone();
                                        if let SceneNodeKind::Text(ref mut new_tn) = new_node.kind {
                                            new_tn.content = value.clone();
                                        }
                                        commands.push(Command::UpdateNode {
                                            old: node.clone(),
                                            new: new_node,
                                        });
                                    }
                                }
                            }
                        }
                    }
                    if !commands.is_empty() {
                        history.execute(Command::Batch(commands), doc);
                        doc_modified = true;
                    }
                }

                PanelAction::DeleteVariable { name } => {
                    doc.variables.retain(|v| v.name != name);
                    doc_modified = true;
                }

                PanelAction::DefineSymbol { node_id, name } => {
                    if let Some(node) = doc.nodes.get(&node_id) {
                        use photonic_core::Symbol;
                        let sym = Symbol::new(name, node.id);
                        doc.symbols.push(sym);
                        doc_modified = true;
                    }
                }

                PanelAction::PlaceSymbol { symbol_name } => {
                    use photonic_core::transform::Transform;
                    if let Some(sym) = doc.symbols.iter().find(|s| s.name == symbol_name).cloned() {
                        if let Some(master) = doc.nodes.get(&sym.master_node_id).cloned() {
                            let layer_id =
                                doc.layers.values().next().map(|l| l.id).unwrap_or_default();
                            let mut instance = master.clone();
                            instance.id = uuid::Uuid::new_v4();
                            instance.name = format!("{} (instance)", sym.name);
                            instance.layer_id = layer_id;
                            instance.transform = Transform::translate(20.0, 20.0);
                            instance.symbol_ref = Some(sym.id);
                            history.execute(
                                Command::AddNode {
                                    node: instance,
                                    layer_id: Some(layer_id),
                                },
                                doc,
                            );
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::BreakLinkToSymbol { node_id } => {
                    if let Some(node) = doc.nodes.get(&node_id).cloned() {
                        let mut new_node = node.clone();
                        new_node.symbol_ref = None;
                        history.execute(
                            Command::UpdateNode {
                                old: node,
                                new: new_node,
                            },
                            doc,
                        );
                        doc_modified = true;
                    }
                }

                PanelAction::DeleteSymbol { name } => {
                    doc.symbols.retain(|s| s.name != name);
                    doc_modified = true;
                }

                PanelAction::SetSymbolOverride {
                    node_id,
                    fill_hex,
                    stroke_hex,
                } => {
                    if let Some(node) = doc.nodes.get(&node_id) {
                        if node.symbol_ref.is_some() {
                            let mut new_node = node.clone();
                            if let Some(hex) = fill_hex {
                                new_node.symbol_fill_override = Some(hex);
                            }
                            if let Some(hex) = stroke_hex {
                                new_node.symbol_stroke_override = Some(hex);
                            }
                            history.execute(
                                Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                },
                                doc,
                            );
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::ClearSymbolOverrides { node_id } => {
                    if let Some(node) = doc.nodes.get(&node_id) {
                        if node.symbol_ref.is_some() {
                            let mut new_node = node.clone();
                            new_node.symbol_fill_override = None;
                            new_node.symbol_stroke_override = None;
                            history.execute(
                                Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                },
                                doc,
                            );
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::SpraySymbolInstances {
                    symbol_name,
                    count,
                    x,
                    y,
                    spread,
                } => {
                    use photonic_core::transform::Transform;
                    let count = count.max(1).min(200);
                    let spread = if spread <= 0.0 { 100.0 } else { spread };

                    if let Some(symbol) =
                        doc.symbols.iter().find(|s| s.name == symbol_name).cloned()
                    {
                        if let Some(master) = doc.nodes.get(&symbol.master_node_id).cloned() {
                            let Some(layer_id) = doc
                                .active_layer_id
                                .or_else(|| doc.layer_order.first().copied())
                            else {
                                continue 'actions;
                            };
                            const GOLDEN_ANGLE: f64 =
                                std::f64::consts::TAU * (1.0 - 1.0 / 1.6180339887498949);
                            for i in 0..count {
                                let r = spread * ((i as f64 + 0.5) / count as f64).sqrt();
                                let theta = i as f64 * GOLDEN_ANGLE;
                                let ix = x + r * theta.cos();
                                let iy = y + r * theta.sin();
                                let mut instance = master.clone();
                                instance.id = uuid::Uuid::new_v4();
                                instance.name = format!("{} (instance {})", symbol.name, i + 1);
                                instance.layer_id = layer_id;
                                instance.transform = Transform::translate(ix, iy);
                                instance.symbol_ref = Some(symbol.id);
                                history.execute(
                                    Command::AddNode {
                                        node: instance,
                                        layer_id: Some(layer_id),
                                    },
                                    doc,
                                );
                            }
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::LoadSymbolLibrary { library_name } => {
                    use photonic_core::node::{PathNode, SceneNodeKind};
                    use photonic_core::path::PathData;
                    use photonic_core::style::Stroke;
                    use photonic_core::transform::Transform;
                    use photonic_core::Symbol;

                    let entries: Vec<(&str, &str)> = match library_name.as_str() {
                        "arrows" => vec![
                            ("arrow-right",    "M10,45 L70,45 L70,30 L90,50 L70,70 L70,55 L10,55 Z"),
                            ("arrow-left",     "M90,45 L30,45 L30,30 L10,50 L30,70 L30,55 L90,55 Z"),
                            ("arrow-up",       "M45,90 L45,30 L30,30 L50,10 L70,30 L55,30 L55,90 Z"),
                            ("arrow-down",     "M45,10 L45,70 L30,70 L50,90 L70,70 L55,70 L55,10 Z"),
                            ("double-arrow-h", "M10,50 L25,35 L25,43 L75,43 L75,35 L90,50 L75,65 L75,57 L25,57 L25,65 Z"),
                            ("arrow-ne",       "M20,80 L70,30 L45,30 L45,20 L80,20 L80,55 L70,55 L70,30"),
                        ],
                        "shapes" => vec![
                            ("diamond",   "M50,5 L95,50 L50,95 L5,50 Z"),
                            ("hexagon",   "M50,5 L91,27 L91,73 L50,95 L9,73 L9,27 Z"),
                            ("pentagon",  "M50,5 L95,34 L79,88 L21,88 L5,34 Z"),
                            ("star-5pt",  "M50,5 L61,35 L95,35 L68,57 L79,91 L50,70 L21,91 L32,57 L5,35 L39,35 Z"),
                            ("cross",     "M35,5 L65,5 L65,35 L95,35 L95,65 L65,65 L65,95 L35,95 L35,65 L5,65 L5,35 L35,35 Z"),
                            ("checkmark", "M10,50 L35,75 L90,20"),
                        ],
                        "ui" => vec![
                            ("checkbox-empty",   "M10,10 L90,10 L90,90 L10,90 Z M15,15 L85,15 L85,85 L15,85 Z"),
                            ("checkbox-checked", "M10,10 L90,10 L90,90 L10,90 Z M20,50 L40,70 L80,25"),
                            ("radio-empty",      "M50,5 A45,45 0 1 1 49.9,5 Z M50,15 A35,35 0 1 1 49.9,15 Z"),
                            ("close-x",          "M15,15 L85,85 M85,15 L15,85"),
                            ("menu-lines",        "M10,25 L90,25 M10,50 L90,50 M10,75 L90,75"),
                            ("plus-icon",         "M50,10 L50,90 M10,50 L90,50"),
                        ],
                        _ => vec![],
                    };

                    if entries.is_empty() {
                        continue 'actions;
                    }

                    let layer_id = doc
                        .active_layer_id
                        .or_else(|| doc.layer_order.first().copied())
                        .unwrap_or(uuid::Uuid::nil());

                    for (i, (name, path_d)) in entries.iter().enumerate() {
                        let sym_name = format!("{}/{}", library_name, name);
                        if doc.symbols.iter().any(|s| s.name == sym_name) {
                            continue;
                        }
                        let Ok(path_data) = PathData::from_svg(path_d) else {
                            continue;
                        };
                        let mut path_node = PathNode::new(path_data);
                        path_node.stroke = Stroke::none();
                        let mut master = photonic_core::node::SceneNode::new(
                            sym_name.clone(),
                            layer_id,
                            SceneNodeKind::Path(path_node),
                        );
                        master.transform =
                            Transform::translate(-9999.0 + i as f64 * 150.0, -9999.0);
                        master.visible = false;
                        let master_id = master.id;
                        history.execute(
                            Command::AddNode {
                                node: master,
                                layer_id: Some(layer_id),
                            },
                            doc,
                        );
                        doc.symbols.push(Symbol::new(&sym_name, master_id));
                    }
                    doc_modified = true;
                }

                PanelAction::SaveWorkspace { name, search_query } => {
                    if let Some(ws) = doc.workspaces.iter_mut().find(|w| w.name == name) {
                        ws.search_query = search_query;
                    } else {
                        doc.workspaces
                            .push(photonic_core::Workspace { name, search_query });
                    }
                    doc_modified = true;
                    self.workspace_name_input.clear();
                }

                PanelAction::LoadWorkspace { name } => {
                    if let Some(ws) = doc.workspaces.iter().find(|w| w.name == name) {
                        self.prop_search = ws.search_query.clone();
                    }
                }

                PanelAction::DeleteWorkspace { name } => {
                    doc.workspaces.retain(|w| w.name != name);
                    doc_modified = true;
                }

                PanelAction::SetTextArea {
                    text_node_id,
                    area_path_id,
                } => {
                    if let Some(node) = doc.nodes.get(&text_node_id) {
                        if matches!(node.kind, SceneNodeKind::Text(_)) {
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
                                tn.area_path_id = Some(area_path_id);
                            }
                            history.execute(
                                Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                },
                                doc,
                            );
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::ClearTextArea { text_node_id } => {
                    if let Some(node) = doc.nodes.get(&text_node_id) {
                        if matches!(node.kind, SceneNodeKind::Text(_)) {
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
                                tn.area_path_id = None;
                            }
                            history.execute(
                                Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                },
                                doc,
                            );
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::SetParagraphOptions {
                    node_id,
                    spacing_before,
                    spacing_after,
                    indent,
                } => {
                    if let Some(node) = doc.nodes.get(&node_id) {
                        if matches!(node.kind, SceneNodeKind::Text(_)) {
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
                                tn.paragraph_spacing_before = spacing_before;
                                tn.paragraph_spacing_after = spacing_after;
                                tn.text_indent = indent;
                            }
                            history.execute(
                                Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                },
                                doc,
                            );
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::SetTabStops { node_id, stops } => {
                    if let Some(node) = doc.nodes.get(&node_id) {
                        if matches!(node.kind, SceneNodeKind::Text(_)) {
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
                                tn.tab_stops = stops;
                            }
                            history.execute(
                                Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                },
                                doc,
                            );
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::ClearTabStops { node_id } => {
                    if let Some(node) = doc.nodes.get(&node_id) {
                        if matches!(node.kind, SceneNodeKind::Text(_)) {
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
                                tn.tab_stops.clear();
                            }
                            history.execute(
                                Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                },
                                doc,
                            );
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::SetTextDecoration {
                    node_id,
                    decoration,
                } => {
                    if let Some(node) = doc.nodes.get(&node_id) {
                        if matches!(node.kind, SceneNodeKind::Text(_)) {
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
                                tn.text_decoration = decoration;
                            }
                            history.execute(
                                Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                },
                                doc,
                            );
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::SetOpenTypeFeatures { node_id, features } => {
                    if let Some(node) = doc.nodes.get(&node_id) {
                        if matches!(node.kind, SceneNodeKind::Text(_)) {
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
                                tn.opentype_features = features;
                            }
                            history.execute(
                                Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                },
                                doc,
                            );
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::LinkTextFrames { from_id, to_id } => {
                    if from_id != to_id {
                        let from_node = doc.nodes.get(&from_id).cloned();
                        let to_node = doc.nodes.get(&to_id).cloned();
                        if let (Some(fn_), Some(tn_)) = (from_node, to_node) {
                            if matches!(fn_.kind, SceneNodeKind::Text(_))
                                && matches!(tn_.kind, SceneNodeKind::Text(_))
                            {
                                let mut new_from = fn_.clone();
                                let mut new_to = tn_.clone();
                                if let SceneNodeKind::Text(ref mut t) = new_from.kind {
                                    t.next_frame = Some(to_id);
                                }
                                if let SceneNodeKind::Text(ref mut t) = new_to.kind {
                                    t.prev_frame = Some(from_id);
                                }
                                history.execute(
                                    Command::Batch(vec![
                                        Command::UpdateNode {
                                            old: fn_,
                                            new: new_from,
                                        },
                                        Command::UpdateNode {
                                            old: tn_,
                                            new: new_to,
                                        },
                                    ]),
                                    doc,
                                );
                                doc_modified = true;
                            }
                        }
                    }
                }

                PanelAction::UnlinkTextFrames { node_id } => {
                    if let Some(node) = doc.nodes.get(&node_id).cloned() {
                        if let SceneNodeKind::Text(ref tn) = node.kind {
                            let prev_id = tn.prev_frame;
                            let next_id = tn.next_frame;
                            let mut cmds: Vec<Command> = Vec::new();
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Text(ref mut t) = new_node.kind {
                                t.prev_frame = None;
                                t.next_frame = None;
                            }
                            cmds.push(Command::UpdateNode {
                                old: node,
                                new: new_node,
                            });
                            if let Some(pid) = prev_id {
                                if let Some(prev) = doc.nodes.get(&pid).cloned() {
                                    let mut np = prev.clone();
                                    if let SceneNodeKind::Text(ref mut t) = np.kind {
                                        t.next_frame = None;
                                    }
                                    cmds.push(Command::UpdateNode { old: prev, new: np });
                                }
                            }
                            if let Some(nid) = next_id {
                                if let Some(next) = doc.nodes.get(&nid).cloned() {
                                    let mut nn = next.clone();
                                    if let SceneNodeKind::Text(ref mut t) = nn.kind {
                                        t.prev_frame = None;
                                    }
                                    cmds.push(Command::UpdateNode { old: next, new: nn });
                                }
                            }
                            history.execute(Command::Batch(cmds), doc);
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::SetTextDirection { node_id, vertical } => {
                    if let Some(node) = doc.nodes.get(&node_id) {
                        if matches!(node.kind, SceneNodeKind::Text(_)) {
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
                                tn.vertical = vertical;
                            }
                            history.execute(
                                Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                },
                                doc,
                            );
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::SetFontStyle { node_id, style } => {
                    use photonic_core::node::FontStyle;
                    if let Some(node) = doc.nodes.get(&node_id) {
                        if matches!(node.kind, SceneNodeKind::Text(_)) {
                            let fs = match style.to_lowercase().as_str() {
                                "italic" => FontStyle::Italic,
                                "oblique" => FontStyle::Oblique,
                                _ => FontStyle::Normal,
                            };
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
                                tn.font_style = fs;
                            }
                            history.execute(
                                Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                },
                                doc,
                            );
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::SetFontWeight { node_id, weight } => {
                    if let Some(node) = doc.nodes.get(&node_id) {
                        if matches!(node.kind, SceneNodeKind::Text(_)) {
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
                                tn.font_weight = weight.clamp(100, 900);
                            }
                            history.execute(
                                Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                },
                                doc,
                            );
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::SetTextPath {
                    text_node_id,
                    path_node_id,
                    offset,
                } => {
                    if let Some(node) = doc.nodes.get(&text_node_id) {
                        if matches!(node.kind, SceneNodeKind::Text(_)) {
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
                                tn.path_spine_id = Some(path_node_id);
                                tn.path_offset = offset;
                            }
                            history.execute(
                                Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                },
                                doc,
                            );
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::ClearTextPath { text_node_id } => {
                    if let Some(node) = doc.nodes.get(&text_node_id) {
                        if matches!(node.kind, SceneNodeKind::Text(_)) {
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
                                tn.path_spine_id = None;
                                tn.path_offset = 0.0;
                            }
                            history.execute(
                                Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                },
                                doc,
                            );
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::MakeClippingMask { group_id } => {
                    if let Some(node) = doc.nodes.get(&group_id) {
                        if let SceneNodeKind::Group(ref g) = node.kind {
                            if g.children.len() >= 2 {
                                let clip_id = *g.children.last().unwrap();
                                let mut new_node = node.clone();
                                if let SceneNodeKind::Group(ref mut gn) = new_node.kind {
                                    gn.clip_node_id = Some(clip_id);
                                }
                                history.execute(
                                    Command::UpdateNode {
                                        old: node.clone(),
                                        new: new_node,
                                    },
                                    doc,
                                );
                                doc_modified = true;
                            }
                        }
                    }
                }

                PanelAction::ReleaseClippingMask { group_id } => {
                    if let Some(node) = doc.nodes.get(&group_id) {
                        if let SceneNodeKind::Group(_) = node.kind {
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Group(ref mut gn) = new_node.kind {
                                gn.clip_node_id = None;
                            }
                            history.execute(
                                Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                },
                                doc,
                            );
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::RoundCorners { node_ids, radius } => {
                    let mut commands = Vec::new();
                    for nid in &node_ids {
                        if let Some(node) = doc.nodes.get(nid) {
                            if let SceneNodeKind::Path(pn) = &node.kind {
                                let bez = pn.path_data.to_bez_path();
                                let new_bez = gui_round_corners(&bez, radius);
                                let mut new_node = node.clone();
                                if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
                                    new_pn.path_data = PathData::from_bez_path(&new_bez);
                                }
                                commands.push(Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                });
                            }
                        }
                    }
                    if !commands.is_empty() {
                        for cmd in commands {
                            history.execute(cmd, doc);
                        }
                        doc_modified = true;
                    }
                }

                PanelAction::WarpEnvelope {
                    node_ids,
                    warp_type,
                    bend,
                } => {
                    let mut commands = Vec::new();
                    for nid in &node_ids {
                        if let Some(node) = doc.nodes.get(nid) {
                            if let SceneNodeKind::Path(pn) = &node.kind {
                                let bez = pn.path_data.to_bez_path();
                                let new_bez = gui_warp_envelope(&bez, &warp_type, bend);
                                let mut new_node = node.clone();
                                if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
                                    new_pn.path_data = PathData::from_bez_path(&new_bez);
                                }
                                commands.push(Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                });
                            }
                        }
                    }
                    if !commands.is_empty() {
                        for cmd in commands {
                            history.execute(cmd, doc);
                        }
                        doc_modified = true;
                    }
                }

                PanelAction::CrystallizePath {
                    node_ids,
                    size,
                    count,
                } => {
                    let mut commands = Vec::new();
                    for nid in &node_ids {
                        if let Some(node) = doc.nodes.get(nid) {
                            if let SceneNodeKind::Path(pn) = &node.kind {
                                let bez = pn.path_data.to_bez_path();
                                let new_bez = gui_crystallize(&bez, size, count);
                                let mut new_node = node.clone();
                                if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
                                    new_pn.path_data = PathData::from_bez_path(&new_bez);
                                }
                                commands.push(Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                });
                            }
                        }
                    }
                    if !commands.is_empty() {
                        for cmd in commands {
                            history.execute(cmd, doc);
                        }
                        doc_modified = true;
                    }
                }

                PanelAction::ScallopPath {
                    node_ids,
                    depth,
                    count,
                } => {
                    let mut commands = Vec::new();
                    for nid in &node_ids {
                        if let Some(node) = doc.nodes.get(nid) {
                            if let SceneNodeKind::Path(pn) = &node.kind {
                                let bez = pn.path_data.to_bez_path();
                                let new_bez = gui_scallop(&bez, depth, count);
                                let mut new_node = node.clone();
                                if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
                                    new_pn.path_data = PathData::from_bez_path(&new_bez);
                                }
                                commands.push(Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                });
                            }
                        }
                    }
                    if !commands.is_empty() {
                        for cmd in commands {
                            history.execute(cmd, doc);
                        }
                        doc_modified = true;
                    }
                }

                PanelAction::BlendObjects {
                    node_id_a,
                    node_id_b,
                    steps,
                } => {
                    gui_blend_objects(node_id_a, node_id_b, steps, doc, history, &mut doc_modified);
                }

                PanelAction::BlendObjectsSmoothColor {
                    node_id_a,
                    node_id_b,
                } => {
                    gui_blend_objects_smooth_color(
                        node_id_a,
                        node_id_b,
                        doc,
                        history,
                        &mut doc_modified,
                    );
                }

                PanelAction::BlendObjectsSpacing {
                    node_id_a,
                    node_id_b,
                    spacing,
                } => {
                    gui_blend_objects_spacing(
                        node_id_a,
                        node_id_b,
                        spacing,
                        doc,
                        history,
                        &mut doc_modified,
                    );
                }

                PanelAction::TwirlPath {
                    node_ids,
                    angle_deg,
                } => {
                    let angle_rad = angle_deg.to_radians();
                    let mut commands = Vec::new();
                    for nid in &node_ids {
                        if let Some(node) = doc.nodes.get(nid) {
                            if let SceneNodeKind::Path(pn) = &node.kind {
                                let bez = pn.path_data.to_bez_path();
                                let center = gui_path_centroid(&bez);
                                let new_bez = gui_twirl(&bez, angle_rad, center);
                                let mut new_node = node.clone();
                                if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
                                    new_pn.path_data = PathData::from_bez_path(&new_bez);
                                }
                                commands.push(Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                });
                            }
                        }
                    }
                    if !commands.is_empty() {
                        for cmd in commands {
                            history.execute(cmd, doc);
                        }
                        doc_modified = true;
                    }
                }

                PanelAction::RoughenPath {
                    node_ids,
                    size,
                    detail,
                    seed,
                } => {
                    let mut commands = Vec::new();
                    let mut idx = 0u64;
                    for nid in &node_ids {
                        if let Some(node) = doc.nodes.get(nid) {
                            if let SceneNodeKind::Path(pn) = &node.kind {
                                let mut bez = pn.path_data.to_bez_path();
                                for _ in 0..detail {
                                    bez = gui_subdivide_bez(&bez);
                                }
                                let new_bez = gui_roughen(&bez, size, seed + idx);
                                let mut new_node = node.clone();
                                if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
                                    new_pn.path_data = PathData::from_bez_path(&new_bez);
                                }
                                commands.push(Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                });
                                idx += 1;
                            }
                        }
                    }
                    if !commands.is_empty() {
                        for cmd in commands {
                            history.execute(cmd, doc);
                        }
                        doc_modified = true;
                    }
                }

                PanelAction::SelectByKind { kind, additive } => {
                    if !additive {
                        doc.selection.clear();
                        self.selected_id = None;
                    }
                    let ids_to_select: Vec<NodeId> = doc
                        .nodes
                        .iter()
                        .filter_map(|(nid, node)| {
                            let matches = match kind.as_str() {
                                "path" => matches!(node.kind, SceneNodeKind::Path(_)),
                                "text" => matches!(node.kind, SceneNodeKind::Text(_)),
                                "group" => matches!(node.kind, SceneNodeKind::Group(_)),
                                "same_layer" => doc
                                    .active_layer_id
                                    .map(|lid| node.layer_id == lid)
                                    .unwrap_or(false),
                                _ => false,
                            };
                            if matches {
                                Some(*nid)
                            } else {
                                None
                            }
                        })
                        .collect();
                    for nid in ids_to_select {
                        doc.selection.add(nid);
                        if self.selected_id.is_none() {
                            self.selected_id = Some(nid);
                        }
                    }
                    doc_modified = true;
                }

                PanelAction::CreateRadarChart => {
                    let cx = doc.width / 2.0;
                    let cy = doc.height / 2.0;
                    gui_create_radar_chart_demo(cx, cy, doc, history, &mut doc_modified);
                }

                PanelAction::CreateStackedBarChart => {
                    let x = doc.width / 2.0 - 150.0;
                    let y = doc.height / 2.0 + 100.0;
                    gui_create_stacked_bar_chart_demo(x, y, doc, history, &mut doc_modified);
                }

                PanelAction::CreateParametricShape { shape_type } => {
                    let cx = doc.width / 2.0;
                    let cy = doc.height / 2.0;
                    gui_create_parametric_shape_demo(
                        &shape_type,
                        cx,
                        cy,
                        doc,
                        history,
                        &mut doc_modified,
                    );
                }

                PanelAction::OffsetPath { node_ids, distance } => {
                    use kurbo::Join;
                    use photonic_core::ops::offset::offset_path as do_offset;

                    let mut commands = Vec::new();
                    for nid in &node_ids {
                        if let Some(node) = doc.nodes.get(nid).cloned() {
                            if let SceneNodeKind::Path(ref pn) = node.kind {
                                if let Ok(offset_data) =
                                    do_offset(&pn.path_data, distance, Join::Miter)
                                {
                                    let layer_id = node.layer_id;
                                    let mut new_pn = pn.clone();
                                    new_pn.path_data = offset_data;
                                    let label = if distance >= 0.0 {
                                        format!("{} +{:.0}px", node.name, distance)
                                    } else {
                                        format!("{} {:.0}px", node.name, distance)
                                    };
                                    let new_node = SceneNode::new(
                                        &label,
                                        layer_id,
                                        SceneNodeKind::Path(new_pn),
                                    );
                                    commands.push(Command::AddNode {
                                        node: new_node,
                                        layer_id: Some(layer_id),
                                    });
                                }
                            }
                        }
                    }
                    if !commands.is_empty() {
                        let batch = if commands.len() == 1 {
                            commands.remove(0)
                        } else {
                            Command::Batch(commands)
                        };
                        history.execute(batch, doc);
                        doc_modified = true;
                    }
                }

                PanelAction::CreateTruchetTiling { style } => {
                    let margin = 20.0_f64;
                    let size = (doc.width.min(doc.height) - 2.0 * margin).max(40.0);
                    let x = (doc.width - size) / 2.0;
                    let y = (doc.height - size) / 2.0;
                    gui_create_truchet_tiling_demo(
                        &style,
                        x,
                        y,
                        size,
                        doc,
                        history,
                        &mut doc_modified,
                    );
                }

                PanelAction::DistributeNoOverlap { node_ids } => {
                    let padding = 4.0_f64;
                    let max_iter = 100_usize;
                    let n = node_ids.len().min(100);
                    if n < 2 {
                        // nothing to do
                    } else {
                        let mut offsets: Vec<(f64, f64)> = vec![(0.0, 0.0); n];

                        let world_bboxes: Vec<(f64, f64, f64, f64)> = node_ids[..n]
                            .iter()
                            .map(|id| -> (f64, f64, f64, f64) {
                                if let Some(node) = doc.nodes.get(id) {
                                    let tx = node.transform.matrix[4];
                                    let ty = node.transform.matrix[5];
                                    if let SceneNodeKind::Path(pn) = &node.kind {
                                        let bb = pn
                                            .path_data
                                            .bounding_box()
                                            .unwrap_or(kurbo::Rect::ZERO);
                                        return (bb.x0 + tx, bb.y0 + ty, bb.x1 + tx, bb.y1 + ty);
                                    }
                                    return (tx, ty, tx, ty);
                                }
                                (0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64)
                            })
                            .collect();

                        for _ in 0..max_iter {
                            let mut any = false;
                            for i in 0..n {
                                for j in (i + 1)..n {
                                    let half_pad = padding / 2.0;
                                    let (ax0, ay0, ax1, ay1) = (
                                        world_bboxes[i].0 + offsets[i].0 - half_pad,
                                        world_bboxes[i].1 + offsets[i].1 - half_pad,
                                        world_bboxes[i].2 + offsets[i].0 + half_pad,
                                        world_bboxes[i].3 + offsets[i].1 + half_pad,
                                    );
                                    let (bx0, by0, bx1, by1) = (
                                        world_bboxes[j].0 + offsets[j].0 - half_pad,
                                        world_bboxes[j].1 + offsets[j].1 - half_pad,
                                        world_bboxes[j].2 + offsets[j].0 + half_pad,
                                        world_bboxes[j].3 + offsets[j].1 + half_pad,
                                    );
                                    let ox: f64 = (ax1.min(bx1) - ax0.max(bx0)).max(0.0);
                                    let oy: f64 = (ay1.min(by1) - ay0.max(by0)).max(0.0);
                                    if ox > 0.0 && oy > 0.0 {
                                        any = true;
                                        let (px, py) = if ox < oy {
                                            let dir = if (ax0 + ax1) / 2.0 <= (bx0 + bx1) / 2.0 {
                                                -1.0
                                            } else {
                                                1.0
                                            };
                                            (dir * ox / 2.0, 0.0)
                                        } else {
                                            let dir = if (ay0 + ay1) / 2.0 <= (by0 + by1) / 2.0 {
                                                -1.0
                                            } else {
                                                1.0
                                            };
                                            (0.0, dir * oy / 2.0)
                                        };
                                        offsets[i].0 += px;
                                        offsets[i].1 += py;
                                        offsets[j].0 -= px;
                                        offsets[j].1 -= py;
                                    }
                                }
                            }
                            if !any {
                                break;
                            }
                        }

                        let mut commands = Vec::new();
                        for (i, nid) in node_ids[..n].iter().enumerate() {
                            let (dx, dy): (f64, f64) = offsets[i];
                            if dx.abs() > 0.01 || dy.abs() > 0.01 {
                                if let Some(node) = doc.nodes.get(nid).cloned() {
                                    let mut new_node = node.clone();
                                    new_node.transform.matrix[4] += dx;
                                    new_node.transform.matrix[5] += dy;
                                    commands.push(Command::UpdateNode {
                                        old: node,
                                        new: new_node,
                                    });
                                }
                            }
                        }
                        if !commands.is_empty() {
                            let batch = if commands.len() == 1 {
                                commands.remove(0)
                            } else {
                                Command::Batch(commands)
                            };
                            history.execute(batch, doc);
                            doc_modified = true;
                        }
                    } // end else n >= 2
                }

                PanelAction::NoiseDeform {
                    node_ids,
                    amplitude,
                    style,
                } => {
                    let frequency = 0.05_f64;
                    let seed = 0.0_f64;
                    let axis: &str = &style;
                    let deform_x = axis == "both" || axis == "x";
                    let deform_y = axis == "both" || axis == "y";

                    let displace = |pt: kurbo::Point| -> kurbo::Point {
                        let dx = if deform_x {
                            amplitude * (pt.y * frequency + seed).sin()
                                + (amplitude * 0.5) * (pt.y * frequency * 2.1 + seed * 1.3).sin()
                        } else {
                            0.0
                        };
                        let dy = if deform_y {
                            amplitude
                                * (pt.x * frequency + seed + std::f64::consts::FRAC_PI_2).sin()
                                + (amplitude * 0.5) * (pt.x * frequency * 2.1 + seed * 1.7).sin()
                        } else {
                            0.0
                        };
                        kurbo::Point::new(pt.x + dx, pt.y + dy)
                    };

                    let mut commands = Vec::new();
                    for nid in &node_ids {
                        if let Some(node) = doc.nodes.get(nid).cloned() {
                            if let SceneNodeKind::Path(ref pn) = node.kind {
                                let bez = pn.path_data.to_bez_path();
                                let new_els: Vec<kurbo::PathEl> = bez
                                    .iter()
                                    .map(|el| match el {
                                        kurbo::PathEl::MoveTo(p) => {
                                            kurbo::PathEl::MoveTo(displace(p))
                                        }
                                        kurbo::PathEl::LineTo(p) => {
                                            kurbo::PathEl::LineTo(displace(p))
                                        }
                                        kurbo::PathEl::QuadTo(p1, p2) => {
                                            kurbo::PathEl::QuadTo(displace(p1), displace(p2))
                                        }
                                        kurbo::PathEl::CurveTo(p1, p2, p3) => {
                                            kurbo::PathEl::CurveTo(
                                                displace(p1),
                                                displace(p2),
                                                displace(p3),
                                            )
                                        }
                                        kurbo::PathEl::ClosePath => kurbo::PathEl::ClosePath,
                                    })
                                    .collect();
                                let new_bez = kurbo::BezPath::from_vec(new_els);
                                let mut new_node = node.clone();
                                if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
                                    new_pn.path_data = PathData::from_bez_path(&new_bez);
                                }
                                commands.push(Command::UpdateNode {
                                    old: node,
                                    new: new_node,
                                });
                            }
                        }
                    }
                    if !commands.is_empty() {
                        let batch = if commands.len() == 1 {
                            commands.remove(0)
                        } else {
                            Command::Batch(commands)
                        };
                        history.execute(batch, doc);
                        doc_modified = true;
                    }
                }

                PanelAction::SetBlendSpine { group_id, path_id } => {
                    if let Some(node) = doc.nodes.get(&group_id) {
                        if matches!(node.kind, SceneNodeKind::Group(_)) {
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Group(ref mut gn) = new_node.kind {
                                gn.blend_spine_id = Some(path_id);
                            }
                            history.execute(
                                Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                },
                                doc,
                            );
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::ClearBlendSpine { group_id } => {
                    if let Some(node) = doc.nodes.get(&group_id) {
                        if matches!(node.kind, SceneNodeKind::Group(_)) {
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Group(ref mut gn) = new_node.kind {
                                gn.blend_spine_id = None;
                            }
                            history.execute(
                                Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                },
                                doc,
                            );
                            doc_modified = true;
                        }
                    }
                }

                PanelAction::ExpandBlend { group_id } => {
                    if let Some(node) = doc.nodes.get(&group_id) {
                        if let SceneNodeKind::Group(ref gn) = node.kind {
                            let children = gn.children.clone();
                            let child_count = children.len();
                            if let Some((layer_id, group_index)) =
                                doc.node_layer_and_index(&group_id)
                            {
                                let cmd = Command::UngroupNodes {
                                    group: node.clone(),
                                    layer_id,
                                    group_index,
                                    children,
                                };
                                history.execute(cmd, doc);
                                doc_modified = true;
                                let _ = child_count; // suppress unused warning
                            }
                        }
                    }
                }

                PanelAction::FitToMargins => {
                    let safe_x = doc.margin_left;
                    let safe_y = doc.margin_top;
                    let safe_w = doc.width - doc.margin_left - doc.margin_right;
                    let safe_h = doc.height - doc.margin_top - doc.margin_bottom;

                    if safe_w > 0.0 && safe_h > 0.0 {
                        // Collect target node IDs (selected or all)
                        let target_ids: Vec<_> = if doc.selection.count() > 0 {
                            doc.selection.node_ids.iter().copied().collect()
                        } else {
                            doc.nodes.keys().copied().collect()
                        };

                        // Compute union bbox
                        let mut ux0 = f64::MAX;
                        let mut uy0 = f64::MAX;
                        let mut ux1 = f64::MIN;
                        let mut uy1 = f64::MIN;
                        let mut valid: Vec<photonic_core::node::NodeId> = Vec::new();
                        for nid in &target_ids {
                            if let Some(node) = doc.nodes.get(nid) {
                                if let Some(lb) = node.local_bounds() {
                                    let (x0, y0) = node.transform.apply(lb.x0, lb.y0);
                                    let (x1, y1) = node.transform.apply(lb.x1, lb.y1);
                                    ux0 = ux0.min(x0.min(x1));
                                    uy0 = uy0.min(y0.min(y1));
                                    ux1 = ux1.max(x0.max(x1));
                                    uy1 = uy1.max(y0.max(y1));
                                    valid.push(*nid);
                                }
                            }
                        }

                        if !valid.is_empty() && ux0 < ux1 && uy0 < uy1 {
                            let cw = ux1 - ux0;
                            let ch = uy1 - uy0;
                            let scale = (safe_w / cw).min(safe_h / ch);
                            let cx = (ux0 + ux1) / 2.0;
                            let cy = (uy0 + uy1) / 2.0;
                            let tcx = safe_x + safe_w / 2.0;
                            let tcy = safe_y + safe_h / 2.0;
                            let mut cmds: Vec<Command> = Vec::new();
                            for nid in &valid {
                                if let Some(node) = doc.nodes.get(nid) {
                                    let tx = node.transform.matrix[4];
                                    let ty = node.transform.matrix[5];
                                    let mut nn = node.clone();
                                    nn.transform.matrix[4] = tcx + (tx - cx) * scale;
                                    nn.transform.matrix[5] = tcy + (ty - cy) * scale;
                                    nn.transform.matrix[0] *= scale;
                                    nn.transform.matrix[3] *= scale;
                                    cmds.push(Command::UpdateNode {
                                        old: node.clone(),
                                        new: nn,
                                    });
                                }
                            }
                            if !cmds.is_empty() {
                                history.execute(Command::Batch(cmds), doc);
                                doc_modified = true;
                            }
                        }
                    }
                }

                PanelAction::AddDimension {
                    from_id,
                    to_id,
                    axis,
                } => {
                    use photonic_core::DimensionAnnotation;
                    let from_center = doc.nodes.get(&from_id).map(|n| {
                        if let Some(lb) = n.local_bounds() {
                            let (x0, y0) = n.transform.apply(lb.x0, lb.y0);
                            let (x1, y1) = n.transform.apply(lb.x1, lb.y1);
                            ((x0 + x1) / 2.0, (y0 + y1) / 2.0)
                        } else {
                            n.transform.apply(0.0, 0.0)
                        }
                    });
                    let to_center = doc.nodes.get(&to_id).map(|n| {
                        if let Some(lb) = n.local_bounds() {
                            let (x0, y0) = n.transform.apply(lb.x0, lb.y0);
                            let (x1, y1) = n.transform.apply(lb.x1, lb.y1);
                            ((x0 + x1) / 2.0, (y0 + y1) / 2.0)
                        } else {
                            n.transform.apply(0.0, 0.0)
                        }
                    });
                    if let (Some((fx, fy)), Some((tx, ty))) = (from_center, to_center) {
                        let dim =
                            DimensionAnnotation::new(from_id, to_id, axis, 20.0, fx, fy, tx, ty);
                        doc.dimensions.push(dim);
                        doc_modified = true;
                    }
                }

                PanelAction::RemoveDimension { id } => {
                    doc.dimensions.retain(|d| d.id != id);
                    doc_modified = true;
                }

                PanelAction::JumpToHistory { index } => {
                    let current = history.undo_depth();
                    let max_index = current + history.redo_depth();
                    let target = index.min(max_index);
                    if target < current {
                        for _ in 0..(current - target) {
                            if !history.undo(doc) {
                                break;
                            }
                        }
                        self.selected_id = doc.selection.ids().next().copied();
                        doc_modified = true;
                    } else if target > current {
                        for _ in 0..(target - current) {
                            if !history.redo(doc) {
                                break;
                            }
                        }
                        self.selected_id = doc.selection.ids().next().copied();
                        doc_modified = true;
                    }
                }

                PanelAction::ReverseBlendSpine { group_id } => {
                    let spine_id = doc.nodes.get(&group_id).and_then(|n| {
                        if let SceneNodeKind::Group(ref gn) = n.kind {
                            gn.blend_spine_id
                        } else {
                            None
                        }
                    });
                    if let Some(sid) = spine_id {
                        if let Some(spine) = doc.nodes.get(&sid) {
                            if matches!(spine.kind, SceneNodeKind::Path(_)) {
                                let mut new_spine = spine.clone();
                                if let SceneNodeKind::Path(ref mut pn) = new_spine.kind {
                                    pn.path_data = pn.path_data.reverse();
                                }
                                history.execute(
                                    Command::UpdateNode {
                                        old: spine.clone(),
                                        new: new_spine,
                                    },
                                    doc,
                                );
                                doc_modified = true;
                            }
                        }
                    }
                }
            }
        }

        // ── Eyedropper overlay ────────────────────────────────────────────────
        if self.eyedropper.active() {
            ctx.set_cursor_icon(egui::CursorIcon::Crosshair);

            let (esc, raw_clicked, cursor) = ctx.input(|i| {
                (
                    i.key_pressed(egui::Key::Escape),
                    i.pointer.primary_clicked(),
                    i.pointer.latest_pos(),
                )
            });
            // Discard the button's own release so it doesn't immediately sample.
            let clicked = if self.eyedropper.skip_click {
                if raw_clicked {
                    self.eyedropper.skip_click = false;
                }
                false
            } else {
                raw_clicked
            };

            if esc {
                self.eyedropper.cancel();
            } else {
                if let Some(pos) = cursor {
                    let sx = self.window_logical_pos.0 as f32 + pos.x;
                    let sy = self.window_logical_pos.1 as f32 + pos.y;

                    // Draw color preview badge near cursor
                    let sampled = self.eyedropper.sample_at_screen_logical(sx, sy);
                    let preview_color = sampled
                        .map(|c| egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]))
                        .unwrap_or(egui::Color32::TRANSPARENT);

                    let painter = ctx.layer_painter(egui::LayerId::new(
                        egui::Order::Tooltip,
                        egui::Id::new("eyedropper_preview"),
                    ));
                    let preview_rect = egui::Rect::from_min_size(
                        pos + egui::vec2(14.0, -28.0),
                        egui::vec2(28.0, 28.0),
                    );
                    painter.rect_filled(preview_rect, 4.0, preview_color);
                    painter.rect_stroke(
                        preview_rect,
                        4.0,
                        egui::Stroke::new(1.5, egui::Color32::WHITE),
                    );

                    if clicked {
                        if let Some(rgba) = sampled {
                            let picked = photonic_core::Color {
                                r: rgba[0] as f32 / 255.0,
                                g: rgba[1] as f32 / 255.0,
                                b: rgba[2] as f32 / 255.0,
                                a: rgba[3] as f32 / 255.0,
                            };
                            self.apply_eyedropper_color(doc, history, picked, &mut doc_modified);
                        }
                        self.eyedropper.cancel();
                    }
                }

                // Full-screen invisible area to block other interactions
                egui::Area::new(egui::Id::new("eyedropper_overlay"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(egui::pos2(0.0, 0.0))
                    .show(ctx, |ui| {
                        ui.allocate_rect(ctx.screen_rect(), egui::Sense::click());
                    });
            }
        }

        doc_modified
    }

    fn apply_eyedropper_color(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
        picked: photonic_core::Color,
        doc_modified: &mut bool,
    ) {
        use photonic_core::history::Command;
        use photonic_core::{style::FillKind, SceneNodeKind};

        match self.eyedropper.target.take() {
            Some(EyedropperTarget::NewShapeFill) => {
                self.fill_color = [picked.r, picked.g, picked.b, picked.a];
            }
            Some(EyedropperTarget::NodeFillSolid { node_id }) => {
                let new_fill = Fill::solid(picked);
                if let Some(node) = doc.get_node(&node_id) {
                    let mut updated = node.clone();
                    if let SceneNodeKind::Path(pn) = &mut updated.kind {
                        pn.fill = new_fill;
                    }
                    history.execute(
                        Command::UpdateNode {
                            old: node.clone(),
                            new: updated,
                        },
                        doc,
                    );
                    *doc_modified = true;
                }
            }
            Some(EyedropperTarget::NodeFillGradStop { node_id, idx }) => {
                if let Some(node) = doc.get_node(&node_id) {
                    let mut updated = node.clone();
                    if let SceneNodeKind::Path(pn) = &mut updated.kind {
                        if let FillKind::Gradient(ref mut g) = pn.fill.kind {
                            if let Some(s) = g.stops.get_mut(idx) {
                                s.color = picked;
                            }
                        }
                    }
                    history.execute(
                        Command::UpdateNode {
                            old: node.clone(),
                            new: updated,
                        },
                        doc,
                    );
                    *doc_modified = true;
                }
            }
            Some(EyedropperTarget::NodeFillFluid { node_id, idx }) => {
                if let Some(node) = doc.get_node(&node_id) {
                    let mut updated = node.clone();
                    if let SceneNodeKind::Path(pn) = &mut updated.kind {
                        if let FillKind::FluidGradient(ref mut fg) = pn.fill.kind {
                            if let Some(p) = fg.points.get_mut(idx) {
                                p.color = picked;
                            }
                        }
                    }
                    history.execute(
                        Command::UpdateNode {
                            old: node.clone(),
                            new: updated,
                        },
                        doc,
                    );
                    *doc_modified = true;
                }
            }
            Some(EyedropperTarget::NodeFillMesh { node_id, idx }) => {
                if let Some(node) = doc.get_node(&node_id) {
                    let mut updated = node.clone();
                    if let SceneNodeKind::Path(pn) = &mut updated.kind {
                        if let FillKind::MeshGradient(ref mut mg) = pn.fill.kind {
                            if let Some(v) = mg.vertices.get_mut(idx) {
                                v.color = picked;
                            }
                        }
                    }
                    history.execute(
                        Command::UpdateNode {
                            old: node.clone(),
                            new: updated,
                        },
                        doc,
                    );
                    *doc_modified = true;
                }
            }
            Some(EyedropperTarget::NodeStroke { node_id }) => {
                if let Some(node) = doc.get_node(&node_id) {
                    let mut updated = node.clone();
                    if let SceneNodeKind::Path(pn) = &mut updated.kind {
                        pn.stroke.color = picked;
                    }
                    history.execute(
                        Command::UpdateNode {
                            old: node.clone(),
                            new: updated,
                        },
                        doc,
                    );
                    *doc_modified = true;
                }
            }
            Some(EyedropperTarget::NodeOuterGlow { node_id }) => {
                if let Some(node) = doc.get_node(&node_id) {
                    let mut updated = node.clone();
                    updated.outer_glow.color = picked;
                    history.execute(
                        Command::UpdateNode {
                            old: node.clone(),
                            new: updated,
                        },
                        doc,
                    );
                    *doc_modified = true;
                }
            }
            Some(EyedropperTarget::NodeInnerGlow { node_id }) => {
                if let Some(node) = doc.get_node(&node_id) {
                    let mut updated = node.clone();
                    updated.inner_glow.color = picked;
                    history.execute(
                        Command::UpdateNode {
                            old: node.clone(),
                            new: updated,
                        },
                        doc,
                    );
                    *doc_modified = true;
                }
            }
            Some(EyedropperTarget::NodeGaussianGlow { node_id }) => {
                if let Some(node) = doc.get_node(&node_id) {
                    let mut updated = node.clone();
                    updated.gaussian_glow.color = picked;
                    history.execute(
                        Command::UpdateNode {
                            old: node.clone(),
                            new: updated,
                        },
                        doc,
                    );
                    *doc_modified = true;
                }
            }
            None => {}
        }
    }

    // ── Select tool handler ───────────────────────────────────────────────────

    fn handle_select_tool(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        doc: &mut Document,
        view: &CanvasView,
        renderer: &mut PhotonicRenderer,
        doc_modified: &mut bool,
        history: &mut CommandHistory,
    ) {
        // ── Keyboard shortcuts (skipped when a text widget has focus) ─────────
        if viewport_kb(ui.ctx()) {
            if let Some(sel_id) = self.selected_id {
                let (delete, ctrl, shift, bracket_right, bracket_left, key_g) = ui.input(|i| {
                    (
                        i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
                        i.modifiers.ctrl,
                        i.modifiers.shift,
                        i.key_pressed(egui::Key::CloseBracket),
                        i.key_pressed(egui::Key::OpenBracket),
                        i.key_pressed(egui::Key::G),
                    )
                });

                // Delete / Backspace: remove all selected nodes
                if delete {
                    let ids_to_delete: Vec<NodeId> = doc.selection.ids().copied().collect();
                    for id in ids_to_delete {
                        doc.remove_node(&id);
                    }
                    doc.selection.clear();
                    self.selected_id = None;
                    *doc_modified = true;
                    return;
                }

                // Z-order shortcuts: Ctrl+] / Ctrl+[ (with Shift for extremes)
                if ctrl && (bracket_right || bracket_left) {
                    if let Some((layer_id, cur_idx)) = doc.node_layer_and_index(&sel_id) {
                        let layer_len = doc
                            .layers
                            .get(&layer_id)
                            .map(|l| l.node_ids.len())
                            .unwrap_or(0);
                        if layer_len > 0 {
                            let new_index = if bracket_right && shift {
                                layer_len - 1 // Bring to Front
                            } else if bracket_left && shift {
                                0 // Send to Back
                            } else if bracket_right {
                                (cur_idx + 1).min(layer_len - 1) // Bring Forward
                            } else {
                                cur_idx.saturating_sub(1) // Send Backward
                            };
                            if new_index != cur_idx {
                                let cmd = Command::ReorderNode {
                                    layer_id,
                                    node_id: sel_id,
                                    old_index: cur_idx,
                                    new_index,
                                };
                                history.execute(cmd, doc);
                                *doc_modified = true;
                            }
                        }
                    }
                }

                // Ctrl+Shift+G: ungroup (only if selected node is a group)
                if ctrl && shift && key_g {
                    if let Some(node) = doc.get_node(&sel_id) {
                        if let SceneNodeKind::Group(g) = &node.kind {
                            let children = g.children.clone();
                            let node_clone = node.clone();
                            if let Some((layer_id, group_index)) = doc.node_layer_and_index(&sel_id)
                            {
                                let first_child = children.first().copied();
                                let cmd = Command::UngroupNodes {
                                    group: node_clone,
                                    layer_id,
                                    group_index,
                                    children,
                                };
                                history.execute(cmd, doc);
                                self.selected_id = first_child;
                                if let Some(fc) = first_child {
                                    doc.selection = Selection::single(fc);
                                } else {
                                    doc.selection.clear();
                                }
                                *doc_modified = true;
                            }
                        }
                    }
                }
            }

            // Ctrl+G: group selected nodes (requires 2+ in selection)
            let (ctrl_g, shift_g) = ui.input(|i| {
                (
                    i.modifiers.ctrl && !i.modifiers.shift && i.key_pressed(egui::Key::G),
                    i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(egui::Key::G),
                )
            });
            if ctrl_g && !shift_g && doc.selection.count() >= 2 {
                self.do_group_selected(doc, history, doc_modified);
            }

            // Ctrl+Y: toggle Outline Mode
            if ui.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Y)) {
                self.outline_mode = !self.outline_mode;
            }

            // Ctrl+;: toggle guide visibility
            if ui.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Semicolon)) {
                self.guides_visible = !self.guides_visible;
            }

            // Ctrl+C: copy selected nodes to in-process clipboard.
            if ui.input(|i| i.modifiers.ctrl && !i.modifiers.shift && i.key_pressed(egui::Key::C)) {
                self.gui_clipboard.clear();
                for nid in doc.selection.ids() {
                    if let Some(node) = doc.nodes.get(nid) {
                        self.gui_clipboard.push(node.clone());
                    }
                }
            }

            // Ctrl+V: paste from clipboard with +10px offset.
            // Ctrl+Shift+V: paste in place (exact original coordinates).
            let (paste, paste_in_place) = ui.input(|i| {
                (
                    i.modifiers.ctrl && !i.modifiers.shift && i.key_pressed(egui::Key::V),
                    i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(egui::Key::V),
                )
            });
            if (paste || paste_in_place) && !self.gui_clipboard.is_empty() {
                let offset = if paste { 10.0_f64 } else { 0.0 };
                if let Some(target_layer) = doc
                    .active_layer_id
                    .or_else(|| doc.layer_order.first().copied())
                {
                    let mut cmds: Vec<Command> = Vec::new();
                    let mut new_ids: Vec<NodeId> = Vec::new();
                    for src in &self.gui_clipboard {
                        let mut new_node = src.clone();
                        new_node.id = uuid::Uuid::new_v4();
                        new_node.layer_id = target_layer;
                        if offset.abs() > 1e-9 {
                            new_node.transform.matrix[4] += offset;
                            new_node.transform.matrix[5] += offset;
                        }
                        new_ids.push(new_node.id);
                        cmds.push(Command::AddNode {
                            node: new_node,
                            layer_id: Some(target_layer),
                        });
                    }
                    if !cmds.is_empty() {
                        history.execute(Command::Batch(cmds), doc);
                        doc.selection = Selection::from_ids(new_ids.iter().copied());
                        if let Some(first) = new_ids.first() {
                            self.selected_id = Some(*first);
                        }
                        *doc_modified = true;
                    }
                }
            }

            // Ctrl+Shift+H: flip horizontal / Ctrl+Shift+V: flip vertical
            if ui.input(|i| i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(egui::Key::H)) {
                let sel: Vec<NodeId> = doc.selection.node_ids.iter().copied().collect();
                for nid in &sel {
                    if let Some(node) = doc.nodes.get(nid) {
                        if let SceneNodeKind::Path(pn) = &node.kind {
                            use kurbo::Shape;
                            let bez = pn.path_data.to_bez_path();
                            let bbox = bez.bounding_box();
                            let cx = bbox.x0 + bbox.width() / 2.0;
                            let mut new_bez = BezPath::new();
                            for el in bez.elements() {
                                let flip = |p: kurbo::Point| kurbo::Point::new(2.0 * cx - p.x, p.y);
                                match *el {
                                    PathEl::MoveTo(p) => new_bez.move_to(flip(p)),
                                    PathEl::LineTo(p) => new_bez.line_to(flip(p)),
                                    PathEl::CurveTo(c1, c2, p) => {
                                        new_bez.curve_to(flip(c1), flip(c2), flip(p))
                                    }
                                    PathEl::QuadTo(c, p) => new_bez.quad_to(flip(c), flip(p)),
                                    PathEl::ClosePath => new_bez.close_path(),
                                }
                            }
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Path(ref mut np) = new_node.kind {
                                np.path_data = PathData::from_bez_path(&new_bez);
                            }
                            history.execute(
                                Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                },
                                doc,
                            );
                            *doc_modified = true;
                        }
                    }
                }
            }
            if ui.input(|i| i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(egui::Key::J)) {
                let sel: Vec<NodeId> = doc.selection.node_ids.iter().copied().collect();
                for nid in &sel {
                    if let Some(node) = doc.nodes.get(nid) {
                        if let SceneNodeKind::Path(pn) = &node.kind {
                            use kurbo::Shape;
                            let bez = pn.path_data.to_bez_path();
                            let bbox = bez.bounding_box();
                            let cy = bbox.y0 + bbox.height() / 2.0;
                            let mut new_bez = BezPath::new();
                            for el in bez.elements() {
                                let flip = |p: kurbo::Point| kurbo::Point::new(p.x, 2.0 * cy - p.y);
                                match *el {
                                    PathEl::MoveTo(p) => new_bez.move_to(flip(p)),
                                    PathEl::LineTo(p) => new_bez.line_to(flip(p)),
                                    PathEl::CurveTo(c1, c2, p) => {
                                        new_bez.curve_to(flip(c1), flip(c2), flip(p))
                                    }
                                    PathEl::QuadTo(c, p) => new_bez.quad_to(flip(c), flip(p)),
                                    PathEl::ClosePath => new_bez.close_path(),
                                }
                            }
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Path(ref mut np) = new_node.kind {
                                np.path_data = PathData::from_bez_path(&new_bez);
                            }
                            history.execute(
                                Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                },
                                doc,
                            );
                            *doc_modified = true;
                        }
                    }
                }
            }

            // Ctrl+Z: undo / Ctrl+R: redo

            let (ctrl_z, ctrl_r) = ui.input(|i| {
                (
                    i.modifiers.ctrl && !i.modifiers.shift && i.key_pressed(egui::Key::Z),
                    i.modifiers.ctrl && i.key_pressed(egui::Key::R),
                )
            });
            if ctrl_z {
                if history.undo(doc) {
                    self.selected_id = doc.selection.ids().next().copied();
                    *doc_modified = true;
                }
            }
            if ctrl_r {
                if history.redo(doc) {
                    self.selected_id = doc.selection.ids().next().copied();
                    *doc_modified = true;
                }
            }
        } // end viewport_kb

        // ── Isolation Mode: Escape exits ─────────────────────────────────────
        if self.isolated_group.is_some() {
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.isolated_group = None;
                doc.selection.clear();
                self.selected_id = None;
            }
        }

        // ── Double-click: enter Isolation Mode on a group ─────────────────────
        if response.double_clicked_by(egui::PointerButton::Primary) {
            if let Some(pos) = response.interact_pointer_pos() {
                let (cx, cy) = view.screen_to_canvas(pos.x as f64, pos.y as f64);
                let hit = hit_test(doc, cx, cy, renderer);
                if let Some(id) = hit {
                    if let Some(node) = doc.nodes.get(&id) {
                        if matches!(node.kind, SceneNodeKind::Group(_)) {
                            self.isolated_group = Some(id);
                            // Select children of the group.
                            if let SceneNodeKind::Group(g) = &node.kind {
                                doc.selection.clear();
                                for cid in &g.children {
                                    doc.selection.add(*cid);
                                }
                                self.selected_id = g.children.first().copied();
                            }
                            *doc_modified = true;
                            return;
                        }
                    }
                }
                // Double-click on non-group or empty: exit isolation if active
                if self.isolated_group.is_some() {
                    self.isolated_group = None;
                    doc.selection.clear();
                    self.selected_id = None;
                }
            }
        }

        // Drag-to-move or resize selected node
        if response.drag_started_by(egui::PointerButton::Primary) {
            // Use press_origin (where the user first clicked) rather than
            // interact_pointer_pos (current position after drag threshold), so that
            // clicks near bounding-box edges still register as "on the selected node".
            if let Some(pos) = ui.input(|i| i.pointer.press_origin()) {
                let (cx, cy) = view.screen_to_canvas(pos.x as f64, pos.y as f64);
                let shift = ui.input(|i| i.modifiers.shift);

                // Compute effective selection bounds: combined bbox for multi, single for one.
                let sel_ids: Vec<NodeId> = doc.selection.ids().copied().collect();
                let effective_bounds = if sel_ids.len() > 1 {
                    selection_canvas_bounds(doc, &sel_ids, renderer)
                } else {
                    self.selected_id
                        .and_then(|id| doc.nodes.get(&id))
                        .and_then(|n| text_aware_canvas_bounds(n, renderer))
                };

                // Check if click lands on a corner resize handle.
                const HANDLE_HIT: f32 = 6.0;
                let resize_hit = effective_bounds.and_then(|(bx0, by0, bx1, by1)| {
                    let (sx0, sy0) = view.canvas_to_screen(bx0, by0);
                    let (sx1, sy1) = view.canvas_to_screen(bx1, by1);
                    let p = pos;
                    let corners = [
                        (egui::pos2(sx0 as f32, sy0 as f32), ResizeHandle::TopLeft),
                        (egui::pos2(sx1 as f32, sy0 as f32), ResizeHandle::TopRight),
                        (egui::pos2(sx0 as f32, sy1 as f32), ResizeHandle::BottomLeft),
                        (
                            egui::pos2(sx1 as f32, sy1 as f32),
                            ResizeHandle::BottomRight,
                        ),
                    ];
                    corners
                        .iter()
                        .find(|(c, _)| (p - *c).length() <= HANDLE_HIT)
                        .map(|(_, h)| *h)
                });

                if let Some(handle) = resize_hit {
                    self.resizing = Some(handle);
                    self.resize_origin_bounds = effective_bounds;
                    if sel_ids.len() > 1 {
                        // Multi-node resize: capture every selected node's transform
                        self.resize_multi_origins = sel_ids
                            .iter()
                            .filter_map(|&id| doc.nodes.get(&id).map(|n| (id, n.transform.matrix)))
                            .collect();
                        self.resize_origin_transform = None;
                        self.resize_origin_font_size = None;
                    } else {
                        // Single-node resize: existing behaviour (text gets font_size scaling)
                        self.resize_multi_origins.clear();
                        self.resize_origin_transform = self
                            .selected_id
                            .and_then(|id| doc.nodes.get(&id))
                            .map(|n| n.transform.matrix);
                        self.resize_origin_font_size = self
                            .selected_id
                            .and_then(|id| doc.nodes.get(&id))
                            .and_then(|n| {
                                if let SceneNodeKind::Text(t) = &n.kind {
                                    Some(t.font_size)
                                } else {
                                    None
                                }
                            });
                    }
                } else {
                    // Check if click is within the effective selection bounds (body).
                    let on_selected = match effective_bounds {
                        Some((x0, y0, x1, y1)) => cx >= x0 && cx <= x1 && cy >= y0 && cy <= y1,
                        None => self.selected_id.is_some(),
                    };

                    if on_selected && !shift {
                        self.moving = true;
                    } else {
                        // Try selecting a new node at the click point
                        let hit = {
                            let raw = hit_test(doc, cx, cy, renderer);
                            // In isolation mode, only accept hits that are children of the isolated group.
                            if let Some(iso_id) = self.isolated_group {
                                raw.filter(|id| {
                                    doc.nodes
                                        .get(&iso_id)
                                        .and_then(|n| {
                                            if let SceneNodeKind::Group(g) = &n.kind {
                                                Some(&g.children)
                                            } else {
                                                None
                                            }
                                        })
                                        .map(|children| children.contains(id))
                                        .unwrap_or(false)
                                })
                            } else {
                                raw
                            }
                        };
                        if shift {
                            if let Some(id) = hit {
                                doc.selection.toggle(id);
                                self.selected_id = Some(id);
                            } else {
                                // Shift+drag on empty space → additive marquee
                                self.marquee_start = Some(pos);
                            }
                        } else {
                            let alt = ui.input(|i| i.modifiers.alt);
                            // Alt+click: if the hit node is a group, select the
                            // topmost child of that group instead (Group Selection behavior).
                            let effective_hit = if alt {
                                hit.and_then(|id| {
                                    if let Some(SceneNodeKind::Group(g)) =
                                        doc.nodes.get(&id).map(|n| &n.kind)
                                    {
                                        // Return topmost (last) child that exists in the document.
                                        g.children
                                            .iter()
                                            .rev()
                                            .find(|cid| doc.nodes.contains_key(*cid))
                                            .copied()
                                    } else {
                                        Some(id)
                                    }
                                })
                            } else {
                                hit
                            };
                            self.selected_id = effective_hit;
                            self.moving = effective_hit.is_some() && !alt;
                            match self.selected_id {
                                Some(id) => doc.selection = Selection::single(id),
                                None => {
                                    doc.selection.clear();
                                    // Drag on empty space → begin marquee selection
                                    self.marquee_start = Some(pos);
                                }
                            }
                        }
                    }
                }
            }
        }

        if response.dragged_by(egui::PointerButton::Primary) {
            if self.resizing.is_some() {
                if let (Some(handle), Some((bx0, by0, bx1, by1))) =
                    (self.resizing, self.resize_origin_bounds)
                {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let (px, py) = view.screen_to_canvas(pos.x as f64, pos.y as f64);
                        let orig_w = bx1 - bx0;
                        let orig_h = by1 - by0;
                        if orig_w.abs() > 1e-9 && orig_h.abs() > 1e-9 {
                            let (anchor_x, anchor_y, sx, sy) = match handle {
                                ResizeHandle::TopLeft => {
                                    (bx1, by1, (bx1 - px) / orig_w, (by1 - py) / orig_h)
                                }
                                ResizeHandle::TopRight => {
                                    (bx0, by1, (px - bx0) / orig_w, (by1 - py) / orig_h)
                                }
                                ResizeHandle::BottomLeft => {
                                    (bx1, by0, (bx1 - px) / orig_w, (py - by0) / orig_h)
                                }
                                ResizeHandle::BottomRight => {
                                    (bx0, by0, (px - bx0) / orig_w, (py - by0) / orig_h)
                                }
                            };

                            if !self.resize_multi_origins.is_empty() {
                                // Multi-node resize: apply the same scale to every node
                                use photonic_core::transform::Transform;
                                let t_scale = Transform::scale_around(sx, sy, anchor_x, anchor_y);
                                let origins = self.resize_multi_origins.clone();
                                for (id, orig_xf) in origins {
                                    if let Some(node) = doc.nodes.get_mut(&id) {
                                        node.transform =
                                            Transform { matrix: orig_xf }.then(&t_scale);
                                    }
                                }
                                *doc_modified = true;
                            } else if let (Some(orig_xf), Some(sel_id)) =
                                (self.resize_origin_transform, self.selected_id)
                            {
                                // Single-node resize (with text font_size special case)
                                if let Some(node) = doc.nodes.get_mut(&sel_id) {
                                    if let SceneNodeKind::Text(text) = &mut node.kind {
                                        if let Some(orig_fs) = self.resize_origin_font_size {
                                            let scale = sy.abs().max(0.01);
                                            text.font_size = (orig_fs * scale).max(1.0);
                                            let new_w = (bx1 - bx0) * scale;
                                            let new_h = (by1 - by0) * scale;
                                            let (tx, ty) = match handle {
                                                ResizeHandle::BottomRight => (bx0, by0),
                                                ResizeHandle::TopLeft => (bx1 - new_w, by1 - new_h),
                                                ResizeHandle::TopRight => (bx0, by1 - new_h),
                                                ResizeHandle::BottomLeft => (bx1 - new_w, by0),
                                            };
                                            node.transform.matrix = [1.0, 0.0, 0.0, 1.0, tx, ty];
                                        }
                                    } else {
                                        use photonic_core::transform::Transform;
                                        let t_orig = Transform { matrix: orig_xf };
                                        let t_scale =
                                            Transform::scale_around(sx, sy, anchor_x, anchor_y);
                                        node.transform = t_orig.then(&t_scale);
                                    }
                                    *doc_modified = true;
                                }
                            }
                        }
                    }
                }
            } else if self.moving {
                let delta = response.drag_delta();
                let dx = delta.x as f64 / view.zoom;
                let dy = delta.y as f64 / view.zoom;
                let ids_to_move: Vec<NodeId> = doc.selection.ids().copied().collect();
                for id in ids_to_move {
                    if let Some(node) = doc.nodes.get_mut(&id) {
                        node.transform.matrix[4] += dx;
                        node.transform.matrix[5] += dy;
                        *doc_modified = true;
                    }
                }
            }
        }

        if response.drag_stopped_by(egui::PointerButton::Primary) {
            self.moving = false;
            self.resizing = None;
            self.resize_origin_bounds = None;
            self.resize_origin_transform = None;
            self.resize_origin_font_size = None;
            self.resize_multi_origins.clear();

            // Complete marquee selection if one was in progress
            if let Some(start_pos) = self.marquee_start.take() {
                let end_pos = response
                    .interact_pointer_pos()
                    .or_else(|| ui.input(|i| i.pointer.hover_pos()))
                    .unwrap_or(start_pos);
                let shift = ui.input(|i| i.modifiers.shift);
                let (cx0, cy0) = view.screen_to_canvas(start_pos.x as f64, start_pos.y as f64);
                let (cx1, cy1) = view.screen_to_canvas(end_pos.x as f64, end_pos.y as f64);
                let mx0 = cx0.min(cx1);
                let my0 = cy0.min(cy1);
                let mx1 = cx0.max(cx1);
                let my1 = cy0.max(cy1);

                // Collect nodes whose bounds intersect the marquee rect
                let to_select: Vec<NodeId> = {
                    let nodes = doc.nodes_in_draw_order();
                    let mut ids = Vec::new();
                    for node in nodes {
                        if let Some((nx0, ny0, nx1, ny1)) = text_aware_canvas_bounds(node, renderer)
                        {
                            if nx1 >= mx0 && nx0 <= mx1 && ny1 >= my0 && ny0 <= my1 {
                                ids.push(node.id);
                            }
                        }
                    }
                    ids
                };

                if !shift {
                    doc.selection.clear();
                    self.selected_id = None;
                }
                for id in to_select {
                    doc.selection.add(id);
                    self.selected_id = Some(id);
                }
            }
        }

        // Click on empty space to deselect (without shift)
        if response.clicked_by(egui::PointerButton::Primary) && !self.moving {
            if let Some(pos) = response.interact_pointer_pos() {
                let (cx, cy) = view.screen_to_canvas(pos.x as f64, pos.y as f64);
                let shift = ui.input(|i| i.modifiers.shift);
                let hit = hit_test(doc, cx, cy, renderer);
                if shift {
                    if let Some(id) = hit {
                        doc.selection.toggle(id);
                        self.selected_id = Some(id);
                    }
                } else {
                    self.selected_id = hit;
                    match self.selected_id {
                        Some(id) => doc.selection = Selection::single(id),
                        None => doc.selection.clear(),
                    }
                }
            }
        }

        // ── Selection overlay ────────────────────────────────────────────────
        let accent = Color32::from_rgb(110, 86, 207);
        let thick_stroke = egui::Stroke::new(1.5, accent);
        let sel_ids: Vec<NodeId> = doc.selection.ids().copied().collect();

        if sel_ids.len() > 1 {
            // Multi-select: one unified bounding box with resize handles over the
            // union of all selected nodes (no per-node boxes — they act as a unit).
            if let Some((cx0, cy0, cx1, cy1)) = selection_canvas_bounds(doc, &sel_ids, renderer) {
                let (sx0, sy0) = view.canvas_to_screen(cx0, cy0);
                let (sx1, sy1) = view.canvas_to_screen(cx1, cy1);
                let sel_rect = egui::Rect::from_min_max(
                    egui::pos2(sx0 as f32, sy0 as f32),
                    egui::pos2(sx1 as f32, sy1 as f32),
                );
                ui.painter().rect_stroke(sel_rect, 0.0, thick_stroke);
                for corner in [
                    sel_rect.left_top(),
                    sel_rect.right_top(),
                    sel_rect.left_bottom(),
                    sel_rect.right_bottom(),
                ] {
                    let handle = egui::Rect::from_center_size(corner, egui::Vec2::splat(7.0));
                    ui.painter().rect_filled(handle, 0.0, Color32::WHITE);
                    ui.painter().rect_stroke(handle, 0.0, thick_stroke);
                }
            }
        } else if let Some(sel_id) = self.selected_id {
            // Single-select: outline + resize handles on that node
            if let Some(node) = doc.nodes.get(&sel_id) {
                if let Some((cx0, cy0, cx1, cy1)) = text_aware_canvas_bounds(node, renderer) {
                    let (sx0, sy0) = view.canvas_to_screen(cx0, cy0);
                    let (sx1, sy1) = view.canvas_to_screen(cx1, cy1);
                    let sel_rect = egui::Rect::from_min_max(
                        egui::pos2(sx0 as f32, sy0 as f32),
                        egui::pos2(sx1 as f32, sy1 as f32),
                    );
                    ui.painter().rect_stroke(sel_rect, 0.0, thick_stroke);
                    for corner in [
                        sel_rect.left_top(),
                        sel_rect.right_top(),
                        sel_rect.left_bottom(),
                        sel_rect.right_bottom(),
                    ] {
                        let handle = egui::Rect::from_center_size(corner, egui::Vec2::splat(7.0));
                        ui.painter().rect_filled(handle, 0.0, Color32::WHITE);
                        ui.painter().rect_stroke(handle, 0.0, thick_stroke);
                    }
                }
            }
        }

        // ── Marquee selection overlay ────────────────────────────────────────
        if let Some(start_pos) = self.marquee_start {
            let current_pos = ui.input(|i| i.pointer.hover_pos()).unwrap_or(start_pos);
            let rect = egui::Rect::from_two_pos(start_pos, current_pos);
            let accent = Color32::from_rgb(110, 86, 207);
            ui.painter().rect(
                rect,
                0.0,
                Color32::from_rgba_unmultiplied(110, 86, 207, 30),
                egui::Stroke::new(1.0, accent),
            );
        }

        // ── Cursor icon ──────────────────────────────────────────────────────
        let cursor = if let Some(handle) = self.resizing {
            // Mid-drag: hold the resize cursor
            match handle {
                ResizeHandle::TopLeft | ResizeHandle::BottomRight => egui::CursorIcon::ResizeNwSe,
                ResizeHandle::TopRight | ResizeHandle::BottomLeft => egui::CursorIcon::ResizeNeSw,
            }
        } else if self.moving {
            egui::CursorIcon::Move
        } else if let Some(hover_pos) = ui.input(|i| i.pointer.hover_pos()) {
            // Use effective (combined) bounds for cursor feedback
            const HANDLE_HIT: f32 = 6.0;
            let hover_sel_ids: Vec<NodeId> = doc.selection.ids().copied().collect();
            let hover_bounds = if hover_sel_ids.len() > 1 {
                selection_canvas_bounds(doc, &hover_sel_ids, renderer)
            } else {
                self.selected_id
                    .and_then(|id| doc.nodes.get(&id))
                    .and_then(|n| text_aware_canvas_bounds(n, renderer))
            };

            let corner_hit = hover_bounds.and_then(|(bx0, by0, bx1, by1)| {
                let (sx0, sy0) = view.canvas_to_screen(bx0, by0);
                let (sx1, sy1) = view.canvas_to_screen(bx1, by1);
                let corners = [
                    (egui::pos2(sx0 as f32, sy0 as f32), ResizeHandle::TopLeft),
                    (egui::pos2(sx1 as f32, sy0 as f32), ResizeHandle::TopRight),
                    (egui::pos2(sx0 as f32, sy1 as f32), ResizeHandle::BottomLeft),
                    (
                        egui::pos2(sx1 as f32, sy1 as f32),
                        ResizeHandle::BottomRight,
                    ),
                ];
                corners
                    .iter()
                    .find(|(c, _)| (hover_pos - *c).length() <= HANDLE_HIT)
                    .map(|(_, h)| *h)
            });

            if let Some(handle) = corner_hit {
                match handle {
                    ResizeHandle::TopLeft | ResizeHandle::BottomRight => {
                        egui::CursorIcon::ResizeNwSe
                    }
                    ResizeHandle::TopRight | ResizeHandle::BottomLeft => {
                        egui::CursorIcon::ResizeNeSw
                    }
                }
            } else {
                let on_body = hover_bounds
                    .map(|(bx0, by0, bx1, by1)| {
                        let (sx0, sy0) = view.canvas_to_screen(bx0, by0);
                        let (sx1, sy1) = view.canvas_to_screen(bx1, by1);
                        egui::Rect::from_min_max(
                            egui::pos2(sx0 as f32, sy0 as f32),
                            egui::pos2(sx1 as f32, sy1 as f32),
                        )
                        .contains(hover_pos)
                    })
                    .unwrap_or(false);
                if on_body {
                    egui::CursorIcon::Move
                } else {
                    egui::CursorIcon::Default
                }
            }
        } else {
            egui::CursorIcon::Default
        };
        ui.ctx().set_cursor_icon(cursor);
    }

    // ── Pen tool handler ──────────────────────────────────────────────────────

    fn handle_pen_tool(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        doc: &mut Document,
        view: &CanvasView,
        doc_modified: &mut bool,
    ) {
        // Escape cancels the in-progress path
        if viewport_kb(ui.ctx()) && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.pen_points.clear();
            return;
        }

        // Double-click finalises the path (also fires clicked, so handle first)
        if response.double_clicked_by(egui::PointerButton::Primary) {
            if self.pen_points.len() >= 2 {
                if let Some(path) = self.build_pen_path() {
                    let stroke_arg = self.prefs.default_stroke_enabled.then(|| {
                        (
                            self.prefs.default_stroke_color,
                            self.prefs.default_stroke_width,
                        )
                    });
                    let node = make_node(
                        path,
                        self.fill_color,
                        stroke_arg,
                        "Pen",
                        doc.node_count() + 1,
                    );
                    doc.add_node(node, None);
                    *doc_modified = true;
                }
            }
            self.pen_points.clear();
            return;
        }

        // Single click: add an anchor point
        if response.clicked_by(egui::PointerButton::Primary) {
            if !ui.input(|i| i.modifiers.alt) {
                if let Some(pos) = response.interact_pointer_pos() {
                    let (cx, cy) = view.screen_to_canvas(pos.x as f64, pos.y as f64);
                    self.pen_points.push((cx, cy));
                }
            }
        }

        // ── Preview ──────────────────────────────────────────────────────────
        let painter = ui.painter();
        let path_stroke = egui::Stroke::new(1.5, Color32::from_rgb(110, 86, 207));
        let rubber_stroke =
            egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(110, 86, 207, 128));

        // Lines between placed points
        for i in 0..self.pen_points.len().saturating_sub(1) {
            let (x0, y0) = self.pen_points[i];
            let (x1, y1) = self.pen_points[i + 1];
            let (sx0, sy0) = view.canvas_to_screen(x0, y0);
            let (sx1, sy1) = view.canvas_to_screen(x1, y1);
            painter.line_segment(
                [
                    egui::pos2(sx0 as f32, sy0 as f32),
                    egui::pos2(sx1 as f32, sy1 as f32),
                ],
                path_stroke,
            );
        }

        // Anchor dots
        for &(cx, cy) in &self.pen_points {
            let (sx, sy) = view.canvas_to_screen(cx, cy);
            let center = egui::pos2(sx as f32, sy as f32);
            painter.rect_filled(
                egui::Rect::from_center_size(center, egui::Vec2::splat(6.0)),
                0.0,
                Color32::WHITE,
            );
            painter.rect_stroke(
                egui::Rect::from_center_size(center, egui::Vec2::splat(6.0)),
                0.0,
                path_stroke,
            );
        }

        // Rubber-band line from last point to cursor
        if let Some(&(lx, ly)) = self.pen_points.last() {
            if let Some(cursor) = ui.input(|i| i.pointer.hover_pos()) {
                let (sx, sy) = view.canvas_to_screen(lx, ly);
                painter.line_segment([egui::pos2(sx as f32, sy as f32), cursor], rubber_stroke);
            }
        }
    }

    /// Build a `PathData` polyline from the accumulated pen points.
    fn build_pen_path(&self) -> Option<PathData> {
        if self.pen_points.len() < 2 {
            return None;
        }
        let mut bez = BezPath::new();
        let (x0, y0) = self.pen_points[0];
        bez.move_to((x0, y0));
        for &(x, y) in &self.pen_points[1..] {
            bez.line_to((x, y));
        }
        Some(PathData::from_bez_path(&bez))
    }

    // ── Direct Selection tool handler ─────────────────────────────────────────

    fn handle_direct_select_tool(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        doc: &mut Document,
        view: &CanvasView,
        renderer: &mut PhotonicRenderer,
        doc_modified: &mut bool,
        history: &mut CommandHistory,
    ) {
        // Generous hit radius — makes anchors easy to grab
        const ANCHOR_RADIUS_PX: f64 = 12.0;

        // Escape: exit point-edit mode
        if viewport_kb(ui.ctx()) && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.point_edit_node = None;
            self.point_selected.clear();
            self.point_drag_origin = None;
            return;
        }

        ui.ctx().set_cursor_icon(egui::CursorIcon::Default);

        let ctrl = ui.input(|i| i.modifiers.ctrl);

        // hover_pos is used ONLY for the visual highlight — NOT for hit-testing on
        // click/drag events.  All interaction positions come from interact_pointer_pos()
        // so that the test point is at the press location, not the current cursor position
        // (by the time drag_started fires the cursor may have moved off the anchor).
        let hover_canvas = ui
            .input(|i| i.pointer.hover_pos())
            .map(|p| view.screen_to_canvas(p.x as f64, p.y as f64));

        // Helper closure: hit-test anchors of the current edit node at canvas pos (cx, cy)
        let find_anchor = |nid: NodeId, cx: f64, cy: f64, doc: &Document| -> Option<usize> {
            doc.nodes.get(&nid).and_then(|node| {
                if let SceneNodeKind::Path(pn) = &node.kind {
                    let bez = pn.path_data.to_bez_path();
                    nearest_anchor_screen(&bez, &node.transform, view, cx, cy, ANCHOR_RADIUS_PX)
                } else {
                    None
                }
            })
        };

        // ── Delete selected anchor points ─────────────────────────────────────
        let delete =
            ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace));
        if delete && !self.point_selected.is_empty() && viewport_kb(ui.ctx()) {
            if let Some(nid) = self.point_edit_node {
                if let Some(node) = doc.nodes.get(&nid) {
                    let old_node = node.clone();
                    if let SceneNodeKind::Path(pn) = &node.kind {
                        let bez = pn.path_data.to_bez_path();
                        let new_bez = bez_remove_elements(&bez, &self.point_selected);
                        let mut new_node = old_node.clone();
                        if let SceneNodeKind::Path(new_pn) = &mut new_node.kind {
                            new_pn.path_data = PathData::from_bez_path(&new_bez);
                        }
                        history.execute(
                            Command::UpdateNode {
                                old: old_node,
                                new: new_node,
                            },
                            doc,
                        );
                        self.point_selected.clear();
                        *doc_modified = true;
                    }
                }
            }
            return;
        }

        // ── Drag start: use interact_pointer_pos() — the press location ───────
        if response.drag_started_by(egui::PointerButton::Primary) {
            if let Some(press_pos) = response.interact_pointer_pos() {
                let (cx, cy) = view.screen_to_canvas(press_pos.x as f64, press_pos.y as f64);

                let hit_anchor = self
                    .point_edit_node
                    .and_then(|nid| find_anchor(nid, cx, cy, doc));

                if let Some(anchor_idx) = hit_anchor {
                    // Select this anchor (replace unless Ctrl is held)
                    if ctrl {
                        if !self.point_selected.contains(&anchor_idx) {
                            self.point_selected.push(anchor_idx);
                        }
                    } else if !self.point_selected.contains(&anchor_idx) {
                        self.point_selected = vec![anchor_idx];
                    }
                    // Snapshot the current node for undo
                    self.point_drag_origin = self
                        .point_edit_node
                        .and_then(|nid| doc.nodes.get(&nid).cloned());
                } else {
                    // Missed all anchors — try switching to a different shape
                    let hit_shape = hit_test(doc, cx, cy, renderer);
                    self.point_edit_node = hit_shape;
                    self.point_selected.clear();
                    self.point_drag_origin = None;
                }
            }
        }

        // ── During drag: move selected anchors by the per-frame delta ─────────
        if response.dragged_by(egui::PointerButton::Primary)
            && self.point_drag_origin.is_some()
            && !self.point_selected.is_empty()
        {
            if let Some(nid) = self.point_edit_node {
                let delta = response.drag_delta();
                let dcx = delta.x as f64 / view.zoom;
                let dcy = delta.y as f64 / view.zoom;
                if let Some(node) = doc.nodes.get_mut(&nid) {
                    // Invert the node's linear transform to get a local-space delta
                    let [a, b, c, d, _, _] = node.transform.matrix;
                    let det = a * d - b * c;
                    let (dlx, dly) = if det.abs() > 1e-10 {
                        ((d * dcx - c * dcy) / det, (-b * dcx + a * dcy) / det)
                    } else {
                        (dcx, dcy)
                    };
                    if let SceneNodeKind::Path(pn) = &mut node.kind {
                        let bez = pn.path_data.to_bez_path();
                        let new_bez = bez_move_anchors(&bez, &self.point_selected, dlx, dly);
                        pn.path_data = PathData::from_bez_path(&new_bez);
                        *doc_modified = true;
                    }
                }
            }
        }

        // ── Drag end: push undo command ───────────────────────────────────────
        if response.drag_stopped_by(egui::PointerButton::Primary) {
            if let Some(old_node) = self.point_drag_origin.take() {
                if let Some(nid) = self.point_edit_node {
                    if let Some(new_node) = doc.nodes.get(&nid).cloned() {
                        let changed = match (&old_node.kind, &new_node.kind) {
                            (SceneNodeKind::Path(op), SceneNodeKind::Path(np)) => {
                                op.path_data != np.path_data
                            }
                            _ => false,
                        };
                        if changed {
                            history.execute(
                                Command::UpdateNode {
                                    old: old_node,
                                    new: new_node,
                                },
                                doc,
                            );
                        }
                    }
                }
            }
        }

        // ── Click (no drag): select anchor or pick shape ──────────────────────
        // Use interact_pointer_pos() here too — same reasoning as drag_started.
        if response.clicked_by(egui::PointerButton::Primary) {
            if let Some(click_pos) = response.interact_pointer_pos() {
                let (cx, cy) = view.screen_to_canvas(click_pos.x as f64, click_pos.y as f64);

                let hit_anchor = self
                    .point_edit_node
                    .and_then(|nid| find_anchor(nid, cx, cy, doc));

                if let Some(anchor_idx) = hit_anchor {
                    if ctrl {
                        // Toggle
                        if let Some(pos) = self.point_selected.iter().position(|&i| i == anchor_idx)
                        {
                            self.point_selected.remove(pos);
                        } else {
                            self.point_selected.push(anchor_idx);
                        }
                    } else {
                        self.point_selected = vec![anchor_idx];
                    }
                } else {
                    let hit_shape = hit_test(doc, cx, cy, renderer);
                    if let Some(nid) = hit_shape {
                        if Some(nid) != self.point_edit_node {
                            self.point_edit_node = Some(nid);
                            self.point_selected.clear();
                        } else if !ctrl {
                            self.point_selected.clear();
                        }
                    } else {
                        self.point_edit_node = None;
                        self.point_selected.clear();
                    }
                }
            }
        }

        // ── Visual overlay ────────────────────────────────────────────────────
        if let Some(nid) = self.point_edit_node {
            if let Some(node) = doc.nodes.get(&nid) {
                if let SceneNodeKind::Path(pn) = &node.kind {
                    let bez = pn.path_data.to_bez_path();
                    let painter = ui.painter();

                    // Path outline (blue, no fill)
                    let outline_pts = bez_to_screen_points_xf(&bez, view, &node.transform);
                    if outline_pts.len() >= 2 {
                        painter.add(egui::Shape::Path(egui::epaint::PathShape {
                            points: outline_pts,
                            closed: true,
                            fill: Color32::TRANSPARENT,
                            stroke: egui::epaint::PathStroke::new(
                                1.5,
                                Color32::from_rgb(110, 86, 207),
                            ),
                        }));
                    }

                    // Which anchor is nearest the hover cursor (for grab highlight)
                    let hovered_anchor = hover_canvas.and_then(|(hx, hy)| {
                        nearest_anchor_screen(&bez, &node.transform, view, hx, hy, ANCHOR_RADIUS_PX)
                    });

                    // Anchor point squares
                    for (idx, local_pt) in path_anchor_points(&bez) {
                        let (cx, cy) = node.transform.apply(local_pt.x, local_pt.y);
                        let (sx, sy) = view.canvas_to_screen(cx, cy);
                        let center = egui::pos2(sx as f32, sy as f32);
                        let half = 4.5f32;
                        let rect =
                            egui::Rect::from_center_size(center, egui::Vec2::splat(half * 2.0));
                        let selected = self.point_selected.contains(&idx);
                        let hovered = hovered_anchor == Some(idx);
                        let accent = Color32::from_rgb(110, 86, 207);
                        if selected {
                            painter.rect_filled(rect, 0.0, accent);
                        } else if hovered {
                            let big = egui::Rect::from_center_size(
                                center,
                                egui::Vec2::splat((half + 2.0) * 2.0),
                            );
                            painter.rect_filled(
                                big,
                                0.0,
                                Color32::from_rgba_unmultiplied(110, 86, 207, 60),
                            );
                            painter.rect_stroke(big, 0.0, egui::Stroke::new(1.5, accent));
                        } else {
                            painter.rect_filled(rect, 0.0, Color32::WHITE);
                            painter.rect_stroke(rect, 0.0, egui::Stroke::new(1.5, accent));
                        }
                    }
                }
            }
        }
    }

    // ── Shape Builder tool handler ────────────────────────────────────────────

    fn handle_shape_builder_tool(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        doc: &mut Document,
        view: &CanvasView,
        renderer: &mut PhotonicRenderer,
        doc_modified: &mut bool,
        history: &mut CommandHistory,
    ) {
        let alt_held = ui.input(|i| i.modifiers.alt);

        // Cursor: minus = subtract, crosshair = union
        ui.ctx().set_cursor_icon(if alt_held {
            egui::CursorIcon::NoDrop
        } else {
            egui::CursorIcon::Crosshair
        });

        // Canvas position under pointer
        let canvas_pos = ui
            .input(|i| i.pointer.hover_pos())
            .map(|p| view.screen_to_canvas(p.x as f64, p.y as f64));

        // Update hovered node
        self.shape_builder_hovered =
            canvas_pos.and_then(|(cx, cy)| hit_test(doc, cx, cy, renderer));

        // Drag start: record mode, reset collected set
        if response.drag_started_by(egui::PointerButton::Primary) {
            self.shape_builder_subtract_mode = alt_held;
            self.shape_builder_drag_ids.clear();
            // Add the initial shape under the cursor
            if let Some(id) = self.shape_builder_hovered {
                self.shape_builder_drag_ids.push(id);
            }
        }

        // During drag: accumulate every new shape the cursor enters
        if response.dragged_by(egui::PointerButton::Primary) {
            let pos = response
                .interact_pointer_pos()
                .map(|p| view.screen_to_canvas(p.x as f64, p.y as f64))
                .or(canvas_pos);
            if let Some((cx, cy)) = pos {
                if let Some(id) = hit_test(doc, cx, cy, renderer) {
                    if !self.shape_builder_drag_ids.contains(&id) {
                        self.shape_builder_drag_ids.push(id);
                    }
                }
            }
        }

        // Drag end: perform the boolean operation
        if response.drag_stopped_by(egui::PointerButton::Primary) {
            let ids = std::mem::take(&mut self.shape_builder_drag_ids);
            let subtract = self.shape_builder_subtract_mode;
            if !ids.is_empty() {
                self.execute_shape_builder(doc, history, &ids, subtract, doc_modified);
            }
        }

        // ── Visual feedback ───────────────────────────────────────────────────
        let painter = ui.painter();

        // Highlight shapes being collected in current drag
        for &id in &self.shape_builder_drag_ids {
            if let Some(node) = doc.nodes.get(&id) {
                if let SceneNodeKind::Path(pn) = &node.kind {
                    let baked = gui_apply_affine_to_path(&pn.path_data, node.transform.to_kurbo());
                    let pts = bez_to_screen_points(&baked.to_bez_path(), view);
                    if pts.len() >= 2 {
                        let fill = if self.shape_builder_subtract_mode {
                            Color32::from_rgba_unmultiplied(248, 113, 113, 100)
                        } else {
                            Color32::from_rgba_unmultiplied(52, 211, 153, 100)
                        };
                        painter.add(egui::Shape::Path(egui::epaint::PathShape {
                            points: pts,
                            closed: true,
                            fill,
                            stroke: egui::epaint::PathStroke::new(0.0, Color32::TRANSPARENT),
                        }));
                    }
                }
            }
        }

        // Highlight the hovered shape (if not already in drag set)
        if let Some(hovered_id) = self.shape_builder_hovered {
            if !self.shape_builder_drag_ids.contains(&hovered_id) {
                if let Some(node) = doc.nodes.get(&hovered_id) {
                    if let SceneNodeKind::Path(pn) = &node.kind {
                        let baked =
                            gui_apply_affine_to_path(&pn.path_data, node.transform.to_kurbo());
                        let pts = bez_to_screen_points(&baked.to_bez_path(), view);
                        if pts.len() >= 2 {
                            let (fill_color, stroke_color) = if alt_held {
                                (
                                    Color32::from_rgba_unmultiplied(248, 113, 113, 60),
                                    Color32::from_rgb(248, 113, 113),
                                )
                            } else {
                                (
                                    Color32::from_rgba_unmultiplied(52, 211, 153, 60),
                                    Color32::from_rgb(52, 211, 153),
                                )
                            };
                            painter.add(egui::Shape::Path(egui::epaint::PathShape {
                                points: pts,
                                closed: true,
                                fill: fill_color,
                                stroke: egui::epaint::PathStroke::new(2.0, stroke_color),
                            }));
                        }
                    }
                }
            }
        }
    }

    /// Execute a Shape Builder operation on `ids`.
    ///
    /// - Union mode (`subtract = false`): union all touched shapes into one.
    /// - Subtract mode (`subtract = true`, Alt held): subtract all touched shapes
    ///   (after the first) from the first one; if only one shape is touched, delete it.
    fn execute_shape_builder(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
        ids: &[NodeId],
        subtract: bool,
        doc_modified: &mut bool,
    ) {
        use photonic_core::ops::boolean::{boolean_op, BooleanOp};

        // Gather (id, layer_id, z-index) for each touched node
        let mut indexed: Vec<(NodeId, photonic_core::layer::LayerId, usize)> = ids
            .iter()
            .filter_map(|&id| doc.node_layer_and_index(&id).map(|(l, i)| (id, l, i)))
            .collect();

        if indexed.is_empty() {
            return;
        }

        // All must be in the same layer
        let layer_id = indexed[0].1;
        if indexed.iter().any(|(_, l, _)| *l != layer_id) {
            return;
        }

        // Sort by ascending z-order
        indexed.sort_by_key(|(_, _, idx)| *idx);

        if subtract && indexed.len() == 1 {
            // Delete single alt-clicked shape
            let node_id = indexed[0].0;
            history.execute(photonic_core::history::Command::RemoveNode { node_id }, doc);
            self.shape_builder_hovered = None;
            *doc_modified = true;
            return;
        }

        if !subtract && indexed.len() < 2 {
            // Nothing to union
            return;
        }

        // Bake transforms for all shapes
        let baked_paths: Vec<_> = indexed
            .iter()
            .filter_map(|(id, _, _)| {
                let n = doc.get_node(id)?;
                if let SceneNodeKind::Path(pn) = &n.kind {
                    Some((
                        *id,
                        gui_apply_affine_to_path(&pn.path_data, n.transform.to_kurbo()),
                    ))
                } else {
                    None
                }
            })
            .collect();

        if baked_paths.is_empty() {
            return;
        }

        // Get style from the bottom-most shape (first in z-order)
        let (fill, stroke) = doc
            .get_node(&indexed[0].0)
            .and_then(|n| {
                if let SceneNodeKind::Path(pn) = &n.kind {
                    Some((pn.fill.clone(), pn.stroke.clone()))
                } else {
                    None
                }
            })
            .unwrap_or_default();

        // Compute result path
        let op = if subtract {
            BooleanOp::Subtract
        } else {
            BooleanOp::Union
        };
        let mut result_path = baked_paths[0].1.clone();
        for (_, path) in &baked_paths[1..] {
            match boolean_op(&result_path, path, op) {
                Ok(p) => result_path = p,
                Err(_) => return,
            }
        }

        // Build result node inheriting the first shape's style
        let mut result_pn = photonic_core::node::PathNode::new(result_path);
        result_pn.fill = fill;
        result_pn.stroke = stroke;
        let result_node = SceneNode::new("Shape", layer_id, SceneNodeKind::Path(result_pn));
        let result_id = result_node.id;

        // Place the result at the z-position of the lowest input shape
        let insert_z = indexed[0].2;
        let layer_len = doc
            .layers
            .get(&layer_id)
            .map(|l| l.node_ids.len())
            .unwrap_or(0);
        let result_pos = layer_len.saturating_sub(indexed.len()); // position after removes + add
        let new_index = insert_z.min(result_pos);

        let mut cmds: Vec<photonic_core::history::Command> = indexed
            .iter()
            .map(|(id, _, _)| photonic_core::history::Command::RemoveNode { node_id: *id })
            .collect();
        cmds.push(photonic_core::history::Command::AddNode {
            node: result_node,
            layer_id: Some(layer_id),
        });
        if new_index != result_pos {
            cmds.push(photonic_core::history::Command::ReorderNode {
                layer_id,
                node_id: result_id,
                old_index: result_pos,
                new_index,
            });
        }

        history.execute(photonic_core::history::Command::Batch(cmds), doc);
        self.selected_id = Some(result_id);
        doc.selection = Selection::single(result_id);
        *doc_modified = true;
    }

    // ── Console panel ─────────────────────────────────────────────────────────

    fn draw_console(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui
                .selectable_label(self.lua_console.tab == ConsoleTab::Lua, "Lua")
                .clicked()
            {
                self.lua_console.tab = ConsoleTab::Lua;
            }
            if ui
                .selectable_label(self.lua_console.tab == ConsoleTab::Claude, "Claude")
                .clicked()
            {
                self.lua_console.tab = ConsoleTab::Claude;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("✕").clicked() {
                    self.lua_console.visible = false;
                }
                let expand_icon = if self.lua_console.expanded {
                    "▼"
                } else {
                    "▲"
                };
                if ui
                    .small_button(expand_icon)
                    .on_hover_text(if self.lua_console.expanded {
                        "Collapse"
                    } else {
                        "Expand"
                    })
                    .clicked()
                {
                    self.lua_console.expanded = !self.lua_console.expanded;
                }
                if ui.small_button("Clear").clicked() {
                    self.lua_console.log.clear();
                }
                if self.lua_console.tab == ConsoleTab::Claude {
                    if ui
                        .small_button("Copy")
                        .on_hover_text("Copy conversation to clipboard")
                        .clicked()
                    {
                        let mut text = String::new();
                        for (is_user, msg) in &self.claude_chat.messages {
                            let role = if *is_user { "You" } else { "Claude" };
                            text.push_str(role);
                            text.push_str(": ");
                            text.push_str(msg);
                            text.push_str("\n\n");
                        }
                        ui.output_mut(|o| o.copied_text = text);
                    }
                }
            });
        });
        ui.separator();

        match self.lua_console.tab {
            ConsoleTab::Lua => self.draw_lua_tab(ui),
            ConsoleTab::Claude => self.draw_claude_tab(ui),
        }
    }

    fn draw_lua_tab(&mut self, ui: &mut egui::Ui) {
        // Output scroll area
        let available = ui.available_height() - 32.0;
        egui::ScrollArea::vertical()
            .id_salt("console_out")
            .max_height(available.max(40.0))
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for (is_err, line) in &self.lua_console.log {
                    let color = if *is_err {
                        Color32::from_rgb(248, 113, 113)
                    } else {
                        Color32::from_rgb(187, 187, 210)
                    };
                    ui.label(egui::RichText::new(line).monospace().color(color));
                }
            });

        ui.separator();

        // Input row
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(">")
                    .monospace()
                    .color(Color32::from_rgb(144, 119, 224)),
            );
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.lua_console.input)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(ui.available_width() - 50.0)
                    .hint_text("photonic.create_rect(100, 100, 200, 150)"),
            );
            let submitted = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Run").clicked() || submitted {
                if !self.lua_console.input.trim().is_empty() {
                    let code = self.lua_console.input.clone();
                    self.lua_console.log.push((false, format!("> {code}")));
                    self.lua_console.pending = Some(code);
                    self.lua_console.input.clear();
                }
                resp.request_focus();
            }
        });
    }

    // ── Shape factory ─────────────────────────────────────────────────────────

    fn build_shape(&self, sx: f64, sy: f64, ex: f64, ey: f64) -> Option<PathData> {
        let min_x = sx.min(ex);
        let min_y = sy.min(ey);
        let max_x = sx.max(ex);
        let max_y = sy.max(ey);
        let w = max_x - min_x;
        let h = max_y - min_y;
        let cx = (min_x + max_x) / 2.0;
        let cy = (min_y + max_y) / 2.0;
        let radius = ((ex - sx).hypot(ey - sy)) / 2.0;

        let path = match self.active_tool {
            Tool::Rectangle => PathData::rect(min_x, min_y, w, h),
            Tool::Ellipse => PathData::ellipse(cx, cy, w / 2.0, h / 2.0),
            Tool::Polygon => PathData::regular_polygon(cx, cy, radius, self.polygon_sides as usize),
            Tool::Star => PathData::star(
                cx,
                cy,
                radius,
                radius * self.star_inner_ratio as f64,
                self.star_points as usize,
            ),
            Tool::Spiral => PathData::spiral(
                cx,
                cy,
                radius,
                (self.spiral_inner_radius as f64).min(radius),
                self.spiral_turns as f64,
                self.spiral_segs_per_turn as usize,
            ),
            // Line uses the raw drag start/end (not a bounding box).
            Tool::Line => PathData::line(sx, sy, ex, ey),
            Tool::Arc => PathData::arc(
                cx,
                cy,
                w / 2.0,
                h / 2.0,
                self.arc_start_angle,
                self.arc_end_angle,
                !self.arc_open,
            ),
            Tool::Grid => PathData::grid(min_x, min_y, w, h, self.grid_cols, self.grid_rows),
            Tool::PolarGrid => {
                let outer_r = (w.min(h)) / 2.0;
                let inner_r = outer_r * self.polar_grid_inner_ratio as f64;
                PathData::polar_grid(
                    cx,
                    cy,
                    outer_r,
                    inner_r,
                    self.polar_grid_rings,
                    self.polar_grid_sectors,
                )
            }
            _ => return None,
        };

        Some(path)
    }

    /// Like `build_shape` but takes an explicit `Tool` instead of reading `self.active_tool`.
    /// Used by `CreateShapeAtPos` so active tool state is not polluted.
    fn build_shape_with_tool(
        &self,
        tool: Tool,
        sx: f64,
        sy: f64,
        ex: f64,
        ey: f64,
    ) -> Option<PathData> {
        let min_x = sx.min(ex);
        let min_y = sy.min(ey);
        let max_x = sx.max(ex);
        let max_y = sy.max(ey);
        let w = max_x - min_x;
        let h = max_y - min_y;
        let cx = (min_x + max_x) / 2.0;
        let cy = (min_y + max_y) / 2.0;
        let radius = ((ex - sx).hypot(ey - sy)) / 2.0;

        let path = match tool {
            Tool::Rectangle => PathData::rect(min_x, min_y, w, h),
            Tool::RoundedRect => {
                PathData::rounded_rect(min_x, min_y, w, h, self.rounded_rect_radius)
            }
            Tool::Ellipse => PathData::ellipse(cx, cy, w / 2.0, h / 2.0),
            Tool::Polygon => PathData::regular_polygon(cx, cy, radius, self.polygon_sides as usize),
            Tool::Star => PathData::star(
                cx,
                cy,
                radius,
                radius * self.star_inner_ratio as f64,
                self.star_points as usize,
            ),
            Tool::Spiral => PathData::spiral(
                cx,
                cy,
                radius,
                (self.spiral_inner_radius as f64).min(radius),
                self.spiral_turns as f64,
                self.spiral_segs_per_turn as usize,
            ),
            Tool::Line => PathData::line(sx, sy, ex, ey),
            Tool::Arc => PathData::arc(
                cx,
                cy,
                w / 2.0,
                h / 2.0,
                self.arc_start_angle,
                self.arc_end_angle,
                !self.arc_open,
            ),
            Tool::Grid => PathData::grid(min_x, min_y, w, h, self.grid_cols, self.grid_rows),
            Tool::PolarGrid => {
                let outer_r = (w.min(h)) / 2.0;
                let inner_r = outer_r * self.polar_grid_inner_ratio as f64;
                PathData::polar_grid(
                    cx,
                    cy,
                    outer_r,
                    inner_r,
                    self.polar_grid_rings,
                    self.polar_grid_sectors,
                )
            }
            _ => return None,
        };

        Some(path)
    }

    /// Group the currently selected nodes. Requires 2+ nodes in selection.
    fn do_group_selected(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
        doc_modified: &mut bool,
    ) {
        if doc.selection.count() < 2 {
            return;
        }
        let sel_ids: Vec<_> = doc.selection.ids().copied().collect();
        if let Some((layer_id, mut indexed)) = doc.nodes_layer_and_indices(&sel_ids) {
            indexed.sort_by_key(|(_, idx)| *idx);
            let children: Vec<_> = indexed.iter().map(|(id, _)| *id).collect();
            let insert_index = indexed[0].1;
            let group_kind = SceneNodeKind::Group(GroupNode {
                children: children.clone(),
                clip_children: false,
                clip_node_id: None,
                blend_spine_id: None,
            });
            let group = SceneNode::new("Group", layer_id, group_kind);
            let group_id = group.id;
            let cmd = Command::GroupNodes {
                group,
                layer_id,
                insert_index,
                children,
            };
            history.execute(cmd, doc);
            self.selected_id = Some(group_id);
            doc.selection = Selection::single(group_id);
            *doc_modified = true;
        }
    }

    fn do_collect_in_new_layer(
        &mut self,
        node_ids: Vec<NodeId>,
        doc: &mut Document,
        history: &mut CommandHistory,
        doc_modified: &mut bool,
    ) {
        // Fall back to current selection when no explicit ids given
        let raw_ids: Vec<NodeId> = if node_ids.is_empty() {
            doc.selection.ids().copied().collect()
        } else {
            node_ids
        };
        if raw_ids.is_empty() {
            return;
        }

        // Resolve group children to their top-level ancestors (deduplicated)
        let mut resolved: Vec<NodeId> = Vec::new();
        for id in raw_ids {
            if let Some(tid) = doc.top_level_ancestor(id) {
                if !resolved.contains(&tid) {
                    resolved.push(tid);
                }
            }
        }
        if resolved.is_empty() {
            return;
        }

        let new_layer = Layer::new("Collected Layer");
        let new_layer_id = new_layer.id;

        let mut cmds = vec![Command::AddLayer { layer: new_layer }];
        for (i, nid) in resolved.iter().enumerate() {
            if let Some((old_layer_id, old_index)) = doc.node_layer_and_index(nid) {
                cmds.push(Command::MoveNodeToLayer {
                    node_id: *nid,
                    old_layer_id,
                    new_layer_id,
                    old_index,
                    new_index: i,
                });
            }
        }
        history.execute(Command::Batch(cmds), doc);
        *doc_modified = true;
    }

    fn do_release_to_layers(
        &mut self,
        node_ids: Vec<NodeId>,
        doc: &mut Document,
        history: &mut CommandHistory,
        doc_modified: &mut bool,
    ) {
        let raw_ids: Vec<NodeId> = if node_ids.is_empty() {
            doc.selection.ids().copied().collect()
        } else {
            node_ids
        };
        if raw_ids.is_empty() {
            return;
        }

        // Resolve group children to top-level ancestors (deduplicated).
        let mut resolved: Vec<NodeId> = Vec::new();
        for id in raw_ids {
            if let Some(tid) = doc.top_level_ancestor(id) {
                if !resolved.contains(&tid) {
                    resolved.push(tid);
                }
            }
        }
        if resolved.is_empty() {
            return;
        }

        // One new layer per node.
        let mut cmds: Vec<Command> = Vec::new();
        for (seq, nid) in resolved.iter().enumerate() {
            if let Some((old_layer_id, old_index)) = doc.node_layer_and_index(nid) {
                let new_layer = Layer::new(&format!("Layer {}", seq + 1));
                let new_layer_id = new_layer.id;
                cmds.push(Command::AddLayer { layer: new_layer });
                cmds.push(Command::MoveNodeToLayer {
                    node_id: *nid,
                    old_layer_id,
                    new_layer_id,
                    old_index,
                    new_index: 0,
                });
            }
        }
        if !cmds.is_empty() {
            history.execute(Command::Batch(cmds), doc);
            *doc_modified = true;
        }
    }

    fn do_merge_layers(
        &mut self,
        layer_ids: Vec<photonic_core::layer::LayerId>,
        doc: &mut Document,
        history: &mut CommandHistory,
        doc_modified: &mut bool,
    ) {
        if layer_ids.len() < 2 {
            return;
        }
        // Validate
        for lid in &layer_ids {
            if !doc.layers.contains_key(lid) {
                return;
            }
        }

        // Target = first of the selected layers in document order (bottom-most).
        let target_id = match doc.layer_order.iter().find(|id| layer_ids.contains(id)) {
            Some(&id) => id,
            None => return,
        };

        let source_ids: Vec<_> = layer_ids
            .iter()
            .filter(|&&id| id != target_id)
            .copied()
            .collect();

        let mut cmds: Vec<Command> = Vec::new();

        // Process sources in document order.
        let ordered_sources: Vec<_> = doc
            .layer_order
            .iter()
            .filter(|id| source_ids.contains(id))
            .copied()
            .collect();

        let mut new_index_offset = doc.layers[&target_id].node_ids.len();

        for src_id in &ordered_sources {
            let src_layer = doc.layers[src_id].clone();
            for node_id in src_layer.node_ids.clone() {
                if let Some((old_layer_id, old_index)) = doc.node_layer_and_index(&node_id) {
                    cmds.push(Command::MoveNodeToLayer {
                        node_id,
                        old_layer_id,
                        new_layer_id: target_id,
                        old_index,
                        new_index: new_index_offset,
                    });
                    new_index_offset += 1;
                }
            }
            cmds.push(Command::RemoveLayerFull { layer: src_layer });
        }

        if !cmds.is_empty() {
            history.execute(Command::Batch(cmds), doc);
            *doc_modified = true;
        }
    }

    /// Snap a canvas coordinate to the grid if snap-to-grid is enabled.
    fn snap(&self, v: f64) -> f64 {
        if self.prefs.snap_to_grid {
            let g = self.prefs.grid_size as f64;
            (v / g).round() * g
        } else {
            v
        }
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Returns `true` when viewport keyboard shortcuts should be processed.
///
/// All tool handlers **must** gate every keyboard shortcut through this
/// check.  When any text widget (e.g. the AI chat box) has keyboard focus,
/// `egui::Context::wants_keyboard_input` returns `true` and we suppress
/// every viewport shortcut so typing never accidentally mutates the canvas.
fn viewport_kb(ctx: &egui::Context) -> bool {
    !ctx.wants_keyboard_input()
}

/// Flatten a kurbo `BezPath` into screen-space egui points, approximating
/// cubic and quadratic bezier segments with line segments.
fn bez_to_screen_points(bez: &BezPath, view: &CanvasView) -> Vec<egui::Pos2> {
    let mut pts: Vec<egui::Pos2> = Vec::new();
    let mut cur = (0.0f64, 0.0f64);
    for el in bez.elements() {
        match el {
            PathEl::MoveTo(p) => {
                cur = (p.x, p.y);
                let (sx, sy) = view.canvas_to_screen(p.x, p.y);
                pts.push(egui::pos2(sx as f32, sy as f32));
            }
            PathEl::LineTo(p) => {
                cur = (p.x, p.y);
                let (sx, sy) = view.canvas_to_screen(p.x, p.y);
                pts.push(egui::pos2(sx as f32, sy as f32));
            }
            PathEl::CurveTo(c1, c2, p) => {
                let (x0, y0) = cur;
                for i in 1..=16u32 {
                    let t = i as f64 / 16.0;
                    let u = 1.0 - t;
                    let x = u * u * u * x0
                        + 3.0 * u * u * t * c1.x
                        + 3.0 * u * t * t * c2.x
                        + t * t * t * p.x;
                    let y = u * u * u * y0
                        + 3.0 * u * u * t * c1.y
                        + 3.0 * u * t * t * c2.y
                        + t * t * t * p.y;
                    let (sx, sy) = view.canvas_to_screen(x, y);
                    pts.push(egui::pos2(sx as f32, sy as f32));
                }
                cur = (p.x, p.y);
            }
            PathEl::QuadTo(c, p) => {
                let (x0, y0) = cur;
                for i in 1..=8u32 {
                    let t = i as f64 / 8.0;
                    let u = 1.0 - t;
                    let x = u * u * x0 + 2.0 * u * t * c.x + t * t * p.x;
                    let y = u * u * y0 + 2.0 * u * t * c.y + t * t * p.y;
                    let (sx, sy) = view.canvas_to_screen(x, y);
                    pts.push(egui::pos2(sx as f32, sy as f32));
                }
                cur = (p.x, p.y);
            }
            PathEl::ClosePath => {}
        }
    }
    pts
}

fn make_node(
    path: PathData,
    fill_color: [f32; 4],
    stroke: Option<([f32; 4], f32)>,
    label: &str,
    num: usize,
) -> SceneNode {
    let [r, g, b, a] = fill_color;
    let fill = Fill::solid(Color { r, g, b, a });
    let mut path_node = PathNode::new(path).with_fill(fill);
    if let Some(([sr, sg, sb, sa], width)) = stroke {
        path_node = path_node.with_stroke(Stroke::solid(
            Color {
                r: sr,
                g: sg,
                b: sb,
                a: sa,
            },
            width as f64,
        ));
    }
    let kind = SceneNodeKind::Path(path_node);
    SceneNode::new(format!("{} {}", label, num), Default::default(), kind)
}

/// Like `canvas_bounds` but uses glyphon layout for accurate TextNode dimensions.
fn text_aware_canvas_bounds(
    node: &SceneNode,
    renderer: &mut PhotonicRenderer,
) -> Option<(f64, f64, f64, f64)> {
    let local = match &node.kind {
        SceneNodeKind::Text(t) => {
            let (w, h) = renderer.measure_text(&t.content, &t.font_family, t.font_size);
            kurbo::Rect::new(0.0, 0.0, w, h)
        }
        _ => node.local_bounds()?,
    };
    let corners = [
        node.transform.apply(local.x0, local.y0),
        node.transform.apply(local.x1, local.y0),
        node.transform.apply(local.x0, local.y1),
        node.transform.apply(local.x1, local.y1),
    ];
    let min_x = corners.iter().map(|&(x, _)| x).fold(f64::MAX, f64::min);
    let min_y = corners.iter().map(|&(_, y)| y).fold(f64::MAX, f64::min);
    let max_x = corners.iter().map(|&(x, _)| x).fold(f64::MIN, f64::max);
    let max_y = corners.iter().map(|&(_, y)| y).fold(f64::MIN, f64::max);
    Some((min_x, min_y, max_x, max_y))
}

/// Returns the axis-aligned bounding box that covers all nodes in `ids`,
/// or `None` if none of them have computable bounds.
fn selection_canvas_bounds(
    doc: &Document,
    ids: &[NodeId],
    renderer: &mut PhotonicRenderer,
) -> Option<(f64, f64, f64, f64)> {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for &id in ids {
        if let Some(node) = doc.nodes.get(&id) {
            if let Some((x0, y0, x1, y1)) = text_aware_canvas_bounds(node, renderer) {
                min_x = min_x.min(x0);
                min_y = min_y.min(y0);
                max_x = max_x.max(x1);
                max_y = max_y.max(y1);
            }
        }
    }
    if min_x.is_finite() {
        Some((min_x, min_y, max_x, max_y))
    } else {
        None
    }
}

// ─── Direct-select helpers ────────────────────────────────────────────────────

/// Like `bez_to_screen_points` but applies a node transform before projecting.
fn bez_to_screen_points_xf(
    bez: &BezPath,
    view: &CanvasView,
    transform: &photonic_core::transform::Transform,
) -> Vec<egui::Pos2> {
    use kurbo::PathEl;
    let mut pts: Vec<egui::Pos2> = Vec::new();
    let mut cur_local = (0.0f64, 0.0f64);
    for el in bez.elements() {
        match el {
            PathEl::MoveTo(p) => {
                cur_local = (p.x, p.y);
                let (cx, cy) = transform.apply(p.x, p.y);
                let (sx, sy) = view.canvas_to_screen(cx, cy);
                pts.push(egui::pos2(sx as f32, sy as f32));
            }
            PathEl::LineTo(p) => {
                cur_local = (p.x, p.y);
                let (cx, cy) = transform.apply(p.x, p.y);
                let (sx, sy) = view.canvas_to_screen(cx, cy);
                pts.push(egui::pos2(sx as f32, sy as f32));
            }
            PathEl::CurveTo(c1, c2, p) => {
                let (x0, y0) = cur_local;
                for i in 1..=16u32 {
                    let t = i as f64 / 16.0;
                    let u = 1.0 - t;
                    let lx = u * u * u * x0
                        + 3.0 * u * u * t * c1.x
                        + 3.0 * u * t * t * c2.x
                        + t * t * t * p.x;
                    let ly = u * u * u * y0
                        + 3.0 * u * u * t * c1.y
                        + 3.0 * u * t * t * c2.y
                        + t * t * t * p.y;
                    let (cx, cy) = transform.apply(lx, ly);
                    let (sx, sy) = view.canvas_to_screen(cx, cy);
                    pts.push(egui::pos2(sx as f32, sy as f32));
                }
                cur_local = (p.x, p.y);
            }
            PathEl::QuadTo(c, p) => {
                let (x0, y0) = cur_local;
                for i in 1..=8u32 {
                    let t = i as f64 / 8.0;
                    let u = 1.0 - t;
                    let lx = u * u * x0 + 2.0 * u * t * c.x + t * t * p.x;
                    let ly = u * u * y0 + 2.0 * u * t * c.y + t * t * p.y;
                    let (cx, cy) = transform.apply(lx, ly);
                    let (sx, sy) = view.canvas_to_screen(cx, cy);
                    pts.push(egui::pos2(sx as f32, sy as f32));
                }
                cur_local = (p.x, p.y);
            }
            PathEl::ClosePath => {}
        }
    }
    pts
}

/// Extract `(element_index, local_point)` for every element that has an endpoint.
/// `ClosePath` is excluded (no anchor).
fn path_anchor_points(bez: &BezPath) -> Vec<(usize, Point)> {
    bez.elements()
        .iter()
        .enumerate()
        .filter_map(|(i, el)| match el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => Some((i, *p)),
            PathEl::CurveTo(_, _, p) => Some((i, *p)),
            PathEl::QuadTo(_, p) => Some((i, *p)),
            PathEl::ClosePath => None,
        })
        .collect()
}

/// Find the element index of the anchor point nearest to `(cursor_cx, cursor_cy)`
/// in canvas space, within `threshold_px` pixels on screen.
fn nearest_anchor_screen(
    bez: &BezPath,
    transform: &photonic_core::transform::Transform,
    view: &CanvasView,
    cursor_cx: f64,
    cursor_cy: f64,
    threshold_px: f64,
) -> Option<usize> {
    let (cursor_sx, cursor_sy) = view.canvas_to_screen(cursor_cx, cursor_cy);
    let mut best: Option<(usize, f64)> = None;
    for (idx, local_pt) in path_anchor_points(bez) {
        let (cx, cy) = transform.apply(local_pt.x, local_pt.y);
        let (sx, sy) = view.canvas_to_screen(cx, cy);
        let dist = ((sx - cursor_sx).powi(2) + (sy - cursor_sy).powi(2)).sqrt();
        if dist < threshold_px {
            if best.map_or(true, |(_, d)| dist < d) {
                best = Some((idx, dist));
            }
        }
    }
    best.map(|(idx, _)| idx)
}

/// Move the selected anchor points in a `BezPath` by `(dx, dy)` in local space.
///
/// For each selected element:
/// - The element's endpoint is shifted by `(dx, dy)`.
/// - If the element is `CurveTo`, its incoming handle (c2) is also shifted.
/// - The next element's outgoing handle (c1 for `CurveTo`, c for `QuadTo`) is
///   shifted only if the next anchor is NOT also in the selection (prevents
///   double-moving shared handles).
fn bez_move_anchors(bez: &BezPath, selected: &[usize], dx: f64, dy: f64) -> BezPath {
    let els: Vec<PathEl> = bez.elements().iter().copied().collect();
    let n = els.len();
    let sel_set: std::collections::HashSet<usize> = selected.iter().copied().collect();
    let mut new_els = els.clone();

    for &i in selected {
        if i >= n {
            continue;
        }
        // Move endpoint (and incoming handle for curved elements)
        new_els[i] = match els[i] {
            PathEl::MoveTo(p) => PathEl::MoveTo(Point::new(p.x + dx, p.y + dy)),
            PathEl::LineTo(p) => PathEl::LineTo(Point::new(p.x + dx, p.y + dy)),
            PathEl::CurveTo(c1, c2, p) => PathEl::CurveTo(
                c1,
                Point::new(c2.x + dx, c2.y + dy),
                Point::new(p.x + dx, p.y + dy),
            ),
            PathEl::QuadTo(c, p) => PathEl::QuadTo(
                Point::new(c.x + dx, c.y + dy),
                Point::new(p.x + dx, p.y + dy),
            ),
            PathEl::ClosePath => PathEl::ClosePath,
        };
        // Move outgoing handle (on the NEXT element) only if next anchor isn't also selected
        let j = i + 1;
        if j < n && !sel_set.contains(&j) {
            new_els[j] = match els[j] {
                PathEl::CurveTo(c1, c2, p) => {
                    PathEl::CurveTo(Point::new(c1.x + dx, c1.y + dy), c2, p)
                }
                PathEl::QuadTo(c, p) => PathEl::QuadTo(Point::new(c.x + dx, c.y + dy), p),
                other => other,
            };
        }
    }

    let mut result = BezPath::new();
    for el in new_els {
        result.push(el);
    }
    result
}

/// Remove the elements at `indices` from a `BezPath`, rebuilding a valid path.
/// Apply zig-zag distortion to a BezPath (GUI version, mirrors MCP logic).
fn gui_zig_zag(bez: &BezPath, size: f64, ridges: usize, smooth: bool) -> BezPath {
    use kurbo::{PathEl, Point};

    let mut result = BezPath::new();
    let mut current = Point::ZERO;
    let mut subpath_start = Point::ZERO;

    for el in bez.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                result.move_to(p);
                current = p;
                subpath_start = p;
            }
            PathEl::ClosePath => {
                if current != subpath_start {
                    gui_zig_zag_segment(&mut result, current, subpath_start, size, ridges, smooth);
                }
                result.close_path();
                current = subpath_start;
            }
            _ => {
                let endpoint = match *el {
                    PathEl::LineTo(p) | PathEl::CurveTo(_, _, p) | PathEl::QuadTo(_, p) => p,
                    _ => unreachable!(),
                };
                // Find previous endpoint.
                let start = {
                    let els = result.elements();
                    let mut pt = Point::ZERO;
                    for e in els.iter().rev() {
                        match e {
                            PathEl::MoveTo(p)
                            | PathEl::LineTo(p)
                            | PathEl::CurveTo(_, _, p)
                            | PathEl::QuadTo(_, p) => {
                                pt = *p;
                                break;
                            }
                            PathEl::ClosePath => {}
                        }
                    }
                    pt
                };
                gui_zig_zag_segment(&mut result, start, endpoint, size, ridges, smooth);
                current = endpoint;
            }
        }
    }
    result
}

fn gui_zig_zag_segment(
    path: &mut BezPath,
    from: kurbo::Point,
    to: kurbo::Point,
    size: f64,
    ridges: usize,
    smooth: bool,
) {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-9 {
        path.line_to(to);
        return;
    }
    let tx = dx / len;
    let ty = dy / len;
    let nx = -ty;
    let ny = tx;
    let steps = ridges * 2;
    let step_len = len / steps as f64;

    for i in 1..=steps {
        let t = i as f64 / steps as f64;
        let px = from.x + dx * t;
        let py = from.y + dy * t;
        let disp = if i == steps {
            0.0
        } else if i % 2 == 1 {
            size / 2.0
        } else {
            -size / 2.0
        };
        let pt = kurbo::Point::new(px + nx * disp, py + ny * disp);

        if smooth && i < steps {
            let handle_len = step_len * 0.3;
            let prev_disp = if i == 1 {
                0.0
            } else if (i - 1) % 2 == 1 {
                size / 2.0
            } else {
                -size / 2.0
            };
            let prev_t = (i - 1) as f64 / steps as f64;
            let prev_x = from.x + dx * prev_t + nx * prev_disp;
            let prev_y = from.y + dy * prev_t + ny * prev_disp;
            let cp1 = kurbo::Point::new(prev_x + tx * handle_len, prev_y + ty * handle_len);
            let cp2 = kurbo::Point::new(pt.x - tx * handle_len, pt.y - ty * handle_len);
            path.curve_to(cp1, cp2, pt);
        } else {
            path.line_to(pt);
        }
    }
}

fn gui_path_centroid(bez: &BezPath) -> kurbo::Point {
    let mut sx = 0.0;
    let mut sy = 0.0;
    let mut n = 0usize;
    for el in bez.elements() {
        let pt = match *el {
            PathEl::MoveTo(p)
            | PathEl::LineTo(p)
            | PathEl::CurveTo(_, _, p)
            | PathEl::QuadTo(_, p) => Some(p),
            PathEl::ClosePath => None,
        };
        if let Some(p) = pt {
            sx += p.x;
            sy += p.y;
            n += 1;
        }
    }
    if n == 0 {
        kurbo::Point::ZERO
    } else {
        kurbo::Point::new(sx / n as f64, sy / n as f64)
    }
}

fn gui_pucker_bloat(bez: &BezPath, strength: f64, center: kurbo::Point) -> BezPath {
    let displace = |p: kurbo::Point| -> kurbo::Point {
        let dx = p.x - center.x;
        let dy = p.y - center.y;
        let dist = (dx * dx + dy * dy).sqrt();
        if dist < 1e-9 {
            return p;
        }
        let factor = 1.0 + strength;
        kurbo::Point::new(center.x + dx * factor, center.y + dy * factor)
    };
    let mut result = BezPath::new();
    for el in bez.elements() {
        match *el {
            PathEl::MoveTo(p) => result.move_to(displace(p)),
            PathEl::LineTo(p) => result.line_to(displace(p)),
            PathEl::CurveTo(c1, c2, p) => result.curve_to(displace(c1), displace(c2), displace(p)),
            PathEl::QuadTo(c, p) => result.quad_to(displace(c), displace(p)),
            PathEl::ClosePath => result.close_path(),
        }
    }
    result
}

fn gui_round_corners(bez: &BezPath, radius: f64) -> BezPath {
    let elements = bez.elements();
    if elements.is_empty() || radius <= 0.0 {
        return bez.clone();
    }

    let mut result = BezPath::new();
    let mut subpath: Vec<kurbo::Point> = Vec::new();
    let mut is_closed = false;

    let flush = |result: &mut BezPath, pts: &[kurbo::Point], closed: bool, radius: f64| {
        if pts.len() < 2 {
            if let Some(&p) = pts.first() {
                result.move_to(p);
            }
            return;
        }
        let n = pts.len();
        for i in 0..n {
            let prev = if i == 0 {
                if closed {
                    pts[n - 1]
                } else {
                    pts[0]
                }
            } else {
                pts[i - 1]
            };
            let curr = pts[i];
            let next = if i == n - 1 {
                if closed {
                    pts[0]
                } else {
                    pts[n - 1]
                }
            } else {
                pts[i + 1]
            };
            let is_ep = !closed && (i == 0 || i == n - 1);
            if is_ep {
                if i == 0 {
                    result.move_to(curr);
                } else {
                    result.line_to(curr);
                }
            } else {
                let dx_in = curr.x - prev.x;
                let dy_in = curr.y - prev.y;
                let len_in = (dx_in * dx_in + dy_in * dy_in).sqrt();
                let dx_out = next.x - curr.x;
                let dy_out = next.y - curr.y;
                let len_out = (dx_out * dx_out + dy_out * dy_out).sqrt();
                if len_in < 1e-9 || len_out < 1e-9 {
                    if i == 0 {
                        result.move_to(curr);
                    } else {
                        result.line_to(curr);
                    }
                    continue;
                }
                let r = radius.min(len_in / 2.0).min(len_out / 2.0);
                let fs =
                    kurbo::Point::new(curr.x - (dx_in / len_in) * r, curr.y - (dy_in / len_in) * r);
                let fe = kurbo::Point::new(
                    curr.x + (dx_out / len_out) * r,
                    curr.y + (dy_out / len_out) * r,
                );
                if i == 0 {
                    result.move_to(fs);
                } else {
                    result.line_to(fs);
                }
                result.quad_to(curr, fe);
            }
        }
        if closed {
            result.close_path();
        }
    };

    for el in elements {
        match *el {
            PathEl::MoveTo(p) => {
                if !subpath.is_empty() {
                    flush(&mut result, &subpath, is_closed, radius);
                }
                subpath.clear();
                subpath.push(p);
                is_closed = false;
            }
            PathEl::LineTo(p) => {
                subpath.push(p);
            }
            PathEl::CurveTo(_, _, p) | PathEl::QuadTo(_, p) => {
                subpath.push(p);
            }
            PathEl::ClosePath => {
                is_closed = true;
            }
        }
    }
    if !subpath.is_empty() {
        flush(&mut result, &subpath, is_closed, radius);
    }
    result
}

fn gui_warp_envelope(bez: &BezPath, warp_type: &str, bend: f64) -> BezPath {
    // Compute bounding box.
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;
    for el in bez.elements() {
        let pts: Vec<kurbo::Point> = match *el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => vec![p],
            PathEl::CurveTo(c1, c2, p) => vec![c1, c2, p],
            PathEl::QuadTo(c, p) => vec![c, p],
            PathEl::ClosePath => vec![],
        };
        for p in pts {
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
        }
    }
    let w = max_x - min_x;
    let h = max_y - min_y;
    if w < 1e-9 || h < 1e-9 {
        return bez.clone();
    }

    let warp = |p: kurbo::Point| -> kurbo::Point {
        let nx = (p.x - min_x) / w;
        let ny = (p.y - min_y) / h;
        let (dx, dy) = match warp_type {
            "arc" => (0.0, bend * (nx * (1.0 - nx) * 4.0) * h * 0.25),
            "bulge" => {
                let cx = nx - 0.5;
                let cy = ny - 0.5;
                let r = (cx * cx + cy * cy).sqrt().min(0.5);
                let f = bend * (1.0 - r * 2.0).max(0.0);
                (cx * f * w, cy * f * h)
            }
            "wave" => (
                0.0,
                bend * (std::f64::consts::PI * 2.0 * nx).sin() * h * 0.25,
            ),
            "flag" => (
                0.0,
                bend * nx * (std::f64::consts::PI * 2.0 * ny).sin() * h * 0.25,
            ),
            _ => (0.0, 0.0),
        };
        kurbo::Point::new(p.x + dx, p.y + dy)
    };

    let mut result = BezPath::new();
    for el in bez.elements() {
        match *el {
            PathEl::MoveTo(p) => result.move_to(warp(p)),
            PathEl::LineTo(p) => result.line_to(warp(p)),
            PathEl::CurveTo(c1, c2, p) => result.curve_to(warp(c1), warp(c2), warp(p)),
            PathEl::QuadTo(c, p) => result.quad_to(warp(c), warp(p)),
            PathEl::ClosePath => result.close_path(),
        }
    }
    result
}

fn gui_crystallize(bez: &BezPath, size: f64, count: usize) -> BezPath {
    let mut result = BezPath::new();
    let mut current = kurbo::Point::ZERO;
    let mut subpath_start = kurbo::Point::ZERO;
    for el in bez.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                result.move_to(p);
                current = p;
                subpath_start = p;
            }
            PathEl::ClosePath => {
                if current != subpath_start {
                    gui_crystallize_seg(&mut result, current, subpath_start, size, count);
                }
                result.close_path();
                current = subpath_start;
            }
            _ => {
                let endpoint = match *el {
                    PathEl::LineTo(p) | PathEl::CurveTo(_, _, p) | PathEl::QuadTo(_, p) => p,
                    _ => unreachable!(),
                };
                let start = {
                    let els = result.elements();
                    let mut pt = kurbo::Point::ZERO;
                    for e in els.iter().rev() {
                        match e {
                            PathEl::MoveTo(p)
                            | PathEl::LineTo(p)
                            | PathEl::CurveTo(_, _, p)
                            | PathEl::QuadTo(_, p) => {
                                pt = *p;
                                break;
                            }
                            PathEl::ClosePath => {}
                        }
                    }
                    pt
                };
                gui_crystallize_seg(&mut result, start, endpoint, size, count);
                current = endpoint;
            }
        }
    }
    result
}

fn gui_crystallize_seg(
    path: &mut BezPath,
    from: kurbo::Point,
    to: kurbo::Point,
    size: f64,
    count: usize,
) {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-9 {
        path.line_to(to);
        return;
    }
    let nx = -dy / len;
    let ny = dx / len;
    for i in 0..count {
        let t_peak = (i as f64 + 0.5) / count as f64;
        let t_end = (i + 1) as f64 / count as f64;
        let peak = kurbo::Point::new(
            from.x + dx * t_peak + nx * size,
            from.y + dy * t_peak + ny * size,
        );
        let base_end = kurbo::Point::new(from.x + dx * t_end, from.y + dy * t_end);
        path.line_to(peak);
        path.line_to(base_end);
    }
}

fn gui_scallop(bez: &BezPath, depth: f64, count: usize) -> BezPath {
    let mut result = BezPath::new();
    let mut current = kurbo::Point::ZERO;
    let mut subpath_start = kurbo::Point::ZERO;

    for el in bez.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                result.move_to(p);
                current = p;
                subpath_start = p;
            }
            PathEl::ClosePath => {
                if current != subpath_start {
                    gui_scallop_seg(&mut result, current, subpath_start, depth, count);
                }
                result.close_path();
                current = subpath_start;
            }
            _ => {
                let endpoint = match *el {
                    PathEl::LineTo(p) | PathEl::CurveTo(_, _, p) | PathEl::QuadTo(_, p) => p,
                    _ => unreachable!(),
                };
                let start = {
                    let els = result.elements();
                    let mut pt = kurbo::Point::ZERO;
                    for e in els.iter().rev() {
                        match e {
                            PathEl::MoveTo(p)
                            | PathEl::LineTo(p)
                            | PathEl::CurveTo(_, _, p)
                            | PathEl::QuadTo(_, p) => {
                                pt = *p;
                                break;
                            }
                            PathEl::ClosePath => {}
                        }
                    }
                    pt
                };
                gui_scallop_seg(&mut result, start, endpoint, depth, count);
                current = endpoint;
            }
        }
    }
    result
}

fn gui_scallop_seg(
    path: &mut BezPath,
    from: kurbo::Point,
    to: kurbo::Point,
    depth: f64,
    count: usize,
) {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-9 {
        path.line_to(to);
        return;
    }
    let nx = dy / len;
    let ny = -dx / len;
    for i in 0..count {
        let t0 = i as f64 / count as f64;
        let t1 = (i + 1) as f64 / count as f64;
        let tmid = (t0 + t1) / 2.0;
        let p1 = kurbo::Point::new(from.x + dx * t1, from.y + dy * t1);
        let p0 = kurbo::Point::new(from.x + dx * t0, from.y + dy * t0);
        let pmid = kurbo::Point::new(
            from.x + dx * tmid + nx * depth,
            from.y + dy * tmid + ny * depth,
        );
        let qx = 2.0 * pmid.x - 0.5 * (p0.x + p1.x);
        let qy = 2.0 * pmid.y - 0.5 * (p0.y + p1.y);
        path.quad_to(kurbo::Point::new(qx, qy), p1);
    }
}

fn gui_blend_objects(
    nid_a: NodeId,
    nid_b: NodeId,
    steps: usize,
    doc: &mut Document,
    history: &mut CommandHistory,
    doc_modified: &mut bool,
) {
    use photonic_core::color::Color;
    use photonic_core::style::{Fill, FillKind};

    let (node_a, node_b) = match (
        doc.nodes.get(&nid_a).cloned(),
        doc.nodes.get(&nid_b).cloned(),
    ) {
        (Some(a), Some(b)) => (a, b),
        _ => return,
    };
    let (pn_a, pn_b) = match (&node_a.kind, &node_b.kind) {
        (SceneNodeKind::Path(a), SceneNodeKind::Path(b)) => (a.clone(), b.clone()),
        _ => return,
    };
    let bez_a = pn_a.path_data.to_bez_path();
    let bez_b = pn_b.path_data.to_bez_path();
    if bez_a.elements().len() != bez_b.elements().len() {
        return;
    }

    let color_a = match &pn_a.fill.kind {
        FillKind::Solid(c) => Some(*c),
        _ => None,
    };
    let color_b = match &pn_b.fill.kind {
        FillKind::Solid(c) => Some(*c),
        _ => None,
    };
    let tx_a = (node_a.transform.matrix[4], node_a.transform.matrix[5]);
    let tx_b = (node_b.transform.matrix[4], node_b.transform.matrix[5]);
    let layer_id = node_a.layer_id;

    let lerp_pt = |a: kurbo::Point, b: kurbo::Point, t: f64| {
        kurbo::Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
    };

    for i in 1..=steps {
        let t = i as f64 / (steps + 1) as f64;
        let mut interp = BezPath::new();
        for (ea, eb) in bez_a.elements().iter().zip(bez_b.elements().iter()) {
            match (*ea, *eb) {
                (PathEl::MoveTo(a), PathEl::MoveTo(b)) => interp.move_to(lerp_pt(a, b, t)),
                (PathEl::LineTo(a), PathEl::LineTo(b)) => interp.line_to(lerp_pt(a, b, t)),
                (PathEl::CurveTo(a1, a2, a3), PathEl::CurveTo(b1, b2, b3)) => {
                    interp.curve_to(lerp_pt(a1, b1, t), lerp_pt(a2, b2, t), lerp_pt(a3, b3, t))
                }
                (PathEl::QuadTo(a1, a2), PathEl::QuadTo(b1, b2)) => {
                    interp.quad_to(lerp_pt(a1, b1, t), lerp_pt(a2, b2, t))
                }
                (PathEl::ClosePath, PathEl::ClosePath) => interp.close_path(),
                _ => interp.push(*ea),
            }
        }
        let mut new_pn = pn_a.clone();
        new_pn.path_data = PathData::from_bez_path(&interp);
        if let (Some(ca), Some(cb)) = (&color_a, &color_b) {
            new_pn.fill = Fill {
                kind: FillKind::Solid(Color::new(
                    ca.r + (cb.r - ca.r) * t as f32,
                    ca.g + (cb.g - ca.g) * t as f32,
                    ca.b + (cb.b - ca.b) * t as f32,
                    ca.a + (cb.a - ca.a) * t as f32,
                )),
                ..pn_a.fill.clone()
            };
        }
        let opacity = node_a.opacity + (node_b.opacity - node_a.opacity) * t as f32;
        let name = format!("Blend {}/{}", i, steps);
        let mut node = SceneNode::new(&name, layer_id, SceneNodeKind::Path(new_pn));
        node.opacity = opacity;
        let itx = (
            tx_a.0 + (tx_b.0 - tx_a.0) * t,
            tx_a.1 + (tx_b.1 - tx_a.1) * t,
        );
        node.transform = photonic_core::transform::Transform::translate(itx.0, itx.1);
        history.execute(
            Command::AddNode {
                node,
                layer_id: Some(layer_id),
            },
            doc,
        );
    }
    *doc_modified = true;
}

/// Blend using Smooth Color mode: auto-compute steps from color distance.
fn gui_blend_objects_smooth_color(
    nid_a: NodeId,
    nid_b: NodeId,
    doc: &mut Document,
    history: &mut CommandHistory,
    doc_modified: &mut bool,
) {
    use photonic_core::style::FillKind;
    let (node_a, node_b) = match (
        doc.nodes.get(&nid_a).cloned(),
        doc.nodes.get(&nid_b).cloned(),
    ) {
        (Some(a), Some(b)) => (a, b),
        _ => return,
    };
    let (pn_a, pn_b) = match (&node_a.kind, &node_b.kind) {
        (SceneNodeKind::Path(a), SceneNodeKind::Path(b)) => (a.clone(), b.clone()),
        _ => return,
    };
    let color_a = match &pn_a.fill.kind {
        FillKind::Solid(c) => Some(*c),
        _ => None,
    };
    let color_b = match &pn_b.fill.kind {
        FillKind::Solid(c) => Some(*c),
        _ => None,
    };
    let steps = if let (Some(ca), Some(cb)) = (&color_a, &color_b) {
        let dr = ((cb.r - ca.r).abs() * 255.0) as f64;
        let dg = ((cb.g - ca.g).abs() * 255.0) as f64;
        let db = ((cb.b - ca.b).abs() * 255.0) as f64;
        (dr.max(dg).max(db).ceil() as usize).max(1)
    } else {
        5
    };
    gui_blend_objects(nid_a, nid_b, steps, doc, history, doc_modified);
}

/// Blend using Specified Distance mode: space steps by pixel distance.
fn gui_blend_objects_spacing(
    nid_a: NodeId,
    nid_b: NodeId,
    spacing: f64,
    doc: &mut Document,
    history: &mut CommandHistory,
    doc_modified: &mut bool,
) {
    if spacing <= 0.0 {
        return;
    }
    let (node_a, node_b) = match (
        doc.nodes.get(&nid_a).cloned(),
        doc.nodes.get(&nid_b).cloned(),
    ) {
        (Some(a), Some(b)) => (a, b),
        _ => return,
    };
    let tx_a = (node_a.transform.matrix[4], node_a.transform.matrix[5]);
    let tx_b = (node_b.transform.matrix[4], node_b.transform.matrix[5]);
    let dx = tx_b.0 - tx_a.0;
    let dy = tx_b.1 - tx_a.1;
    let dist = (dx * dx + dy * dy).sqrt();
    let steps = ((dist / spacing).ceil() as usize).saturating_sub(1).max(1);
    gui_blend_objects(nid_a, nid_b, steps, doc, history, doc_modified);
}

fn gui_twirl(bez: &BezPath, angle_rad: f64, center: kurbo::Point) -> BezPath {
    let mut max_dist = 0.0f64;
    for el in bez.elements() {
        let pts: Vec<kurbo::Point> = match *el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => vec![p],
            PathEl::CurveTo(c1, c2, p) => vec![c1, c2, p],
            PathEl::QuadTo(c, p) => vec![c, p],
            PathEl::ClosePath => vec![],
        };
        for p in pts {
            let d = ((p.x - center.x).powi(2) + (p.y - center.y).powi(2)).sqrt();
            if d > max_dist {
                max_dist = d;
            }
        }
    }
    if max_dist < 1e-9 {
        return bez.clone();
    }

    let twirl = |p: kurbo::Point| -> kurbo::Point {
        let dx = p.x - center.x;
        let dy = p.y - center.y;
        let dist = (dx * dx + dy * dy).sqrt();
        let t = 1.0 - (dist / max_dist).min(1.0);
        let a = angle_rad * t;
        kurbo::Point::new(
            center.x + dx * a.cos() - dy * a.sin(),
            center.y + dx * a.sin() + dy * a.cos(),
        )
    };

    let mut result = BezPath::new();
    for el in bez.elements() {
        match *el {
            PathEl::MoveTo(p) => result.move_to(twirl(p)),
            PathEl::LineTo(p) => result.line_to(twirl(p)),
            PathEl::CurveTo(c1, c2, p) => result.curve_to(twirl(c1), twirl(c2), twirl(p)),
            PathEl::QuadTo(c, p) => result.quad_to(twirl(c), twirl(p)),
            PathEl::ClosePath => result.close_path(),
        }
    }
    result
}

fn gui_xorshift64(state: &mut u64) -> f64 {
    let mut s = *state;
    s ^= s << 13;
    s ^= s >> 7;
    s ^= s << 17;
    *state = s;
    (s as f64 / u64::MAX as f64) * 2.0 - 1.0
}

fn gui_subdivide_bez(bez: &BezPath) -> BezPath {
    let mut result = BezPath::new();
    let mut current = kurbo::Point::ZERO;
    let mid =
        |a: kurbo::Point, b: kurbo::Point| kurbo::Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
    for el in bez.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                result.move_to(p);
                current = p;
            }
            PathEl::LineTo(p) => {
                result.line_to(mid(current, p));
                result.line_to(p);
                current = p;
            }
            PathEl::CurveTo(c1, c2, p) => {
                let m01 = mid(current, c1);
                let m12 = mid(c1, c2);
                let m23 = mid(c2, p);
                let m012 = mid(m01, m12);
                let m123 = mid(m12, m23);
                let m0123 = mid(m012, m123);
                result.curve_to(m01, m012, m0123);
                result.curve_to(m123, m23, p);
                current = p;
            }
            PathEl::QuadTo(c, p) => {
                let mc0 = mid(current, c);
                let mc1 = mid(c, p);
                let m = mid(mc0, mc1);
                result.quad_to(mc0, m);
                result.quad_to(mc1, p);
                current = p;
            }
            PathEl::ClosePath => {
                result.close_path();
            }
        }
    }
    result
}

fn gui_roughen(bez: &BezPath, size: f64, seed: u64) -> BezPath {
    let mut rng = seed.max(1);
    let displace = |p: kurbo::Point, rng: &mut u64| -> kurbo::Point {
        kurbo::Point::new(
            p.x + gui_xorshift64(rng) * size,
            p.y + gui_xorshift64(rng) * size,
        )
    };
    let mut result = BezPath::new();
    for el in bez.elements() {
        match *el {
            PathEl::MoveTo(p) => result.move_to(displace(p, &mut rng)),
            PathEl::LineTo(p) => result.line_to(displace(p, &mut rng)),
            PathEl::CurveTo(c1, c2, p) => result.curve_to(
                displace(c1, &mut rng),
                displace(c2, &mut rng),
                displace(p, &mut rng),
            ),
            PathEl::QuadTo(c, p) => result.quad_to(displace(c, &mut rng), displace(p, &mut rng)),
            PathEl::ClosePath => result.close_path(),
        }
    }
    result
}

fn bez_remove_elements(bez: &BezPath, indices: &[usize]) -> BezPath {
    let remove_set: std::collections::HashSet<usize> = indices.iter().copied().collect();
    let mut result = BezPath::new();
    let mut needs_move = true;
    for (i, el) in bez.elements().iter().enumerate() {
        if remove_set.contains(&i) {
            needs_move = true;
            continue;
        }
        if needs_move {
            // Patch: replace a non-MoveTo element that follows a gap with a MoveTo
            let endpoint = match el {
                PathEl::MoveTo(p) | PathEl::LineTo(p) => Some(*p),
                PathEl::CurveTo(_, _, p) => Some(*p),
                PathEl::QuadTo(_, p) => Some(*p),
                PathEl::ClosePath => None,
            };
            if let Some(p) = endpoint {
                result.push(PathEl::MoveTo(p));
                needs_move = false;
                // Skip emitting the original element if it was already a MoveTo
                if !matches!(el, PathEl::MoveTo(_)) {
                    result.push(*el);
                }
            }
        } else {
            result.push(*el);
        }
    }
    result
}

// ─── Claude tab ───────────────────────────────────────────────────────────────

impl PhotonicApp {
    fn draw_claude_tab(&mut self, ui: &mut egui::Ui) {
        // bottom_up pins the input row to the bottom; the scroll area fills
        // whatever space remains above it. We read available_height() after
        // the input row and separator are laid out (in bottom_up order) so we
        // can give the ScrollArea an explicit min height — otherwise egui
        // defaults to a tiny minimum and the messages are invisible.
        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            // ── Input row (pinned to bottom) ─────────────────────────────────
            ui.horizontal(|ui| {
                let send_enabled = !self.claude_chat.busy;
                let resp = ui.add_enabled(
                    send_enabled,
                    egui::TextEdit::singleline(&mut self.claude_chat.input)
                        .desired_width(ui.available_width() - 60.0)
                        .hint_text("Ask Claude to create or edit graphics…"),
                );

                let submitted = resp.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && send_enabled;

                let send_clicked = ui
                    .add_enabled(send_enabled, egui::Button::new("Send"))
                    .clicked();

                if (send_clicked || submitted) && !self.claude_chat.input.trim().is_empty() {
                    let msg = self.claude_chat.input.trim().to_string();
                    self.claude_chat.messages.push((true, msg.clone()));
                    self.claude_chat.pending = Some(msg);
                    self.claude_chat.input.clear();
                    self.claude_chat.busy = true;
                    resp.request_focus();
                }
            });

            ui.separator();

            // ── Message history (scrollable, fills remaining space) ───────────
            // available_height() here is the space above the input row + separator.
            let scroll_h = ui.available_height().max(40.0);
            egui::ScrollArea::vertical()
                .id_salt("claude_chat")
                .min_scrolled_height(scroll_h)
                .max_height(scroll_h)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                if self.claude_chat.messages.is_empty() {
                    ui.label(
                        egui::RichText::new(
                            "Ask Claude to create vector graphics — e.g. \"Draw a red star in the centre of the canvas\"",
                        )
                        .weak()
                        .italics(),
                    );
                }

                for (is_user, text) in &self.claude_chat.messages {
                    if *is_user {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                            egui::Frame::none()
                                .fill(Color32::from_rgb(45, 38, 90))
                                .inner_margin(egui::Margin::symmetric(8.0, 4.0))
                                .rounding(6.0)
                                .show(ui, |ui| {
                                    ui.set_max_width(ui.available_width() * 0.75);
                                    ui.add(egui::Label::new(egui::RichText::new(text).color(Color32::WHITE)).wrap());
                                });
                        });
                    } else if text.starts_with("$ ") {
                        // Bash tool log — monospace terminal style
                        egui::Frame::none()
                            .fill(Color32::from_rgb(7, 7, 11))
                            .inner_margin(egui::Margin::symmetric(8.0, 4.0))
                            .rounding(4.0)
                            .show(ui, |ui| {
                                ui.set_max_width(ui.available_width());
                                for line in text.lines() {
                                    let color = if line.starts_with("$ ") {
                                        Color32::from_rgb(52, 211, 153)
                                    } else {
                                        Color32::from_rgb(187, 187, 210)
                                    };
                                    ui.add(egui::Label::new(egui::RichText::new(line).monospace().color(color).small()).wrap());
                                }
                            });
                        ui.add_space(2.0);
                    } else {
                        let is_err = text.starts_with("⚠");
                        let frame_color = if is_err {
                            Color32::from_rgb(35, 10, 15)
                        } else {
                            Color32::from_rgb(19, 19, 31)
                        };
                        egui::Frame::none()
                            .fill(frame_color)
                            .inner_margin(egui::Margin::symmetric(8.0, 4.0))
                            .rounding(6.0)
                            .show(ui, |ui| {
                                ui.set_max_width(ui.available_width() * 0.85);
                                let text_color = if is_err {
                                    Color32::from_rgb(248, 113, 113)
                                } else {
                                    Color32::from_rgb(187, 187, 210)
                                };
                                ui.add(egui::Label::new(egui::RichText::new(text).color(text_color)).wrap());
                            });
                        ui.add_space(2.0);
                    }
                }

                if self.claude_chat.busy {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(egui::RichText::new("Claude is thinking…").weak().italics());
                    });
                }
                }); // end top_down layout
            });
        });
    }

    // ── Export modal ─────────────────────────────────────────────────────────

    fn draw_export_modal(&mut self, ctx: &egui::Context, doc: &Document) {
        let Some(dlg) = &mut self.export_dialog else {
            return;
        };

        // Collect the button the user clicked without holding a mutable borrow
        // inside the egui closure at the same time as `.open(&mut open)`.
        #[derive(PartialEq)]
        enum Action {
            None,
            Cancel,
            Export,
        }
        let mut action = Action::None;
        let mut open = true;

        egui::Window::new("Export")
            .collapsible(false)
            .resizable(false)
            .fixed_size([340.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                // ── Format ───────────────────────────────────────────────────
                ui.horizontal(|ui| {
                    ui.label("Format");
                    ui.selectable_value(&mut dlg.format, ExportFormat::Png, "PNG");
                    ui.selectable_value(&mut dlg.format, ExportFormat::Jpeg, "JPEG");
                    ui.selectable_value(&mut dlg.format, ExportFormat::WebP, "WebP");
                    ui.selectable_value(&mut dlg.format, ExportFormat::Gif, "GIF");
                    ui.selectable_value(&mut dlg.format, ExportFormat::Tiff, "TIFF");
                    ui.selectable_value(&mut dlg.format, ExportFormat::Ico, "ICO");
                    ui.selectable_value(&mut dlg.format, ExportFormat::Svg, "SVG");
                });

                ui.add_space(6.0);
                ui.separator();
                ui.add_space(4.0);

                // ── Background (all formats, incl. transparent SVG) + bounds ──
                ui.horizontal(|ui| {
                    ui.label("Background");
                    ui.radio_value(
                        &mut dlg.background,
                        ExportBackground::Transparent,
                        "Transparent",
                    );
                    ui.radio_value(
                        &mut dlg.background,
                        ExportBackground::Artboard,
                        "Artboard (white)",
                    );
                });
                // Bounds/crop only applies to raster export; SVG uses the full artboard viewBox.
                if dlg.format != ExportFormat::Svg {
                    ui.horizontal(|ui| {
                        ui.label("Bounds       ");
                        ui.checkbox(&mut dlg.crop_to_content, "Crop to artwork");
                    });
                }
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                // ── Format-specific settings ──────────────────────────────
                match dlg.format {
                    ExportFormat::Png
                    | ExportFormat::Jpeg
                    | ExportFormat::WebP
                    | ExportFormat::Gif
                    | ExportFormat::Tiff => {
                        ui.horizontal(|ui| {
                            ui.label("Width ");
                            let prev_w = dlg.png_width;
                            let r = ui.add(
                                egui::DragValue::new(&mut dlg.png_width)
                                    .range(1..=8192)
                                    .suffix(" px"),
                            );
                            if r.changed() && dlg.aspect > 0.0 {
                                dlg.png_height =
                                    ((dlg.png_width as f64 / dlg.aspect) as u32).max(1);
                            }
                            let _ = prev_w;
                            ui.label("  Height ");
                            let r = ui.add(
                                egui::DragValue::new(&mut dlg.png_height)
                                    .range(1..=8192)
                                    .suffix(" px"),
                            );
                            if r.changed() && dlg.aspect > 0.0 {
                                dlg.png_width =
                                    ((dlg.png_height as f64 * dlg.aspect) as u32).max(1);
                            }
                        });
                        if dlg.format == ExportFormat::Jpeg || dlg.format == ExportFormat::WebP {
                            ui.horizontal(|ui| {
                                ui.label("Quality");
                                ui.add(
                                    egui::Slider::new(&mut dlg.jpeg_quality, 1..=100).suffix("%"),
                                );
                            });
                        }
                    }
                    ExportFormat::Ico => {
                        ui.label("Sizes");
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut dlg.ico_size_16, "16");
                            ui.checkbox(&mut dlg.ico_size_32, "32");
                            ui.checkbox(&mut dlg.ico_size_48, "48");
                            ui.checkbox(&mut dlg.ico_size_256, "256");
                        });
                    }
                    ExportFormat::Svg => {}
                }

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);

                // ── Action buttons ────────────────────────────────────────
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        action = Action::Cancel;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Export…").clicked() {
                            action = Action::Export;
                        }
                    });
                });
            });

        // X button closed the window
        if !open {
            self.export_dialog = None;
            return;
        }

        match action {
            Action::Cancel => {
                self.export_dialog = None;
            }
            Action::Export => {
                self.run_export(doc);
            }
            Action::None => {}
        }
    }

    fn draw_simplify_dialog(
        &mut self,
        ctx: &egui::Context,
        doc: &mut Document,
        history: &mut CommandHistory,
    ) {
        if self.simplify_dialog.is_none() {
            return;
        }

        #[derive(PartialEq)]
        enum Action {
            None,
            Cancel,
            Apply,
        }
        let mut action = Action::None;
        let mut open = true;

        let node_name = self.simplify_dialog.as_ref().unwrap().node_name.clone();
        let node_id = self.simplify_dialog.as_ref().unwrap().node_id;

        egui::Window::new("Simplify Path")
            .collapsible(false)
            .resizable(false)
            .fixed_size([260.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(format!("Node: {}", node_name));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Tolerance");
                    ui.add(
                        egui::DragValue::new(&mut self.simplify_dialog.as_mut().unwrap().tolerance)
                            .range(0.01..=100.0)
                            .speed(0.05)
                            .max_decimals(2),
                    );
                });
                ui.label(
                    RichText::new("Larger = more aggressive reduction")
                        .weak()
                        .small(),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        action = Action::Cancel;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Apply").clicked() {
                            action = Action::Apply;
                        }
                    });
                });
            });

        let tolerance = self
            .simplify_dialog
            .as_ref()
            .map(|d| d.tolerance)
            .unwrap_or(1.0);

        if !open {
            self.simplify_dialog = None;
            return;
        }

        match action {
            Action::None => {}
            Action::Cancel => {
                self.simplify_dialog = None;
            }
            Action::Apply => {
                self.simplify_dialog = None;
                if let Some(node) = doc.nodes.get(&node_id) {
                    if let SceneNodeKind::Path(pn) = &node.kind {
                        let simplified =
                            photonic_core::ops::simplify::simplify_path(&pn.path_data, tolerance);
                        let mut new_path = pn.clone();
                        new_path.path_data = simplified;
                        let mut new_node = node.clone();
                        new_node.kind = SceneNodeKind::Path(new_path);
                        let cmd = Command::UpdateNode {
                            old: node.clone(),
                            new: new_node,
                        };
                        history.execute(cmd, doc);
                    }
                }
            }
        }
    }

    fn draw_find_replace_text_dialog(
        &mut self,
        ctx: &egui::Context,
        doc: &mut Document,
        history: &mut CommandHistory,
    ) {
        if self.find_replace_text_dialog.is_none() {
            return;
        }

        #[derive(PartialEq)]
        enum Action {
            None,
            Cancel,
            Apply,
        }
        let mut action = Action::None;
        let mut open = true;

        egui::Window::new("Find / Replace Text")
            .collapsible(false)
            .resizable(false)
            .fixed_size([320.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                let dlg = self.find_replace_text_dialog.as_mut().unwrap();
                ui.horizontal(|ui| {
                    ui.label("Find    ");
                    ui.add(egui::TextEdit::singleline(&mut dlg.find).desired_width(f32::INFINITY));
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Replace ");
                    ui.add(
                        egui::TextEdit::singleline(&mut dlg.replace).desired_width(f32::INFINITY),
                    );
                });
                ui.add_space(6.0);
                ui.checkbox(&mut dlg.regex, "Regular expression");
                ui.checkbox(&mut dlg.case_sensitive, "Case sensitive");
                ui.checkbox(&mut dlg.selection_only, "Selection only");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        action = Action::Cancel;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Apply").clicked() {
                            action = Action::Apply;
                        }
                    });
                });
            });

        if !open {
            self.find_replace_text_dialog = None;
            return;
        }

        match action {
            Action::None => {}
            Action::Cancel => {
                self.find_replace_text_dialog = None;
            }
            Action::Apply => {
                let dlg = self.find_replace_text_dialog.take().unwrap();

                // Build regex pattern
                let pattern = if dlg.regex {
                    dlg.find.clone()
                } else {
                    regex::escape(&dlg.find)
                };
                let pattern = if dlg.case_sensitive {
                    pattern
                } else {
                    format!("(?i){}", pattern)
                };
                let re = match regex::Regex::new(&pattern) {
                    Ok(r) => r,
                    Err(_) => return,
                };

                // Collect candidates
                let candidate_ids: Vec<NodeId> = if dlg.selection_only {
                    doc.selection.ids().copied().collect()
                } else {
                    doc.nodes
                        .values()
                        .filter(|n| matches!(n.kind, SceneNodeKind::Text(_)))
                        .map(|n| n.id)
                        .collect()
                };

                let mut cmds: Vec<Command> = Vec::new();
                for id in &candidate_ids {
                    if let Some(node) = doc.nodes.get(id) {
                        if let SceneNodeKind::Text(tn) = &node.kind {
                            let new_content = re
                                .replace_all(&tn.content, dlg.replace.as_str())
                                .into_owned();
                            if new_content != tn.content {
                                let mut new_node = node.clone();
                                if let SceneNodeKind::Text(ref mut new_tn) = new_node.kind {
                                    new_tn.content = new_content;
                                }
                                cmds.push(Command::UpdateNode {
                                    old: node.clone(),
                                    new: new_node,
                                });
                            }
                        }
                    }
                }
                if !cmds.is_empty() {
                    history.execute(Command::Batch(cmds), doc);
                }
            }
        }
    }

    fn run_export(&mut self, doc: &Document) {
        let Some(dlg) = &self.export_dialog else {
            return;
        };
        let format = dlg.format;
        let opts = dlg.export_opts();
        let png_w = dlg.png_width;
        let png_h = dlg.png_height;

        let (filter_name, ext) = match format {
            ExportFormat::Png => ("PNG image", "png"),
            ExportFormat::Jpeg => ("JPEG image", "jpg"),
            ExportFormat::WebP => ("WebP image", "webp"),
            ExportFormat::Gif => ("GIF image", "gif"),
            ExportFormat::Tiff => ("TIFF image", "tiff"),
            ExportFormat::Ico => ("Icon file", "ico"),
            ExportFormat::Svg => ("SVG vector", "svg"),
        };
        let default_name = format!("{}.{ext}", doc.name);
        let start_dir = self
            .current_file
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf());
        let mut file_dialog = rfd::FileDialog::new()
            .add_filter(filter_name, &[ext])
            .set_file_name(&default_name);
        if let Some(dir) = start_dir {
            file_dialog = file_dialog.set_directory(dir);
        }
        let Some(path) = run_file_dialog(move || file_dialog.save_file()) else {
            return;
        };
        let path = if path.extension().is_none() {
            path.with_extension(ext)
        } else {
            path
        };

        let result = match format {
            ExportFormat::Svg => {
                // Honor the Background selector: Transparent => no rect,
                // Artboard => a white background rect.
                let background = match opts.background {
                    ExportBackground::Transparent => None,
                    ExportBackground::Artboard => Some(Color::WHITE),
                };
                let svg = photonic_core::export::export_svg(
                    doc,
                    &photonic_core::export::SvgExportOptions {
                        background,
                        ..Default::default()
                    },
                );
                std::fs::write(&path, svg).map_err(|e| e.to_string())
            }
            ExportFormat::Png => {
                let renderer = pollster::block_on(photonic_render::HeadlessRenderer::new());
                let bytes = renderer.render_png_with_opts(doc, png_w, png_h, &opts);
                std::fs::write(&path, bytes).map_err(|e| e.to_string())
            }
            ExportFormat::Jpeg => {
                let renderer = pollster::block_on(photonic_render::HeadlessRenderer::new());
                let bytes = renderer.render_jpeg_with_opts(doc, png_w, png_h, &opts);
                std::fs::write(&path, bytes).map_err(|e| e.to_string())
            }
            ExportFormat::WebP => {
                let renderer = pollster::block_on(photonic_render::HeadlessRenderer::new());
                let bytes = renderer.render_webp_with_opts(doc, png_w, png_h, &opts);
                std::fs::write(&path, bytes).map_err(|e| e.to_string())
            }
            ExportFormat::Gif => {
                let renderer = pollster::block_on(photonic_render::HeadlessRenderer::new());
                let bytes = renderer.render_gif_with_opts(doc, png_w, png_h, &opts);
                std::fs::write(&path, bytes).map_err(|e| e.to_string())
            }
            ExportFormat::Tiff => {
                let renderer = pollster::block_on(photonic_render::HeadlessRenderer::new());
                let bytes = renderer.render_tiff_with_opts(doc, png_w, png_h, &opts);
                std::fs::write(&path, bytes).map_err(|e| e.to_string())
            }
            ExportFormat::Ico => {
                let renderer = pollster::block_on(photonic_render::HeadlessRenderer::new());
                renderer
                    .render_ico_with_opts(doc, &opts)
                    .and_then(|b| std::fs::write(&path, b).map_err(Into::into))
                    .map_err(|e| e.to_string())
            }
        };

        self.export_dialog = None;
        self.file_status = Some(match result {
            Ok(_) => format!(
                "Exported {}",
                path.file_name().unwrap_or_default().to_string_lossy()
            ),
            Err(e) => format!("Export failed: {e}"),
        });
    }
}

/// Apply a kurbo Affine to all points in a PathData (bakes transform into path coords).
fn gui_apply_affine_to_path(path: &PathData, affine: kurbo::Affine) -> PathData {
    use kurbo::PathEl;
    let mut result = BezPath::new();
    for el in path.to_bez_path().elements() {
        let t = match *el {
            PathEl::MoveTo(p) => PathEl::MoveTo(affine * p),
            PathEl::LineTo(p) => PathEl::LineTo(affine * p),
            PathEl::CurveTo(c1, c2, p) => PathEl::CurveTo(affine * c1, affine * c2, affine * p),
            PathEl::QuadTo(c, p) => PathEl::QuadTo(affine * c, affine * p),
            PathEl::ClosePath => PathEl::ClosePath,
        };
        result.push(t);
    }
    PathData::from_bez_path(&result)
}

/// Return the topmost node (reverse draw order) whose bounding box contains (cx, cy).
fn hit_test(doc: &Document, cx: f64, cy: f64, renderer: &mut PhotonicRenderer) -> Option<NodeId> {
    for node in doc.nodes_in_draw_order().into_iter().rev() {
        if node.locked {
            continue;
        }
        if let Some((x0, y0, x1, y1)) = text_aware_canvas_bounds(node, renderer) {
            if cx >= x0 && cx <= x1 && cy >= y0 && cy <= y1 {
                return Some(node.id);
            }
        }
    }
    None
}

/// Horizontal center of a path node's bounding box in local space.
fn gui_path_center_x(node: &SceneNode) -> f32 {
    if let SceneNodeKind::Path(p) = &node.kind {
        if let Some(bb) = p.path_data.bounding_box() {
            return ((bb.x0 + bb.x1) / 2.0) as f32;
        }
    }
    0.0
}

/// Vertical center of a path node's bounding box in local space.
fn gui_path_center_y(node: &SceneNode) -> f32 {
    if let SceneNodeKind::Path(p) = &node.kind {
        if let Some(bb) = p.path_data.bounding_box() {
            return ((bb.y0 + bb.y1) / 2.0) as f32;
        }
    }
    0.0
}

/// Extract the solid fill color from a node's path fill, or None if absent.
fn gui_solid_fill_color(node: &SceneNode) -> Option<photonic_core::color::Color> {
    use photonic_core::style::FillKind;
    if let SceneNodeKind::Path(pn) = &node.kind {
        if pn.fill.enabled {
            if let FillKind::Solid(c) = pn.fill.kind {
                return Some(c);
            }
        }
    }
    None
}

/// Euclidean distance between two RGBA colors in [0,1] space.
fn gui_color_dist(a: photonic_core::color::Color, b: photonic_core::color::Color) -> f32 {
    let dr = a.r - b.r;
    let dg = a.g - b.g;
    let db = a.b - b.b;
    let da = a.a - b.a;
    (dr * dr + dg * dg + db * db + da * da).sqrt()
}

/// Snap the line endpoint `(ex, ey)` from start `(sx, sy)` to the nearest 45° angle.
/// The distance from start to the snapped end is preserved.
fn snap_line_to_45(sx: f64, sy: f64, ex: f64, ey: f64) -> (f64, f64) {
    let dx = ex - sx;
    let dy = ey - sy;
    let len = dx.hypot(dy);
    if len < 1e-6 {
        return (ex, ey);
    }
    let angle = dy.atan2(dx);
    // Round to nearest multiple of 45° (π/4 radians).
    let snapped = (angle / (std::f64::consts::PI / 4.0)).round() * (std::f64::consts::PI / 4.0);
    (sx + len * snapped.cos(), sy + len * snapped.sin())
}

/// Extract the solid fill RGBA from a node (used by the Magic Wand tool).
fn magic_wand_solid_fill(node: &SceneNode) -> Option<Color> {
    use photonic_core::style::FillKind;
    if let SceneNodeKind::Path(pn) = &node.kind {
        if pn.fill.enabled {
            if let FillKind::Solid(c) = pn.fill.kind {
                return Some(c);
            }
        }
    }
    None
}

/// Euclidean distance between two RGBA colors in [0, 1] space (Magic Wand helper).
fn magic_wand_color_dist(a: Color, b: Color) -> f32 {
    let dr = a.r - b.r;
    let dg = a.g - b.g;
    let db = a.b - b.b;
    let da = a.a - b.a;
    (dr * dr + dg * dg + db * db + da * da).sqrt()
}

/// Shared logic for ConvertToSmooth / ConvertToCorner panel actions.
fn convert_anchor_points_gui(
    smooth: bool,
    node_ids: Vec<photonic_core::node::NodeId>,
    doc: &mut Document,
    history: &mut photonic_core::history::CommandHistory,
    doc_modified: &mut bool,
) {
    let mut cmds: Vec<Command> = Vec::new();
    for nid in node_ids {
        if let Some(node) = doc.nodes.get(&nid).cloned() {
            if let SceneNodeKind::Path(ref pn) = node.kind {
                let new_path = if smooth {
                    pn.path_data.convert_to_smooth()
                } else {
                    pn.path_data.convert_to_corner()
                };
                let mut new_node = node.clone();
                if let SceneNodeKind::Path(ref mut np) = new_node.kind {
                    np.path_data = new_path;
                }
                cmds.push(Command::UpdateNode {
                    old: node,
                    new: new_node,
                });
            }
        }
    }
    if !cmds.is_empty() {
        let cmd = if cmds.len() == 1 {
            cmds.remove(0)
        } else {
            Command::Batch(cmds)
        };
        history.execute(cmd, doc);
        *doc_modified = true;
    }
}

/// Compute the world-space AABB of a node as (x0, y0, x1, y1), or None if the node
/// has no computable bounding box (e.g. groups without children).
fn node_world_aabb_opt(node: &SceneNode) -> Option<(f64, f64, f64, f64)> {
    use photonic_core::node::SceneNodeKind;
    let local_rect = match &node.kind {
        SceneNodeKind::Path(pn) => pn.path_data.bounding_box()?,
        SceneNodeKind::Text(_) => return None,
        SceneNodeKind::Group(_) => return None,
    };
    let tf = node.transform.to_kurbo();
    let corners = [
        kurbo::Point::new(local_rect.x0, local_rect.y0),
        kurbo::Point::new(local_rect.x1, local_rect.y0),
        kurbo::Point::new(local_rect.x1, local_rect.y1),
        kurbo::Point::new(local_rect.x0, local_rect.y1),
    ];
    let world: Vec<kurbo::Point> = corners.iter().map(|p| tf * *p).collect();
    let wx0 = world.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
    let wy0 = world.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
    let wx1 = world.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
    let wy1 = world.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
    Some((wx0, wy0, wx1, wy1))
}

/// Ray-casting point-in-polygon test (Jordan curve theorem).
fn lasso_point_in_polygon(px: f64, py: f64, poly: &[[f64; 2]]) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let xi = poly[i][0];
        let yi = poly[i][1];
        let xj = poly[j][0];
        let yj = poly[j][1];
        if ((yi > py) != (yj > py)) && (px < (xj - xi) * (py - yi) / (yj - yi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Create a sample 5-axis radar chart at (cx, cy) for the GUI demo button.
/// Two series: "Alpha" [80, 60, 90, 50, 70] and "Beta" [50, 80, 40, 75, 55].
fn gui_create_radar_chart_demo(
    cx: f64,
    cy: f64,
    doc: &mut Document,
    history: &mut CommandHistory,
    doc_modified: &mut bool,
) {
    use photonic_core::color::Color;
    use photonic_core::style::{Fill, FillKind};

    let radius = 100.0_f64;
    let grid_rings = 4_usize;
    let n_axes = 5_usize;
    let series_data: &[(&str, &[f64], Color)] = &[
        (
            "Alpha",
            &[80.0, 60.0, 90.0, 50.0, 70.0],
            Color::from_hex("#4E79A7").unwrap_or(Color::new(0.31, 0.47, 0.65, 1.0)),
        ),
        (
            "Beta",
            &[50.0, 80.0, 40.0, 75.0, 55.0],
            Color::from_hex("#F28E2B").unwrap_or(Color::new(0.95, 0.56, 0.17, 1.0)),
        ),
    ];

    let axis_angle = |i: usize| -> f64 {
        -std::f64::consts::FRAC_PI_2 + (i as f64 / n_axes as f64) * std::f64::consts::TAU
    };

    let layer_id = doc.active_layer_id.unwrap_or(uuid::Uuid::nil());
    let mut child_ids: Vec<uuid::Uuid> = Vec::new();

    // Grid rings
    for ring in 1..=grid_rings {
        let r = radius * (ring as f64 / grid_rings as f64);
        let mut bez = BezPath::new();
        for i in 0..n_axes {
            let angle = axis_angle(i);
            let pt = Point::new(cx + r * angle.cos(), cy + r * angle.sin());
            if i == 0 {
                bez.move_to(pt);
            } else {
                bez.line_to(pt);
            }
        }
        bez.close_path();
        let mut pn = PathNode::new(PathData::from_bez_path(&bez));
        pn.fill = Fill {
            kind: FillKind::None,
            ..Default::default()
        };
        pn.stroke = Stroke::solid(Color::new(0.7, 0.7, 0.75, 1.0), 0.75);
        let node = SceneNode::new(
            &format!("Grid Ring {ring}"),
            layer_id,
            SceneNodeKind::Path(pn),
        );
        child_ids.push(node.id);
        history.execute(
            Command::AddNode {
                node,
                layer_id: Some(layer_id),
            },
            doc,
        );
    }

    // Axis lines
    for i in 0..n_axes {
        let angle = axis_angle(i);
        let tip = Point::new(cx + radius * angle.cos(), cy + radius * angle.sin());
        let mut bez = BezPath::new();
        bez.move_to(Point::new(cx, cy));
        bez.line_to(tip);
        let mut pn = PathNode::new(PathData::from_bez_path(&bez));
        pn.fill = Fill {
            kind: FillKind::None,
            ..Default::default()
        };
        pn.stroke = Stroke::solid(Color::new(0.7, 0.7, 0.75, 1.0), 0.75);
        let node = SceneNode::new(
            &format!("Axis {}", i + 1),
            layer_id,
            SceneNodeKind::Path(pn),
        );
        child_ids.push(node.id);
        history.execute(
            Command::AddNode {
                node,
                layer_id: Some(layer_id),
            },
            doc,
        );
    }

    // Series polygons
    let axis_max = 100.0_f64; // both series scaled to 0–100
    for (name, values, color) in series_data {
        let mut bez = BezPath::new();
        for (ai, &val) in values.iter().enumerate() {
            let r = radius * (val / axis_max).clamp(0.0, 1.0);
            let angle = axis_angle(ai);
            let pt = Point::new(cx + r * angle.cos(), cy + r * angle.sin());
            if ai == 0 {
                bez.move_to(pt);
            } else {
                bez.line_to(pt);
            }
        }
        bez.close_path();
        let mut pn = PathNode::new(PathData::from_bez_path(&bez));
        pn.fill = Fill {
            kind: FillKind::Solid(Color::new(color.r, color.g, color.b, 0.2)),
            ..Default::default()
        };
        pn.stroke = Stroke::solid(*color, 1.5);
        let node = SceneNode::new(*name, layer_id, SceneNodeKind::Path(pn));
        child_ids.push(node.id);
        history.execute(
            Command::AddNode {
                node,
                layer_id: Some(layer_id),
            },
            doc,
        );
    }

    let group = SceneNode::new(
        "Radar Chart",
        layer_id,
        SceneNodeKind::Group(GroupNode::new()),
    );
    history.execute(
        Command::GroupNodes {
            group,
            layer_id,
            insert_index: 0,
            children: child_ids,
        },
        doc,
    );

    *doc_modified = true;
}

/// Create a sample 3-series stacked column chart for the GUI demo button.
fn gui_create_stacked_bar_chart_demo(
    x: f64,
    y: f64,
    doc: &mut Document,
    history: &mut CommandHistory,
    doc_modified: &mut bool,
) {
    use kurbo::Shape;
    use photonic_core::color::Color;
    use photonic_core::style::{Fill, FillKind};

    let chart_w = 300.0_f64;
    let chart_h = 200.0_f64;
    let gap_frac = 0.2_f64;
    let series_data: &[(&str, &[f64], Color)] = &[
        (
            "Alpha",
            &[40.0, 55.0, 30.0, 65.0],
            Color::from_hex("#4E79A7").unwrap_or(Color::new(0.31, 0.47, 0.65, 1.0)),
        ),
        (
            "Beta",
            &[30.0, 25.0, 45.0, 20.0],
            Color::from_hex("#F28E2B").unwrap_or(Color::new(0.95, 0.56, 0.17, 1.0)),
        ),
        (
            "Gamma",
            &[20.0, 15.0, 20.0, 10.0],
            Color::from_hex("#E15759").unwrap_or(Color::new(0.88, 0.34, 0.35, 1.0)),
        ),
    ];
    let n_stacks = 4_usize;

    let max_total = (0..n_stacks)
        .map(|ci| series_data.iter().map(|(_, vals, _)| vals[ci]).sum::<f64>())
        .fold(0.0_f64, f64::max);
    if max_total <= 0.0 {
        return;
    }

    let bar_total = chart_w / n_stacks as f64;
    let bar_w = bar_total * (1.0 - gap_frac);
    let bar_gap = bar_total * gap_frac;

    let layer_id = doc.active_layer_id.unwrap_or(uuid::Uuid::nil());
    let mut child_ids: Vec<uuid::Uuid> = Vec::new();

    for ci in 0..n_stacks {
        let bx = x + (ci as f64 * bar_total) + bar_gap / 2.0;
        let mut cursor_y = y;
        for (sname, vals, color) in series_data {
            let val = vals[ci];
            if val <= 0.0 {
                continue;
            }
            let seg_h = (val / max_total) * chart_h;
            let rect = kurbo::Rect::new(bx, cursor_y - seg_h, bx + bar_w, cursor_y);
            let mut pn = PathNode::new(PathData::from_bez_path(&rect.to_path(0.0)));
            pn.fill = Fill {
                kind: FillKind::Solid(*color),
                ..Default::default()
            };
            pn.stroke = Stroke::none();
            let node = SceneNode::new(
                format!("{sname} / Bar {}", ci + 1),
                layer_id,
                SceneNodeKind::Path(pn),
            );
            child_ids.push(node.id);
            history.execute(
                Command::AddNode {
                    node,
                    layer_id: Some(layer_id),
                },
                doc,
            );
            cursor_y -= seg_h;
        }
    }

    let group = SceneNode::new(
        "Stacked Column Chart",
        layer_id,
        SceneNodeKind::Group(GroupNode::new()),
    );
    history.execute(
        Command::GroupNodes {
            group,
            layer_id,
            insert_index: 0,
            children: child_ids,
        },
        doc,
    );

    *doc_modified = true;
}

/// Create a parametric shape demo (Lissajous / Superellipse / Rose) at canvas center.
fn gui_create_parametric_shape_demo(
    shape_type: &str,
    cx: f64,
    cy: f64,
    doc: &mut Document,
    history: &mut CommandHistory,
    doc_modified: &mut bool,
) {
    use std::f64::consts::{PI, TAU};

    let radius = 100.0_f64;
    let n_pts = 360_usize;

    let (pts, label, fill_color, stroke_color): (Vec<(f64, f64)>, &str, Color, Color) =
        match shape_type {
            "lissajous" => {
                let freq_a = 3.0_f64;
                let freq_b = 2.0_f64;
                let delta = PI / 4.0_f64;
                let pts = (0..n_pts)
                    .map(|i| {
                        let t = i as f64 / n_pts as f64 * TAU;
                        (
                            radius * (freq_a * t + delta).sin(),
                            radius * (freq_b * t).sin(),
                        )
                    })
                    .collect();
                (
                    pts,
                    "Lissajous (3:2)",
                    Color::new(0.27, 0.51, 0.71, 0.63),
                    Color::new(0.12, 0.31, 0.55, 0.86),
                )
            }
            "superellipse" => {
                let n = 2.5_f64;
                let pts = (0..n_pts)
                    .map(|i| {
                        let t = i as f64 / n_pts as f64 * TAU;
                        let cos_t = t.cos();
                        let sin_t = t.sin();
                        let x = radius * cos_t.signum() * cos_t.abs().powf(2.0 / n);
                        let y = radius * sin_t.signum() * sin_t.abs().powf(2.0 / n);
                        (x, y)
                    })
                    .collect();
                (
                    pts,
                    "Superellipse (n=2.5)",
                    Color::new(0.78, 0.39, 0.24, 0.63),
                    Color::new(0.63, 0.24, 0.08, 0.86),
                )
            }
            _ => {
                // "rose" or default
                let k = 5.0_f64;
                let t_max = PI; // odd k -> integrate over PI for a closed rose
                let pts = (0..n_pts)
                    .map(|i| {
                        let t = i as f64 / n_pts as f64 * t_max;
                        let r = radius * (k * t).cos();
                        (r * t.cos(), r * t.sin())
                    })
                    .collect();
                (
                    pts,
                    "Rose Curve (k=5)",
                    Color::new(0.78, 0.24, 0.47, 0.63),
                    Color::new(0.63, 0.08, 0.31, 0.86),
                )
            }
        };

    if pts.is_empty() {
        return;
    }

    let mut bez = BezPath::new();
    for (i, (px, py)) in pts.iter().enumerate() {
        let pt = Point::new(cx + px, cy + py);
        if i == 0 {
            bez.move_to(pt);
        } else {
            bez.line_to(pt);
        }
    }
    bez.close_path();

    let mut pn = photonic_core::node::PathNode::new(photonic_core::PathData::from_bez_path(&bez));
    pn.fill = Fill::solid(fill_color);
    pn.stroke = Stroke::solid(stroke_color, 1.5);

    let layer_id = doc.active_layer_id.unwrap_or(uuid::Uuid::nil());

    let node = SceneNode::new(label, layer_id, SceneNodeKind::Path(pn));
    history.execute(
        Command::AddNode {
            node,
            layer_id: Some(layer_id),
        },
        doc,
    );

    *doc_modified = true;
}

/// Generate a demo Truchet tiling at the given position.
fn gui_create_truchet_tiling_demo(
    style: &str,
    x: f64,
    y: f64,
    size: f64,
    doc: &mut Document,
    history: &mut CommandHistory,
    doc_modified: &mut bool,
) {
    let ts = 32.0_f64;
    let cols = (size / ts).floor() as usize;
    let rows = cols;
    if cols == 0 || rows == 0 {
        return;
    }

    let tile_color = Color::new(0.10, 0.10, 0.18, 1.0);
    let sw = 2.0_f64;

    // Simple LCG for reproducible demo pattern.
    let mut rng: u64 = 42u64
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    let mut next_bool = move || -> bool {
        rng = rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (rng >> 33) & 1 == 0
    };

    let layer_id = doc.active_layer_id.unwrap_or(uuid::Uuid::nil());
    let mut child_ids: Vec<photonic_core::node::NodeId> = Vec::new();

    for row in 0..rows {
        for col in 0..cols {
            let tx = x + col as f64 * ts;
            let ty = y + row as f64 * ts;
            let flip = next_bool();

            let mut bez = BezPath::new();

            match style {
                "triangles" => {
                    if flip {
                        bez.move_to(Point::new(tx, ty));
                        bez.line_to(Point::new(tx + ts, ty));
                        bez.line_to(Point::new(tx, ty + ts));
                    } else {
                        bez.move_to(Point::new(tx + ts, ty));
                        bez.line_to(Point::new(tx + ts, ty + ts));
                        bez.line_to(Point::new(tx, ty + ts));
                    }
                    bez.close_path();
                }
                _ => {
                    // "arcs"
                    let mid = ts / 2.0;
                    let k = mid * 0.5523;
                    if flip {
                        bez.move_to(Point::new(tx + mid, ty));
                        bez.curve_to(
                            Point::new(tx + mid - k, ty),
                            Point::new(tx, ty + mid - k),
                            Point::new(tx, ty + mid),
                        );
                        bez.move_to(Point::new(tx + mid, ty + ts));
                        bez.curve_to(
                            Point::new(tx + mid + k, ty + ts),
                            Point::new(tx + ts, ty + mid + k),
                            Point::new(tx + ts, ty + mid),
                        );
                    } else {
                        bez.move_to(Point::new(tx + mid, ty));
                        bez.curve_to(
                            Point::new(tx + mid + k, ty),
                            Point::new(tx + ts, ty + mid - k),
                            Point::new(tx + ts, ty + mid),
                        );
                        bez.move_to(Point::new(tx + mid, ty + ts));
                        bez.curve_to(
                            Point::new(tx + mid - k, ty + ts),
                            Point::new(tx, ty + mid + k),
                            Point::new(tx, ty + mid),
                        );
                    }
                }
            }

            let mut pn =
                photonic_core::node::PathNode::new(photonic_core::PathData::from_bez_path(&bez));
            if style == "triangles" {
                pn.fill = Fill::solid(tile_color);
                pn.stroke = Stroke::none();
            } else {
                pn.fill = Fill::none();
                pn.stroke = Stroke::solid(tile_color, sw);
            }

            let node = SceneNode::new(&format!("t{row}_{col}"), layer_id, SceneNodeKind::Path(pn));
            let nid = node.id;
            history.execute(
                Command::AddNode {
                    node,
                    layer_id: Some(layer_id),
                },
                doc,
            );
            child_ids.push(nid);
        }
    }

    let label = format!("Truchet {style} {cols}×{rows}");
    let group = SceneNode::new(&label, layer_id, SceneNodeKind::Group(GroupNode::new()));
    history.execute(
        Command::GroupNodes {
            group,
            layer_id,
            insert_index: 0,
            children: child_ids,
        },
        doc,
    );

    *doc_modified = true;
}
