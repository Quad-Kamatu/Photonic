export const meta = {
  name: 'photonic-issue-fixer',
  description: 'Plan/read then implement, adversarial-gate, and land each Photonic issue onto one shared PR branch',
  whenToUse: 'Turn open Quad-Kamatu/Photonic issues into real, verified code aggregated onto a single shared PR. Pass args { issues, baseBranch, workBranch, prTitle, maxRounds, push }; if issues is omitted a triage agent picks tractable ones.',
  phases: [
    { title: 'Setup', detail: 'create/checkout the shared work branch off base; ensure clean tree' },
    { title: 'Triage', detail: 'pick tractable open issues when none were passed in' },
    { title: 'Plan', detail: 'read docs/proposals/<n>-*.md or research+draft a plan per issue' },
    { title: 'Implement', detail: 'one dev agent per issue, sequential (shared working tree)' },
    { title: 'Review', detail: 'adversarial reviewers: build/test, wiring/stub, regression/scope' },
    { title: 'Fix', detail: 'address blocker/major findings, re-review up to maxRounds' },
    { title: 'Land', detail: 'fmt + release build + commit + push onto the shared branch; or revert+skip' },
    { title: 'Finalize', detail: 'ensure the shared PR exists/updated; report landed vs skipped' },
  ],
}

// ---------------------------------------------------------------------------
// Photonic is a Rust/wgpu vector editor. All subagents inherit the SESSION cwd
// (KamatuStudio), so every prompt pins the repo path and tells the agent to
// operate there. Issues + PRs live on Quad-Kamatu/Photonic via `gh`. Plans live
// in docs/proposals/<n>-*.md. Joseph's rule: always `cargo build --release`
// after edits. Implementation is SEQUENTIAL across issues — every agent shares
// ONE working tree and ONE git branch, so parallel edits would corrupt each
// other. Adversarial review fans out (read-only/report-only) per issue.
// ---------------------------------------------------------------------------

const REPO = '/home/josephh/Code Bases/KamatuStudio/Photonic'

// Normalize args: the harness may deliver it as a real object OR as a JSON
// string. A stringified args silently makes every `args?.x` undefined, which
// is what made the first run ignore baseBranch/workBranch and fall back to
// defaults. Parse defensively so config always lands.
const A = typeof args === 'string' ? (() => { try { return JSON.parse(args) } catch { return {} } })() : args || {}

const BASE = A.baseBranch || 'main'
const WORK = A.workBranch || 'pre-deploy/issue-fixer-batch'
const PR_TITLE = A.prTitle || 'feat: batch issue fixes (aggregated)'
const MAX_ROUNDS = A.maxRounds || 3
const DO_PUSH = A.push !== false // default: push + open/update the PR
const TRAILERS =
  'Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>\n' +
  'Claude-Session: https://claude.ai/code/session_014SPodx3C8hNCc8DC4V8oF8'

const CTX =
  `Repo: ${REPO} (cd into it for EVERY git/cargo/gh command — your cwd starts elsewhere).\n` +
  `Rust workspace crates: photonic-core, photonic-render, photonic-gui, photonic-mcp, photonic-app, photonic-embed.\n` +
  `Issues + PRs live on GitHub remote Quad-Kamatu/Photonic (use \`gh\`). Plans live in docs/proposals/<n>-*.md.\n` +
  `Shared work branch for this run: ${WORK} (already checked out off ${BASE}). Do NOT switch branches.\n` +
  `House rule: after any source edit, \`cargo build --release\` must succeed. GPU headless rendering works here (RTX 4060 Ti).`

// ----------------------------- schemas -------------------------------------

const SETUP_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['ready', 'branch', 'baseSha', 'notes'],
  properties: {
    ready: { type: 'boolean', description: 'true if WORK branch is checked out off BASE and the tree is clean' },
    branch: { type: 'string' },
    baseSha: { type: 'string' },
    notes: { type: 'string' },
  },
}

const TRIAGE_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['issues'],
  properties: {
    issues: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['number', 'title', 'rationale'],
        properties: {
          number: { type: 'number' },
          title: { type: 'string' },
          rationale: { type: 'string', description: 'why this is tractable in a single focused pass' },
        },
      },
    },
  },
}

const PLAN_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['issue', 'title', 'planExists', 'planPath', 'approach', 'targetCrates', 'filesToTouch', 'deferred', 'tractable'],
  properties: {
    issue: { type: 'number' },
    title: { type: 'string' },
    planExists: { type: 'boolean', description: 'true if a docs/proposals/<n>-*.md plan already existed' },
    planPath: { type: 'string' },
    approach: { type: 'string', description: 'concrete implementation approach grounded in real files/symbols' },
    targetCrates: { type: 'array', items: { type: 'string' } },
    filesToTouch: { type: 'array', items: { type: 'string' } },
    deferred: { type: 'array', items: { type: 'string' }, description: 'parts intentionally out of scope for this pass' },
    tractable: { type: 'boolean', description: 'false if too large/risky for one focused pass — workflow will skip it' },
  },
}

const IMPL_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['summary', 'filesChanged', 'buildPassed', 'testsPassed', 'deferred', 'notes'],
  properties: {
    summary: { type: 'string' },
    filesChanged: { type: 'array', items: { type: 'string' } },
    buildPassed: { type: 'boolean', description: 'cargo build --release succeeded' },
    testsPassed: { type: 'boolean', description: 'cargo test for affected crates succeeded' },
    deferred: { type: 'array', items: { type: 'string' } },
    notes: { type: 'string' },
  },
}

const REVIEW_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['lens', 'pass', 'severity', 'findings', 'verdict'],
  properties: {
    lens: { type: 'string' },
    pass: { type: 'boolean', description: 'true ONLY if you could not find a real blocker/major problem' },
    severity: { type: 'string', enum: ['none', 'minor', 'major', 'blocker'] },
    findings: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['title', 'severity', 'detail', 'file', 'suggestedFix'],
        properties: {
          title: { type: 'string' },
          severity: { type: 'string', enum: ['minor', 'major', 'blocker'] },
          detail: { type: 'string' },
          file: { type: 'string' },
          suggestedFix: { type: 'string' },
        },
      },
    },
    verdict: { type: 'string' },
  },
}

const LAND_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['landed', 'sha', 'pushed', 'prNumber', 'message'],
  properties: {
    landed: { type: 'boolean', description: 'true if committed onto the shared branch; false if reverted/skipped' },
    sha: { type: 'string' },
    pushed: { type: 'boolean' },
    prNumber: { type: 'string', description: 'PR number as a string, or "" if not pushed/created' },
    message: { type: 'string' },
  },
}

// ----------------------------- helpers -------------------------------------

const blocking = (reviews) =>
  reviews.filter(Boolean).flatMap((r) => r.findings).filter((f) => f.severity === 'major' || f.severity === 'blocker')

// =============================== run =======================================

// Echo the resolved config up front so a missing/empty `args` is visible in the
// progress log instead of silently falling back to defaults.
log(
  `Config — base=${BASE} work=${WORK} push=${DO_PUSH} maxRounds=${MAX_ROUNDS} ` +
    `issues=${Array.isArray(A.issues) ? A.issues.join(',') : 'triage'} ` +
    `(argsType=${typeof args}, parsedKeys=${Object.keys(A).join('|') || 'none'})`,
)

// --- Setup: one agent, must finish before anything else touches the tree ---
phase('Setup')
const setup = await agent(
  `${CTX}\n\nSet up the shared work branch for a batch of issue fixes.\n` +
    `1. cd ${REPO}\n` +
    `2. \`git fetch origin\` then ensure the working tree is clean (\`git status -s\`). If there are stray uncommitted changes you did NOT create, STOP and report ready=false with what you saw — do not discard another agent's work.\n` +
    `3. Check out the base branch (${BASE}), pull latest, then create-or-reset the work branch ${WORK} off it: if ${WORK} already exists locally, check it out and report its current HEAD; otherwise \`git checkout -b ${WORK} ${BASE}\`.\n` +
    `4. Report the base SHA and whether the branch is ready.`,
  { label: 'setup-branch', phase: 'Setup', schema: SETUP_SCHEMA },
)

if (!setup || !setup.ready) {
  log(`Setup failed — aborting: ${setup ? setup.notes : 'agent returned null'}`)
  return { aborted: true, reason: setup ? setup.notes : 'setup agent died', setup }
}
log(`Work branch ${WORK} ready off ${BASE} @ ${setup.baseSha}`)

// --- Triage: only when the caller did not pass an explicit issue list -------
let issueNums = Array.isArray(A.issues) ? A.issues.map(Number).filter((n) => !Number.isNaN(n)) : null
if (!issueNums || issueNums.length === 0) {
  phase('Triage')
  const triage = await agent(
    `${CTX}\n\nNo issue list was provided. Pick the most tractable open issues to fix in this batch.\n` +
      `Run \`cd ${REPO} && gh issue list --state open --limit 40\`. Read the most promising with \`gh issue view <n>\` ` +
      `and check whether a plan already exists at docs/proposals/<n>-*.md.\n` +
      `Prefer self-contained core-logic / tooling / wiring fixes that can be implemented and unit-tested in a single focused pass. ` +
      `Avoid large greenfield GPU/format/installer epics. Return up to 5 issues, ranked best-first, each with a one-line rationale.`,
    { label: 'triage-issues', phase: 'Triage', schema: TRIAGE_SCHEMA },
  )
  issueNums = (triage?.issues || []).map((i) => i.number)
  log(`Triage selected issues: ${issueNums.join(', ') || '(none)'}`)
}

if (!issueNums || issueNums.length === 0) {
  return { aborted: true, reason: 'no issues to work on', setup }
}

// --- Per-issue pipeline: SEQUENTIAL (shared working tree + git branch) ------
const results = []
for (let i = 0; i < issueNums.length; i++) {
  const n = issueNums[i]
  log(`=== Issue #${n} (${i + 1}/${issueNums.length}) ===`)
  try {

  // 1) Plan — read an existing proposal or research + draft one.
  phase('Plan')
  const plan = await agent(
    `${CTX}\n\nPlan the fix for issue #${n}.\n` +
      `1. \`cd ${REPO} && gh issue view ${n}\` — read title, body, labels.\n` +
      `2. Look for an existing plan at docs/proposals/${n}-*.md. If it exists, READ it and base your approach on it (set planExists=true, planPath to that file).\n` +
      `3. If no plan exists, study the relevant crates to ground a real approach, then WRITE a concise plan to docs/proposals/${n}-<slug>.md following the style of the existing proposals (Summary / Scope (In/Out) / approach). Set planExists=false and planPath to the new file.\n` +
      `4. Decide tractable HONESTLY. This workflow does ONE focused single-agent implementation pass per issue, so it fits well-scoped bugs and small self-contained features. Set tractable=false when the issue is greenfield or infrastructure-scale — a new GPU pipeline, a new file format, installers, a test/regression HARNESS, i18n frameworks, or anything spanning many crates or needing a new subsystem. When an issue is large but has a genuinely self-contained tractable SLICE, set tractable=true and scope 'approach' to just that slice, deferring the rest (document it). Better to skip or slice than to attempt-and-revert. Identify target crates, files to touch, and anything you will defer.`,
    { label: `plan:#${n}`, phase: 'Plan', schema: PLAN_SCHEMA },
  )

  if (!plan) {
    results.push({ issue: n, status: 'error', stage: 'plan', detail: 'plan agent died' })
    continue
  }
  if (!plan.tractable) {
    log(`#${n} judged not tractable for a single pass — skipping (plan kept at ${plan.planPath}).`)
    // The plan doc may have been written; commit just that doc so the work isn't lost.
    results.push({ issue: n, status: 'skipped-untractable', planPath: plan.planPath, title: plan.title })
    continue
  }

  // 2) Implement — one dev agent, on the shared branch.
  phase('Implement')
  let impl = await agent(
    `${CTX}\n\nImplement the REAL fix for issue #${n}: "${plan.title}".\n` +
      `Approach (from ${plan.planExists ? 'existing plan' : 'your fresh plan'} ${plan.planPath}):\n${plan.approach}\n\n` +
      `Target crates: ${plan.targetCrates.join(', ') || '(determine yourself)'}. Likely files: ${plan.filesToTouch.join(', ') || '(determine yourself)'}.\n` +
      `Rules:\n` +
      `- Write production-quality code that genuinely delivers the capability — NO stubs, TODO-as-done, fake returns, or orphaned code that nothing calls. Wire it end-to-end.\n` +
      `- Stay in scope for this issue; do not refactor unrelated areas.\n` +
      `- After editing, run \`cargo build --release\` and \`cargo test -p <each affected crate>\` plus \`cargo check --workspace\`. Report pass/fail honestly.\n` +
      `- If a sub-part is genuinely out of scope, implement the rest and DOCUMENT the deferral in the proposal md's "Remaining work" section + note it in your final summary.\n` +
      `- Update docs/proposals/${n}-*.md header from a design scaffold into an honest "What this PR implements / Remaining work" status. If the fix touches MCP tools, regenerate docs/mcp-api.md if the repo has a generator.\n` +
      `Do NOT commit — a later step lands it. Leave changes in the working tree.\n` +
      `Return a short plain-text summary of what you implemented and anything you deferred.`,
    // No schema: the adversarial reviewers (which read the git diff) are the real
    // gate, so impl output is informational. Forcing IMPL_SCHEMA here made large
    // issues crash on the StructuredOutput retry cap (#20, #54). Free text can't.
    { label: `impl:#${n}`, phase: 'Implement', agentType: 'arcwright-dev' },
  )

  if (!impl) {
    results.push({ issue: n, status: 'error', stage: 'impl', detail: 'impl agent died' })
    continue
  }

  // 3) Adversarial review loop — fan out lenses, fix blockers, repeat.
  let round = 0
  let lastReviews = []
  while (round < MAX_ROUNDS) {
    round++
    phase('Review')
    const reviewPrompt = (lens, instr) =>
      `${CTX}\n\nADVERSARIAL REVIEW (round ${round}) of the uncommitted fix for issue #${n}: "${plan.title}".\n` +
      `Lens: ${lens}. Your job is to REFUTE the claim that this fix is complete and correct. Default to skepticism; only pass=true if you genuinely cannot find a major/blocker problem.\n` +
      `Inspect the working-tree changes (\`cd ${REPO} && git diff ${BASE} -- .\` and \`git status\`).\n` +
      `${instr}\n` +
      `Report findings with severity (minor/major/blocker), the file, and a concrete suggestedFix. Only major/blocker findings force another fix round.`

    const reviews = await parallel([
      () =>
        agent(
          reviewPrompt(
            'build-and-test',
            `Actually RUN it: \`cargo build --release\`, \`cargo test -p <affected crates>\`, \`cargo check --workspace\`, and \`cargo clippy\` on the touched crates. ` +
              `Fail (blocker) on any build/test error; major on NEW clippy warnings introduced by this diff.`,
          ),
          { label: `review:build #${n}`, phase: 'Review', schema: REVIEW_SCHEMA, agentType: 'arcwright-qa' },
        ),
      () =>
        agent(
          reviewPrompt(
            'wiring-and-stubs',
            `Goal-backward verification: does the issue's capability ACTUALLY work end-to-end, or is it stubbed/mocked/orphaned? Trace the new code from its entry point (MCP tool, GUI action, render path) to real data flow. ` +
              `Verify it matches the proposal's stated scope and that every deferral is documented honestly. Flag fake completeness as blocker.`,
          ),
          { label: `review:wiring #${n}`, phase: 'Review', schema: REVIEW_SCHEMA, agentType: 'arcwright-verifier' },
        ),
      () =>
        agent(
          reviewPrompt(
            'regression-and-scope',
            `Hunt for collateral damage: behavior changes to existing features, out-of-scope edits, leftover debug code/TODOs, broken invariants, or risky unwraps/panics on the happy path. ` +
              `Check the diff stays within the issue's scope.`,
          ),
          { label: `review:scope #${n}`, phase: 'Review', schema: REVIEW_SCHEMA },
        ),
    ])

    lastReviews = reviews.filter(Boolean)
    const blockers = blocking(reviews)
    if (blockers.length === 0) {
      log(`#${n} passed adversarial review in round ${round}.`)
      break
    }
    if (round >= MAX_ROUNDS) {
      log(`#${n} still has ${blockers.length} major/blocker finding(s) after ${MAX_ROUNDS} rounds.`)
      break
    }

    // Fix round — single dev agent addresses the batched blockers.
    phase('Fix')
    const findingsText = blockers
      .map((f, k) => `${k + 1}. [${f.severity}] ${f.title} (${f.file})\n   ${f.detail}\n   suggested: ${f.suggestedFix}`)
      .join('\n')
    impl = await agent(
      `${CTX}\n\nFix round ${round} for issue #${n}: "${plan.title}". Adversarial reviewers raised these major/blocker findings:\n\n${findingsText}\n\n` +
        `Address every one of them in the working tree (do not commit). Re-run \`cargo build --release\` and the affected \`cargo test\`. ` +
        `Keep deferrals documented in the proposal md. Return a short plain-text summary of what you changed.`,
      // No schema — same reason as the impl agent above (avoid StructuredOutput crash).
      { label: `fix:#${n} r${round}`, phase: 'Fix', agentType: 'arcwright-dev' },
    )
    if (!impl) {
      log(`#${n} fix agent died in round ${round}.`)
      break
    }
  }

  // 4) Land — commit+push onto the shared branch if clean; else revert+skip.
  phase('Land')
  const stillBlocked = blocking(lastReviews).length > 0 || !impl
  if (stillBlocked) {
    const why = !impl ? 'fix agent died' : `${blocking(lastReviews).length} unresolved major/blocker finding(s) after ${MAX_ROUNDS} rounds`
    const land = await agent(
      `${CTX}\n\nIssue #${n} did NOT pass the adversarial gate (${why}). To keep the shared branch ${WORK} green, REVERT this issue's uncommitted changes.\n` +
        `\`cd ${REPO}\`, then restore tracked files (\`git restore --source=HEAD --staged --worktree -- .\` or \`git checkout -- .\`) and remove untracked files this issue added (\`git clean -fd\` — but be careful not to delete unrelated files; list with \`git clean -nd\` first). Leave the tree clean at HEAD.\n` +
        `Report landed=false with a message explaining the skip.`,
      { label: `revert:#${n}`, phase: 'Land', schema: LAND_SCHEMA },
    )
    results.push({ issue: n, status: 'skipped-failed-gate', detail: why, title: plan.title, land })
    continue
  }

  const land = await agent(
    `${CTX}\n\nLand the fix for issue #${n}: "${plan.title}" onto the shared branch ${WORK}.\n` +
      `1. \`cd ${REPO} && cargo fmt --all\` then \`cargo build --release\` to confirm still green.\n` +
      `2. Stage all changes for this issue and commit with a conventional-commit subject referencing (#${n}) and a body summarizing what shipped + what was deferred. End the message with EXACTLY these trailers:\n${TRAILERS}\n` +
      (DO_PUSH
        ? `3. \`git push -u origin ${WORK}\`.\n` +
          `4. Ensure the shared PR exists: \`gh pr view ${WORK} 2>/dev/null\`. If none, create it with \`gh pr create --base ${BASE} --head ${WORK} --title "${PR_TITLE}" --body "<summary of the batch so far; list each landed issue>"\`. If it exists, that's fine — the push already updated it; optionally append this issue to the PR body. Report the PR number.\n`
        : `3. Do NOT push (push disabled for this run). Report pushed=false, prNumber="".\n`) +
      `Report the commit SHA and whether it was pushed.`,
    { label: `land:#${n}`, phase: 'Land', schema: LAND_SCHEMA },
  )
  results.push({
    issue: n,
    status: land?.landed ? 'landed' : 'land-failed',
    title: plan.title,
    deferred: plan.deferred,
    sha: land?.sha,
    prNumber: land?.prNumber,
    rounds: round,
  })
  log(`#${n}: ${land?.landed ? 'LANDED' : 'land failed'} — ${land?.message || ''}`)
  } catch (e) {
    // One issue's agent throwing (e.g. StructuredOutput retry-cap) must NOT kill
    // the whole run. Revert its partial tree changes so the shared branch stays
    // clean, record the failure, and move on to the next issue + finalize.
    const msg = String(e?.message || e).slice(0, 300)
    log(`#${n} pipeline crashed: ${msg.slice(0, 160)} — reverting tree to keep ${WORK} clean`)
    await agent(
      `${CTX}\n\nIssue #${n} crashed mid-pipeline. Revert ALL uncommitted changes so the shared branch ${WORK} stays clean at its last commit: ` +
        `\`cd ${REPO} && git status -s\` (inspect first), then \`git restore --source=HEAD --staged --worktree -- .\` and remove only newly-untracked files this issue added (\`git clean -nd\` to preview, then \`git clean -fd\` — never delete the .claude/ directory). Report what you reverted.`,
      { label: `recover:#${n}`, phase: 'Land' },
    ).catch(() => {})
    results.push({ issue: n, status: 'error-crashed', detail: msg })
  }
}

// --- Finalize: make sure the PR reflects everything landed -----------------
const landed = results.filter((r) => r.status === 'landed')
const skipped = results.filter((r) => r.status !== 'landed')

if (DO_PUSH && landed.length > 0) {
  phase('Finalize')
  await agent(
    `${CTX}\n\nFinalize the shared PR for branch ${WORK}.\n` +
      `Landed issues: ${landed.map((r) => `#${r.issue} (${r.title})`).join('; ')}.\n` +
      `Skipped: ${skipped.map((r) => `#${r.issue} [${r.status}]`).join('; ') || 'none'}.\n` +
      `\`cd ${REPO}\`. Confirm the branch is pushed and the PR exists (\`gh pr view ${WORK}\`). Update the PR body to list every landed issue with a one-line summary and a "Deferred / not in this batch" section. ` +
      `Run a final \`cargo build --release\` + \`cargo fmt --all --check\` to confirm the branch is green, and report the PR url.`,
    { label: 'finalize-pr', phase: 'Finalize' },
  )
}

return {
  branch: WORK,
  base: BASE,
  landed: landed.map((r) => ({ issue: r.issue, title: r.title, sha: r.sha, rounds: r.rounds, deferred: r.deferred })),
  skipped: skipped.map((r) => ({ issue: r.issue, status: r.status, detail: r.detail })),
  pushed: DO_PUSH,
}
