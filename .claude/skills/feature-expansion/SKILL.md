---
name: feature-expansion
description: Use when the user invokes /feature-expansion and wants Claude to select one unimplemented feature from the gap analysis, plan it in plan mode, implement it, update the feature docs, and commit.
---

# Feature Expansion

## Overview

A single-shot feature implementation loop. Read the gap list, pick the best candidate feature, claim it, plan it in plan mode, implement it, update `docs/Features.md`, remove it from `docs/illustrator-feature-gaps.md`, and commit.

**Core principle:** One feature per invocation. Full plan → implement → validate → document cycle. Leave the codebase strictly better than you found it.

---

## Reference Files

| File | Role |
|---|---|
| `docs/illustrator-feature-gaps.md` | Source of features to implement — both Illustrator gaps and bespoke ideas |
| `docs/Features.md` | Living record of everything currently implemented |

---

## HARD STOP — Snapshot Pre-Session Git State

**Before scanning or touching any file**, capture what was already staged:

```bash
git diff --cached --name-only
```

Save this list in memory as **PRE_SESSION_STAGED**. It is used in Step 8 to ensure only session changes are committed.

---

## Step 1: SCAN THE GAP LIST

Read `docs/illustrator-feature-gaps.md` in full.

Build a list of all unclaimed, unimplemented features. A feature is **claimed** if its bullet or heading contains `*(in progress)*` or `*(done)*`.

For each candidate, note:
- Category and section (e.g. "Category J — Quality-of-Life", "J3. Non-Destructive Boolean Operations")
- Scope (single MCP tool / GUI panel / core document model change / multi-system)
- Dependencies (does it require another feature to land first?)

---

## Step 2: SELECT A FEATURE

Pick the best candidate using this priority order:

1. **Smallest self-contained scope** — prefer features that touch one crate or system
2. **No unresolved dependencies** — skip features that require another gap item first
3. **Highest user-facing impact for the effort** — MCP tool additions, export improvements, and QoL fixes rank above large architectural changes
4. **Bespoke (Category A–J) items** over Illustrator parity items when effort is equal — they differentiate Photonic

Announce your selection:

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Feature Expansion — Selected:
  {Section} — {Feature Name}
  Scope: {one-line scope summary}
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

If every remaining feature has unresolved dependencies or requires months of architectural work, output:

```
⚠️  No autonomously-implementable features found.
    All remaining items require multi-sprint architectural changes.
    Consider /wildcard-improvement or manually planning a large feature first.
```

Then stop.

---

## Step 3: CLAIM THE FEATURE

Before writing any code, mark the selected feature as in-progress in `docs/illustrator-feature-gaps.md`.

Append `*(in progress)*` to the feature's heading or leading bullet:

```markdown
<!-- Before -->
### J3. Non-Destructive Boolean Operations (All of them)

<!-- After -->
### J3. Non-Destructive Boolean Operations (All of them) *(in progress)*
```

Commit this claim immediately so parallel sessions don't double-up:

```bash
git add docs/illustrator-feature-gaps.md
git commit -m "docs: claim {Feature Name} for implementation"
```

---

## Step 4: PLAN (Enter Plan Mode)

Use the `EnterPlanMode` tool. Do not write any implementation code until plan mode is active and the plan is approved.

Read every file in the codebase that is relevant to the feature:
- The affected crate(s) in `crates/`
- Existing similar MCP tool handlers in `crates/photonic-mcp/src/handlers/`
- Core types in `crates/photonic-core/src/`
- GUI panels if the feature has a UI component in `crates/photonic-gui/src/`

Draft a concrete, ordered implementation plan:
1. Which files change and what changes in each
2. New types, structs, enums, or trait impls required
3. New or modified MCP tool handler(s) — include tool name, parameters, and return shape
4. GUI changes (new panel controls, tool options, etc.) if applicable
5. How the feature is validated (compile check, manual MCP call, screenshot observation)
6. Anything explicitly out of scope for this invocation

Present the plan for confirmation. Wait for user approval or amendment via `ExitPlanMode` before proceeding.

---

## Step 5: IMPLEMENT

Execute the approved plan exactly. For each file:
- Read the current state before editing
- Make targeted changes — do not refactor surrounding code not covered by the plan
- Add the new MCP tool to the server's tool registry if adding a new handler
- Keep changes minimal — this is a feature addition, not a cleanup pass

Follow Photonic project standards:
- Rust: safe code, no `unwrap()` on user-facing paths (use `?` or explicit error types)
- MCP tools: follow existing handler conventions in `photonic-mcp/src/handlers/`
- Document model changes: ensure undo/redo is supported via the command pattern
- All new MCP tools must be reachable by the existing `DocumentController` command channel

---

## Step 6: VALIDATE

Confirm the implementation compiles and behaves correctly:

```bash
cargo build --workspace
```

If the feature adds an MCP tool, also confirm it is registered and callable:
- Check the tool appears in the server's tool list handler
- If the app is running, call it via the MCP console or a quick `curl` to `localhost:7842`

If the build fails, fix all errors before proceeding. Do not commit a broken state.

---

## Step 7: UPDATE DOCS

### Add to Features.md

Open `docs/Features.md` and add the new feature in the appropriate section. Follow the existing table/list format. If no suitable section exists, add one.

Example for a new MCP tool:

```markdown
| `new_tool_name` | One-line description of what it does |
```

Example for a new GUI panel:

```markdown
| New Panel Name | What it shows and what actions it exposes |
```

### Remove from illustrator-feature-gaps.md

Remove the feature's entire entry (heading + bullet list) from `docs/illustrator-feature-gaps.md`. Do not leave a stub or placeholder. If the feature was in both the Illustrator-parity section and the bespoke section, remove it from both places.

Also remove the `*(in progress)*` marker — the entry is gone, not just updated.

---

## Step 8: COMMIT (Session-Isolated)

Only changes made **this session** go into the commit.

### 8a — Unstage pre-existing staged files

If **PRE_SESSION_STAGED** (captured at startup) is non-empty, those files were already staged before this session and must not be included. Unstage them first:

```bash
git restore --staged <pre-session-file-1> <pre-session-file-2> ...
```

### 8b — Stage only session files

```bash
git add docs/Features.md docs/illustrator-feature-gaps.md <crate-files-changed-this-session...>
```

Verify the staged set before committing:

```bash
git diff --cached --stat
```

Confirm only files you changed this session appear. If any unrelated file is staged, unstage it.

### 8c — Commit

```bash
git commit -m "$(cat <<'EOF'
feat({crate}): {short feature name}

{1-2 sentences describing what was added and why it matters.}

Removes this item from the gap analysis and adds it to Features.md.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

### 8d — Restore pre-existing staged files

If you ran step 8a, re-stage those files now so the user's working state is restored:

```bash
git add <pre-session-file-1> <pre-session-file-2> ...
```

After the commit, output:

```
✓ Feature implemented and committed.
  {Feature Name} is now live in Photonic.
  docs/Features.md updated.
  docs/illustrator-feature-gaps.md updated.

Run /feature-expansion again to implement the next feature.
```

---

## Safety Rules (Non-Negotiable)

1. **Claim before coding** — always mark `*(in progress)*` and commit the claim before writing any implementation code
2. **Plan mode required** — never write implementation code without entering plan mode and receiving approval first
3. **Read before editing** — always read current file state; never edit from memory
4. **One feature per run** — do not attempt to implement multiple features in a single invocation
5. **No scope creep** — only implement what the selected feature describes; ignore adjacent improvements
6. **Build must pass** — never commit a state that fails `cargo build --workspace`
7. **Both docs must update** — Features.md addition and gap list removal are mandatory; a feature is not done until both files are updated
8. **Session-only commit** — snapshot PRE_SESSION_STAGED at startup; only stage files changed this session; unstage and restore any pre-existing staged files around the commit

---

## Common Mistakes

| Mistake | Fix |
|---|---|
| Starting implementation before claiming the feature | Claim and commit first — Step 3 before Step 4 |
| Writing code before entering plan mode | EnterPlanMode is required; no exceptions |
| Leaving `*(in progress)*` in the gap list after completion | Step 7 removes the entire entry, not just the marker |
| Picking a feature with an unresolved dependency | Re-read Step 2 — skip blocked features |
| Forgetting to register a new MCP tool in the server's tool list | New handlers must be wired into the registry to be callable |
| Committing the whole working tree | Stage only the relevant crate files + both doc files |
| Pre-existing staged files ending up in the feature commit | Run `git diff --cached --name-only` at startup; unstage them in step 8a before committing |
| Forgetting to restore pre-existing staged files after commit | Step 8d is mandatory if step 8a ran |
