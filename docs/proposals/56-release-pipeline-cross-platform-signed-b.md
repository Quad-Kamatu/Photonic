# Release Pipeline: Cross-Platform Signed Binaries Published to GitHub Releases (#56) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

Only `.github/workflows/ci.yml` exists. There is no tag-triggered release job, no
binary artifact build, no signing, and no GitHub Release creation. Users must build from
source. This blocks distribution to non-Rust users and is a prerequisite for issue #57
(installers).

## Scope

**In**
- Tag-triggered GitHub Actions workflow (`release.yml`) building release binaries for
  Linux (x86_64-unknown-linux-gnu), Windows (x86_64-pc-windows-msvc), and macOS
  (x86_64-apple-darwin + aarch64-apple-darwin).
- Binary signing: Windows Authenticode (signtool via a GitHub secret PFX), macOS
  codesign + notarization (Apple Developer ID, via `notarytool`).
- Attach binaries + SHA-256 checksums to a GitHub Release; auto-generate release notes
  from conventional commits / milestone.
- Integration with `deny.toml` license check before release.

**Out**
- Flatpak/Snap/Chocolatey/Homebrew publication (issue #57).
- Automated version bumping / changelog generation (future tooling).
- ARM Linux cross-compilation (can be added later).

## Proposed Approach

### 1. Toolchain choice

Use **`cargo-dist`** (axodotdev) — it generates the release workflow, manages cross
compilation, produces archives + checksums, and creates GitHub Releases. It has first-class
support for macOS universal binaries and Windows MSVC. Alternatives (hand-rolled matrix,
`cross`, `goreleaser`) are heavier to maintain.

```toml
# Cargo.toml [workspace.metadata.dist]
[workspace.metadata.dist]
cargo-dist-version = "0.22"   # pin current stable
targets = [
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
]
installers = []   # bare archives for now; issue #57 adds installers
ci = ["github"]
create-release = true
auto-includes = true   # picks up LICENSE, README.md
```

Run `cargo dist generate` to emit `.github/workflows/release.yml` and
`.github/workflows/release-pr.yml`. Commit and pin; do not hand-edit the generated files.

### 2. Signing — Windows Authenticode

Store the PFX certificate as GitHub secret `WINDOWS_PFX_BASE64` and password as
`WINDOWS_PFX_PASSWORD`. Add a signing step after the Windows build:

```yaml
- name: Sign Windows binary (Authenticode)
  if: runner.os == 'Windows'
  shell: pwsh
  env:
    PFX_BASE64: ${{ secrets.WINDOWS_PFX_BASE64 }}
    PFX_PASSWORD: ${{ secrets.WINDOWS_PFX_PASSWORD }}
  run: |
    $pfx = [System.Convert]::FromBase64String($env:PFX_BASE64)
    [IO.File]::WriteAllBytes("cert.pfx", $pfx)
    & "C:\Program Files (x86)\Windows Kits\10\bin\10.0.19041.0\x64\signtool.exe" `
      sign /f cert.pfx /p $env:PFX_PASSWORD /tr http://timestamp.digicert.com `
      /td sha256 /fd sha256 target\release\photonic.exe
    Remove-Item cert.pfx
```

If no EV certificate is available initially, ship unsigned and document the SmartScreen
bypass. Add signing once a cert is procured.

### 3. Signing — macOS codesign + notarization

Requires Apple Developer ID Application certificate stored as secrets
`MACOS_CERTIFICATE` (base64 .p12), `MACOS_CERTIFICATE_PWD`, `APPLE_ID`,
`APPLE_TEAM_ID`, `APPLE_APP_PASSWORD`.

```yaml
- name: Import macOS certificate
  if: runner.os == 'macOS'
  run: |
    echo "$MACOS_CERTIFICATE" | base64 --decode > cert.p12
    security create-keychain -p "" build.keychain
    security import cert.p12 -k build.keychain -P "$MACOS_CERTIFICATE_PWD" -T /usr/bin/codesign
    security set-key-partition-list -S apple-tool:,apple: -s -k "" build.keychain
    security list-keychains -d user -s build.keychain

- name: Codesign + notarize
  if: runner.os == 'macOS'
  run: |
    codesign --deep --force --options runtime --sign "Developer ID Application: ..." \
      target/release/photonic
    xcrun notarytool submit target/release/photonic \
      --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" \
      --password "$APPLE_APP_PASSWORD" --wait
    xcrun stapler staple target/release/photonic
```

`cargo-dist` can handle parts of this automatically if `[dist.macos-sign]` is configured.

### 4. Checksums + GitHub Release

After all platform builds complete, a final job runs:

```yaml
- name: Generate checksums
  run: sha256sum dist/artifacts/* > dist/SHA256SUMS.txt

- name: Create GitHub Release
  uses: softprops/action-gh-release@v2
  with:
    generate_release_notes: true
    files: |
      dist/artifacts/*
      dist/SHA256SUMS.txt
```

`generate_release_notes: true` pulls from merged PRs since the last tag using GitHub's
built-in algorithm. For milestone-based notes, use `gh release create --notes-from-tag`.

### 5. Trigger and versioning

```yaml
on:
  push:
    tags:
      - 'v[0-9]+.[0-9]+.[0-9]+'
```

Require the tag to match the `version` in `Cargo.toml` (enforced via a pre-release check
step or a `release-pr.yml` that bumps the version and creates the tag). A `deny check`
step runs before any build to gate on license compliance.

## Affected Modules

- `.github/workflows/release.yml` — new file (generated by `cargo-dist` or hand-written)
- `.github/workflows/release-pr.yml` — optional version-bump PR automation
- `Cargo.toml` — `[workspace.metadata.dist]` section
- `deny.toml` — invoked in pre-release check
- GitHub repository secrets — `WINDOWS_PFX_*`, `MACOS_*`, `APPLE_*`

## Risks & Open Questions

- **Code signing cost**: Windows EV cert ~$400–700/yr; Apple Developer Program $99/yr.
  If certs are not yet available, ship unsigned initially with a documented workaround.
- **`cargo-dist` opinionation**: it generates workflow files that it then "owns". Manual
  edits will be overwritten by `cargo dist generate`. Custom signing steps must be added
  via `cargo-dist` hooks or in a wrapper workflow that calls the generated one.
- **macOS universal binary**: `cargo-dist` supports `universal2` targets; confirm wgpu +
  winit compile correctly for `aarch64-apple-darwin` (they should, but Metal shader
  compilation may require macOS 12+ runner).
- **Linux runner glibc floor**: `ubuntu-latest` currently targets glibc 2.35+. Older
  distros (CentOS 7, Ubuntu 20.04) may not run the binary. If broad Linux compat is
  needed, use `ubuntu-20.04` or cross-compile with `manylinux` container.
- **Notarization time**: `notarytool --wait` can take 1–10 minutes. Budget accordingly in
  the release job timeout.

## Acceptance Criteria

- [ ] Pushing a `vX.Y.Z` tag triggers the release workflow.
- [ ] Signed binaries for Linux, Windows, and macOS are attached to the GitHub Release.
- [ ] SHA-256 checksums file is attached alongside binaries.
- [ ] Release notes are auto-generated from commit/PR history.
- [ ] `cargo deny check` passes as a pre-release gate.

## Effort Estimate

**L** — `cargo-dist` scaffolding is S; signing setup (secrets, certs, notarization loop)
is M per platform; debugging cross-compilation edge cases is the long tail.
