//! Headless text rasterization (TD-018).
//!
//! Both headless output paths — the pure-GPU `render_rgba_with_opts` fast path
//! and the CPU `composite_document` path — historically dropped every `Text`
//! node, so PNG/JPEG/… export and MCP renders lost all text. This drives the
//! **same glyphon pipeline the interactive canvas uses** against the headless
//! device, so headless text matches the canvas pixel-for-pixel (the registry's
//! preferred approach).
//!
//! Text is composited **on top** of the vector/raster result, matching the
//! windowed renderer (which draws its glyphon pass last, over the geometry) —
//! headless z-order for text is "always on top", the same simplification the
//! canvas makes. Flat text only; **text-on-path** (`path_spine_id`) is the
//! separate TD-019 gap and is intentionally skipped here (those nodes stay
//! unrendered headlessly until TD-019 lands).
//!
//! `HeadlessText` holds glyphon's `&mut`-only state (font system, atlas, …) and
//! lives behind a `Mutex` on `HeadlessRenderer` because that renderer is shared
//! through `Arc` (script/REPL/MCP) and rendered through `&self`.

use glyphon::{
    Attrs, Buffer, Cache, Color as GlyphonColor, Family, FontSystem, Metrics, Resolution, Shaping,
    Style as GlyphonStyle, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
    Weight,
};
use photonic_core::{node::SceneNodeKind, style::FillKind, Document};

use crate::canvas::CanvasView;

/// The headless colour target format (mirrors `headless::FORMAT`).
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// One flat text node, in screen space — the headless twin of the windowed
/// renderer's `TextSnapshot`, computed identically (mod.rs `build_geometry`).
pub(crate) struct TextSnapshot {
    content: String,
    font_family: String,
    /// Font size in physical pixels (already scaled by view zoom + script size).
    font_size: f32,
    line_height_mul: f32,
    font_weight: u16,
    /// 0 = Normal, 1 = Italic, 2 = Oblique.
    font_style: u8,
    color: [u8; 4],
    screen_x: f32,
    screen_y: f32,
    top_offset: f32,
}

/// Collect flat text nodes from `doc` in draw order, projected through `view` —
/// the same computation as the windowed `build_geometry` text arm. Skips
/// text-on-path nodes (TD-019).
pub(crate) fn collect_text_snapshots(doc: &Document, view: &CanvasView) -> Vec<TextSnapshot> {
    use photonic_core::layer::BlendMode;

    // Fold each non-opaque/non-Normal layer's opacity onto its nodes (matches
    // the windowed renderer so text alpha agrees with the canvas).
    let mut layer_opacity: std::collections::HashMap<photonic_core::node::NodeId, f32> =
        std::collections::HashMap::new();
    for lid in &doc.layer_order {
        if let Some(layer) = doc.layers.get(lid) {
            if (layer.opacity - 1.0).abs() > f32::EPSILON || layer.blend_mode != BlendMode::Normal {
                for n in doc.draw_nodes_in_layer(lid) {
                    layer_opacity.insert(n.id, layer.opacity);
                }
            }
        }
    }

    let mut out = Vec::new();
    for node in doc.nodes_in_draw_order() {
        let orig_id = node.id;
        let resolved = doc.resolve_render_node(node);
        let node = resolved.as_ref();
        let SceneNodeKind::Text(text_node) = &node.kind else {
            continue;
        };
        // Text-on-path is rendered as glyph outlines by the windowed renderer
        // only (TD-019) — skip it here so headless doesn't half-render it.
        if text_node
            .path_spine_id
            .and_then(|id| doc.get_node(&id))
            .is_some_and(|s| matches!(s.kind, SceneNodeKind::Path(_)))
        {
            continue;
        }

        let layer_mul = layer_opacity.get(&orig_id).copied().unwrap_or(1.0);
        let node_opacity = node.opacity * layer_mul;

        let (doc_x, doc_y) = node.transform.apply(0.0, 0.0);
        let (screen_x, screen_y) = view.canvas_to_screen(doc_x, doc_y);
        let opacity = text_node.fill.opacity * node_opacity;
        let color = match &text_node.fill.kind {
            FillKind::Solid(c) => [
                (c.r * 255.0) as u8,
                (c.g * 255.0) as u8,
                (c.b * 255.0) as u8,
                (c.a * opacity * 255.0) as u8,
            ],
            _ => [0, 0, 0, (opacity * 255.0) as u8],
        };
        let font_style = match text_node.font_style {
            photonic_core::node::FontStyle::Normal => 0,
            photonic_core::node::FontStyle::Italic => 1,
            photonic_core::node::FontStyle::Oblique => 2,
        };
        let script = text_node.script_position;
        let base_font_screen = text_node.font_size * view.zoom;
        let effective_font_screen = base_font_screen * script.size_scale();
        let top_offset = (-(script.baseline_offset_em() * base_font_screen)
            - text_node.baseline_shift * view.zoom) as f32;

        out.push(TextSnapshot {
            content: text_node.content.clone(),
            font_family: text_node.font_family.clone(),
            font_size: effective_font_screen as f32,
            line_height_mul: text_node.line_height as f32,
            font_weight: text_node.font_weight,
            font_style,
            color,
            screen_x: screen_x as f32,
            screen_y: screen_y as f32,
            top_offset,
        });
    }
    out
}

/// glyphon state for the headless device (all `&mut`-only, hence one owner).
pub(crate) struct HeadlessText {
    font_system: FontSystem,
    swash_cache: SwashCache,
    #[allow(dead_code)] // kept alive for the atlas/viewport it backs
    cache: Cache,
    viewport: Viewport,
    atlas: TextAtlas,
    renderer: TextRenderer,
}

impl HeadlessText {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = Cache::new(device);
        let viewport = Viewport::new(device, &cache);
        let mut atlas = TextAtlas::new(device, queue, &cache, FORMAT);
        let renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        Self {
            font_system,
            swash_cache,
            cache,
            viewport,
            atlas,
            renderer,
        }
    }

    /// Render `snapshots` onto `target_view` (`LoadOp::Load`, so it composites
    /// over whatever the target already holds) at pixel size `w`×`h`. Records
    /// and submits its own encoder.
    pub(crate) fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_view: &wgpu::TextureView,
        snapshots: &[TextSnapshot],
        w: u32,
        h: u32,
    ) {
        self.viewport.update(
            queue,
            Resolution {
                width: w,
                height: h,
            },
        );

        let mut buffers: Vec<Buffer> = Vec::with_capacity(snapshots.len());
        for snap in snapshots {
            let font_size = snap.font_size.max(1.0);
            let line_height = font_size * snap.line_height_mul.max(0.1);
            let mut buf = Buffer::new(&mut self.font_system, Metrics::new(font_size, line_height));
            buf.set_size(&mut self.font_system, None, None);
            let style = match snap.font_style {
                1 => GlyphonStyle::Italic,
                2 => GlyphonStyle::Oblique,
                _ => GlyphonStyle::Normal,
            };
            let attrs = Attrs::new()
                .family(Family::Name(&snap.font_family))
                .weight(Weight(snap.font_weight))
                .style(style);
            buf.set_text(
                &mut self.font_system,
                &snap.content,
                attrs,
                Shaping::Advanced,
            );
            buf.shape_until_scroll(&mut self.font_system, false);
            buffers.push(buf);
        }

        let text_areas: Vec<TextArea> = snapshots
            .iter()
            .zip(buffers.iter())
            .map(|(snap, buf)| TextArea {
                buffer: buf,
                left: snap.screen_x,
                top: snap.screen_y + snap.top_offset,
                scale: 1.0,
                bounds: TextBounds {
                    left: i32::MIN,
                    top: i32::MIN,
                    right: i32::MAX,
                    bottom: i32::MAX,
                },
                default_color: GlyphonColor::rgba(
                    snap.color[0],
                    snap.color[1],
                    snap.color[2],
                    snap.color[3],
                ),
                custom_glyphs: &[],
            })
            .collect();

        if let Err(e) = self.renderer.prepare(
            device,
            queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            text_areas,
            &mut self.swash_cache,
        ) {
            tracing::warn!("headless glyphon prepare failed: {:?}", e);
            return;
        }

        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = enc
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("headless_text_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                })
                .forget_lifetime();
            if let Err(e) = self.renderer.render(&self.atlas, &self.viewport, &mut pass) {
                tracing::warn!("headless glyphon render failed: {:?}", e);
            }
        }
        queue.submit([enc.finish()]);
        self.atlas.trim();
    }
}
