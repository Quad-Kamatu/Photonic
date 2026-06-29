//! Application quit helpers.
//!
//! Closing the window already exits this process cleanly (the detached MCP
//! server thread dies with it). These helpers add an explicit, discoverable
//! quit from the GUI — including a "quit every Photonic process" option for
//! when several windows or a headless `photonic mcp` server are running.

/// Terminate **this** process immediately — a full, clean shutdown.
pub fn quit_self() -> ! {
    std::process::exit(0);
}

/// Terminate **every** running Photonic process — sibling GUI windows and any
/// headless MCP servers — then exit this one. Best-effort and dependency-free:
/// it shells out to the platform's process-kill tool.
pub fn quit_all() -> ! {
    let me = std::process::id();

    #[cfg(target_os = "linux")]
    {
        // SIGTERM every other process whose comm is "photonic" (read from /proc).
        if let Ok(entries) = std::fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let Some(pid) = entry
                    .file_name()
                    .to_str()
                    .and_then(|s| s.parse::<u32>().ok())
                else {
                    continue;
                };
                if pid == me {
                    continue;
                }
                if let Ok(comm) = std::fs::read_to_string(format!("/proc/{pid}/comm")) {
                    if comm.trim() == "photonic" {
                        let _ = std::process::Command::new("kill")
                            .arg(pid.to_string())
                            .status();
                    }
                }
            }
        }
    }

    #[cfg(all(unix, not(target_os = "linux")))]
    {
        // macOS / other unix: pkill by exact name (best effort), excluding self
        // is not trivial, so we TERM others then exit ourselves below.
        let _ = std::process::Command::new("pkill")
            .args(["-x", "photonic"])
            .status();
    }

    #[cfg(windows)]
    {
        // Kill all photonic.exe except ourselves, then exit.
        let _ = std::process::Command::new("taskkill")
            .args(["/IM", "photonic.exe", "/F", "/FI", &format!("PID ne {me}")])
            .status();
    }

    std::process::exit(0);
}
