mod args;
mod cli;
mod mcp_proxy;
mod repl;
mod script;

use anyhow::Result;
use args::Args;
use clap::Parser;
use egui_wgpu::ScreenDescriptor;
use photonic_core::{document::Document, history::CommandHistory, AuditLog};
use photonic_gui::PhotonicApp;
use photonic_mcp::{McpServer, McpServerConfig};
use photonic_render::PhotonicRenderer;
use repl::LuaRepl;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{oneshot, Mutex};
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowAttributes, WindowId},
};

// ─── Entry point ─────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    let args = Args::parse();

    // ── CLI client mode: a subcommand was given ───────────────────────────────
    if let Some(command) = args.command {
        tracing_subscriber::registry()
            .with(fmt::layer())
            .with(EnvFilter::new("warn"))
            .init();
        return cli::run(&args.server, command);
    }

    // ── Server / GUI mode: full logging ──────────────────────────────────────
    // Use %APPDATA%\Photonic\ — a real Windows path that always exists.
    // Fall back to the binary's directory if APPDATA is unavailable.
    let log_dir = {
        let candidate = std::env::var("APPDATA")
            .map(|p| std::path::PathBuf::from(p).join("Photonic"))
            .unwrap_or_else(|_| {
                std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
            });
        let _ = std::fs::create_dir_all(&candidate);
        candidate
    };
    let log_path = log_dir.join("photonic.log");

    // Panic hook: write to log file before the process dies.
    {
        let path = log_path.clone();
        let orig = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            orig(info);
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
            {
                let _ = writeln!(f, "[PANIC] {info}");
            }
        }));
    }

    // Synchronous file appender — writes each event directly to disk so nothing
    // is lost when the process is killed (non_blocking would lose buffered events).
    let file_appender = tracing_appender::rolling::never(&log_dir, "photonic.log");

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn,photonic=debug"));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer())
        .with(fmt::layer().with_writer(file_appender).with_ansi(false))
        .init();

    eprintln!("Photonic log: {}", log_path.display());

    register_file_association();

    let document = if let Some(path) = &args.file {
        let json = std::fs::read_to_string(path)?;
        Document::from_json(&json)?
    } else {
        Document::default_artboard()
    };

    info!("Photonic — document: '{}'", document.name);

    let document_arc = Arc::new(Mutex::new(document));
    let history_arc: Arc<Mutex<CommandHistory>> = Arc::new(Mutex::new(CommandHistory::new(200)));
    let (capture_tx, capture_rx) = std::sync::mpsc::channel::<oneshot::Sender<Vec<u8>>>();

    // Audit log shared between the MCP server thread and the GUI Audit panel.
    let audit_log = Arc::new(std::sync::Mutex::new(AuditLog::new()));

    let mcp_config = McpServerConfig {
        port: args.mcp_port,
        secret: args.mcp_secret,
    };

    // ── Headless mode ─────────────────────────────────────────────────────────
    if args.headless {
        info!("Running in headless mode (MCP server only)");
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        let mcp_server = McpServer::new(
            Arc::clone(&document_arc),
            Arc::clone(&history_arc),
            capture_tx,
            mcp_config,
            Arc::new(AtomicBool::new(false)),
            audit_log,
        );
        rt.block_on(mcp_server.run())?;
        return Ok(());
    }

    // ── GUI mode: MCP server on background thread, winit on main thread ───────
    let mcp_running = Arc::new(AtomicBool::new(false));
    {
        let doc_for_mcp = Arc::clone(&document_arc);
        let hist_for_mcp = Arc::clone(&history_arc);
        let tx_for_mcp = capture_tx;
        let running_flag = Arc::clone(&mcp_running);
        let audit_for_mcp = Arc::clone(&audit_log);
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            let mcp_server = McpServer::new(
                doc_for_mcp,
                hist_for_mcp,
                tx_for_mcp,
                mcp_config,
                running_flag,
                audit_for_mcp,
            );
            if let Err(e) = rt.block_on(mcp_server.run()) {
                tracing::error!("MCP server error: {}", e);
            }
        });
    }

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = PhotonicWinitApp {
        document: document_arc,
        history: history_arc,
        mcp_running,
        capture_rx: Some(capture_rx),
        state: None,
        show_welcome: args.file.is_none(),
        initial_file: args.file.clone(),
        audit_log,
    };

    event_loop.run_app(&mut app)?;
    Ok(())
}

// ─── Claude streaming events ─────────────────────────────────────────────────

/// Events streamed from the Claude subprocess thread to the render loop.
enum ClaudeEvent {
    /// A tool was called and returned; show tool name + first line of result.
    ToolResult { name: String, summary: String },
    /// Claude's final text response.
    FinalText(String),
    /// Fatal error (process failed to start, etc.).
    Error(String),
    /// Subprocess exited — no more events will follow.
    Done,
}

// ─── Winit application ───────────────────────────────────────────────────────

struct RenderState {
    window: Arc<Window>,
    renderer: PhotonicRenderer,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    gui: PhotonicApp,
    lua_repl: LuaRepl,
    /// Receives streaming events from an in-flight `claude` subprocess.
    claude_rx: Option<std::sync::mpsc::Receiver<ClaudeEvent>>,
    /// True after the first message has been sent; subsequent turns use `--continue`.
    claude_session_started: bool,
    /// Undo/redo history shared between the GUI and the MCP server.
    gui_history: Arc<Mutex<CommandHistory>>,
}

struct PhotonicWinitApp {
    document: Arc<Mutex<Document>>,
    history: Arc<Mutex<CommandHistory>>,
    mcp_running: Arc<AtomicBool>,
    capture_rx: Option<std::sync::mpsc::Receiver<oneshot::Sender<Vec<u8>>>>,
    state: Option<RenderState>,
    show_welcome: bool,
    /// File to mark as `current_file` in the GUI once the window is ready.
    initial_file: Option<std::path::PathBuf>,
    /// Shared audit log — passed to the GUI panel for display.
    audit_log: Arc<std::sync::Mutex<AuditLog>>,
}

impl ApplicationHandler for PhotonicWinitApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }

        let window_icon = load_window_icon();
        let window = Arc::new(
            event_loop
                .create_window(
                    WindowAttributes::default()
                        .with_title("Photonic")
                        .with_inner_size(PhysicalSize::new(1280u32, 800u32))
                        .with_window_icon(window_icon),
                )
                .expect("Failed to create window"),
        );

        let capture_rx = self.capture_rx.take().expect("capture_rx already consumed");

        let renderer = pollster::block_on(PhotonicRenderer::new(
            Arc::clone(&window),
            Arc::clone(&self.document),
            capture_rx,
        ));

        // ── Lua REPL (binds to the live document) ────────────────────────────
        let lua_repl = match LuaRepl::new(Arc::clone(&self.document), Arc::clone(&self.history)) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Lua REPL init failed: {e}");
                LuaRepl::new_empty()
            }
        };

        // ── egui setup ───────────────────────────────────────────────────────
        let egui_ctx = egui::Context::default();
        egui_ctx.set_visuals(photonic_gui::build_dark_theme());
        egui_ctx.style_mut(|s| {
            s.spacing.item_spacing = egui::vec2(6.0, 4.0);
            s.spacing.button_padding = egui::vec2(8.0, 3.0);
        });

        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        egui_ctx.set_fonts(fonts);

        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            None,
            None,
        );

        let egui_renderer =
            egui_wgpu::Renderer::new(renderer.device(), renderer.surface_format(), None, 1, false);

        write_mcp_config();
        info!("GPU renderer + egui initialized — window open");
        window.request_redraw();

        let mut gui = if self.show_welcome {
            PhotonicApp::new_with_welcome()
        } else {
            PhotonicApp::new()
        };
        gui.audit.log = Some(Arc::clone(&self.audit_log));

        self.state = Some(RenderState {
            window,
            renderer,
            egui_ctx,
            egui_state,
            egui_renderer,
            gui,
            lua_repl,
            claude_rx: None,
            claude_session_started: false,
            gui_history: Arc::clone(&self.history),
        });

        // If we were launched with a file path, tell the GUI which file is open.
        if let Some(path) = self.initial_file.take() {
            if let Some(state) = &mut self.state {
                if let Ok(doc) = self.document.try_lock() {
                    state.gui.welcome.add_recent(path.clone(), doc.name.clone());
                }
                state.gui.current_file = Some(path);
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = &mut self.state else { return };

        let response = state.egui_state.on_window_event(&state.window, &event);

        match event {
            WindowEvent::CloseRequested => {
                info!("Window closed");
                event_loop.exit();
            }
            WindowEvent::Resized(PhysicalSize { width, height }) => {
                state.renderer.resize(width, height);
                state.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                self.render_frame();
                if let Some(s) = &self.state {
                    s.window.request_redraw();
                }
            }
            _ => {
                if response.repaint {
                    state.window.request_redraw();
                }
            }
        }
    }
}

impl PhotonicWinitApp {
    fn render_frame(&mut self) {
        let Some(state) = &mut self.state else { return };

        // 1. Build document geometry + push camera
        let (verts, idxs) = state.renderer.update();

        // 2. Acquire surface frame
        let mut frame = match state.renderer.begin_frame(&verts, &idxs) {
            Some(f) => f,
            None => return,
        };

        // 2b. Render text nodes over the document (before egui).
        state.renderer.render_text_pass(&mut frame);

        // 2c. Render Gaussian glow effects (GPU blur passes, additive composite).
        state.renderer.render_gaussian_glow_pass(&mut frame);

        // 3. Run egui (doc lock is held only for the duration of this closure)
        let raw_input = state.egui_state.take_egui_input(&state.window);
        let (w, h) = state.renderer.size();
        let pixels_per_point = state.window.scale_factor() as f32;

        let doc_arc = Arc::clone(&self.document);
        let mcp_ok = self.mcp_running.load(Ordering::Relaxed);
        let egui_ctx = state.egui_ctx.clone();
        // Keep window position and scale factor up-to-date for the eyedropper.
        let sf = state.window.scale_factor() as f32;
        if let Ok(outer) = state.window.outer_position() {
            state.gui.window_logical_pos =
                ((outer.x as f32 / sf) as i32, (outer.y as f32 / sf) as i32);
        }
        state.gui.window_scale_factor = sf;

        let full_output = egui_ctx.run(raw_input, |ctx| {
            // try_lock — never block; skip GUI draw for this frame if the doc
            // lock is currently held by an MCP handler so the render loop
            // cannot be frozen indefinitely by lock contention.
            if let Ok(mut doc) = doc_arc.try_lock() {
                if let Ok(mut hist) = state.gui_history.try_lock() {
                    let mut view = state.renderer.view.clone();
                    state.gui.draw(
                        ctx,
                        &mut doc,
                        &mut view,
                        &mut state.renderer,
                        mcp_ok,
                        &mut *hist,
                    );
                    state.renderer.view = view;
                }
            }
        });
        // doc lock released here ↑

        // Flush debounced checkpoint: if a user action happened ≥30 s ago with
        // no further actions since, write the snapshot now.
        if let Ok(doc) = doc_arc.try_lock() {
            if let Ok(mut hist) = state.gui_history.try_lock() {
                hist.tick_checkpoint(&doc);
            }
        }

        // 4. Execute any Lua code queued by the console (doc lock is FREE now)
        if let Some(code) = state.gui.lua_console.pending.take() {
            let (prints, error) = state.lua_repl.eval(&code);
            for line in prints {
                state.gui.lua_console.log.push((false, line));
            }
            if let Some(err) = error {
                state
                    .gui
                    .lua_console
                    .log
                    .push((true, format!("Error: {err}")));
            }
        }

        // 4b. Dispatch a pending Claude message via `claude` subprocess.
        if let Some(user_msg) = state.gui.claude_chat.pending.take() {
            tracing::info!(
                "Dispatching Claude: {:?} (first={})",
                &user_msg[..user_msg.len().min(60)],
                !state.claude_session_started
            );
            let is_first = !state.claude_session_started;
            state.claude_session_started = true;
            let (tx, rx) = std::sync::mpsc::channel::<ClaudeEvent>();
            state.claude_rx = Some(rx);
            std::thread::spawn(move || {
                run_claude_stream(user_msg, is_first, tx);
            });
        }

        // Drain all available Claude events — stream them into the chat as they arrive.
        if let Some(rx) = &state.claude_rx {
            loop {
                match rx.try_recv() {
                    Ok(ClaudeEvent::ToolResult { name, summary }) => {
                        let icon = tool_icon(&name);
                        let first_line = summary.lines().next().unwrap_or("").trim();
                        let msg = if first_line.is_empty() {
                            format!("{icon} {name}")
                        } else {
                            format!("{icon} {name} — {first_line}")
                        };
                        tracing::debug!("Claude tool: {name}");
                        state.gui.claude_chat.messages.push((false, msg));
                        state.window.request_redraw();
                    }
                    Ok(ClaudeEvent::FinalText(text)) => {
                        tracing::info!("Claude final response ({} chars)", text.len());
                        state.gui.claude_chat.messages.push((false, text));
                        state.window.request_redraw();
                    }
                    Ok(ClaudeEvent::Error(e)) => {
                        tracing::warn!("Claude error: {}", e);
                        state
                            .gui
                            .claude_chat
                            .messages
                            .push((false, format!("⚠ {e}")));
                        state.window.request_redraw();
                    }
                    Ok(ClaudeEvent::Done) => {
                        state.claude_rx = None;
                        state.gui.claude_chat.busy = false;
                        // Snapshot the document after each AI session so the
                        // change log reflects AI-driven edits.
                        if let Ok(doc) = doc_arc.try_lock() {
                            if let Ok(mut hist) = state.gui_history.try_lock() {
                                hist.create_checkpoint("AI edit".to_string(), &doc);
                            }
                        }
                        state.window.request_redraw();
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        state.claude_rx = None;
                        state.gui.claude_chat.busy = false;
                        state.window.request_redraw();
                        break;
                    }
                }
            }
        }

        state
            .egui_state
            .handle_platform_output(&state.window, full_output.platform_output);

        // 5. Tessellate + prepare egui resources
        let tris = state
            .egui_ctx
            .tessellate(full_output.shapes, full_output.pixels_per_point);
        let screen_desc = ScreenDescriptor {
            size_in_pixels: [w, h],
            pixels_per_point,
        };

        for (id, delta) in &full_output.textures_delta.set {
            state.egui_renderer.update_texture(
                state.renderer.device(),
                state.renderer.queue(),
                *id,
                delta,
            );
        }

        let extra_cmds = state.egui_renderer.update_buffers(
            state.renderer.device(),
            state.renderer.queue(),
            &mut frame.encoder,
            &tris,
            &screen_desc,
        );
        if !extra_cmds.is_empty() {
            state.renderer.queue().submit(extra_cmds);
        }

        // 6. egui render pass (LoadOp::Load — draws over the document)
        {
            let mut rpass = frame
                .encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &frame.view,
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
            state.egui_renderer.render(&mut rpass, &tris, &screen_desc);
        }

        for id in &full_output.textures_delta.free {
            state.egui_renderer.free_texture(id);
        }

        // 7. Submit + present
        state.renderer.finish_frame(frame);

        // 8. Service screenshot requests
        state.renderer.service_captures(&verts, &idxs);

        // Periodic heartbeat so we can see the render loop is alive in logs
        {
            use std::sync::atomic::{AtomicU64, Ordering};
            static FRAME: AtomicU64 = AtomicU64::new(0);
            let n = FRAME.fetch_add(1, Ordering::Relaxed);
            if n % 600 == 0 {
                tracing::info!("render loop alive — frame {}", n);
            }
        }
    }
}

// ─── Windows file association ────────────────────────────────────────────────

/// Register `.photon` files with the current user's shell (HKCU — no elevation
/// required).  After this, Explorer shows the Photonic icon for `.photon` files
/// and double-clicking opens them in Photonic.
///
/// Safe to call on every launch; it is idempotent and only touches HKCU keys
/// owned by this application.
#[cfg(windows)]
fn register_file_association() {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
    use winreg::RegKey;

    let exe = match std::env::current_exe() {
        Ok(p) => p.to_string_lossy().into_owned(),
        Err(e) => {
            tracing::warn!("file assoc: could not get exe path: {e}");
            return;
        }
    };

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let classes = match hkcu.open_subkey_with_flags("Software\\Classes", KEY_SET_VALUE) {
        Ok(k) => k,
        Err(e) => {
            tracing::warn!("file assoc: could not open HKCU\\Software\\Classes: {e}");
            return;
        }
    };

    // .photon → ProgID
    if let Ok((ext, _)) = classes.create_subkey(".photon") {
        let _ = ext.set_value("", &"PhotonicDocument");
    }

    // ProgID description
    if let Ok((prog, _)) = classes.create_subkey("PhotonicDocument") {
        let _ = prog.set_value("", &"Photonic Document");

        // Icon — first resource icon in the exe (the one we embedded via winresource)
        if let Ok((icon, _)) = prog.create_subkey("DefaultIcon") {
            let _ = icon.set_value("", &format!("\"{exe}\",0"));
        }

        // Open command
        if let Ok((shell, _)) = prog.create_subkey("shell\\open\\command") {
            let _ = shell.set_value("", &format!("\"{exe}\" \"%1\""));
        }
    }

    // Notify the shell so changes take effect without a log-off.
    unsafe {
        windows_sys::Win32::UI::Shell::SHChangeNotify(
            windows_sys::Win32::UI::Shell::SHCNE_ASSOCCHANGED as i32,
            windows_sys::Win32::UI::Shell::SHCNF_IDLIST,
            std::ptr::null(),
            std::ptr::null(),
        );
    }

    tracing::info!("file assoc: .photon registered → PhotonicDocument");
}

#[cfg(not(windows))]
fn register_file_association() {}

// ─── Window icon ─────────────────────────────────────────────────────────────

/// Load the bundled ICO file and decode the largest 32×32 (or largest available)
/// RGBA frame for use as the winit window icon.
fn load_window_icon() -> Option<winit::window::Icon> {
    let ico_bytes = include_bytes!("../assets/photonic.ico");
    let img = match image::load_from_memory_with_format(ico_bytes, image::ImageFormat::Ico) {
        Ok(i) => i,
        Err(e) => {
            tracing::warn!("window icon: failed to decode ICO: {e}");
            return None;
        }
    };
    let rgba = img.into_rgba8();
    let (w, h) = rgba.dimensions();
    tracing::info!("window icon: loaded {}×{}", w, h);
    match winit::window::Icon::from_rgba(rgba.into_raw(), w, h) {
        Ok(icon) => Some(icon),
        Err(e) => {
            tracing::warn!("window icon: failed to create: {e}");
            None
        }
    }
}

// ─── Claude subprocess helpers ───────────────────────────────────────────────

/// Build the system prompt for a Claude session.
/// Tells Claude to use the registered Photonic MCP tools directly — no Bash/CLI needed.
fn photonic_skill() -> String {
    "You are an AI design assistant embedded inside Photonic, a vector graphics editor.\n\
\n\
TOOL ACCESS: Your Photonic MCP tools (create_shape, create_path, get_document_state, \
screenshot, update_node, etc.) are registered natively — call them directly as tools. \
NEVER use Bash, shell commands, or JSON-RPC to interact with Photonic. \
NEVER invoke the photonic-plan skill — it is for scratch builds only.\n\
\n\
Canvas: 1123 × 794 px, origin (0,0) top-left, centre ≈ (561, 397).\n\
\n\
Fill format: {\"type\":\"solid\",\"color\":\"#rrggbb\"} | {\"type\":\"none\"} | \
{\"type\":\"gradient\",\"gradient_type\":\"linear\"|\"radial\",\"colors\":[\"#hex1\",\"#hex2\"]}\n\
Stroke format: {\"color\":\"#rrggbb\",\"width\":2,\"enabled\":true}\n\
\n\
IMPROVEMENT WORKFLOW (use when editing an existing design):\n\
1. Call get_document_state AND screenshot in parallel — understand what exists.\n\
2. Make targeted changes: update_node to change colors/opacity, reorder_node for z-order, \
   create shapes to add elements, delete_nodes only for things being replaced.\n\
3. Preserve existing nodes unless a specific node is being replaced. Do not wipe and redraw.\n\
4. Take a final screenshot to confirm the result.\n\
\n\
CREATION WORKFLOW (use when building from scratch):\n\
1. get_document_state + screenshot in parallel.\n\
2. create_layer for each semantic group (background, base, detail, highlight).\n\
3. Draw back-to-front. Group each component after completing it.\n\
4. Take a final screenshot to confirm the result.\n\
\n\
Speed: batch independent tool calls into the same turn for parallel execution. \
Skip intermediate screenshots unless you need visual feedback to proceed."
        .to_string()
}

/// Register the Photonic MCP server in the user's Claude `~/.claude.json` so it
/// is always available when `claude` runs.
///
/// Uses the HTTP transport — Claude Code connects directly to the already-running
/// Photonic MCP HTTP server on port 7842.  No proxy subprocess needed.
fn write_mcp_config() {
    let server_entry = serde_json::json!({
        "type": "http",
        "url": format!("http://127.0.0.1:{}/mcp", 7842)
    });

    let claude_settings_path = claude_settings_path();
    if let Some(p) = &claude_settings_path {
        let mut settings: serde_json::Value = p
            .exists()
            .then(|| std::fs::read_to_string(p).ok())
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| serde_json::json!({}));

        if !settings["mcpServers"].is_object() {
            settings["mcpServers"] = serde_json::json!({});
        }
        settings["mcpServers"]["photonic"] = server_entry;

        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::write(
            p,
            serde_json::to_string_pretty(&settings).unwrap_or_default(),
        ) {
            Ok(_) => info!(
                "Registered Photonic MCP server in Claude settings at {:?}",
                p
            ),
            Err(e) => tracing::warn!("Could not update Claude settings: {e}"),
        }
    }
}

/// Return the path to Claude Code's `~/.claude.json`.
///
/// This is the primary Claude Code configuration file that stores MCP server
/// registrations, user preferences, and project state.  It is distinct from
/// `~/.claude/settings.json` which only holds model/permission settings.
fn claude_settings_path() -> Option<std::path::PathBuf> {
    // Claude Code always uses ~/.claude.json (note: NOT ~/.claude/settings.json).
    // On Windows, prefer USERPROFILE over HOME for the home directory.
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()?;
    Some(std::path::PathBuf::from(home).join(".claude.json"))
}

/// Build a PATH string that includes Node.js and npm directories so that
/// subprocesses spawned from a GUI process (which may inherit a stripped PATH)
/// can resolve `node` and npm-installed shims like `claude.cmd`.
#[cfg(windows)]
fn augmented_path() -> String {
    let current = std::env::var("PATH").unwrap_or_default();
    let mut extras: Vec<String> = Vec::new();

    // npm global bin dir
    if let Ok(appdata) = std::env::var("APPDATA") {
        extras.push(format!("{appdata}\\npm"));
    }

    // Common Node.js install locations
    for candidate in &[r"C:\Program Files\nodejs", r"C:\Program Files (x86)\nodejs"] {
        if std::path::Path::new(candidate).exists() {
            extras.push(candidate.to_string());
        }
    }

    // nvm on Windows typically lives in %APPDATA%\nvm
    if let Ok(appdata) = std::env::var("APPDATA") {
        let nvm_root = std::path::PathBuf::from(&appdata).join("nvm");
        if nvm_root.exists() {
            // Add the currently-active version dir (first subdir found)
            if let Ok(mut entries) = std::fs::read_dir(&nvm_root) {
                if let Some(Ok(entry)) = entries.next() {
                    extras.push(entry.path().to_string_lossy().into_owned());
                }
            }
        }
    }

    if extras.is_empty() {
        current
    } else {
        format!("{};{}", extras.join(";"), current)
    }
}

/// Find the `claude` executable, handling Windows npm installs where the binary
/// is a `.cmd` shim not visible to CreateProcess without a full path.
fn find_claude() -> Result<std::process::Command, String> {
    #[cfg(windows)]
    {
        let path_env = augmented_path();

        // 1. Check %APPDATA%\npm\claude.cmd — standard npm global install
        if let Ok(appdata) = std::env::var("APPDATA") {
            let p = std::path::PathBuf::from(&appdata)
                .join("npm")
                .join("claude.cmd");
            if p.exists() {
                let mut c = std::process::Command::new("cmd");
                c.args(["/c", &p.to_string_lossy().into_owned()]);
                c.env("PATH", &path_env);
                return Ok(c);
            }
        }
        // 2. Ask cmd.exe where it lives (works if PATH happens to be inherited)
        if let Ok(out) = std::process::Command::new("cmd")
            .args(["/c", "where", "claude"])
            .env("PATH", &path_env)
            .output()
        {
            if let Some(line) = String::from_utf8_lossy(&out.stdout)
                .lines()
                .next()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                let mut c = std::process::Command::new("cmd");
                c.args(["/c", line]);
                c.env("PATH", &path_env);
                return Ok(c);
            }
        }
        return Err(
            "claude not found — install with: npm install -g @anthropic-ai/claude-code".into(),
        );
    }
    #[cfg(not(windows))]
    Ok(std::process::Command::new("claude"))
}

/// Emoji prefix for each MCP tool name shown in the chat stream.
///
/// Claude Code prefixes MCP tools as `mcp__<server>__<tool>` — strip that
/// prefix before matching so both bare names and namespaced names work.
fn tool_icon(name: &str) -> &'static str {
    // Strip the `mcp__photonic__` namespace prefix added by Claude Code.
    let bare = name
        .strip_prefix("mcp__photonic__")
        .or_else(|| name.strip_prefix("mcp__"))
        .unwrap_or(name);
    match bare {
        "screenshot" => "📸",
        "get_document_state" | "get_node" => "📋",
        "create_shape" | "create_path" | "build_shape_from_points" => "✏",
        "update_node" => "✎",
        "delete_nodes" => "✗",
        "apply_transform" => "⟳",
        "reorder_node" => "⇅",
        "group_nodes" | "ungroup_nodes" => "⊞",
        "boolean_operation" => "∩",
        "create_layer" => "▤",
        "undo" | "redo" => "↩",
        "create_checkpoint" | "list_checkpoints" | "restore_checkpoint" => "◈",
        _ => "·",
    }
}

/// Spawn `claude` with `--output-format stream-json` and forward events to `tx`
/// as they arrive so the UI can render progress in real time.
fn run_claude_stream(user_msg: String, is_first: bool, tx: std::sync::mpsc::Sender<ClaudeEvent>) {
    use std::io::BufRead;

    let skill = photonic_skill();
    // settings.json was already updated at startup via write_mcp_config().
    // Passing --mcp-config on top of that caused duplicate server registration,
    // which Claude Code treats as a conflict and silently drops the tools.
    // Rely solely on settings.json here.

    let mut cmd = match find_claude() {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(ClaudeEvent::Error(e));
            let _ = tx.send(ClaudeEvent::Done);
            return;
        }
    };

    cmd.arg("-p")
        .arg(&user_msg)
        .arg("--dangerously-skip-permissions")
        .arg("--output-format")
        .arg("stream-json")
        .arg("--verbose")
        .arg("--append-system-prompt")
        .arg(&skill)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());

    if !is_first {
        cmd.arg("--continue");
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(ClaudeEvent::Error(format!("Failed to launch claude: {e}")));
            let _ = tx.send(ClaudeEvent::Done);
            return;
        }
    };

    let stdout = child.stdout.take().expect("stdout was piped");
    let reader = std::io::BufReader::new(stdout);

    // Maps tool_use id → tool name so we can label results when they arrive.
    let mut pending: std::collections::HashMap<String, String> = Default::default();
    let mut got_final = false;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let Ok(ev) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };

        match ev["type"].as_str() {
            // Note which tools were called so we can label their results.
            Some("assistant") => {
                if let Some(content) = ev["message"]["content"].as_array() {
                    for item in content {
                        if item["type"] == "tool_use" {
                            let id = item["id"].as_str().unwrap_or("").to_string();
                            let name = item["name"].as_str().unwrap_or("").to_string();
                            if !id.is_empty() && !name.is_empty() {
                                pending.insert(id, name);
                            }
                        }
                    }
                }
            }
            // Emit a ToolResult event as soon as each result arrives.
            Some("user") => {
                if let Some(content) = ev["message"]["content"].as_array() {
                    for item in content {
                        if item["type"] == "tool_result" {
                            let id = item["tool_use_id"].as_str().unwrap_or("").to_string();
                            let summary = item["content"]
                                .as_array()
                                .and_then(|a| a.first())
                                .and_then(|c| c["text"].as_str())
                                .or_else(|| item["content"].as_str())
                                .unwrap_or("")
                                .trim()
                                .to_string();
                            if let Some(name) = pending.remove(&id) {
                                if tx.send(ClaudeEvent::ToolResult { name, summary }).is_err() {
                                    return; // receiver dropped (window closed)
                                }
                            }
                        }
                    }
                }
            }
            // Final assistant reply.
            Some("result") => {
                let text = ev["result"].as_str().unwrap_or("").trim().to_string();
                if !text.is_empty() {
                    got_final = true;
                    let _ = tx.send(ClaudeEvent::FinalText(text));
                }
            }
            _ => {}
        }
    }

    let _ = child.wait();

    if !got_final {
        let _ = tx.send(ClaudeEvent::Error("(no response from claude)".into()));
    }
    let _ = tx.send(ClaudeEvent::Done);
}
