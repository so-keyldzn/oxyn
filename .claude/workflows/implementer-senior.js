export const meta = {
  name: 'implementer-senior',
  description: 'Implement an Oxyn change: framing (code, libraries, invariants), critiqued plan, batched implementation, make qualite gate, adversarial review and fix',
  whenToUse: 'A feature or a fix that touches several files, in Rust or in apps/desktop. Pass the task as args (text, or { tache, contexte }).',
  phases: [
    { title: 'Framing', detail: 'code, libraries and skills, invariants — in parallel', model: 'sonnet' },
    { title: 'Plan', detail: 'batched plan, adversarial critique, one revision', model: 'opus' },
    { title: 'Implementation', detail: 'domain agents, sequential batches, disjoint steps in parallel', model: 'sonnet' },
    { title: 'Gate', detail: 'make qualite, repair bounded to three rounds', model: 'haiku' },
    { title: 'Review', detail: 'repository reviewers and library compliance', model: 'sonnet' },
    { title: 'Verification', detail: 'two skeptics from different models per finding', model: 'opus' },
    { title: 'Fix', detail: 'confirmed findings, then gate again' },
  ],
}

// --- Inputs ---------------------------------------------------------------
const TASK = typeof args === 'string' ? args : (args && args.tache) || ''
const CONTEXT = (args && typeof args === 'object' && args.contexte) || ''
// Decisions the user settled after an earlier run stopped on them. Kept out of
// SHARED so a resumed run replays the scouting agents from cache.
const DECIDED = (args && typeof args === 'object' && args.decisions) || []
const DECISIONS = DECIDED.length
  ? `\nDecisions settled by the user (apply them, do not reopen them):\n${DECIDED.map(d => `- ${d}`).join('\n')}\n`
  : ''
if (!TASK.trim()) {
  throw new Error('implementer-senior: pass the task as args (string or { tache, contexte })')
}

const MAX_GATE_ROUNDS = 3
const MAX_REVIEW_ROUNDS = 2

// The working tree usually carries the user's own work in progress: every
// writing agent is told so, because "cleaning up" it is the costliest mistake.
const SHARED = `
Task: ${TASK}
${CONTEXT ? `Context provided: ${CONTEXT}\n` : ''}
Rules of this workflow:
- The working tree may contain changes in progress that are not yours: never revert, reformat or "clean up" them. No git checkout, restore, reset, stash or commit.
- Code, identifiers, comments and any text meant for docs/ in English (ADR-0047).
- If the code contradicts a document of docs/, do not settle it: report it.
- No external version or limit from memory (I-12): the installed version is read in apps/desktop/package.json, Cargo.lock or node_modules.`

const DOMAIN_AGENTS = ['frontiste', 'rustacien', 'driveriste', 'ia-workspace', 'documentaliste']

// --- Schemas --------------------------------------------------------------
const CODE_MAP = {
  type: 'object',
  properties: {
    fichiersConcernes: { type: 'array', items: { type: 'object', properties: {
      chemin: { type: 'string' }, role: { type: 'string' } }, required: ['chemin', 'role'] } },
    motifsExistants: { type: 'array', items: { type: 'string' },
      description: 'Conventions already in place to imitate, with path:line' },
    modificationsEnCours: { type: 'array', items: { type: 'string' },
      description: 'Files already modified in the tree (git status) that overlap the task' },
  },
  required: ['fichiersConcernes', 'motifsExistants', 'modificationsEnCours'],
}

const LIB_BRIEF = {
  type: 'object',
  properties: {
    bibliotheques: { type: 'array', items: { type: 'object', properties: {
      nom: { type: 'string' },
      versionInstallee: { type: 'string' },
      sources: { type: 'array', items: { type: 'string' }, description: 'Paths of the SKILL.md / rules read' },
      reglesApplicables: { type: 'array', items: { type: 'string' },
        description: 'Concrete rules for THIS task, each with its source' },
      pieges: { type: 'array', items: { type: 'string' } },
    }, required: ['nom', 'versionInstallee', 'sources', 'reglesApplicables', 'pieges'] } },
  },
  required: ['bibliotheques'],
}

const CONTRACT_BRIEF = {
  type: 'object',
  properties: {
    invariants: { type: 'array', items: { type: 'object', properties: {
      id: { type: 'string' }, pourquoiConcerne: { type: 'string' } }, required: ['id', 'pourquoiConcerne'] } },
    documentsAutorite: { type: 'array', items: { type: 'string' } },
    procedures: { type: 'array', items: { type: 'string' },
      description: '.claude/commands commands to follow (/commande, /ecran, /driver…) and their requirements' },
    divergencesDocCode: { type: 'array', items: { type: 'string' } },
  },
  required: ['invariants', 'documentsAutorite', 'procedures', 'divergencesDocCode'],
}

const PLAN = {
  type: 'object',
  properties: {
    decisionsOuvertes: { type: 'array', items: { type: 'object', properties: {
      question: { type: 'string' }, options: { type: 'array', items: { type: 'string' } },
      recommandation: { type: 'string' } }, required: ['question', 'options', 'recommandation'] },
      description: 'What belongs to an ADR or to the user. Non-empty = the workflow stops before coding.' },
    lots: { type: 'array', items: { type: 'object', properties: {
      titre: { type: 'string' },
      etapes: { type: 'array', items: { type: 'object', properties: {
        titre: { type: 'string' },
        agent: { type: 'string', enum: DOMAIN_AGENTS },
        fichiers: { type: 'array', items: { type: 'string' } },
        consigne: { type: 'string', description: 'What to do, precisely, with the library rules that apply' },
        verificationLocale: { type: 'string', description: 'Quick command to run after the step (cargo check -p …, pnpm -C apps/desktop typecheck…)' },
      }, required: ['titre', 'agent', 'fichiers', 'consigne', 'verificationLocale'] } },
    }, required: ['titre', 'etapes'] },
      description: 'Batches executed in order; the steps of a batch run in parallel and share no file. The bus command always precedes the screen.' },
    risques: { type: 'array', items: { type: 'string' } },
  },
  required: ['decisionsOuvertes', 'lots', 'risques'],
}

const CRITIQUE = {
  type: 'object',
  properties: {
    acceptable: { type: 'boolean' },
    objections: { type: 'array', items: { type: 'object', properties: {
      gravite: { type: 'string', enum: ['bloquant', 'important', 'mineur'] },
      objection: { type: 'string' }, correction: { type: 'string' } },
      required: ['gravite', 'objection', 'correction'] } },
  },
  required: ['acceptable', 'objections'],
}

const STEP_REPORT = {
  type: 'object',
  properties: {
    fichiersModifies: { type: 'array', items: { type: 'string' } },
    verificationLocale: { type: 'string', description: 'Command run and its result' },
    ecartsAuPlan: { type: 'array', items: { type: 'string' } },
    signalements: { type: 'array', items: { type: 'string' }, description: 'Doc/code divergences, unsettled decisions' },
  },
  required: ['fichiersModifies', 'verificationLocale', 'ecartsAuPlan', 'signalements'],
}

const GATE = {
  type: 'object',
  properties: {
    ok: { type: 'boolean' },
    echecs: { type: 'array', items: { type: 'object', properties: {
      etape: { type: 'string', description: 'socle, todo, front, format, lint, test, doc, deny' },
      fichier: { type: 'string' },
      message: { type: 'string' },
      imputable: { type: 'boolean', description: 'true if the file is among the files touched by this workflow' },
    }, required: ['etape', 'fichier', 'message', 'imputable'] } },
  },
  required: ['ok', 'echecs'],
}

const FINDINGS = {
  type: 'object',
  properties: {
    constats: { type: 'array', items: { type: 'object', properties: {
      fichier: { type: 'string' },
      ligne: { type: 'integer' },
      gravite: { type: 'string', enum: ['bloquant', 'important', 'mineur'] },
      resume: { type: 'string' },
      scenario: { type: 'string', description: 'Concrete input or state → wrong result' },
      correction: { type: 'string' },
    }, required: ['fichier', 'ligne', 'gravite', 'resume', 'scenario', 'correction'] } },
  },
  required: ['constats'],
}

const VERDICT = {
  type: 'object',
  properties: {
    reel: { type: 'boolean' },
    raison: { type: 'string' },
  },
  required: ['reel', 'raison'],
}

// --- Helpers --------------------------------------------------------------
const list = xs => (xs && xs.length ? xs.map(x => `- ${x}`).join('\n') : '(nothing)')
const json = x => JSON.stringify(x, null, 2)
const unique = xs => [...new Set(xs)]
// Agents report absolute or relative paths at will; reviewer routing matches
// on repo-relative prefixes, so an absolute path silently skipped a reviewer.
const repoRelative = f => {
  const m = /(?:^|\/)((?:apps|crates|drivers|docs|script|\.claude)\/.*)$/.exec(f)
  return m ? m[1] : f
}

function partitionDisjoint(steps) {
  // A lot the plan claims is parallel may still share a file; those steps
  // then run one after another instead of racing on the same buffer.
  const waves = []
  for (const step of steps) {
    const wave = waves.find(w => !w.files.some(f => step.fichiers.includes(f)))
    if (wave) { wave.steps.push(step); wave.files.push(...step.fichiers) }
    else waves.push({ steps: [step], files: [...step.fichiers] })
  }
  return waves.map(w => w.steps)
}

async function runGate(touched, label) {
  let last = null
  for (let round = 1; round <= MAX_GATE_ROUNDS; round++) {
    last = await agent(`Run Oxyn's quality gate at the repository root and report its result.

Command: \`set -o pipefail; make qualite 2>&1 | tail -n 400\` (long timeout: it compiles and runs Storybook). No redirection to a file.

Two known failures are not regressions: "extern location does not exist" (target/debug emptied by a cleaner) and an undefined .llvm symbol at link time (incremental cache). In those cases, purge target/debug/incremental/<faulty crate>-* and rerun once before concluding.

Files touched by this workflow (used to fill "imputable"):
${list(touched)}

Fix nothing.`, { label: `gate:${label}:${round}`, phase: 'Gate', schema: GATE, model: 'haiku', effort: 'low' })
    if (!last) return { ok: false, echecs: [], abandon: true }
    if (last.ok) return last
    const mine = last.echecs.filter(e => e.imputable)
    if (!mine.length) {
      log(`Gate failing on files outside the scope (${last.echecs.length}): left to the user`)
      return last
    }
    if (round === MAX_GATE_ROUNDS) break
    await agent(`${SHARED}

The \`make qualite\` gate fails on files this workflow modified. Fix the cause, not the symptom: no \`#[allow]\`, no \`eslint-disable\`, no \`@ts-expect-error\`, no disabled test.

Failures:
${json(mine)}

Then rerun the fastest targeted check (cargo clippy -p <crate>, pnpm -C apps/desktop typecheck, lint or test) until it passes.`, { label: `repair:${label}:${round}`, phase: 'Gate', model: 'sonnet' })
  }
  log(`Gate still failing after ${MAX_GATE_ROUNDS} rounds`)
  return last
}

// --- 1. Framing -----------------------------------------------------------
phase('Framing')
const [codeMap, libBrief, contractBrief] = await parallel([
  () => agent(`${SHARED}

Map the code concerned by this task, without modifying anything: the files to touch or read (Rust in crates/ and drivers/, front end in apps/desktop/src), the conventions already in place to imitate (with path:line), and, via \`git status --short\`, the files already modified in the tree that overlap the task.`,
    { label: 'framing:code', phase: 'Framing', schema: CODE_MAP, agentType: 'Explore', model: 'sonnet' }),

  () => agent(`${SHARED}

Establish what the libraries require for THIS task. Modify nothing.

1. Installed versions: apps/desktop/package.json (TanStack Query, Router, Start, Form, Store, Table, Virtual, Pacer, Hotkeys; Base UI, shadcn, zod, CodeMirror, Tauri API…) and Cargo.lock on the Rust side (tauri, arrow, tokio…).
2. Repository skills: .claude/skills/{tanstack-query,tanstack-router-best-practices,tanstack-start-best-practices,tauri-v2,shadcn}/ — SKILL.md then only the rules/ or references/ files useful to the task.
3. Skills shipped by the packages, aligned with the installed version and therefore taking precedence in case of disagreement: \`find -L apps/desktop/node_modules/@tanstack apps/desktop/node_modules/.pnpm -path '*@tanstack*' -name SKILL.md\` (router-core, start-client-core, router-plugin, devtools…).
4. For a library without a skill (Form, Store, Table, Virtual, Pacer, Hotkeys), read its .d.ts types in node_modules and an existing usage in apps/desktop/src.

Project reminders that take precedence over generic skills: TanStack Start runs in SPA mode without a server (no createServerFn, no SSR); the backend is Rust behind Tauri; invoke is only called by call() in src/lib/ipc/client.ts, with a zod schema; components go through Base UI (render, not asChild) and Hugeicons.

Keep only the libraries the task actually touches.`,
    { label: 'framing:libraries', phase: 'Framing', schema: LIB_BRIEF, model: 'sonnet' }),

  () => agent(`${SHARED}

Establish the contract to honor, without modifying anything: the invariants I-01 to I-13 of CLAUDE.md the task engages (and why), the authoritative documents of docs/ to honor, the procedure(s) of .claude/commands/ that apply (/commande, /ecran, /driver, /securite…) with their concrete requirements, and any divergence already visible between the code and docs/.`,
    { label: 'framing:contract', phase: 'Framing', schema: CONTRACT_BRIEF, model: 'sonnet' }),
])

if (!codeMap || !libBrief || !contractBrief) {
  throw new Error('implementer-senior: a scouting agent failed; nothing was written')
}
const BRIEF = `Code map:
${json(codeMap)}

Libraries and skills:
${json(libBrief)}

Contract:
${json(contractBrief)}`

// --- 2. Plan --------------------------------------------------------------
phase('Plan')
let plan = await agent(`${SHARED}

${BRIEF}
${DECISIONS}
Write the implementation plan, in ordered batch(es). Requirements:
- the bus command (and the Tauri command that emits it) precedes any screen that uses it;
- within a batch, the steps share no file; each step names its domain agent (frontiste for apps/desktop and oxyn-desktop, ia-workspace for oxyn-ai, driveriste for drivers/, rustacien for the rest of the core, documentaliste for docs/);
- each instruction cites the library rules that apply to it, and requires one story per state for a component of src/components/oxyn;
- one step = one change verifiable by its verificationLocale;
- no abstraction for a single caller, no dead code.
Any choice that is expensive to undo or not settled by docs/ goes into decisionsOuvertes, not into the plan.`,
  { label: 'plan', phase: 'Plan', schema: PLAN, agentType: 'Plan', model: 'opus', effort: 'high' })

const critique = await agent(`${SHARED}

${BRIEF}
${DECISIONS}
Proposed plan:
${json(plan)}

You are the critic of this plan. Look for what will make it fail: an invariant violated (especially I-01, I-05, I-06, I-09), wrong order between command and screen, files shared within a batch, a library rule ignored or contradicted by the installed version, an error case or interface state forgotten, an unverifiable step, over-engineering. Read the code to check each objection; keep none you cannot back up.`,
  { label: 'critique:plan', phase: 'Plan', schema: CRITIQUE, model: 'sonnet', effort: 'high' })

if (critique && critique.objections.some(o => o.gravite !== 'mineur')) {
  log(`Plan revised: ${critique.objections.filter(o => o.gravite !== 'mineur').length} objection(s) retained`)
  plan = await agent(`${SHARED}

${BRIEF}
${DECISIONS}
Initial plan:
${json(plan)}

Critic's objections:
${json(critique.objections)}

Return the revised plan. Integrate each well-founded objection; for an objection you reject, say why in risques.`,
    { label: 'plan:revision', phase: 'Plan', schema: PLAN, agentType: 'Plan', model: 'opus', effort: 'high' })
}

if (plan.decisionsOuvertes.length) {
  log('Decisions to settle before coding: the workflow stops without writing anything')
  return { statut: 'decisions-a-trancher', decisionsOuvertes: plan.decisionsOuvertes, plan, cadrage: { codeMap, libBrief, contractBrief } }
}

// --- 3. Implementation ----------------------------------------------------
phase('Implementation')
const reports = []
for (const [i, lot] of plan.lots.entries()) {
  log(`Batch ${i + 1}/${plan.lots.length}: ${lot.titre}`)
  for (const wave of partitionDisjoint(lot.etapes)) {
    const done = await parallel(wave.map(step => () => agent(`${SHARED}

Library rules established during framing:
${json(libBrief)}

Contract:
${json(contractBrief)}
${DECISIONS}
Your step (batch "${lot.titre}"): ${step.titre}
Files entrusted to you: ${step.fichiers.join(', ')}
Instruction: ${step.consigne}

Other agents are working in parallel on other files: touch only yours, except for an indispensable declaration line (mod, generate_handler!, export) that you report. Follow the .claude/commands/ procedure that matches the move. Finish with: ${step.verificationLocale}`,
      { label: `impl:${step.agent}:${step.titre}`, phase: 'Implementation', schema: STEP_REPORT, agentType: step.agent, model: 'sonnet' })))
    reports.push(...done.filter(Boolean))
    const lost = done.length - done.filter(Boolean).length
    if (lost) log(`${lost} step(s) of batch ${i + 1} without a report: check by hand`)
  }
}

let touched = unique(reports.flatMap(r => r.fichiersModifies).map(repoRelative))
const flagged = unique(reports.flatMap(r => r.signalements))

// --- 4. Gate --------------------------------------------------------------
phase('Gate')
let gate = await runGate(touched, 'initial')

// --- 5–7. Review, verification, fix ---------------------------------------
function reviewers(files) {
  const has = re => files.some(f => re.test(f))
  const scope = `Files modified by this workflow (review their diff with \`git diff -- <file>\`, and the whole file for an untracked file):\n${list(files)}\n\nThe tree contains other modifications that do not belong to this change: ignore them.\n\nTask implemented: ${TASK}`
  const r = [
    { cle: 'invariants', agentType: 'relecteur-invariants', model: 'opus',
      prompt: `${scope}\n\nReview this change against the thirteen invariants.` },
    { cle: 'correction', model: 'opus',
      prompt: `${scope}\n\nLook for correctness defects: wrong logic, edge case, swallowed error, inconsistent React state, race between queries, missing cancellation, zod schema that does not match the Rust (Option → .nullable(), union tag, flattened variant). No style.` },
    { cle: 'bibliotheques', model: 'sonnet',
      prompt: `${scope}\n\nCheck library usage against these rules established during framing (and against the SKILL.md files cited):\n${json(libBrief)}\n\nStable and invalidated TanStack Query keys, options shared via queryOptions, Store selectors, virtualization of long lists, API of the installed version and not of another one.` },
  ]
  if (has(/^apps\/desktop\//)) r.push({ cle: 'interface', model: 'sonnet',
    prompt: `${scope}\n\nReview the interface with .claude/checklists/revue-ui.md and docs/UX-SPEC.md: one story per state, accessibility, Base UI components rather than styled divs, semantic tokens, Hugeicons, no invoke outside call().` })
  if (has(/^crates\/oxyn-desktop\/src\/(commands|ipc)|^crates\/oxyn-(ai|llm|secrets|plugin)\/|^drivers\//)) r.push({ cle: 'securite', agentType: 'relecteur-securite', model: 'opus',
    prompt: `${scope}\n\nSecurity review of this change.` })
  if (has(/^drivers\/|^crates\/oxyn-(ai|llm|driver)\//)) r.push({ cle: 'frontiere', agentType: 'relecteur-frontiere', model: 'opus',
    prompt: `${scope}\n\nReview what crosses an external boundary in this change.` })
  if (has(/^crates\//)) r.push({ cle: 'divergence', agentType: 'detecteur-divergence', model: 'sonnet', effort: 'medium',
    prompt: `${scope}\n\nLook for the gaps this change creates or reveals between the code and docs/.` })
  return r
}

async function verify(finding, dimension) {
  // Two refuters on two model tiers: a shared blind spot is less likely than
  // with two copies of the same model.
  const votes = await parallel([['opus', 'high'], ['sonnet', 'high']].map(([model, effort]) => () =>
    agent(`A reviewer (${dimension}) reports this defect in the Oxyn repository:
${json(finding)}

Try to REFUTE it: read the code, follow the calls, look for what makes the scenario impossible or already handled. Return reel=false if you refute it or if you cannot establish the scenario; reel=true only if the defect is demonstrable from the code.`,
      { label: `verify:${model}:${finding.fichier}:${finding.ligne}`, phase: 'Verification', schema: VERDICT, model, effort })))
  const yes = votes.filter(Boolean).filter(v => v.reel).length
  const kept = finding.gravite === 'bloquant' ? yes >= 1 : yes >= 2
  return { ...finding, dimension, confirme: kept, votes: votes.filter(Boolean) }
}

const fixed = []
const minor = []
let remaining = []
for (let round = 1; round <= MAX_REVIEW_ROUNDS; round++) {
  phase('Review')
  const dims = reviewers(touched)
  const judged = await pipeline(
    dims,
    d => agent(d.prompt, { label: `review:${d.cle}:${round}`, phase: 'Review', schema: FINDINGS, agentType: d.agentType, model: d.model, effort: d.effort }),
    (res, d) => {
      const all = (res && res.constats) || []
      minor.push(...all.filter(c => c.gravite === 'mineur').map(c => ({ ...c, dimension: d.cle })))
      return parallel(all.filter(c => c.gravite !== 'mineur').map(c => () => verify(c, d.cle)))
    },
  )
  const verdicts = judged.filter(Boolean).flat().filter(Boolean)
  const confirmed = verdicts.filter(v => v.confirme)
  log(`Review ${round}: ${verdicts.length} finding(s) verified, ${confirmed.length} confirmed`)
  remaining = confirmed
  if (!confirmed.length) break
  if (round === MAX_REVIEW_ROUNDS) {
    log('Confirmed findings remaining at the last round: left to the user')
    break
  }

  phase('Fix')
  const fix = await agent(`${SHARED}

Library rules:
${json(libBrief)}

Fix these defects confirmed by an adversarial review. Each one survived two refutation attempts: fix the cause, with a test that would have failed before when possible. If a defect still looks wrong to you, do not fix it and explain why in ecartsAuPlan.

${json(confirmed)}

Finish with the fastest targeted check of the files touched.`,
    { label: `fix:${round}`, phase: 'Fix', schema: STEP_REPORT, model: 'sonnet' })
  if (fix) {
    // A fixer may decline a finding (excluded file, disputed defect): only
    // findings in a file it actually changed count as fixed.
    const changed = new Set(fix.fichiersModifies.map(repoRelative))
    fixed.push(...confirmed.filter(c => changed.has(repoRelative(c.fichier))))
    touched = unique([...touched, ...changed])
  }
  gate = await runGate(touched, `after-fix-${round}`)
}

return {
  statut: gate && gate.ok && !remaining.length ? 'termine' : 'a-reprendre',
  porte: gate,
  fichiersModifies: touched,
  plan: plan.lots.map(l => ({ lot: l.titre, etapes: l.etapes.map(e => `${e.agent} — ${e.titre}`) })),
  ecartsAuPlan: unique(reports.flatMap(r => r.ecartsAuPlan)),
  signalements: flagged,
  risques: plan.risques,
  constatsCorriges: fixed.map(c => `${c.fichier}:${c.ligne} — ${c.resume}`),
  constatsRestants: remaining,
  constatsMineurs: minor,
}
