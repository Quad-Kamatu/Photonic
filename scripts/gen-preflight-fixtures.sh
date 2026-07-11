#!/usr/bin/env bash
#
# gen-preflight-fixtures.sh — (re)generate the PDF fixtures used by
# scripts/test-preflight.sh. Run this only when the fixtures need refreshing;
# the resulting files are committed under scripts/preflight-fixtures/ so CI can
# run the self-test with just qpdf + poppler (no build / Ghostscript needed).
#
# Requires (generation-time only): a built workspace, Ghostscript (gs), qpdf.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
FX="$SCRIPT_DIR/preflight-fixtures"
mkdir -p "$FX"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT

echo "• core fixtures (good_x1a, bad_rgb, bad_transparency)"
( cd "$REPO_DIR" && cargo run --quiet -p photonic-core --example gen_preflight_fixtures -- "$FX" )

echo "• bad_font.pdf — a non-embedded standard-14 font (Helvetica) via Ghostscript"
cat > "$tmp/font.ps" <<'PS'
%!PS
/Helvetica findfont 24 scalefont setfont
72 72 moveto (Preflight non-embedded font fixture) show
showpage
PS
gs -q -dNOPAUSE -dBATCH -dSAFER -sDEVICE=pdfwrite \
   -dEmbedAllFonts=false -dSubsetFonts=false -dCompatibilityLevel=1.4 \
   -o "$FX/bad_font.pdf" "$tmp/font.ps"

echo "• bad_encrypted.pdf — encrypt the good fixture (empty passwords, 256-bit AES)"
rm -f "$FX/bad_encrypted.pdf"
qpdf --encrypt "" "" 256 -- "$FX/good_x1a.pdf" "$FX/bad_encrypted.pdf"

echo "done → $FX"
ls -la "$FX"
