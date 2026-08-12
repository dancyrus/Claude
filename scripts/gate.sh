#!/usr/bin/env bash
# FlowPaint change gate. Run from the repo root before merging any unit.
#   scripts/gate.sh <base-ref>        e.g. scripts/gate.sh origin/main
# Exit 0 = every mechanical check passed. Exit 1 = at least one FAIL.
# WARN does not fail the gate; it means a human must look.
set -uo pipefail

BASE="${1:-origin/main}"
FAIL=0; WARN=0
ok()   { printf '  PASS  %s\n' "$1"; }
bad()  { printf '  FAIL  %s\n' "$1"; FAIL=$((FAIL+1)); }
warn() { printf '  WARN  %s\n' "$1"; WARN=$((WARN+1)); }
hdr()  { printf '\n== %s\n' "$1"; }

command -v git >/dev/null || { echo "git not found"; exit 1; }
git rev-parse --git-dir >/dev/null 2>&1 || { echo "not a git repo"; exit 1; }
git rev-parse --verify "$BASE" >/dev/null 2>&1 || { echo "unknown base ref: $BASE"; exit 1; }

HEAD_REF=$(git rev-parse --short HEAD)
CHANGED=$(git diff --name-only "$BASE"...HEAD)
echo "FlowPaint gate — HEAD $HEAD_REF vs $BASE"
[ -n "$CHANGED" ] || { echo "no changes vs $BASE — nothing to gate"; exit 0; }

hdr "Base"
if git merge-base --is-ancestor "$BASE" HEAD; then ok "branched off $BASE (no rebase needed)"
else bad "$BASE is NOT an ancestor of HEAD — you branched off something stale"; fi

hdr "Shaders"
SH_CHANGED=$(echo "$CHANGED" | grep -E '^FlowPaint/src/shaders/' || true)
if [ -z "$SH_CHANGED" ]; then ok "no shader edits"
else bad "shader files edited without approval:"; echo "$SH_CHANGED" | sed 's/^/          /'; fi

hdr "Dependencies"
if echo "$CHANGED" | grep -q 'FlowPaint/Cargo.toml'; then
  warn "Cargo.toml changed — a new dependency needs explicit approval; record it in docs/unit-decisions.md"
  git diff "$BASE"...HEAD -- FlowPaint/Cargo.toml | grep -E '^\+[a-zA-Z]' | sed 's/^/          /'
else ok "no dependency change"; fi

hdr "CLAUDE.md budget"
if [ -f CLAUDE.md ]; then
  L=$(wc -l < CLAUDE.md)
  if [ "$L" -le 150 ]; then ok "CLAUDE.md $L/150 lines"
  else bad "CLAUDE.md $L lines, over the 150 cap — route detail to docs/"; fi
else bad "CLAUDE.md missing"; fi

hdr "Frame-time bench"
PERF=$(echo "$CHANGED" | grep -E 'ui/canvas\.rs|model\.rs|geomops\.rs|sim\.rs' || true)
if [ -n "$PERF" ]; then
  if git diff --name-only "$BASE"...HEAD | grep -q 'docs/theme.md'; then ok "perf-sensitive files changed and docs/theme.md has a new entry"
  else bad "perf-sensitive files changed but docs/theme.md is untouched — run the paired A/B (see docs/agent-protocol.md)"; echo "$PERF" | sed 's/^/          /'; fi
else ok "no perf-sensitive file touched; no bench required"; fi

hdr "Scene format"
if git diff "$BASE"...HEAD -- FlowPaint/src/app.rs | grep -qE '^\+.*(SCENE_V[0-9]+|struct SceneV[0-9]+)'; then
  CUR=$(grep -oE 'const SCENE_V[0-9]+' FlowPaint/src/app.rs | grep -oE '[0-9]+' | sort -n | tail -1)
  warn "scene format touched — current max is v$CUR; confirm every earlier load path is retained and round-trip tested"
else ok "scene format unchanged"; fi

hdr "Build and tests"
if ( cd FlowPaint && cargo test --release --message-format short >/tmp/gate_test.log 2>&1 ); then
  N=$(grep -oE '[0-9]+ passed' /tmp/gate_test.log | grep -oE '[0-9]+' | paste -sd+ | bc)
  ok "cargo test --release: ${N:-?} passed, 0 failed"
  W=$(grep -c '^warning' /tmp/gate_test.log || true)
  [ "${W:-0}" -eq 0 ] && ok "zero warnings" || warn "$W build warnings"
else bad "cargo test --release FAILED — tail:"; tail -15 /tmp/gate_test.log | sed 's/^/          /'; fi

hdr "Docs"
if echo "$CHANGED" | grep -q 'docs/unit-decisions.md'; then ok "docs/unit-decisions.md updated"
else warn "docs/unit-decisions.md untouched — did this change record no decision?"; fi

hdr "Summary"
printf '  %d fail, %d warn\n' "$FAIL" "$WARN"
[ "$FAIL" -eq 0 ] && echo "  GATE PASSED (warnings still need a human)" || echo "  GATE FAILED"
exit $((FAIL > 0))
