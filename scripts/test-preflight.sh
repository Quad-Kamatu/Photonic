#!/usr/bin/env bash
#
# test-preflight.sh — self-test for scripts/preflight-pdfx.sh.
#
# Proves the gate can say NO: it PASSes a valid PDF/X-1a file and FAILs each
# class of defect (RGB colour, missing OutputIntent, wrong version, live
# transparency, non-embedded font, encryption). Without this, a checker that
# always returned PASS would look identical to a correct one.
#
# Fixtures live in scripts/preflight-fixtures/ (regenerate with
# scripts/gen-preflight-fixtures.sh). This test needs only qpdf + poppler-utils.
#
# Exit: 0 = every assertion held; 1 = at least one assertion failed.
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHK="$DIR/preflight-pdfx.sh"
FX="$DIR/preflight-fixtures"
fails=0

# A valid X-1a file must pass (exit 0).
expect_pass() {
  if "$CHK" "$1" --quiet >/dev/null 2>&1; then
    printf '  \033[32mok\033[0m   PASS  %s\n' "$(basename "$1")"
  else
    printf '  \033[31mFAIL\033[0m unexpected FAIL on a valid file: %s\n' "$(basename "$1")"
    fails=$((fails + 1))
  fi
}

# A defective file must fail (exit ≠ 0) AND the checker must name the expected
# reason — `needle` is a substring unique to that invariant's failure message.
expect_fail() {
  local file="$1" needle="$2"
  local out rc
  out="$("$CHK" "$file" 2>&1)"
  rc=$?
  if [ "$rc" -eq 0 ]; then
    printf '  \033[31mFAIL\033[0m expected FAIL but passed: %s\n' "$(basename "$file")"
    fails=$((fails + 1))
    return
  fi
  if printf '%s' "$out" | grep -qF "$needle"; then
    printf '  \033[32mok\033[0m   FAIL  %-22s → detected: %s\n' "$(basename "$file")" "$needle"
  else
    printf '  \033[31mFAIL\033[0m %s failed, but not for "%s":\n%s\n' "$(basename "$file")" "$needle" "$out"
    fails=$((fails + 1))
  fi
}

echo "preflight-pdfx self-test"
expect_pass "$FX/good_x1a.pdf"
expect_fail "$FX/bad_rgb.pdf"          "RGB colour present"
expect_fail "$FX/bad_rgb.pdf"          "missing PDF/X OutputIntent"
expect_fail "$FX/bad_rgb.pdf"          "PDF version is"
expect_fail "$FX/bad_transparency.pdf" "transparency present"
expect_fail "$FX/bad_font.pdf"         "NOT embedded"
expect_fail "$FX/bad_encrypted.pdf"    "file is encrypted"

echo
if [ "$fails" -eq 0 ]; then
  printf '\033[1;32mpreflight self-test: ALL ASSERTIONS HELD\033[0m\n'
  exit 0
else
  printf '\033[1;31mpreflight self-test: %d assertion(s) failed\033[0m\n' "$fails"
  exit 1
fi
