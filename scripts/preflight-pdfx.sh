#!/usr/bin/env bash
#
# preflight-pdfx.sh — Deterministic PDF/X-1a:2001 invariant checker.
#
# verapdf and PDFBox Preflight validate PDF/A, not PDF/X. This script asserts the
# specific contract a commercial printer enforces for PDF/X-1a (ISO 15930-1), so
# a PASS here is the project's "definition of done" for print-readiness:
#
#   1. PDF version 1.3            (X-1a is PDF 1.3; no PDF 1.4+ features)
#   2. Not encrypted
#   3. All fonts embedded         (zero font dependencies on the printer)
#   4. No RGB colour              (no DeviceRGB/CalRGB `rg`/`RG` operators, no RGB ICCBased,
#                                  no /Lab) — only DeviceCMYK / DeviceGray / Separation
#   5. No live transparency       (no /Group /Transparency, /SMask, non-Normal /BM,
#                                  no constant alpha /CA|/ca < 1)
#   6. OutputIntent present        (/S /GTS_PDFX with an embedded /DestOutputProfile)
#
# Complementary structural cross-check: `qpdf --check` (well-formed xref, streams,
# and objects). Reported for information only — it does NOT affect the exit status.
# (Note: verapdf validates PDF/A, not PDF/X; a valid X-1a file has no pdfaid XMP so
# `verapdf -f 1b` always fails on a metadata rule — it is not a meaningful X-1a
# cross-check and is deliberately not used here.)
#
# Usage:   scripts/preflight-pdfx.sh FILE.pdf [--quiet]
# Exit:    0 = all X-1a invariants hold; 1 = one or more failed; 2 = usage/tool error.
set -uo pipefail

QUIET=0
PDF=""
while [ $# -gt 0 ]; do
  case "$1" in
    --quiet) QUIET=1; shift ;;
    -h|--help) sed -n '2,25p' "$0"; exit 0 ;;
    *) PDF="$1"; shift ;;
  esac
done
[ -n "$PDF" ] || { echo "usage: preflight-pdfx.sh FILE.pdf" >&2; exit 2; }
[ -f "$PDF" ] || { echo "preflight: file not found: $PDF" >&2; exit 2; }
command -v qpdf >/dev/null 2>&1 || { echo "preflight: qpdf required" >&2; exit 2; }
command -v pdfinfo >/dev/null 2>&1 || { echo "preflight: pdfinfo (poppler) required" >&2; exit 2; }

FAILS=0
pass() { [ "$QUIET" -eq 1 ] || printf '\033[32m  ✓\033[0m %s\n' "$*"; }
fail() { printf '\033[31m  ✗\033[0m %s\n' "$*" >&2; FAILS=$((FAILS+1)); }
hdr()  { [ "$QUIET" -eq 1 ] || printf '\033[1mPDF/X-1a preflight: %s\033[0m\n' "$1"; }

hdr "$(basename "$PDF")"

# Normalise: decompress every stream so operators/colorspaces are greppable.
QDF="$(mktemp --suffix=.pdf)"
trap 'rm -f "$QDF"' EXIT
if ! qpdf --qdf --decode-level=all --object-streams=disable --stream-data=uncompress "$PDF" "$QDF" 2>/dev/null; then
  # qpdf may warn on GS output but still produce a usable normalised file.
  qpdf --qdf --decode-level=all --object-streams=disable "$PDF" "$QDF" 2>/dev/null || cp "$PDF" "$QDF"
fi

INFO="$(pdfinfo "$PDF" 2>/dev/null)"

# 1 ─ PDF version 1.3 --------------------------------------------------------
VER="$(printf '%s\n' "$INFO" | awk -F': +' '/PDF version/{print $2}')"
if [ "$VER" = "1.3" ]; then pass "PDF version 1.3"; else fail "PDF version is '$VER' (X-1a requires 1.3)"; fi

# 2 ─ Not encrypted ----------------------------------------------------------
ENC="$(printf '%s\n' "$INFO" | awk -F': +' '/Encrypted/{print $2}')"
case "$ENC" in no|"") pass "not encrypted" ;; *) fail "file is encrypted ($ENC)" ;; esac

# 3 ─ All fonts embedded -----------------------------------------------------
FONTS="$(pdffonts "$PDF" 2>/dev/null | tail -n +3)"
if [ -z "$FONTS" ]; then
  pass "no fonts (all text outlined to paths)"
elif printf '%s\n' "$FONTS" | awk '{print $(NF-4)}' | grep -qw no; then
  fail "one or more fonts are NOT embedded:"; printf '%s\n' "$FONTS" >&2
else
  pass "all $(printf '%s\n' "$FONTS" | grep -c .) font(s) embedded"
fi

# 4 ─ No RGB colour ----------------------------------------------------------
#   Content-stream RGB fill/stroke operators: `<r> <g> <b> rg` / `... RG`.
RGB_OPS="$(grep -aE '(^|[^A-Za-z])(rg|RG)([^A-Za-z]|$)' "$QDF" | grep -aE '[0-9.]+ +[0-9.]+ +[0-9.]+ +(rg|RG)' | grep -vc 'endobj' || true)"
RGB_CS="$(grep -acE '/DeviceRGB|/CalRGB|/Lab\b' "$QDF" || true)"
# ICCBased RGB = an ICC stream with /N 3. The OutputIntent profile is /N 4 (CMYK), so it won't match.
ICC_RGB="$(grep -aA3 '/ICCBased' "$QDF" | grep -acE '/N +3' || true)"
if [ "${RGB_OPS:-0}" -eq 0 ] && [ "${RGB_CS:-0}" -eq 0 ] && [ "${ICC_RGB:-0}" -eq 0 ]; then
  pass "no RGB colour (CMYK/Gray/Spot only)"
else
  fail "RGB colour present — rg/RG ops:${RGB_OPS} DeviceRGB/CalRGB/Lab:${RGB_CS} ICCBased-RGB:${ICC_RGB}"
fi

# 5 ─ No live transparency ---------------------------------------------------
TR_GROUP="$(grep -ac '/Transparency' "$QDF" || true)"
TR_SMASK="$(grep -aE '/SMask' "$QDF" | grep -avc '/SMask */None' || true)"
TR_BM="$(grep -aoE '/BM */[A-Za-z]+' "$QDF" | grep -avc '/BM */Normal' || true)"
TR_CA="$(grep -aoE '/ca +[0-9.]+|/CA +[0-9.]+' "$QDF" | awk '{if ($2+0 < 1) c++} END{print c+0}')"
if [ "${TR_GROUP:-0}" -eq 0 ] && [ "${TR_SMASK:-0}" -eq 0 ] && [ "${TR_BM:-0}" -eq 0 ] && [ "${TR_CA:-0}" -eq 0 ]; then
  pass "no live transparency"
else
  fail "transparency present — Group:${TR_GROUP} SMask:${TR_SMASK} non-Normal BM:${TR_BM} alpha<1:${TR_CA}"
fi

# 6 ─ OutputIntent /GTS_PDFX with embedded profile ---------------------------
OI="$(grep -ac '/GTS_PDFX' "$QDF" || true)"
DOP="$(grep -ac '/DestOutputProfile' "$QDF" || true)"
if [ "${OI:-0}" -ge 1 ] && [ "${DOP:-0}" -ge 1 ]; then
  pass "OutputIntent /GTS_PDFX with embedded DestOutputProfile"
else
  fail "missing PDF/X OutputIntent (GTS_PDFX:${OI} DestOutputProfile:${DOP})"
fi

# ── Structural cross-check: qpdf --check (informational; never affects exit) ──
# qpdf --check exits 0 = clean, 3 = recoverable warnings, 2 = errors.
qpdf --check "$PDF" >/dev/null 2>&1
case $? in
  0) [ "$QUIET" -eq 1 ] || printf '\033[36m  ·\033[0m cross-check: qpdf --check OK (structurally sound)\n' ;;
  3) [ "$QUIET" -eq 1 ] || printf '\033[36m  ·\033[0m cross-check: qpdf --check — recoverable warnings (informational)\n' ;;
  *) [ "$QUIET" -eq 1 ] || printf '\033[33m  ·\033[0m cross-check: qpdf --check reported structural errors (informational)\n' ;;
esac

echo
if [ "$FAILS" -eq 0 ]; then
  printf '\033[1;32mPDF/X-1a: PASS\033[0m — %s\n' "$PDF"
  exit 0
else
  printf '\033[1;31mPDF/X-1a: FAIL\033[0m — %d invariant(s) violated in %s\n' "$FAILS" "$PDF"
  exit 1
fi
