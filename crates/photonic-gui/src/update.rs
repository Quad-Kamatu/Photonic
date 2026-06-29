//! In-app self-update from GitHub Releases (Quad-Kamatu/Photonic).
//!
//! Pulls the latest release's binary asset, verifies it, and replaces the
//! running executable (applied on next launch). No server required — GitHub
//! Releases is both the host and the manifest. Network + file replacement runs
//! on a background thread; the UI polls the returned channel.
//!
//! Integrity: downloads are over TLS from GitHub. For tamper-proof updates,
//! sign release archives with `zipsign` (ed25519) and pass the public key to
//! `.verifying_keys(...)` below — a release-pipeline step (see `PUBLIC_KEY`).

use std::sync::mpsc::{channel, Receiver};

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

const REPO_OWNER: &str = "Quad-Kamatu";
const REPO_NAME: &str = "Photonic";
const BIN_NAME: &str = "photonic";

#[derive(Clone, Debug)]
pub enum UpdateStatus {
    UpToDate(String),
    Updated(String),
    Error(String),
}

/// Start a check-and-update on a background thread; poll the receiver each frame.
pub fn check_and_update() -> Receiver<UpdateStatus> {
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("photonic-update".into())
        .spawn(move || {
            let _ = tx.send(run());
        })
        .ok();
    rx
}

fn run() -> UpdateStatus {
    let mut builder = self_update::backends::github::Update::configure();
    builder
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .bin_name(BIN_NAME)
        .current_version(CURRENT_VERSION)
        .show_download_progress(false)
        .no_confirm(true);
    // Use a token for private repos / to avoid the 60/hr anonymous API limit.
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        if !token.is_empty() {
            builder.auth_token(&token);
        }
    }
    let updater = match builder.build() {
        Ok(u) => u,
        Err(e) => return UpdateStatus::Error(e.to_string()),
    };
    match updater.update() {
        Ok(self_update::Status::UpToDate(v)) => UpdateStatus::UpToDate(v),
        Ok(self_update::Status::Updated(v)) => UpdateStatus::Updated(v),
        Err(e) => UpdateStatus::Error(e.to_string()),
    }
}
