use crate::commands::KeyBinding;
use crate::tools::Tool;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppPreferences {
    // APPEARANCE
    pub dark_mode: bool,
    pub ui_scale: f32, // 0.75, 1.0, 1.25, 1.5, 2.0

    // CANVAS
    pub show_grid: bool,
    pub grid_size: u32, // 8, 16, 32, 64
    pub snap_to_grid: bool,
    pub grid_color: [f32; 4], // RGBA as f32 (matches egui color picker API)
    pub show_rulers: bool,
    /// Object-aware snapping: align a dragged node's edges/centers to nearby
    /// nodes during a move drag (#66). Additive with `snap_to_grid`.
    #[serde(default = "default_true")]
    pub snap_to_objects: bool,
    /// Snap pull radius in screen pixels (converted to canvas units via zoom).
    #[serde(default = "default_snap_tolerance")]
    pub snap_tolerance_px: f32,
    /// Draw the dashed smart-guide lines + distance labels while snapping.
    #[serde(default = "default_true")]
    pub snap_show_guides: bool,
    /// Measurement unit used for ruler labels and the live cursor readout.
    #[serde(default)]
    pub document_units: photonic_core::DocumentUnit,

    // TOOL DEFAULTS
    pub default_fill_color: [f32; 4],
    pub default_stroke_enabled: bool,
    pub default_stroke_color: [f32; 4],
    pub default_stroke_width: f32,

    // BEHAVIOR
    pub console_open_on_start: bool,
    /// Arrow-key nudge distance in document pixels (Shift multiplies by 10).
    #[serde(default = "default_nudge_distance")]
    pub nudge_distance: f64,

    // HISTORY — bound on the project undo/redo history persisted in the .photon
    // file. The user picks the unit: a step count, or a serialized-size budget
    // (in MB) applied to the history payload specifically (separate from the
    // document's own size). Once the history exceeds the cap, the oldest steps
    // are discarded to make room (with a warning the first time).
    #[serde(default)]
    pub history_limit_mode: HistoryLimitMode,
    /// Max retained undo steps when `history_limit_mode == Steps`.
    #[serde(default = "default_history_max_steps")]
    pub history_max_steps: usize,
    /// Max serialized history size in MB when `history_limit_mode == Size`.
    #[serde(default = "default_history_max_mb")]
    pub history_max_mb: f64,
    /// Check GitHub for a newer release once on launch and prompt if available.
    #[serde(default = "default_true")]
    pub auto_check_updates: bool,
    /// Last app version this user actually ran. Drives the "What's New" popup:
    /// when it differs from the current build, show notes for the gap. Empty on
    /// a fresh install (no popup the very first time).
    #[serde(default)]
    pub last_seen_version: String,

    // HOTBAR — tools pinned to the sidebar by the user
    #[serde(default)]
    pub pinned_tools: Vec<Tool>,

    // KEYBOARD — user shortcut overrides, keyed by `commands::CommandId`.
    // Empty by default (every command uses its registry default). User remaps in
    // the Keyboard Shortcuts settings page populate this and persist to disk.
    #[serde(default)]
    pub keymap: HashMap<String, KeyBinding>,
}

fn default_nudge_distance() -> f64 {
    1.0
}

/// How the project-history retention limit is measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum HistoryLimitMode {
    /// Cap by number of undo steps.
    #[default]
    Steps,
    /// Cap by serialized size of the history payload (MB).
    Size,
}

/// Hard ceiling applied in Size mode so memory stays bounded regardless of how
/// large the byte budget is. The size cap does the real trimming; this just
/// prevents an unbounded step count.
pub const HISTORY_SIZE_MODE_STEP_CEILING: usize = 100_000;

fn default_history_max_steps() -> usize {
    200
}

fn default_history_max_mb() -> f64 {
    50.0
}

fn default_true() -> bool {
    true
}

fn default_snap_tolerance() -> f32 {
    6.0
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            dark_mode: true,
            ui_scale: 1.0,
            show_grid: false,
            grid_size: 16,
            snap_to_grid: false,
            grid_color: [0.31, 0.31, 0.47, 0.24], // muted violet, semi-transparent
            show_rulers: false,
            snap_to_objects: true,
            snap_tolerance_px: 6.0,
            snap_show_guides: true,
            document_units: photonic_core::DocumentUnit::Px,
            default_fill_color: [0.22, 0.47, 0.87, 1.0],
            default_stroke_enabled: false,
            default_stroke_color: [0.0, 0.0, 0.0, 1.0],
            default_stroke_width: 1.0,
            console_open_on_start: false,
            nudge_distance: 1.0,
            history_limit_mode: HistoryLimitMode::Steps,
            history_max_steps: 200,
            history_max_mb: 50.0,
            auto_check_updates: true,
            last_seen_version: String::new(),
            pinned_tools: Vec::new(),
            keymap: HashMap::new(),
        }
    }
}

impl AppPreferences {
    /// The active binding for a command: the user override if present, otherwise
    /// the registry default. `None` means the command has no shortcut.
    pub fn resolve_binding(&self, id: &str) -> Option<KeyBinding> {
        if let Some(b) = self.keymap.get(id) {
            return Some(*b);
        }
        crate::commands::default_binding(id)
    }

    /// Any other command whose *resolved* binding equals `binding`, excluding
    /// `for_id`. Used for conflict warnings in the Keyboard Shortcuts UI.
    pub fn binding_conflict(&self, for_id: &str, binding: KeyBinding) -> Option<String> {
        for def in crate::commands::REGISTRY {
            if def.id == for_id {
                continue;
            }
            if self.resolve_binding(def.id) == Some(binding) {
                return Some(def.label.to_string());
            }
        }
        None
    }

    /// Resolve the configured history retention limits as
    /// `(max_steps, size_limit_bytes)` for [`photonic_core::CommandHistory::set_limits`].
    /// In Steps mode the size cap is `None`; in Size mode a high step ceiling
    /// keeps memory bounded while the byte budget does the trimming.
    pub fn history_limits(&self) -> (usize, Option<u64>) {
        match self.history_limit_mode {
            HistoryLimitMode::Steps => (self.history_max_steps.max(1), None),
            HistoryLimitMode::Size => {
                let bytes = (self.history_max_mb.max(0.1) * 1_048_576.0) as u64;
                (HISTORY_SIZE_MODE_STEP_CEILING, Some(bytes))
            }
        }
    }

    fn prefs_path() -> Option<std::path::PathBuf> {
        crate::welcome::config_dir().map(|d| d.join("preferences.json"))
    }

    /// Load from disk, falling back to Default on any error.
    pub fn load() -> Self {
        let path = match Self::prefs_path() {
            Some(p) => p,
            None => return Self::default(),
        };
        let json = match std::fs::read_to_string(&path) {
            Ok(j) => j,
            Err(_) => return Self::default(),
        };
        serde_json::from_str(&json).unwrap_or_default()
    }

    /// Serialize and write to disk, silently ignoring errors.
    pub fn save(&self) {
        let Some(path) = Self::prefs_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(&path, json);
        }
    }
}
