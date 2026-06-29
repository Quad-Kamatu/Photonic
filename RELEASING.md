# Releasing Photonic (auto-update pipeline)

Photonic auto-updates from **GitHub Releases** — no server. CI builds a signed
binary per platform; the app downloads the latest, **verifies its ed25519
signature**, and replaces itself (applied on restart).

```
git tag vX.Y.Z  ──▶  GitHub Actions (release.yml)
                       ├─ build per platform (linux/mac-x64/mac-arm/windows)
                       ├─ package (.tar.gz / .zip)
                       ├─ zipsign sign  (private key from secret)
                       └─ upload to the GitHub Release
                                 │
   app ── "Check for Updates" ──▶ download newest asset ▶ verify sig ▶ swap on restart
```

## One-time setup (do this once)

1. **Add the signing key as a repo secret.** The ed25519 key pair lives in
   `release/`:
   - `release/photonic-signing.pub` — public, **committed** + embedded in the app.
   - `release/photonic-signing.priv` — private, **gitignored**, never commit.

   Base64-encode the private key and paste it into a GitHub secret named
   **`PHOTONIC_SIGNING_KEY`** (Repo ▸ Settings ▸ Secrets and variables ▸ Actions):

   ```sh
   base64 -w0 release/photonic-signing.priv   # copy the output into the secret
   ```

   Keep `release/photonic-signing.priv` backed up somewhere safe (a password
   manager). If it's lost you must rotate keys (see below) and ship a new app
   build before old clients can update again.

## Cutting a release

Releases go out through **one command**, `scripts/release.sh`. First write the
user-facing changes under the `## [Unreleased]` heading in
[`CHANGELOG.md`](CHANGELOG.md) (those bullets become both the GitHub release
notes and the in-app "What's New" popup), then:

```sh
scripts/release.sh 0.2.0          # the whole release
scripts/release.sh 0.2.0 --dry-run    # preview the version/changelog edits, no push
scripts/release.sh 0.2.0 --no-build   # skip the local release-build smoke test
```

The script:
1. validates the semver + a clean working tree (and that `[Unreleased]` has notes),
2. bumps the workspace `Cargo.toml` version (single source of truth),
3. rolls `[Unreleased]` into `## [0.2.0] - <date>` and opens a fresh empty
   `[Unreleased]`,
4. builds `photonic-app` in release as a smoke test (unless `--no-build`),
5. commits `release: v0.2.0`, tags `v0.2.0`, and pushes the branch + tag.

Pushing the tag triggers the `release` workflow, which builds, signs, and
publishes the per-platform assets — and now also sets the GitHub release body
from the matching `CHANGELOG.md` section. (The app compares its
`CARGO_PKG_VERSION` to the latest tag, so the Cargo version and the tag always
match — `0.2.0` ↔ `v0.2.0` — because the script writes both.)

That's it. Existing installs surface the update via the launch banner (or
**Check for Updates**), and after updating they see a **What's New** popup
listing every version they skipped.

### Manual fallback

If you ever need to do it by hand: bump `[workspace.package] version` in
`Cargo.toml`, move the `[Unreleased]` notes under a dated `## [X.Y.Z]` heading
in `CHANGELOG.md`, `git commit`, then `git tag vX.Y.Z && git push --tags`.

## "What's New" popup

`CHANGELOG.md` is embedded in the binary (`release_notes.rs`). On launch the app
compares the running build to `last_seen_version` in prefs; if it moved forward
it shows the changelog sections for the skipped versions, then records the new
version. Fresh installs are never nagged. Keep changelog entries written for
users (what changed, not commit-speak) — they're the release's public face.

## How updating works in the app

- Global search ▸ **Check for Updates** runs a background check. If a newer,
  validly-signed release exists it's downloaded and staged; the status bar says
  "Updated to vX.Y.Z — restart Photonic to apply".
- Only archives signed by `release/photonic-signing.pub` are accepted, so a
  tampered or third-party binary is rejected even if served over GitHub.

## Optional: OS code signing (removes "unknown publisher" warnings)

This is **separate** from update integrity (which is already handled by the
ed25519 signature above). It's about the OS not scaring users at install:

- **Windows** — Authenticode cert (~$100–400/yr) or Azure Trusted Signing;
  `signtool` in CI.
- **macOS** — Apple Developer ($99/yr): `codesign` + `notarytool` notarization +
  staple. Without it, Gatekeeper blocks unsigned `.app`s.
- **Linux** — none needed; optionally ship SHA-256 sums.

Add these to `release.yml` when you have the certs; they don't change the
auto-update mechanism.

## Rotating the signing key

If the private key leaks or is lost:
1. `zipsign gen-key release/photonic-signing.priv release/photonic-signing.pub -f`
2. Update the `PHOTONIC_SIGNING_KEY` secret with the new base64.
3. Ship a new app build (the new public key is embedded) **before** the next
   release — clients can only verify releases signed by the key they shipped with.
