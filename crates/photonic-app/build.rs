fn main() {
    // Embed the application icon into the Windows executable so it appears
    // in Explorer, the taskbar, and Alt+Tab — not just the window title bar.
    #[cfg(target_os = "windows")]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/photonic.ico");
        if let Err(e) = res.compile() {
            eprintln!("cargo:warning=winresource failed: {e}");
        }
    }
}
