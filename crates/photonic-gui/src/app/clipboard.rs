//! Native clipboard paste support for raster images and editable SVG content.
//!
//! The window host reads the platform clipboard because `egui-winit` only
//! exposes a text paste event. This module turns that payload into the same
//! undoable scene commands used by the rest of the GUI.

use super::*;

/// Clipboard data captured by the native window host.
#[derive(Debug)]
pub enum NativeClipboardPaste {
    /// Straight-alpha RGBA8 pixels from the platform clipboard.
    Image {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    /// Plain text or HTML. SVG fragments are imported as editable scene nodes;
    /// other text is ignored when the canvas has focus.
    Text(String),
}

/// Keep accidental or maliciously large clipboard images from allocating an
/// unbounded raster node while the user is pasting.
const MAX_CLIPBOARD_PIXELS: u64 = 64_000_000;

impl PhotonicApp {
    /// Paste a native clipboard payload into the active layer as one undoable
    /// operation. Returns true only when the document was changed.
    pub(crate) fn paste_native_clipboard(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
        payload: NativeClipboardPaste,
        paste_in_place: bool,
    ) -> bool {
        match payload {
            NativeClipboardPaste::Image {
                width,
                height,
                rgba,
            } => self.paste_clipboard_image(doc, history, width, height, rgba),
            NativeClipboardPaste::Text(text) => {
                if text.trim() == INTERNAL_OBJECT_CLIPBOARD_MARKER {
                    self.paste_gui_clipboard(doc, history, paste_in_place)
                } else {
                    self.paste_clipboard_svg(doc, history, &text, paste_in_place)
                }
            }
        }
    }

    /// Paste the in-process Photonic object snapshot. Keeping this helper next
    /// to native paste makes both clipboard sources share the same shortcut
    /// path and selection behavior.
    pub(crate) fn paste_gui_clipboard(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
        paste_in_place: bool,
    ) -> bool {
        let Some(target_layer) = target_layer(doc) else {
            return false;
        };
        let offset = if paste_in_place { 0.0 } else { 10.0 };
        let Some((cmd, new_ids)) = self
            .gui_clipboard
            .paste_command(target_layer, offset, offset)
        else {
            return false;
        };

        history.execute(cmd, doc);
        select_roots(self, doc, &new_ids);
        true
    }

    fn paste_clipboard_image(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    ) -> bool {
        if target_layer(doc).is_none() {
            self.file_status = Some("Paste blocked: the active layer is locked".into());
            return false;
        }
        let pixels = u64::from(width).checked_mul(u64::from(height));
        let Some(pixels) = pixels.filter(|pixels| *pixels > 0 && *pixels <= MAX_CLIPBOARD_PIXELS)
        else {
            self.file_status = Some("Paste image failed: clipboard image is too large".into());
            return false;
        };
        let expected_len = pixels
            .checked_mul(4)
            .and_then(|len| usize::try_from(len).ok());
        if expected_len != Some(rgba.len()) {
            self.file_status = Some("Paste image failed: invalid clipboard pixel data".into());
            return false;
        }

        let image = match photonic_core::raster::image::RasterImage::from_rgba(width, height, rgba)
        {
            Ok(image) => image,
            Err(error) => {
                self.file_status = Some(format!("Paste image failed: {error}"));
                return false;
            }
        };

        let mut node = SceneNode::new(
            "Pasted Image",
            uuid::Uuid::nil(),
            SceneNodeKind::Raster(photonic_core::node::RasterNode::new(image)),
        );
        node.transform = photonic_core::Transform::translate(
            (doc.width - width as f64) / 2.0,
            (doc.height - height as f64) / 2.0,
        );
        let node_id = node.id;
        history.execute(
            Command::AddNode {
                node,
                layer_id: None,
            },
            doc,
        );
        doc.selection = Selection::single(node_id);
        self.selected_id = Some(node_id);
        self.file_status = Some(format!("Pasted image ({width}×{height})"));
        true
    }

    fn paste_clipboard_svg(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
        text: &str,
        paste_in_place: bool,
    ) -> bool {
        let Some(svg) = extract_svg_fragment(text) else {
            return false;
        };
        let imported = match photonic_core::import_svg(svg) {
            Ok(document) => document,
            Err(error) => {
                self.file_status = Some(format!("Paste vector failed: {error}"));
                return false;
            }
        };
        let Some(target_layer) = target_layer(doc) else {
            return false;
        };
        let Some(source_layer) = imported
            .active_layer_id
            .or_else(|| imported.layer_order.first().copied())
        else {
            self.file_status = Some("Paste vector failed: SVG has no layer".into());
            return false;
        };
        let Some(source_roots) = imported
            .layers
            .get(&source_layer)
            .map(|layer| layer.node_ids.clone())
        else {
            self.file_status = Some("Paste vector failed: SVG has no layer".into());
            return false;
        };
        if source_roots.is_empty() {
            self.file_status = Some("Paste vector failed: SVG has no visible objects".into());
            return false;
        }

        let offset = if paste_in_place { 0.0 } else { 10.0 };
        let dx = (doc.width - imported.width) / 2.0 + offset;
        let dy = (doc.height - imported.height) / 2.0 + offset;
        let (new_roots, nodes) = photonic_core::ops::cloning::clone_subtrees(
            &imported.nodes,
            &source_roots,
            target_layer,
            dx,
            dy,
        );
        if new_roots.is_empty() {
            self.file_status = Some("Paste vector failed: SVG has no importable objects".into());
            return false;
        }

        history.execute(
            Command::AddSubtree {
                layer_id: target_layer,
                roots: new_roots.clone(),
                nodes,
            },
            doc,
        );
        select_roots(self, doc, &new_roots);
        let object_label = if new_roots.len() == 1 {
            "object"
        } else {
            "objects"
        };
        self.file_status = Some(format!(
            "Pasted vector ({} {object_label})",
            new_roots.len()
        ));
        true
    }
}

fn target_layer(doc: &Document) -> Option<LayerId> {
    let layer_id = doc
        .active_layer_id
        .or_else(|| doc.layer_order.first().copied())?;
    (!doc.is_layer_locked(&layer_id)).then_some(layer_id)
}

fn select_roots(app: &mut PhotonicApp, doc: &mut Document, roots: &[NodeId]) {
    doc.selection = Selection::from_ids(roots.iter().copied());
    app.selected_id = roots.first().copied();
}

/// Return an SVG root from either raw SVG/XML text or an HTML clipboard
/// wrapper. Clipboard providers commonly include fragment comments and a
/// surrounding `<body>`, so looking for the root pair is more reliable than
/// requiring the whole payload to be an XML document.
fn extract_svg_fragment(text: &str) -> Option<&str> {
    let text = text.trim().trim_start_matches('\u{feff}').trim();
    if text.is_empty() {
        return None;
    }

    let lower = text.to_ascii_lowercase();
    let mut search_from = 0;
    let start = loop {
        let relative = lower[search_from..].find("<svg")?;
        let candidate = search_from + relative;
        let next = lower.as_bytes().get(candidate + 4).copied();
        if matches!(
            next,
            Some(b' ' | b'\t' | b'\r' | b'\n' | b'>' | b'/' | b':')
        ) {
            break candidate;
        }
        search_from = candidate + 4;
    };

    let close_start = start + lower[start..].find("</svg")?;
    let close_end = close_start + lower[close_start..].find('>')? + 1;
    Some(&text[start..close_end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use photonic_core::history::CommandHistory;

    const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="30"><rect x="1" y="2" width="10" height="12" fill="#ff0000"/></svg>"##;

    #[test]
    fn extracts_svg_from_html_wrapper() {
        let html = format!("<html><body><!--StartFragment-->{SVG}<!--EndFragment--></body></html>");
        assert_eq!(extract_svg_fragment(&html), Some(SVG));
    }

    #[test]
    fn pastes_svg_as_editable_nodes_and_undoes_as_one_step() {
        let mut app = PhotonicApp::default();
        let mut doc = Document::new("test", 100.0, 100.0);
        let mut history = CommandHistory::new(200);

        assert!(app.paste_native_clipboard(
            &mut doc,
            &mut history,
            NativeClipboardPaste::Text(SVG.into()),
            false,
        ));
        assert_eq!(doc.nodes.len(), 1);
        let pasted_id = app.selected_id.expect("pasted vector is selected");
        assert!(matches!(
            doc.nodes.get(&pasted_id).map(|node| &node.kind),
            Some(SceneNodeKind::Path(_))
        ));
        assert_eq!(history.undo_depth(), 1);

        assert!(history.undo(&mut doc));
        assert!(doc.nodes.is_empty());
    }

    #[test]
    fn pastes_rgba_clipboard_as_raster() {
        let mut app = PhotonicApp::default();
        let mut doc = Document::new("test", 100.0, 100.0);
        let mut history = CommandHistory::new(200);

        assert!(app.paste_native_clipboard(
            &mut doc,
            &mut history,
            NativeClipboardPaste::Image {
                width: 2,
                height: 1,
                rgba: vec![255, 0, 0, 255, 0, 255, 0, 128],
            },
            false,
        ));
        let pasted_id = app.selected_id.expect("pasted image is selected");
        let Some(SceneNode {
            kind: SceneNodeKind::Raster(raster),
            ..
        }) = doc.nodes.get(&pasted_id)
        else {
            panic!("expected a raster node");
        };
        assert_eq!(raster.image.width, 2);
        assert_eq!(raster.image.pixels, vec![255, 0, 0, 255, 0, 255, 0, 128]);
    }

    #[test]
    fn ignores_non_svg_text() {
        let mut app = PhotonicApp::default();
        let mut doc = Document::new("test", 100.0, 100.0);
        let mut history = CommandHistory::new(200);

        assert!(!app.paste_native_clipboard(
            &mut doc,
            &mut history,
            NativeClipboardPaste::Text("ordinary text".into()),
            false,
        ));
        assert!(doc.nodes.is_empty());
        assert_eq!(history.undo_depth(), 0);
    }
}
