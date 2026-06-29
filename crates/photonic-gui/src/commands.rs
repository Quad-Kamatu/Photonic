//! Command registry + key bindings for customizable keyboard shortcuts and the
//! searchable command palette (Ctrl/Cmd+K).
//!
//! Every user-facing editor action that can carry a keyboard shortcut is given a
//! stable [`CommandId`] (`&'static str`) and a default [`KeyBinding`] here. The
//! user's overrides live in `AppPreferences::keymap` (a `HashMap<String,
//! KeyBinding>` keyed by command id); `AppPreferences::resolve_binding` layers
//! the user map over these registry defaults. Tool activations are surfaced in
//! the palette too via [`TOOL_COMMANDS`].
//!
//! A `KeyBinding` serializes to/from a compact string like `"ctrl+shift+g"` so
//! the keymap round-trips through the JSON preferences file as a plain object.

use crate::Tool;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Stable identifier for a command. Used as the keymap key and palette id.
pub type CommandId = &'static str;

/// A single keyboard shortcut: a key plus modifier flags. `ctrl` and `command`
/// are both treated as the "primary" modifier so a binding works on Linux/Windows
/// (Ctrl) and macOS (Cmd) without per-platform tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyBinding {
    pub key: egui::Key,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub command: bool,
}

impl KeyBinding {
    /// A primary-modifier (Ctrl/Cmd) + key binding, e.g. Ctrl+Z.
    pub const fn ctrl(key: egui::Key) -> Self {
        Self { key, ctrl: true, shift: false, alt: false, command: false }
    }
    /// Ctrl/Cmd + Shift + key, e.g. Ctrl+Shift+G.
    pub const fn ctrl_shift(key: egui::Key) -> Self {
        Self { key, ctrl: true, shift: true, alt: false, command: false }
    }
    /// A bare key with no modifiers, e.g. Delete.
    pub const fn plain(key: egui::Key) -> Self {
        Self { key, ctrl: false, shift: false, alt: false, command: false }
    }

    /// True if this binding fires for the given live modifier state. Ctrl and Cmd
    /// are interchangeable (primary). Shift/Alt must match exactly.
    pub fn matches(&self, m: egui::Modifiers) -> bool {
        let want_primary = self.ctrl || self.command;
        let have_primary = m.ctrl || m.command || m.mac_cmd;
        want_primary == have_primary && self.shift == m.shift && self.alt == m.alt
    }

    /// Storage form, e.g. `"ctrl+shift+g"`. Lower-cased; key uses egui's name.
    pub fn to_storage_string(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.ctrl || self.command {
            parts.push("ctrl".to_string());
        }
        if self.shift {
            parts.push("shift".to_string());
        }
        if self.alt {
            parts.push("alt".to_string());
        }
        parts.push(self.key.name().to_ascii_lowercase());
        parts.join("+")
    }

    /// Parse the storage form back into a binding. Case-insensitive.
    pub fn parse(s: &str) -> Option<Self> {
        let mut ctrl = false;
        let mut shift = false;
        let mut alt = false;
        let mut key: Option<egui::Key> = None;
        for tok in s.split('+') {
            let t = tok.trim();
            if t.is_empty() {
                continue;
            }
            match t.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "cmd" | "command" | "super" | "meta" => ctrl = true,
                "shift" => shift = true,
                "alt" | "option" | "opt" => alt = true,
                _ => {
                    let found = egui::Key::ALL
                        .iter()
                        .copied()
                        .find(|k| k.name().eq_ignore_ascii_case(t))
                        .or_else(|| egui::Key::from_name(t));
                    key = key.or(found);
                }
            }
        }
        Some(Self { key: key?, ctrl, shift, alt, command: false })
    }

    /// Human-readable label for the UI, e.g. `"Ctrl+Shift+["`.
    pub fn display(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.ctrl || self.command {
            parts.push("Ctrl".to_string());
        }
        if self.shift {
            parts.push("Shift".to_string());
        }
        if self.alt {
            parts.push("Alt".to_string());
        }
        parts.push(display_key(self.key).to_string());
        parts.join("+")
    }
}

/// Friendlier glyphs for keys whose egui name is verbose.
fn display_key(k: egui::Key) -> &'static str {
    match k {
        egui::Key::OpenBracket => "[",
        egui::Key::CloseBracket => "]",
        egui::Key::Semicolon => ";",
        egui::Key::Plus => "+",
        egui::Key::Minus => "-",
        egui::Key::Equals => "=",
        egui::Key::Comma => ",",
        egui::Key::Period => ".",
        egui::Key::Slash => "/",
        egui::Key::Backslash => "\\",
        _ => k.name(),
    }
}

impl Serialize for KeyBinding {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_storage_string())
    }
}

impl<'de> Deserialize<'de> for KeyBinding {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        KeyBinding::parse(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid key binding: {s:?}")))
    }
}

/// A registered command: a stable id, a human label, and an optional default
/// shortcut. `default == None` means "no default key" (still palette-reachable).
pub struct CommandDef {
    pub id: CommandId,
    pub label: &'static str,
    pub default: Option<KeyBinding>,
}

use egui::Key;

/// Every shortcut-bearing editor action. Ids are stable and used as keymap keys.
pub static REGISTRY: &[CommandDef] = &[
    // ── Edit ──────────────────────────────────────────────────────────────
    CommandDef { id: "edit.undo", label: "Undo", default: Some(KeyBinding::ctrl(Key::Z)) },
    CommandDef { id: "edit.redo", label: "Redo", default: Some(KeyBinding::ctrl(Key::R)) },
    CommandDef { id: "edit.copy", label: "Copy", default: Some(KeyBinding::ctrl(Key::C)) },
    CommandDef { id: "edit.paste", label: "Paste", default: Some(KeyBinding::ctrl(Key::V)) },
    CommandDef {
        id: "edit.paste_in_place",
        label: "Paste in Place",
        default: Some(KeyBinding::ctrl_shift(Key::V)),
    },
    CommandDef {
        id: "edit.duplicate",
        label: "Duplicate",
        default: Some(KeyBinding::ctrl(Key::D)),
    },
    CommandDef {
        id: "edit.delete",
        label: "Delete Selection",
        default: Some(KeyBinding::plain(Key::Delete)),
    },
    // ── Selection ─────────────────────────────────────────────────────────
    CommandDef {
        id: "selection.select_all",
        label: "Select All",
        default: Some(KeyBinding::ctrl(Key::A)),
    },
    CommandDef {
        id: "selection.deselect",
        label: "Deselect All",
        default: Some(KeyBinding::ctrl_shift(Key::A)),
    },
    // ── Object / arrange ──────────────────────────────────────────────────
    CommandDef { id: "object.group", label: "Group", default: Some(KeyBinding::ctrl(Key::G)) },
    CommandDef {
        id: "object.ungroup",
        label: "Ungroup",
        default: Some(KeyBinding::ctrl_shift(Key::G)),
    },
    CommandDef {
        id: "object.bring_forward",
        label: "Bring Forward",
        default: Some(KeyBinding::ctrl(Key::CloseBracket)),
    },
    CommandDef {
        id: "object.send_backward",
        label: "Send Backward",
        default: Some(KeyBinding::ctrl(Key::OpenBracket)),
    },
    CommandDef {
        id: "object.bring_to_front",
        label: "Bring to Front",
        default: Some(KeyBinding::ctrl_shift(Key::CloseBracket)),
    },
    CommandDef {
        id: "object.send_to_back",
        label: "Send to Back",
        default: Some(KeyBinding::ctrl_shift(Key::OpenBracket)),
    },
    CommandDef {
        id: "object.flip_horizontal",
        label: "Flip Horizontal",
        default: Some(KeyBinding::ctrl_shift(Key::H)),
    },
    CommandDef {
        id: "object.flip_vertical",
        label: "Flip Vertical",
        default: Some(KeyBinding::ctrl_shift(Key::J)),
    },
    // ── View ──────────────────────────────────────────────────────────────
    CommandDef {
        id: "view.outline_mode",
        label: "Toggle Outline Mode",
        default: Some(KeyBinding::ctrl(Key::Y)),
    },
    CommandDef {
        id: "view.toggle_guides",
        label: "Toggle Guides",
        default: Some(KeyBinding::ctrl(Key::Semicolon)),
    },
    CommandDef { id: "view.toggle_grid", label: "Toggle Grid", default: None },
    CommandDef { id: "view.fit", label: "Fit to View", default: None },
    CommandDef { id: "view.toggle_audit", label: "Toggle Audit Log", default: None },
    // ── Palette ───────────────────────────────────────────────────────────
    CommandDef {
        id: "palette.open",
        label: "Open Command Palette",
        default: Some(KeyBinding::ctrl(Key::K)),
    },
];

/// Tool-activation commands surfaced in the palette. Labels come from
/// `Tool::label()` so they never drift from the toolbar.
pub static TOOL_COMMANDS: &[(CommandId, Tool)] = &[
    ("tool.select", Tool::Select),
    ("tool.direct_select", Tool::DirectSelect),
    ("tool.pan", Tool::Pan),
    ("tool.rectangle", Tool::Rectangle),
    ("tool.rounded_rect", Tool::RoundedRect),
    ("tool.ellipse", Tool::Ellipse),
    ("tool.polygon", Tool::Polygon),
    ("tool.star", Tool::Star),
    ("tool.spiral", Tool::Spiral),
    ("tool.line", Tool::Line),
    ("tool.arc", Tool::Arc),
    ("tool.grid", Tool::Grid),
    ("tool.polar_grid", Tool::PolarGrid),
    ("tool.pen", Tool::Pen),
    ("tool.shape_builder", Tool::ShapeBuilder),
    ("tool.text", Tool::Text),
    ("tool.scissors", Tool::Scissors),
    ("tool.knife", Tool::Knife),
    ("tool.eraser", Tool::Eraser),
    ("tool.magic_wand", Tool::MagicWand),
    ("tool.lasso", Tool::Lasso),
    ("tool.pencil", Tool::Pencil),
    ("tool.smooth", Tool::Smooth),
    ("tool.width", Tool::Width),
    ("tool.raster_brush", Tool::RasterBrush),
    ("tool.raster_eraser", Tool::RasterEraser),
];

/// Resolve a tool-activation command id to its [`Tool`].
pub fn tool_for_command(id: &str) -> Option<Tool> {
    TOOL_COMMANDS
        .iter()
        .find(|(cid, _)| *cid == id)
        .map(|(_, t)| *t)
}

/// The registry default binding for a command (ignores user overrides).
pub fn default_binding(id: &str) -> Option<KeyBinding> {
    REGISTRY
        .iter()
        .find(|d| d.id == id)
        .and_then(|d| d.default)
}

/// A flattened command for the palette + settings list (core + tool commands).
pub struct CommandEntry {
    pub id: CommandId,
    pub label: String,
    /// `true` for tool-activation entries (no remappable default binding).
    pub is_tool: bool,
}

/// All commands the palette can list and run: registry commands first, then
/// tool activations.
pub fn all_commands() -> Vec<CommandEntry> {
    let mut v: Vec<CommandEntry> = REGISTRY
        .iter()
        .map(|d| CommandEntry { id: d.id, label: d.label.to_string(), is_tool: false })
        .collect();
    for (id, t) in TOOL_COMMANDS {
        v.push(CommandEntry { id, label: format!("Tool: {}", t.label()), is_tool: true });
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_binding_roundtrips_through_string() {
        let cases = [
            KeyBinding::ctrl(Key::Z),
            KeyBinding::ctrl_shift(Key::G),
            KeyBinding::plain(Key::Delete),
            KeyBinding::ctrl(Key::OpenBracket),
            KeyBinding::ctrl_shift(Key::CloseBracket),
            KeyBinding::ctrl(Key::Semicolon),
        ];
        for b in cases {
            let s = b.to_storage_string();
            let back = KeyBinding::parse(&s).expect("parse");
            assert_eq!(b, back, "round-trip failed for {s}");
        }
    }

    #[test]
    fn storage_string_is_lowercase_plus_joined() {
        assert_eq!(KeyBinding::ctrl_shift(Key::G).to_storage_string(), "ctrl+shift+g");
        assert_eq!(KeyBinding::plain(Key::Delete).to_storage_string(), "delete");
    }

    #[test]
    fn serde_roundtrip_as_string() {
        let b = KeyBinding::ctrl(Key::K);
        let json = serde_json::to_string(&b).unwrap();
        assert_eq!(json, "\"ctrl+k\"");
        let back: KeyBinding = serde_json::from_str(&json).unwrap();
        assert_eq!(b, back);
    }

    #[test]
    fn registry_ids_are_unique() {
        let mut ids: Vec<&str> = REGISTRY.iter().map(|d| d.id).collect();
        ids.extend(TOOL_COMMANDS.iter().map(|(id, _)| *id));
        let mut seen = std::collections::HashSet::new();
        for id in ids {
            assert!(seen.insert(id), "duplicate command id: {id}");
        }
    }

    #[test]
    fn matches_distinguishes_shift() {
        let z = KeyBinding::ctrl(Key::Z);
        let plain = egui::Modifiers { ctrl: true, ..Default::default() };
        let with_shift = egui::Modifiers { ctrl: true, shift: true, ..Default::default() };
        assert!(z.matches(plain));
        assert!(!z.matches(with_shift));
    }
}
