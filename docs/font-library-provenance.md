# Font library provenance

## Decision

Photonic accepts the Fontsource API as a live discovery and update channel. The
catalog and font details are not pinned or signature-verified: the endpoints
used here do not expose a signed provenance contract, and maintaining a
precomputed digest allowlist for the whole catalog would make the library
unusable as an up-to-date picker.

The CDN artifact identity is pinned at install time instead. Fontsource exposes
an exact `npmVersion` for a font, and its CDN documentation says exact semver
URLs are immutable while `@latest` is a floating release. Photonic therefore
rewrites the API's `@latest` TTF URLs to `@<npmVersion>` before downloading.
See the [Fontsource API version endpoint](https://fontsource.org/docs/api/version)
and [Fontsource CDN versioning](https://fontsource.org/docs/getting-started/cdn).

This is a trust decision, not a signature scheme: a compromised or unexpectedly
updated Fontsource API response can still select a different valid semver
release. TLS, the Fontsource API, and the jsDelivr Fontsource namespace remain
the trusted upstream boundary.

## Installed cache records

Each installed manifest records:

- `fontsource_version`: the exact package version used for the install.
- `artifact_sha256`: a SHA-256 digest for every installed TTF, keyed by its
  manifest filename.

The existing size and font-parser checks still apply. Managed-font discovery,
preview reuse, and cached-install reuse require the recorded version, a complete
digest map, matching file digests, and successful font parsing. The digest is an
observed content identity that detects local cache mutation; it is not a
pre-download upstream digest or a cryptographic signature.

Manifests from before this policy are treated as legacy and are not managed or
reused. Their files are left in place; the next install downloads a versioned
replacement, while an uncached preview downloads a versioned face without
persisting it. No automatic cache deletion is performed.
