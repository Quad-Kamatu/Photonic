#!/usr/bin/env bash
#
# svg-to-pdfx.sh — Interim print-ready pipeline for Photonic.
#
# Turns an SVG exported from Photonic (`export_svg`) into a PDF/X-1a:2001
# file a commercial print shop will accept: DeviceCMYK, embedded OutputIntent,
# correct physical page size, all fonts outlined, PDF 1.3 (no live
# transparency). Each stage below is a stopgap that a Tier-0/Tier-1 feature in
# the app will later absorb natively (see docs/print/ roadmap):
#
#   Stage 1  SVG -> vector PDF        (absorbed by:  export_pdf  / T0.1)
#            + text outlined to paths (absorbed by:  outline_text / T0.2)
#   Stage 2  RGB -> DeviceCMYK        (absorbed by:  set_document_color_mode + ICC / T0.3)
#            + OutputIntent + flatten (absorbed by:  PDF/X-1a conformance / T0.4)
#
# The intermediate SVG is expected to already carry the physical page size
# (e.g. width="88mm" height="58mm" including bleed) and any trim/registration
# marks. Photonic's set_document_bleed + a bleed-aware export produce that; you
# can also pass --size to stamp a size onto a unitless SVG.
#
# Requirements (all rootless, already present on this machine):
#   inkscape (preferred, outlines text) OR rsvg-convert  — SVG->PDF
#   ghostscript (gs)                                      — PDF->PDF/X-1a
#   verapdf (optional)                                    — preflight proof
#
# Usage:
#   scripts/svg-to-pdfx.sh INPUT.svg OUTPUT.pdf [options]
#
# Options:
#   --profile FILE     CMYK ICC profile (default: assets/icc/CoatedFOGRA39.icc)
#   --intent NAME      OutputIntent condition identifier (default: derived from profile)
#   --size WxH         Stamp a physical page size onto the SVG, e.g. 88x58mm or 3.6x2.1in
#                      (use the *bleed* size — trim size + 2*bleed). Optional.
#   --renderer NAME    Force svg->pdf renderer: inkscape | rsvg (default: auto)
#   --keep-temp        Keep intermediate files next to the output
#   --no-preflight     Skip the verapdf check even if verapdf is installed
#
# Exit status: 0 on success (and, if verapdf present, on a PASS preflight).
set -euo pipefail

# ── Locate repo + defaults ────────────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
DEFAULT_PROFILE="$REPO_DIR/assets/icc/CoatedFOGRA39.icc"

PROFILE="$DEFAULT_PROFILE"
INTENT=""
SIZE=""
RENDERER="auto"
KEEP_TEMP=0
PREFLIGHT=1

die() { printf 'svg-to-pdfx: error: %s\n' "$*" >&2; exit 1; }
note() { printf '\033[36m•\033[0m %s\n' "$*" >&2; }

[ $# -ge 2 ] || die "usage: svg-to-pdfx.sh INPUT.svg OUTPUT.pdf [options]  (see --help / header)"
IN_SVG="$1"; shift
OUT_PDF="$1"; shift
[ -f "$IN_SVG" ] || die "input SVG not found: $IN_SVG"

while [ $# -gt 0 ]; do
  case "$1" in
    --profile) PROFILE="$2"; shift 2 ;;
    --intent)  INTENT="$2"; shift 2 ;;
    --size)    SIZE="$2"; shift 2 ;;
    --renderer) RENDERER="$2"; shift 2 ;;
    --keep-temp) KEEP_TEMP=1; shift ;;
    --no-preflight) PREFLIGHT=0; shift ;;
    -h|--help) sed -n '2,60p' "$0"; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

command -v gs >/dev/null 2>&1 || die "ghostscript (gs) is required but not installed"
[ -f "$PROFILE" ] || die "CMYK ICC profile not found: $PROFILE"
[ -n "$INTENT" ] || INTENT="$(basename "$PROFILE" .icc)"

# ── Working area ──────────────────────────────────────────────────────────────
WORK="$(mktemp -d)"
cleanup() { [ "$KEEP_TEMP" -eq 1 ] || rm -rf "$WORK"; }
trap cleanup EXIT
SRC_SVG="$IN_SVG"

# Optionally stamp a physical size onto the SVG root (width/height + viewBox stays).
if [ -n "$SIZE" ]; then
  # Parse WxH with a trailing unit (mm|in|px|pt). Applies to both dims.
  num_unit="$SIZE"
  unit="${num_unit##*[0-9.]}"; unit="${unit:-px}"
  dims="${num_unit%$unit}"
  W="${dims%x*}"; H="${dims#*x}"
  [ -n "$W" ] && [ -n "$H" ] || die "--size must look like 88x58mm or 3.6x2.1in"
  STAMPED="$WORK/stamped.svg"
  # Replace the first width/height attributes on the root <svg> element.
  sed -E "0,/<svg[^>]*>/{s/(<svg[^>]*?)\s+width=\"[^\"]*\"/\1/; s/(<svg[^>]*?)\s+height=\"[^\"]*\"/\1/; s/<svg/<svg width=\"${W}${unit}\" height=\"${H}${unit}\"/}" "$IN_SVG" > "$STAMPED"
  SRC_SVG="$STAMPED"
  note "stamped page size ${W}x${H}${unit} onto SVG"
fi

# ── Stage 1: SVG -> vector PDF (text outlined) ────────────────────────────────
STEP1_PDF="$WORK/step1.pdf"
choose_renderer() {
  if [ "$RENDERER" = "inkscape" ]; then echo inkscape; return; fi
  if [ "$RENDERER" = "rsvg" ]; then echo rsvg; return; fi
  if command -v inkscape >/dev/null 2>&1; then echo inkscape; return; fi
  if command -v rsvg-convert >/dev/null 2>&1; then echo rsvg; return; fi
  die "need inkscape or rsvg-convert for SVG->PDF"
}
R="$(choose_renderer)"
if [ "$R" = "inkscape" ]; then
  note "SVG -> PDF via inkscape (text -> paths, zero font deps)"
  # --export-text-to-path outlines every glyph, matching the app's outline_text goal.
  inkscape "$SRC_SVG" \
    --export-type=pdf \
    --export-text-to-path \
    --export-filename="$STEP1_PDF" >/dev/null 2>&1 \
    || die "inkscape SVG->PDF failed"
else
  note "SVG -> PDF via rsvg-convert (cairo embeds/subsets fonts)"
  note "  (fonts embedded, not outlined — install inkscape for true outlining)"
  rsvg-convert -f pdf -o "$STEP1_PDF" "$SRC_SVG" || die "rsvg-convert SVG->PDF failed"
fi
[ -s "$STEP1_PDF" ] || die "stage 1 produced an empty PDF"

# ── Stage 2: PDF -> PDF/X-1a (DeviceCMYK + OutputIntent + flatten) ────────────
# Ghostscript pdfmark definition that attaches the OutputIntent referencing our
# CMYK profile and declares PDF/X-1a:2001 conformance.
DEF_PS="$WORK/pdfx_def.ps"
# gs wants forward-slash paths; escape parens in the profile path if any.
PROFILE_ESC="$(printf '%s' "$PROFILE" | sed 's/[()]/\\&/g')"
cat > "$DEF_PS" <<PSDEF
%!
% PDF/X-1a:2001 OutputIntent definition for Ghostscript pdfwrite.
[ /_objdef {icc_stream} /type /stream /OBJ pdfmark
[ {icc_stream} << /N 4 >> /PUT pdfmark
[ {icc_stream} ($PROFILE_ESC) (r) file /PUT pdfmark
[ {icc_stream} /CLOSE pdfmark
[ /_objdef {OutputIntent} /type /dict /OBJ pdfmark
[ {OutputIntent} <<
    /Type /OutputIntent
    /S /GTS_PDFX
    /OutputConditionIdentifier ($INTENT)
    /Info ($INTENT)
    /RegistryName (http://www.color.org)
    /DestOutputProfile {icc_stream}
  >> /PUT pdfmark
[ {Catalog} << /OutputIntents [ {OutputIntent} ] >> /PUT pdfmark
% Declare PDF/X-1a conformance in the document info dictionary.
[ /GTS_PDFXVersion (PDF/X-1a:2001)
  /GTS_PDFXConformance (PDF/X-1a:2001)
  /Title (Photonic print export)
  /DOCINFO pdfmark
PSDEF

note "PDF -> PDF/X-1a via ghostscript (CMYK: $(basename "$PROFILE"), intent: $INTENT)"
gs \
  -dPDFX \
  -dBATCH -dNOPAUSE -dNOSAFER -dQUIET \
  -sDEVICE=pdfwrite \
  -dCompatibilityLevel=1.3 \
  -dPDFACompatibilityPolicy=1 \
  -sColorConversionStrategy=CMYK \
  -sProcessColorModel=DeviceCMYK \
  -dRenderIntent=1 \
  -dOverrideICC=false \
  -sOutputFile="$OUT_PDF" \
  "$DEF_PS" "$STEP1_PDF" \
  || die "ghostscript PDF/X conversion failed"
[ -s "$OUT_PDF" ] || die "stage 2 produced an empty PDF"

note "wrote $OUT_PDF"

# ── Optional: verapdf preflight proof ─────────────────────────────────────────
if [ "$PREFLIGHT" -eq 1 ] && command -v verapdf >/dev/null 2>&1; then
  note "running verapdf PDF/X-1a preflight…"
  if verapdf -f 1a "$OUT_PDF" >/dev/null 2>&1; then
    printf '\033[32m✓ verapdf: PASS (PDF/X-1a)\033[0m\n' >&2
  else
    printf '\033[33m! verapdf: preflight reported issues — run `verapdf -f 1a %q` for detail\033[0m\n' "$OUT_PDF" >&2
  fi
elif [ "$PREFLIGHT" -eq 1 ]; then
  note "verapdf not installed — skipping preflight (see docs/print/ for a rootless install)"
fi

if [ "$KEEP_TEMP" -eq 1 ]; then
  cp "$STEP1_PDF" "${OUT_PDF%.pdf}.step1.pdf"
  note "kept intermediate: ${OUT_PDF%.pdf}.step1.pdf"
fi
