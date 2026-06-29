use crate::tools::Tool;
use serde::{Deserialize, Serialize};

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
    /// Check GitHub for a newer release once on launch and prompt if available.
    #[serde(default = "default_true")]
    pub auto_check_updates: bool,

    // HOTBAR — tools pinned to the sidebar by the user
    #[serde(default)]
    pub pinned_tools: Vec<Tool>,
}

fn default_nudge_distance() -> f64 {
    1.0
}

fn default_true() -> bool {
    true
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
            default_fill_color: [0.22, 0.47, 0.87, 1.0],
            default_stroke_enabled: false,
            default_stroke_color: [0.0, 0.0, 0.0, 1.0],
            default_stroke_width: 1.0,
            console_open_on_start: false,
            nudge_distance: 1.0,
            auto_check_updates: true,
            pinned_tools: Vec::new(),
        }
    }
}

impl AppPreferences {
    fn prefs_path() -> Option<std::path::PathBuf> {
        let appdata = std::env::var("APPDATA").ok()?;
        Some(
            std::path::Path::new(&appdata)
                .join("Photonic")
                .join("preferences.json"),
        )
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
