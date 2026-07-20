# 23 — Legal and Open-Source Implementation Routes

Status: Accepted implementation policy; D-8 CPU reference, GPU parity, and GPU safety slices implemented  
Date: 2026-07-12  
Audience: Photonic product owner, maintainers, legal reviewer, implementation agents

## 1. Purpose and authority

This document defines narrow, legally conservative implementation routes for:

- G-20 local-file multicam;
- D-3 starter music and ambient SFX;
- D-8 still-panorama reframe and little-planet projection;
- D-12 gyro-metadata stabilization;
- D-13 HDR/HLG/PQ and 10-bit delivery;
- D-14 still-panorama stitching.

It supplements [ROADMAP.md](ROADMAP.md), [20-pro-workflows.md](20-pro-workflows.md), [21-dji-core-workflows.md](21-dji-core-workflows.md), and [22-dji-advanced-workflows.md](22-dji-advanced-workflows.md). Those owner documents remain the detailed feature contracts.

Product/legal/engineering acceptance of S1–S5, §4.6 defaults, and applicable implementation policy was recorded from the user on 2026-07-12. [SPEC.md](SPEC.md) and [ROADMAP.md](ROADMAP.md) carry resulting scope/status. Authorization does not create missing fixture, dependency, patent, trademark, or distribution evidence; each release gate still requires its named record. This packet is engineering policy, not legal advice.

Research rule: an upstream repository's code license does not automatically license its example images, media, LUTs, lens profiles, test fixtures, model weights, trademarks, patents, or dependencies.

## 2. Outcome

This packet proposes a Photonic-owned route for every item. Selected routes avoid copying GPL code and do not require redistributed DJI assets; implementation intake and legal review must still validate each route.

| Item | Recommended route | Status after §4 approval | Residual hard gate |
|---|---|---|---|
| G-20 | Native sync, group, multiview, and cut planner; permissive DSP crates only after audit | `legal-or-fixture-blocked` | Synthetic/owned sync corpus; decoder budget |
| D-3 | `Photonic Starter Audio` pack: commissioned, internally authored, or verified CC0/CC-BY assets | `legal-or-fixture-blocked` | Per-asset composition, master, performer, release, and redistribution evidence |
| D-8 | Photonic CPU/WGSL projection math from published geometry | `legal-or-fixture-blocked` | Owned equirectangular fixtures and projection goldens |
| D-12 | Audited permissive telemetry adapter plus Photonic stabilization math and warp | `legal-or-fixture-blocked` | Parser dependency audit, camera metadata, lens provenance, captured fixtures |
| D-13 | Photonic color math from standards; existing FFmpeg sidecar for approved encoders | `legal-or-fixture-blocked` | Reference vectors, encoder matrix, patent/distribution review, color defaults |
| D-14 | Photonic deterministic stitch pipeline; Apache OpenCV limited to optional prototype/validation | `legal-or-fixture-blocked` | Capture corpus, CV algorithm/patent review, quality thresholds |

`legal-or-fixture-blocked` means design and documentation may proceed. No affected asset, public fixture, dependency, codec binary, or compatibility claim enters a release until its evidence record passes. Private/local-only fixtures remain subject to §12 and cannot satisfy release acceptance.

## 3. Governing policy

### 3.1 Intake classifications

| Classification | Meaning |
|---|---|
| `ADOPT` | A bounded dependency or vendored module may enter implementation review after transitive, patent, security, maintenance, and architecture gates pass. |
| `ADAPT` | Reuse a permissive component behind a Photonic interface, or maintain a narrow attributed fork. |
| `VALIDATE` | Dev-only comparison or fixture-generation tool; never a shipped runtime dependency or source-copy oracle. Generated outputs need their own provenance and redistribution review. |
| `CLEAN-ROOM` | Implement from public standards, published equations, original requirements, and owned fixtures; do not inspect incompatible source. |
| `SUBPROCESS` | Optional external executable with an explicit process boundary and separate distribution/runtime review. |
| `HOLD` | Research candidate with unresolved license, dependency, provenance, or architecture evidence; unavailable for implementation until reclassified. |
| `REJECT` | Do not link, vendor, translate, copy, bundle, or use as implementation source. |

### 3.2 Default license policy

- Preferred code licenses: `MIT`, `Apache-2.0`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, and `Zlib`.
- `Apache-2.0` is preferred where patent exposure is material because it includes an express contributor patent grant; it is not a complete freedom-to-operate opinion.
- `LGPL`, `MPL`, `EPL`, `CDDL`, custom, source-available, or ambiguous multi-license offers require written legal and architecture approval before use. An unambiguous `MIT OR Apache-2.0` choice may follow the normal preferred-license intake.
- `GPL` and `AGPL` code must not be linked, vendored, translated, or copied into Photonic. An executable may be considered only under the SPEC's subprocess rule and a separate distribution decision; no such executable is selected here.
- Repository labels and package-registry metadata are discovery aids, not approval evidence. Intake reads the actual license files, file headers, manifests, submodules, generated-code notices, and enabled-feature dependency graph.
- Every adopted file retains required copyright, license, and NOTICE text. Use precise SPDX expressions, including `OR`, `AND`, and `WITH`.
- No Cargo dependency is added until `cargo deny`, advisory, unsafe-code, build-script, reproducibility, and maintenance-owner reviews pass. Native tools and subprocesses require equivalent license, SBOM, build-configuration, packaging, and security review.

### 3.3 Required evidence record

Add one record per upstream component before an implementation change:

```rust
pub struct ThirdPartyUseRecord {
    pub component: String,
    pub upstream_url: String,
    pub pinned_revision: String,
    pub source_digest: String,
    pub spdx_expression: String,
    pub selected_files_or_features: Vec<String>,
    pub enabled_build_features: Vec<String>,
    pub transitive_license_report: String,
    pub build_script_review: String,
    pub notices: Vec<String>,
    pub patent_review: PatentReviewState,
    pub trademark_review: TrademarkReviewState,
    pub security_review: SecurityReviewState,
    pub maintenance_owner: String,
    pub approved_for: ApprovedUse,
}

pub enum ApprovedUse {
    RuntimeDependency,
    VendoredFork,
    DevValidationOnly,
    OptionalSubprocess,
}
```

Tracked evidence belongs in `THIRD_PARTY.md`, dependency manifests, and a machine-readable third-party registry chosen during implementation planning. `_arcwright-output` is research history, not release evidence.

### 3.4 Clean-room protocol

The clean-room label is mandatory for code written to replace a rejected or incompatible implementation.

1. A requirements author records only standards, public papers, mathematical facts, file-format documentation, observed device facts, and owned fixture behavior.
2. The implementer does not inspect, translate, paraphrase, or use line-by-line output from rejected source.
3. The provenance record names every normative source and fixture, the author, review date, and excluded codebases.
4. Tests derive from published equations, independently generated values, synthetic signals, or rights-cleared captures—not copied upstream test assets.
5. A second reviewer checks source separation, identifiers, comments, constants, control flow, and test provenance before merge.
6. If rejected source has already been inspected for a specific subsystem, assign that subsystem to an independent implementer or adopt a legally compatible component instead.

Black-box observation of public product behavior requires a written observation protocol, owned inputs, independently recorded outputs, and legal approval. It must not expose rejected source, copied test assets, or implementation-specific traces to the clean-room implementer.

Algorithms and facts are not a substitute for patent review. Avoid claims such as “patent free” without counsel-supported evidence.

## 4. Accepted narrow SPEC amendments

Accepted 2026-07-12. These changes avoid reopening full excluded categories.

### S1 — D-3 starter audio

Replace the stock-content non-goal with:

> Online stock catalogs, stock footage, account-backed licensing services, and automatic content recommendation remain out of scope. A small offline `Photonic Starter Audio` pack of rights-cleared music beds and ambient SFX is in scope. Every bundled item requires a release-grade rights manifest and must work without a network.

Use `Photonic Starter Audio` as the pack name. Reserve the `Photonic Original` source badge for Photonic-owned or commissioned assets; label CC0 and CC-BY items by their actual source/license.

Replace **`SPEC.md` → Decisions → D-11 (v1 title templates / stock media)** in full with:

> D-11: v1 ships a small starter set of vector-based title/lower-third templates and may ship a small offline `Photonic Starter Audio` pack under the asset-rights gate in `23-legal-open-source-implementation-routes.md`. Online stock catalogs, stock footage, account-backed licensing services, automatic content recommendation, and media without release-grade rights evidence remain out of scope. (Product/legal review; proposed 2026-07-10)

This SPEC decision is not roadmap feature D-11, the beat-conformed edit-template feature owned by `22-dji-advanced-workflows.md`.

This is an installed starter pack, not a general stock library.

### S2 — D-12 stabilization

Replace the stabilization non-goal with:

> Optical-flow stabilization, motion tracking, object tracking, rolling-shutter correction, and ML horizon detection remain out of scope. Gyro-metadata stabilization with explicit synchronization and calibrated lens profiles is in scope for approved camera dialects.

### S3 — D-13 HDR and 10-bit

Replace the HDR/10-bit non-goal with:

> Native HDR display presentation, Dolby Vision, bundled HEVC encoding, and HDR authoring beyond HLG/PQ remain out of scope. Ten-bit HLG/PQ decode, explicit Rec.2020 HDR working state, HDR scopes, deterministic SDR tone mapping, and preflighted AV1 Main10 or ProRes-compatible 10-bit delivery are in scope.

Replace **`SPEC.md` → Decisions → D-09 (working color space)** in full with:

> D-09: Video working color state is explicit per sequence. `LinearRec709Sdr` remains the default for new sequences and the compatibility default for every existing project. An explicitly selected `LinearRec2020Hdr` mode is permitted for approved HLG/PQ workflows; in that mode `1.0` represents 203 cd/m² HDR Reference White and the initial mastering-peak default is 1,000 cd/m². Every existing SDR golden must remain pixel-stable. (Product/color review; proposed 2026-07-10)

After S3 acceptance and before any D-13 code authorization, update the authority cascade together:

1. `SPEC.md` non-goals and Decision D-09;
2. `00-overview.md` working-color summary and locked-decision references;
3. `03-render-color-pipeline.md` normative decode, working, display, and export boundaries;
4. `22-dji-advanced-workflows.md` D-13 color-state and compatibility contract; and
5. `11-testing-phasing.md` and the existing SDR golden/SS-1 regression policy.

### S4 — G-20 multicam

Replace the multicam non-goal with:

> Live capture, broadcast switching, remote sources, collaborative switching, and more than nine simultaneously keyboard-addressable angles remain out of scope. Local-file multicam grouping, timecode/audio/marker/manual sync, multiview preview, and frame-accurate angle cuts are in scope.

### S5 — D-8/D-14 still panoramas

Replace the 360/VR non-goal with:

> VR authoring, headset preview, spatial audio, stereoscopic delivery, and 360-degree video timeline editing remain out of scope. Still-image panorama-set stitching, equirectangular/spherical projection, virtual-camera reframe, and little-planet output are in scope.

Each amendment is independently approvable. Acceptance of S1 moves only D-3; S2 only D-12; S3 only D-13; S4 only G-20; and S5 only D-8/D-14. An item moves from `product-blocked` to `legal-or-fixture-blocked` immediately after its own SPEC amendment; unrelated amendments do not hold it. It moves to `open` or `partial` only after its named legal/fixture gates pass and the ROADMAP status audit confirms its implementation evidence.

S1–S5 scope and §4.6 defaults are authorized. D-8 CPU Slice 0, standalone GPU parity, and GPU device/layout safety slices have explicit code authorization and were implemented under the clean-room fence on 2026-07-12. Other item slices still require their named empirical evidence and scoped dispatch record before code or dependency work.

### 4.6 Accepted product defaults

| Decision | Recommended v1 default | Expansion boundary |
|---|---|---|
| G-20 audio | Preserve the user-designated primary camera audio across angle cuts. `FollowVideo`, another camera, `MixAll`, or `None` requires explicit selection. This follows `20-pro-workflows.md` D-G20-01 and, after S4 acceptance, supersedes its softer FollowVideo blocker prose. | No automatic ISO mix or silent audio-source switching. |
| D-3 pack policy | Pack name `Photonic Starter Audio`; permit Photonic-owned, commissioned, verified CC0, and reviewed CC-BY 4.0. Show source/license badges and offline credits. | CC-BY-SA, NC, ND, unclear, scraped, and platform-only assets stay excluded. |
| D-12 first matrix | Photonic gyro JSON interchange plus one advertised native target: DJI Avata 2 MP4 with the exact firmware/camera mode/lens entries proven by owned fixtures. | O3/O4, Action, Neo, and additional modes remain unadvertised until their independent adapter/profile fixtures pass. |
| D-13 working defaults | HDR Reference White `203 cd/m²`; mastering peak `1,000 cd/m²`; SDR remains project/import default. | User overrides require explicit sequence state and scope/export relabeling. |
| D-13 initial export | AV1 Main10 4:2:0 10-bit in MP4 for HLG and PQ; ProRes 422 HQ-compatible 4:2:2 10-bit in MOV only where the shipped FFmpeg encoder/tag matrix passes. | Native HDR display, Dolby Vision, bundled HEVC, 12-bit, and image-sequence HDR delivery remain deferred. |

Accepted 2026-07-12. Expansion boundaries remain hard limits until separately amended.

## 5. Upstream evidence and dispositions

The links below identify research-time upstream license signals. They are not revision-specific release approvals. At implementation intake, pin a revision and re-verify license files, file headers, manifests, dependencies, notices, and selected features in a §3.3 evidence record.

| Project | Research license signal | Relevant capability | Disposition |
|---|---|---|---|
| [Gyroflow](https://github.com/gyroflow/gyroflow) | `GPL-3.0` with additional permissions | Mature gyro stabilization | `REJECT` for linking, copying, porting, or implementation reference. Public product behavior may inform requirements only under §3.4 separation. |
| [telemetry-parser](https://github.com/AdrianEddy/telemetry-parser) | `MIT OR Apache-2.0`; actual license files and manifest | Rust parser advertises DJI Avata, O3/O4, Action, and Neo formats | `ADAPT`. Direct adoption is conditional because the manifest uses git-sourced parser dependencies. Prefer a feature-reduced upstream contribution or narrow attributed fork after full transitive audit. |
| [Gyroflow lens profiles](https://github.com/gyroflow/lens_profiles) | Repository `CC0-1.0` legal text | Lens calibration records | Conditional `ADOPT` as a pinned data snapshot only after per-entry provenance, camera mapping, accuracy, privacy, and trademark review. CC0 gives no warranty and does not clear patent/trademark/third-party rights. |
| [stmap-undistort](https://github.com/gyroflow/stmap-undistort) | MIT; archived | ST-map conversion prototype | `VALIDATE` only. Archived Python/OpenCV utility is not a production architecture. |
| [OpenCV](https://github.com/opencv/opencv) | Apache-2.0 for current 4.x releases; inspect contrib modules separately | D-14 features, homography, stitching, and calibration; general validation | Conditional `VALIDATE`/`ADAPT`. It is not a D-12 stabilization path. Default plan keeps it out of the shipped runtime; a later native-dependency proposal must justify binary size, C++ ABI, enabled modules, notices, determinism, and patent review. |
| [Pannellum](https://github.com/mpetroff/pannellum) | MIT code; README separately identifies a CC-BY-SA example panorama | Equirectangular web viewer | `VALIDATE` for projection behavior only. Do not import the web stack or example asset. Photonic owns native CPU/WGSL projection. |
| [polysync](https://github.com/jianshuo/polysync) | MIT repository | Python audio-envelope multicam sync | `VALIDATE` only. Useful low-maturity comparison for offset/drift reports; do not add Python runtime or treat project claims as acceptance evidence. |
| [Rust-CV](https://github.com/rust-cv/cv) | Monorepo; licenses vary by crate | Rust geometry and vision building blocks | `HOLD`. Verify every selected crate and transitive dependency; never approve the monorepo from a blanket badge. Prefer existing `nalgebra`, `glam`, and Photonic-owned bounded algorithms where feasible. |
| [rustfft](https://github.com/ejmahler/RustFFT) | `MIT OR Apache-2.0` | FFT for audio correlation | `ADOPT` candidate after dependency/performance audit; own windowing, envelope, normalization, confidence, and drift policy. |
| [Rubato](https://github.com/HEnquist/rubato) | `MIT OR Apache-2.0` | Deterministic audio resampling | `ADOPT` candidate if existing decode resampling is insufficient. |
| [OpenColorIO](https://github.com/AcademySoftwareFoundation/OpenColorIO) | BSD-3-Clause plus third-party notices | Production color transforms/configuration | `VALIDATE` only in D-13 v1. C++ runtime and ACES/OCIO scope are explicitly deferred. |
| [Colour](https://github.com/colour-science/colour) | BSD-3-Clause | Color-science equations and numerical comparison | `VALIDATE` in an isolated dev environment. Do not add Python to Photonic runtime or copy datasets without separate license checks. |
| [rav1e](https://github.com/xiph/rav1e) | BSD-2-Clause; repository includes a separate patent file | AV1 8/10/12-bit encoding | Conditional `SUBPROCESS`; existing shipped FFmpeg sidecar remains the default encoder boundary. A direct library would violate the selected sidecar architecture unless SPEC changes. |
| [dav1d](https://code.videolan.org/videolan/dav1d/) | Primarily BSD-2-Clause with permissive ISC/OpenBSD-variant files | AV1 decode | `SUBPROCESS` through the approved FFmpeg distribution. Preserve all file-specific notices if ever distributed separately. |

Known rejections for production linking also include GPL Hugin/libpano, GPL Kdenlive, and GPL OBS. Their existence proves feasibility, not license compatibility.

## 6. G-20 — Photonic multicam route

### 6.1 Dependency choice

Build the multicam model, sync report, apply operation, multiview scheduler, and cut semantics in Photonic. Reuse existing FFmpeg-sidecar PCM decode and waveform caches. Add `rustfft` only if measured direct correlation needs it; add Rubato only for explicit sample-rate/drift normalization. Keep polysync as a dev-only comparison, not a dependency.

### 6.2 Service boundary

```rust
pub trait MulticamSyncEngine {
    fn analyze(
        &self,
        sources: &[SyncSource],
        method: SyncMethod,
        cancel: &CancelToken,
    ) -> Result<MulticamSyncReport, MulticamSyncError>;
}

pub struct AudioSyncConfig {
    pub analysis_rate_hz: u32,
    pub envelope_window_ms: u32,
    pub max_offset: Tick,
    pub drift_segments: u16,
    pub min_confidence: f32,
}
```

Audio algorithm: decode local mono analysis audio; resample to the frozen analysis rate; remove DC; derive log-energy envelope; normalize robustly; correlate bounded lags; refine peak; fit offset/drift across windows; emit confidence, ambiguity ratio, and warnings. It produces a report only. A user or explicit MCP call applies offsets atomically.

Timecode, marker, and manual sync remain dependency-free. No voice identification, face recognition, fingerprints, uploads, or remote matching.

### 6.3 Legal and fixture exit

- Dependency evidence records for enabled DSP crates.
- Synthetic pulse, music-like, silence, low-SNR, repeated-pattern, and known-drift audio authored in tests.
- Rights-cleared real multi-camera capture for acceptance; source audio never enters logs or repository unless its release permits it.
- Frozen low-confidence refusal and one-frame alignment tolerances.
- Multiview decoder/memory cap measured before advertising nine-angle support.

## 7. D-3 — Photonic Originals content route

### 7.1 Rights hierarchy

Use sources in this order:

1. Photonic-authored procedural SFX and music with documented human authorship and company assignment.
2. Commissioned recordings/compositions with written worldwide, perpetual rights to reproduce, modify, synchronize, sublicense with the application, and let users distribute rendered works.
3. Verified CC0 material from the actual rightsholder, with provenance and privacy/publicity review.
4. CC-BY 4.0 only when attribution, modification marking, sublicensing/distribution behavior, and offline credit surfaces are product-approved.

Reject default bundling of CC-BY-SA, NC, ND, editorial-only, personal-use, platform-only, revocable, unclear, scraped, or license-by-search-filter media. Reject generative-service output unless counsel approves its terms, training/provenance posture, copyrightability risk, and commercial redistribution rights.

Music requires separate evidence for the composition and sound recording. A performer or producer cannot grant rights they do not own. Field recordings require voice, location, privacy, publicity, and incidental copyrighted-performance checks where applicable.

### 7.2 Manifest

Extend §21's bundled asset manifest with:

```rust
pub struct AssetRightsManifest {
    pub item: String,
    pub sha256: String,
    pub spdx_or_contract_id: String,
    pub license_version: Option<String>,
    pub copyright_owner: String,
    pub composition_owner: Option<String>,
    pub master_owner: Option<String>,
    pub territories: Vec<String>,
    pub term: RightsTerm,
    pub restrictions: Vec<String>,
    pub third_party_materials_cleared: bool,
    pub performers_released: bool,
    pub locations_released: bool,
    pub release_evidence: Vec<String>,
    pub permitted_uses: Vec<AssetUse>,
    pub attribution_text: Option<String>,
    pub modification_notice: Option<String>,
    pub provenance_url_or_contract: String,
    pub evidence_digest: String,
    pub reviewer: String,
    pub review_date: String,
}

pub enum RightsTerm {
    Perpetual,
    Fixed { starts_on: String, ends_on: String },
}

pub enum AssetUse {
    Bundle,
    Audition,
    Edit,
    Synchronize,
    Render,
    CommercialOutput,
    RedistributeWithApp,
}
```

Build validation fails on a missing field, mismatched digest, expired term, unsupported release territory/channel, uncleared third-party material, disallowed use, missing release evidence, or absent license text. A limitation is acceptable only when packaging and product output enforce it; otherwise exclude the item. Pack bytes and evidence are release artifacts; project JSON stores only immutable pack/item/version/digest identity and an attribution snapshot. Confidential contracts stay in access-controlled evidence storage; tracked manifests reference their immutable evidence digests.

### 7.3 Own-system alternative

The first pack may be entirely Photonic-authored:

- procedural pink-noise/wind/water texture beds generated offline from deterministic authored recipes;
- recorded room tone and nature ambiences captured under controlled releases;
- original loopable music beds using owned synthesis patches and no third-party samples.

The generator itself may be MIT; generated audio receives an explicit Photonic asset license granting application bundling and unrestricted rendered-output use. Never assume the code license determines generated-media rights.

## 8. D-8 — Native still-panorama projection

Implement equirectangular-to-rectilinear and stereographic little-planet projection from published spherical geometry. Use one CPU reference and WGSL implementation with shared coordinate conventions. Pannellum may validate expected navigation/projection behavior; do not copy its source or bundled panorama.

```rust
pub struct PanoramaProjectionSpec {
    pub input: PanoramaInputProjection,
    pub output: PanoramaOutputProjection,
    pub yaw_deg: f32,
    pub pitch_deg: f32,
    pub roll_deg: f32,
    pub field_of_view_deg: f32,
    pub zoom: f32,
    pub seam_offset_deg: f32,
    pub seam_wrap: bool,
}

pub enum PanoramaInputProjection { Equirectangular }
pub enum PanoramaOutputProjection { Rectilinear, StereographicLittlePlanet }
```

All angular parameters must be finite. Rectilinear `field_of_view_deg` is horizontal, with vertical extent derived from output aspect ratio; require `1° < field_of_view_deg < 179°`. Stereographic plane +X points right and plane +Y points toward screen top; both axes divide by finite `zoom > 0`, horizontal also includes output aspect. Inverse mapping is tangent at south pole: center = `-Y`, radius 1 = horizon, infinity → `+Y`; `field_of_view_deg` is ignored. `seam_offset_deg` wraps modulo 360°. Slice 0 always uses seam-aware horizontal sampling and vertical pole clamp; transparent-edge output remains a later effect-integration concern.

Acceptance uses Photonic-authored analytic grids, longitude/latitude labels, seam impulses, poles, checkerboards, and owned panoramas. CPU/GPU goldens cover seam wrap, pole singularities, extreme FOV rejection, alpha, keyframes, proxy equivalence, and export.

No DJI panorama file is bundled. Metadata detection uses documented tags or user confirmation; trademark language is descriptive (“compatible with”), never “official.”

### 8.1 First-code scope and provenance fence

Authorized D-8 CPU Slice 0 is a pure CPU reference kernel plus generated in-memory analytic tests. It may add only projection input/output types, validation, deterministic sampling, and tests. It must not add persisted effect state, IR/compiler/evaluator integration, WGSL, GUI, MCP, fixture files, downloads, dependencies, or D-14 work.

Authorized D-8 GPU parity slice is a standalone one-pass WGSL kernel, driver, and generated in-memory CPU/GPU parity suite. It reuses CPU validation and rotation invariants and may expose only the minimum crate-private CPU helpers needed for parity. It must not add persisted effect state, IR/compiler/evaluator integration, GUI, MCP, metadata detection, fixture files, downloads, dependencies, or D-14 work. This slice was completed on 2026-07-12 with Luna `SPEC_COMPLIANT`, Grok `APPROVE`, 14 focused CPU/GPU tests passing, and worst observed absolute channel error `0.00061941147`.

Before assignment, record: accepted S5 text; branch baseline commit/digest; first-release input/projection matrix; normative geometry sources and revisions; clean-room implementer attestation; independent provenance reviewer; synthetic-fixture plan; rights-cleared real-panorama acquisition owner. The implementer must not inspect Pannellum or any rejected/incompatible projection source. Review checks identifiers, comments, constants, control flow, and test provenance against this record.

CPU reference is normative. The implemented GPU parity slice reuses its coordinate convention/constants and passes seam, pole, alpha, validation, and repeatability tests in the linear-premultiplied working convention. A rights-cleared real corpus remains required for effect integration and release acceptance; generated analytic pixels authorize only the isolated CPU/GPU kernels.

Pre-integration hardening gate: the standalone GPU kernel now validates input/output dimensions against active device texture limits, uses checked upload/readback row and transfer arithmetic, rejects layouts unsafe for the existing readback contract, and returns structured diagnostics before per-call resource creation. Effect/IR integration must preserve this preflight when it adds direct source-texture rendering; it must not rely on a wgpu validation failure or integer wrap.

## 9. D-12 — Permissive parser plus native stabilization

### 9.1 Parser decision

Use `MotionMetadataAdapter` from §22 as the stable boundary. Preferred intake order:

1. contribute or request an upstream feature-reduced `telemetry-parser` configuration that excludes unused formats and git dependencies;
2. maintain a narrow attributed fork of approved DJI/container modules under the revision's verified license option, retaining all required MIT/Apache notices and upstream history;
3. a Photonic-authored parser for a documented interchange and one camera dialect at a time.

Do not vendor the full crate until its git-sourced `mp4parse` and `fc-blackbox` dependencies, build scripts, generated protocol code, enabled format modules, unsafe code, and file-specific licenses pass. Do not copy Gyroflow stabilization code.

### 9.2 Lens profiles

Support three sources:

- Photonic-calibrated profile from owned checkerboard/grid captures;
- user-installed profile, treated as user data;
- pinned CC0 Gyroflow lens-profile snapshot after per-entry intake.

Every bundled profile records upstream path/revision/digest, contributor/provenance if available, camera/lens/mode/resolution, calibration dimensions, model and coefficients, RMS error, sample count, independent validation error, and reviewer. An unclear entry is excluded without blocking user import.

### 9.3 Native math

Photonic owns unit/axis normalization, bias estimation, resampling, quaternion integration, deterministic smoothing, gravity confidence, correction orientation, crop path, and CPU/WGSL mesh warp. Normative equations come from cited textbooks/papers or standards in a provenance record. GPL output is not a calibration oracle.

Release sequence:

1. Photonic gyro JSON and synthetic rotations.
2. One permissively parsed DJI dialect with owned capture.
3. One validated Photonic lens profile.
4. CPU stabilization and static-safe crop.
5. GPU parity, GUI, MCP, export.
6. Additional devices only through repeated adapter/profile/fixture gates.

### 9.4 Exit criteria

- Parser corpus contains malicious-size/count/truncation cases and no copyrighted source payload beyond approved fixtures.
- Clock, axes, scale, sampling rate, rolling orientation, and lens mode are verified for each advertised device/mode.
- Static, constant-rate, impulse, drift, vibration, horizon, and impossible-crop cases pass.
- Dependency and data notices ship; no `Gyroflow` endorsement is implied.
- Device serials, precise timestamps, GPS, operator names, and other unrelated metadata are discarded or redacted before caching, logging, fixture publication, and default MCP output.

## 10. D-13 — Standards-based HDR and 10-bit route

### 10.1 Runtime decision

Implement BT.2100 HLG/PQ transfer, Rec.2020 matrices, luminance normalization, scopes, gamut warnings, and BT.2446 SDR tone mapping in Photonic CPU/WGSL modules. Centralize constants and cite the exact standards revision. Do not ship OCIO or Colour as v1 runtime dependencies.

Use OpenColorIO and Colour only for independently scripted numerical cross-checks under their BSD-3-Clause terms. Validation scripts must record version/config/dataset licenses and cannot import third-party LUTs or images by default.

Keep encode/decode behind the existing FFmpeg sidecar. Prefer an audited AV1 Main10 path. A separate rav1e executable is an optional fallback only after its BSD notice, AOM patent-license obligations, packaging, quality, performance, and metadata support are reviewed. ProRes output must be described as compatible unless trademark/certification review permits stronger wording. No bundled HEVC promise.

### 10.2 Reference data

Generate redistributable scalar and image vectors from the normative equations with Photonic-authored scripts. Store source citations, standard revision, generator digest/version, precision, expected values, tolerance, and reviewer. Do not commit copied standards PDFs or upstream test images unless their redistribution license is separately recorded.

Owned fixture set:

- 10-bit code-value ramps and chroma ramps;
- Rec.2020 primaries/secondaries and out-of-gamut patches;
- HLG and PQ scalar boundary vectors;
- reference-white and peak-nit steps;
- mixed SDR/HLG/PQ timelines;
- metadata-missing, range-mismatch, and encoder-rejection cases;
- decode-back bit-depth/tag verification files produced by the approved toolchain.

### 10.3 Patent and distribution gate

Software copyright permission and codec patent permission are separate. Before release, record:

- exact FFmpeg binary source/configuration and every enabled library/license;
- codec/container/pixel-format/metadata combinations;
- AOM patent-license notice and reciprocal/defensive terms for AV1 distribution;
- territory/channel review for distributed encoder binaries;
- trademark wording for AV1, ProRes, DJI, and other compatibility names;
- generated SBOM, license notices, and reproducible binary digest.

The AOM terms are one license input, not a freedom-to-operate conclusion. No encoder availability, legal assumption, or 10-bit capability may be inferred at runtime. Export preflight probes the shipped binary and fails closed.

## 11. D-14 — Native deterministic panorama stitcher

### 11.1 Production route

Build a bounded Photonic pipeline behind internal interfaces:

```rust
pub trait FeatureExtractor {
    fn extract(&self, image: &AnalysisImage) -> Result<FeatureSet, StitchError>;
}

pub trait MatchEstimator {
    fn match_pair(
        &self,
        left: &FeatureSet,
        right: &FeatureSet,
        seed: u64,
    ) -> Result<PairModel, StitchError>;
}

pub trait PanoramaSolver {
    fn solve(&self, graph: &MatchGraph, seed: u64) -> Result<CameraSolution, StitchError>;
}
```

Stages remain those in §22: validate; normalize previews; extract bounded features; match expected neighbors; robustly estimate pair models; solve global cameras; warp; compensate exposure; choose seams; multiband blend; crop; register derived panorama.

Use existing `image`, `nalgebra`, `glam`, worker, cache, and render infrastructure first. Select feature/descriptor algorithms only after a documented patent and quality review. A permissive implementation license does not prove the underlying technique is free of third-party patent claims.

### 11.2 OpenCV boundary

OpenCV 4.x is Apache-2.0 and can be considered for:

- an isolated prototype to measure achievable quality;
- a dev-only validation oracle on owned fixtures;
- a later optional analysis backend if native quality is insufficient.

It is not the default shipped dependency. A production proposal must identify exact `opencv` and `opencv_contrib` modules/files, native libraries, transitive codecs, platform packaging, C++ ABI, security updates, determinism differences, required notices, and patent assessment. Hugin/libpano GPL code is rejected.

### 11.3 Fixture and quality gate

Capture or commission owned Sphere/Wide/Vertical grids with documented camera, firmware, mode, exposure, ordering, overlaps, missing/duplicate variants, and expected output. Strip or permission EXIF GPS before committing. Synthetic homography/rotation sets cover exact geometry; real sets establish seam, exposure, solve, and performance thresholds.

First release is processed JPEG/TIFF only. RAW/DNG remains a separate license, camera-color, and D-13 gate. Do not advertise “DJI RAW panorama” until that slice passes.

## 12. Cross-cutting provenance manifests

```rust
pub struct FixtureRightsManifest {
    pub fixture: String,
    pub sha256: String,
    pub owner: String,
    pub source_method: FixtureSource,
    pub redistribution_allowed: bool,
    pub modification_allowed: bool,
    pub public_ci_allowed: bool,
    pub contains_voice_or_likeness: bool,
    pub contains_location_or_gps: bool,
    pub contains_device_or_account_identifier: bool,
    pub redaction_report: Option<String>,
    pub releases: Vec<String>,
    pub license_or_contract: String,
    pub reviewer: String,
}

pub enum FixtureSource {
    Synthetic,
    PhotonicCaptured,
    Commissioned,
    ThirdPartyLicensed,
    LocalOnlyUserFixture,
}
```

Local-only fixtures may validate a developer machine but cannot satisfy public CI, release reproducibility, or redistributed acceptance. Synthetic fixtures prove math and error handling; at least one rights-cleared real corpus proves each advertised device/workflow.

All manifests validate schema, path containment, digest, duplicate identity, required notices, release presence, use restrictions, and sensitive-metadata disposition. Network URLs are provenance only; release and CI never fetch mutable assets.

## 13. Delivery waves

Implementation waves do not start until applicable predecessor exits are accepted. Planning, legal review, and evidence acquisition may proceed in parallel. “Implementation” includes dependency installation, source download into the repo, migrations, product tests, and code; every such activity needs explicit authorization under §14.

| Wave | Work | Exit |
|---|---|---|
| `L0` | Review this packet; legal counsel selects review jurisdiction/channel; name owners | Product, legal, and engineering owners recorded |
| `L1-S1`…`L1-S5` | Independently apply an accepted SPEC amendment; update only its ROADMAP items and owner-doc gates | That amendment's SPEC/ROADMAP review clean; still no code authorization |
| `L2` | During product review, define third-party/asset/fixture manifest schemas, clean-room procedure, and evidence storage | Compliance contracts ready for approval; no product code |
| `L3A-*` | In parallel per item, acquire/create rights-cleared multicam, panorama, gyro/lens, HDR, or audio corpora | That item has synthetic plus required real evidence |
| `L3B-*` | After separate authorization, run per-item dependency spikes in throwaway branches/worktrees: telemetry parser, DSP, OpenCV validation, encoder matrix | Item-specific license/security/perf report; no automatic merge |
| `L4-*` | After separate code authorization, implement only the shared contracts required by the approved item: provenance, derived media, color state, projection, sync, or motion interfaces | Item-specific serde/migration/undo architecture review |
| `L5A` | G-20 and D-8 independent implementation | Owner-doc acceptance green |
| `L5B` | D-3 pack registry/content, D-12 adapter/integration, D-13 color core | Per-item legal/fixture gates green |
| `L6` | D-14 processed-image stitcher after D-8; D-13 export after color core | CPU/GPU/export and offline acceptance green |
| `L7` | GUI/MCP parity, docs, attribution, SBOM, packaging, final legal review | Goal-backward L1-L4 verification and release sign-off |

An item's `L1`, `L3A`, and `L3B` gates are independent of unrelated scope amendments and corpora. G-20/D-8 may run in parallel. D-12 parser work and D-13 color work may run in parallel after separate fixture gates. D-14 waits on D-8 output contract. D-13 export waits on decode/working/scopes/tone-map acceptance. Rights acquisition and compliance-schema design may run in parallel with product review but do not authorize product code.

## 14. Stop/go checklist before any code

Acceptance recorded 2026-07-12. D-8 CPU Slice 0 disposition:

- [x] Product accepted S1–S5 and §4.6 defaults.
- [x] SPEC and ROADMAP updated; affected items moved to `legal-or-fixture-blocked`.
- [x] Legal policy, escalation rules, compatibility wording, and rights templates accepted for engineering intake; release evidence remains per item.
- [x] D-8 Slice 0 uses no `HOLD`/conditional component or new dependency.
- [x] D-8 clean-room implementer must attest no incompatible projection-source inspection; independent review required before merge.
- [x] In-memory analytic-test plan and rights-cleared real-panorama acquisition plan accepted; real corpus remains a release gate.
- [x] D-8 Slice 0 needs no dependency spike.
- [x] D-8 Slice 0 follows existing D-8 eval budget/target-platform contract; measurement occurs before expansion.
- [x] D-8 Slice 0 changes no migration, undo, cache, privacy, GUI, or MCP surface; later owners remain required before those slices.

D-8 GPU parity-slice disposition:

- [x] User authorized continuation as original Photonic code; Terra recorded the clean-room allowed/excluded-source attestation.
- [x] GPU code is limited to the standalone projection kernel, original embedded WGSL, upload/readback driver, and generated parity tests.
- [x] No dependency, fixture, effect/IR integration, persistence, GUI, MCP, metadata, release asset, or D-14 surface changed.
- [x] Luna re-review returned `SPEC_COMPLIANT`; Grok returned `APPROVE` with no blockers.
- [x] Focused tests, library check, focused Clippy, direct formatting check, and diff/scope audit passed.
- [x] GPU safety follow-up rejects device-limit, pitch/alignment, transfer-size, host-size, and readback-limit failures before per-call resources; Luna returned `SPEC_COMPLIANT` and Grok returned `APPROVE`.

User authorization is recorded as the product/legal/engineering decision input. Reviewer identity, jurisdiction, and professional qualification were not independently verified by the implementation agents; distribution decisions remain subject to project governance.

Until every applicable box is checked, agents may research and edit plans only. They must not run project code, add dependencies, download upstream source into the workspace, implement features, generate migrations, or change release assets.

Agent-proof boundary: editing `23`, its review artifacts, or other planning documents is allowed. Editing product crates, any `Cargo.toml`/lockfile, application `assets/`, fixtures, or migrations—and running `cargo`, FFmpeg, dependency installers, fixture generators, or downloaded binaries—is forbidden until separately authorized by the applicable checklist.

## 15. Final acceptance

This packet is complete when:

1. Every selected upstream component has a revision-specific evidence record and no unreviewed transitive license.
2. Every bundled byte has a rights manifest, digest, required notice, and permitted-output grant.
3. Every public fixture is synthetic, owned, commissioned, or explicitly redistribution-licensed.
4. GPL/AGPL code and assets are absent from Photonic binaries/source; subprocess exceptions are explicit.
5. Codec, CV, and color-standard patent questions have a recorded disposition rather than an assumption.
6. Compatibility names avoid endorsement and certification claims.
7. Each feature passes its owner spec's deterministic, undo, serde, privacy, offline, performance, CPU/GPU, GUI/MCP, and export gates.
8. Existing SDR, vector, file-format, and FFmpeg-sidecar protections do not regress.
