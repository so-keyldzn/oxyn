export const meta = {
  name: 'implementer-senior',
  description: 'Implémenter un changement Oxyn : cadrage (code, bibliothèques, invariants), plan critiqué, implémentation par lots, porte make qualite, relecture adversariale et correction',
  whenToUse: 'Une fonctionnalité ou un correctif qui touche plusieurs fichiers, en Rust ou dans apps/desktop. Passer la tâche en args (texte, ou { tache, contexte }).',
  phases: [
    { title: 'Cadrage', detail: 'code, bibliothèques et skills, invariants — en parallèle', model: 'sonnet' },
    { title: 'Plan', detail: 'plan par lots, critique adversariale, une révision', model: 'opus' },
    { title: 'Implémentation', detail: 'agents du domaine, lots séquentiels, étapes disjointes en parallèle', model: 'sonnet' },
    { title: 'Porte', detail: 'make qualite, réparation bornée à trois tours', model: 'haiku' },
    { title: 'Relecture', detail: 'relecteurs du dépôt et conformité des bibliothèques', model: 'sonnet' },
    { title: 'Vérification', detail: 'deux sceptiques de modèles différents par constat', model: 'opus' },
    { title: 'Correction', detail: 'constats confirmés, puis porte à nouveau' },
  ],
}

// --- Inputs ---------------------------------------------------------------
const TASK = typeof args === 'string' ? args : (args && args.tache) || ''
const CONTEXT = (args && typeof args === 'object' && args.contexte) || ''
// Decisions the user settled after an earlier run stopped on them. Kept out of
// SHARED so a resumed run replays the scouting agents from cache.
const DECIDED = (args && typeof args === 'object' && args.decisions) || []
const DECISIONS = DECIDED.length
  ? `\nDécisions tranchées par l'utilisateur (à appliquer, ne pas les rouvrir) :\n${DECIDED.map(d => `- ${d}`).join('\n')}\n`
  : ''
if (!TASK.trim()) {
  throw new Error('implementer-senior: pass the task as args (string or { tache, contexte })')
}

const MAX_GATE_ROUNDS = 3
const MAX_REVIEW_ROUNDS = 2

// The working tree usually carries the user's own work in progress: every
// writing agent is told so, because "cleaning up" it is the costliest mistake.
const SHARED = `
Tâche : ${TASK}
${CONTEXT ? `Contexte fourni : ${CONTEXT}\n` : ''}
Règles de ce workflow :
- L'arbre de travail contient peut-être des modifications en cours qui ne sont pas les tiennes : ne les annule, ne les reformate et ne les « nettoie » jamais. Aucun git checkout, restore, reset, stash ni commit.
- Code, identifiants et commentaires en anglais ; tout texte destiné à l'utilisateur ou à docs/ en français.
- Si le code contredit un document de docs/, ne tranche pas : signale-le.
- Aucune version ni limite externe de mémoire (I-12) : la version installée se lit dans apps/desktop/package.json, Cargo.lock ou node_modules.`

const DOMAIN_AGENTS = ['frontiste', 'rustacien', 'driveriste', 'ia-workspace', 'documentaliste']

// --- Schemas --------------------------------------------------------------
const CODE_MAP = {
  type: 'object',
  properties: {
    fichiersConcernes: { type: 'array', items: { type: 'object', properties: {
      chemin: { type: 'string' }, role: { type: 'string' } }, required: ['chemin', 'role'] } },
    motifsExistants: { type: 'array', items: { type: 'string' },
      description: 'Conventions déjà en place à imiter, avec chemin:ligne' },
    modificationsEnCours: { type: 'array', items: { type: 'string' },
      description: 'Fichiers déjà modifiés dans l’arbre (git status) qui recoupent la tâche' },
  },
  required: ['fichiersConcernes', 'motifsExistants', 'modificationsEnCours'],
}

const LIB_BRIEF = {
  type: 'object',
  properties: {
    bibliotheques: { type: 'array', items: { type: 'object', properties: {
      nom: { type: 'string' },
      versionInstallee: { type: 'string' },
      sources: { type: 'array', items: { type: 'string' }, description: 'Chemins des SKILL.md / règles lus' },
      reglesApplicables: { type: 'array', items: { type: 'string' },
        description: 'Règles concrètes pour CETTE tâche, chacune avec sa source' },
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
      description: 'Commandes .claude/commands à suivre (/commande, /ecran, /driver…) et leurs exigences' },
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
      description: 'Ce qui relève d’un ADR ou de l’utilisateur. Non vide = le workflow s’arrête avant de coder.' },
    lots: { type: 'array', items: { type: 'object', properties: {
      titre: { type: 'string' },
      etapes: { type: 'array', items: { type: 'object', properties: {
        titre: { type: 'string' },
        agent: { type: 'string', enum: DOMAIN_AGENTS },
        fichiers: { type: 'array', items: { type: 'string' } },
        consigne: { type: 'string', description: 'Quoi faire, précisément, avec les règles de bibliothèque qui s’appliquent' },
        verificationLocale: { type: 'string', description: 'Commande rapide à lancer après l’étape (cargo check -p …, pnpm -C apps/desktop typecheck…)' },
      }, required: ['titre', 'agent', 'fichiers', 'consigne', 'verificationLocale'] } },
    }, required: ['titre', 'etapes'] },
      description: 'Lots exécutés dans l’ordre ; les étapes d’un lot tournent en parallèle et ne partagent aucun fichier. La commande du bus précède toujours l’écran.' },
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
    verificationLocale: { type: 'string', description: 'Commande lancée et son résultat' },
    ecartsAuPlan: { type: 'array', items: { type: 'string' } },
    signalements: { type: 'array', items: { type: 'string' }, description: 'Divergences doc/code, décisions non tranchées' },
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
      imputable: { type: 'boolean', description: 'true si le fichier fait partie des fichiers touchés par ce workflow' },
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
      scenario: { type: 'string', description: 'Entrée ou état concret → résultat faux' },
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
const list = xs => (xs && xs.length ? xs.map(x => `- ${x}`).join('\n') : '(rien)')
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
    last = await agent(`Lance la porte de qualité d'Oxyn à la racine du dépôt et rapporte son résultat.

Commande : \`set -o pipefail; make qualite 2>&1 | tail -n 400\` (timeout long : elle compile et lance Storybook). Aucune redirection vers un fichier.

Deux échecs connus ne sont pas des régressions : « extern location does not exist » (target/debug vidé par un nettoyeur) et un symbole .llvm indéfini au lien (cache incrémental). Dans ces cas, purge target/debug/incremental/<crate fautive>-* et relance une fois avant de conclure.

Fichiers touchés par ce workflow (sert à remplir « imputable ») :
${list(touched)}

Ne corrige rien.`, { label: `porte:${label}:${round}`, phase: 'Porte', schema: GATE, model: 'haiku', effort: 'low' })
    if (!last) return { ok: false, echecs: [], abandon: true }
    if (last.ok) return last
    const mine = last.echecs.filter(e => e.imputable)
    if (!mine.length) {
      log(`Porte en échec sur des fichiers hors du périmètre (${last.echecs.length}) : laissés à l'utilisateur`)
      return last
    }
    if (round === MAX_GATE_ROUNDS) break
    await agent(`${SHARED}

La porte \`make qualite\` échoue sur des fichiers que ce workflow a modifiés. Corrige la cause, pas le symptôme : ni \`#[allow]\`, ni \`eslint-disable\`, ni \`@ts-expect-error\`, ni test désactivé.

Échecs :
${json(mine)}

Relance ensuite la vérification ciblée la plus rapide (cargo clippy -p <crate>, pnpm -C apps/desktop typecheck, lint ou test) jusqu'à ce qu'elle passe.`, { label: `réparation:${label}:${round}`, phase: 'Porte', model: 'sonnet' })
  }
  log(`Porte toujours en échec après ${MAX_GATE_ROUNDS} tours`)
  return last
}

// --- 1. Cadrage -----------------------------------------------------------
phase('Cadrage')
const [codeMap, libBrief, contractBrief] = await parallel([
  () => agent(`${SHARED}

Cartographie le code concerné par cette tâche, sans rien modifier : les fichiers à toucher ou à lire (Rust dans crates/ et drivers/, front dans apps/desktop/src), les conventions déjà en place à imiter (avec chemin:ligne), et, via \`git status --short\`, les fichiers déjà modifiés dans l'arbre qui recoupent la tâche.`,
    { label: 'cadrage:code', phase: 'Cadrage', schema: CODE_MAP, agentType: 'Explore', model: 'sonnet' }),

  () => agent(`${SHARED}

Établis ce que les bibliothèques exigent pour CETTE tâche. Ne modifie rien.

1. Versions installées : apps/desktop/package.json (TanStack Query, Router, Start, Form, Store, Table, Virtual, Pacer, Hotkeys ; Base UI, shadcn, zod, CodeMirror, Tauri API…) et Cargo.lock côté Rust (tauri, arrow, tokio…).
2. Skills du dépôt : .claude/skills/{tanstack-query,tanstack-router-best-practices,tanstack-start-best-practices,tauri-v2,shadcn}/ — SKILL.md puis seulement les fichiers de rules/ ou references/ utiles à la tâche.
3. Skills embarqués par les paquets, alignés sur la version installée donc prioritaires en cas de désaccord : \`find -L apps/desktop/node_modules/@tanstack apps/desktop/node_modules/.pnpm -path '*@tanstack*' -name SKILL.md\` (router-core, start-client-core, router-plugin, devtools…).
4. Pour une bibliothèque sans skill (Form, Store, Table, Virtual, Pacer, Hotkeys), lis ses types .d.ts dans node_modules et un usage existant dans apps/desktop/src.

Rappels du projet qui priment sur les skills génériques : TanStack Start tourne en mode SPA sans serveur (pas de createServerFn, pas de SSR) ; le backend est Rust derrière Tauri ; invoke n'est appelé que par call() dans src/lib/ipc/client.ts, avec un schéma zod ; les composants passent par Base UI (render, pas asChild) et Hugeicons.

Ne retiens que les bibliothèques que la tâche touche réellement.`,
    { label: 'cadrage:bibliothèques', phase: 'Cadrage', schema: LIB_BRIEF, model: 'sonnet' }),

  () => agent(`${SHARED}

Établis le contrat à respecter, sans rien modifier : les invariants I-01 à I-13 de CLAUDE.md que la tâche engage (et pourquoi), les documents d'autorité de docs/ à respecter, la ou les procédures de .claude/commands/ qui s'appliquent (/commande, /ecran, /driver, /securite…) avec leurs exigences concrètes, et toute divergence déjà visible entre le code et docs/.`,
    { label: 'cadrage:contrat', phase: 'Cadrage', schema: CONTRACT_BRIEF, model: 'sonnet' }),
])

if (!codeMap || !libBrief || !contractBrief) {
  throw new Error('implementer-senior: a scouting agent failed; nothing was written')
}
const BRIEF = `Carte du code :
${json(codeMap)}

Bibliothèques et skills :
${json(libBrief)}

Contrat :
${json(contractBrief)}`

// --- 2. Plan --------------------------------------------------------------
phase('Plan')
let plan = await agent(`${SHARED}

${BRIEF}
${DECISIONS}
Écris le plan d'implémentation, en lot(s) ordonnés. Exigences :
- la commande du bus (et la commande Tauri qui l'émet) précède tout écran qui l'utilise ;
- dans un lot, les étapes ne partagent aucun fichier ; chaque étape nomme son agent du domaine (frontiste pour apps/desktop et oxyn-desktop, ia-workspace pour oxyn-ai, driveriste pour drivers/, rustacien pour le reste du cœur, documentaliste pour docs/) ;
- chaque consigne cite les règles de bibliothèque qui s'y appliquent, et exige les stories par état pour un composant de src/components/oxyn ;
- une étape = un changement vérifiable par sa verificationLocale ;
- pas d'abstraction pour un seul appelant, pas de code mort.
Tout choix coûteux à défaire ou non tranché par docs/ va dans decisionsOuvertes, pas dans le plan.`,
  { label: 'plan', phase: 'Plan', schema: PLAN, agentType: 'Plan', model: 'opus', effort: 'high' })

const critique = await agent(`${SHARED}

${BRIEF}
${DECISIONS}
Plan proposé :
${json(plan)}

Tu es le critique de ce plan. Cherche ce qui le fera échouer : invariant violé (surtout I-01, I-05, I-06, I-09), ordre faux entre commande et écran, fichiers partagés dans un même lot, règle de bibliothèque ignorée ou contredite par la version installée, cas d'erreur ou état d'interface oublié, étape invérifiable, sur-conception. Lis le code pour vérifier chaque objection ; n'en garde aucune que tu ne peux pas étayer.`,
  { label: 'critique:plan', phase: 'Plan', schema: CRITIQUE, model: 'sonnet', effort: 'high' })

if (critique && critique.objections.some(o => o.gravite !== 'mineur')) {
  log(`Plan révisé : ${critique.objections.filter(o => o.gravite !== 'mineur').length} objection(s) retenue(s)`)
  plan = await agent(`${SHARED}

${BRIEF}
${DECISIONS}
Plan initial :
${json(plan)}

Objections du critique :
${json(critique.objections)}

Rends le plan révisé. Intègre chaque objection fondée ; pour une objection que tu rejettes, dis pourquoi dans risques.`,
    { label: 'plan:révision', phase: 'Plan', schema: PLAN, agentType: 'Plan', model: 'opus', effort: 'high' })
}

if (plan.decisionsOuvertes.length) {
  log('Décisions à trancher avant de coder : le workflow s’arrête sans rien écrire')
  return { statut: 'decisions-a-trancher', decisionsOuvertes: plan.decisionsOuvertes, plan, cadrage: { codeMap, libBrief, contractBrief } }
}

// --- 3. Implémentation ----------------------------------------------------
phase('Implémentation')
const reports = []
for (const [i, lot] of plan.lots.entries()) {
  log(`Lot ${i + 1}/${plan.lots.length} : ${lot.titre}`)
  for (const wave of partitionDisjoint(lot.etapes)) {
    const done = await parallel(wave.map(step => () => agent(`${SHARED}

Règles de bibliothèque établies au cadrage :
${json(libBrief)}

Contrat :
${json(contractBrief)}
${DECISIONS}
Ton étape (lot « ${lot.titre} ») : ${step.titre}
Fichiers qui te sont confiés : ${step.fichiers.join(', ')}
Consigne : ${step.consigne}

D'autres agents travaillent en parallèle sur d'autres fichiers : ne touche que les tiens, sauf une ligne de déclaration indispensable (mod, generate_handler!, export) que tu signales. Suis la procédure de .claude/commands/ qui correspond au geste. Termine par : ${step.verificationLocale}`,
      { label: `impl:${step.agent}:${step.titre}`, phase: 'Implémentation', schema: STEP_REPORT, agentType: step.agent, model: 'sonnet' })))
    reports.push(...done.filter(Boolean))
    const lost = done.length - done.filter(Boolean).length
    if (lost) log(`${lost} étape(s) du lot ${i + 1} sans rapport : à vérifier à la main`)
  }
}

let touched = unique(reports.flatMap(r => r.fichiersModifies).map(repoRelative))
const flagged = unique(reports.flatMap(r => r.signalements))

// --- 4. Porte -------------------------------------------------------------
phase('Porte')
let gate = await runGate(touched, 'initiale')

// --- 5–7. Relecture, vérification, correction ------------------------------
function reviewers(files) {
  const has = re => files.some(f => re.test(f))
  const scope = `Fichiers modifiés par ce workflow (relis leur diff avec \`git diff -- <fichier>\`, et le fichier entier pour un fichier non suivi) :\n${list(files)}\n\nL'arbre contient d'autres modifications qui ne relèvent pas de ce changement : ignore-les.\n\nTâche implémentée : ${TASK}`
  const r = [
    { cle: 'invariants', agentType: 'relecteur-invariants', model: 'opus',
      prompt: `${scope}\n\nRelis ce changement contre les treize invariants.` },
    { cle: 'correction', model: 'opus',
      prompt: `${scope}\n\nCherche les défauts de justesse : logique fausse, cas limite, erreur avalée, état React incohérent, course entre requêtes, annulation manquante, schéma zod qui ne correspond pas au Rust (Option → .nullable(), tag d'union, variante aplatie). Pas de style.` },
    { cle: 'bibliotheques', model: 'sonnet',
      prompt: `${scope}\n\nVérifie l'usage des bibliothèques contre ces règles établies au cadrage (et contre les SKILL.md cités) :\n${json(libBrief)}\n\nClés de requête TanStack Query stables et invalidées, options partagées via queryOptions, sélecteurs de Store, virtualisation des longues listes, API de la version installée et non d'une autre.` },
  ]
  if (has(/^apps\/desktop\//)) r.push({ cle: 'interface', model: 'sonnet',
    prompt: `${scope}\n\nRelis l'interface avec .claude/checklists/revue-ui.md et docs/UX-SPEC.md : stories par état, accessibilité, composants Base UI plutôt que div stylés, jetons sémantiques, Hugeicons, aucun invoke hors de call().` })
  if (has(/^crates\/oxyn-desktop\/src\/(commands|ipc)|^crates\/oxyn-(ai|llm|secrets|plugin)\/|^drivers\//)) r.push({ cle: 'securite', agentType: 'relecteur-securite', model: 'opus',
    prompt: `${scope}\n\nRelecture de sécurité de ce changement.` })
  if (has(/^drivers\/|^crates\/oxyn-(ai|llm|driver)\//)) r.push({ cle: 'frontiere', agentType: 'relecteur-frontiere', model: 'opus',
    prompt: `${scope}\n\nRelis ce qui traverse une frontière externe dans ce changement.` })
  if (has(/^crates\//)) r.push({ cle: 'divergence', agentType: 'detecteur-divergence', model: 'sonnet', effort: 'medium',
    prompt: `${scope}\n\nCherche les écarts que ce changement crée ou révèle entre le code et docs/.` })
  return r
}

async function verify(finding, dimension) {
  // Two refuters on two model tiers: a shared blind spot is less likely than
  // with two copies of the same model.
  const votes = await parallel([['opus', 'high'], ['sonnet', 'high']].map(([model, effort]) => () =>
    agent(`Un relecteur (${dimension}) signale ce défaut dans le dépôt Oxyn :
${json(finding)}

Essaie de le RÉFUTER : lis le code, suis les appels, cherche ce qui rend le scénario impossible ou déjà traité. Rends reel=false si tu réfutes ou si tu ne peux pas établir le scénario ; reel=true seulement si le défaut est démontrable depuis le code.`,
      { label: `vérif:${model}:${finding.fichier}:${finding.ligne}`, phase: 'Vérification', schema: VERDICT, model, effort })))
  const yes = votes.filter(Boolean).filter(v => v.reel).length
  const kept = finding.gravite === 'bloquant' ? yes >= 1 : yes >= 2
  return { ...finding, dimension, confirme: kept, votes: votes.filter(Boolean) }
}

const fixed = []
const minor = []
let remaining = []
for (let round = 1; round <= MAX_REVIEW_ROUNDS; round++) {
  phase('Relecture')
  const dims = reviewers(touched)
  const judged = await pipeline(
    dims,
    d => agent(d.prompt, { label: `relecture:${d.cle}:${round}`, phase: 'Relecture', schema: FINDINGS, agentType: d.agentType, model: d.model, effort: d.effort }),
    (res, d) => {
      const all = (res && res.constats) || []
      minor.push(...all.filter(c => c.gravite === 'mineur').map(c => ({ ...c, dimension: d.cle })))
      return parallel(all.filter(c => c.gravite !== 'mineur').map(c => () => verify(c, d.cle)))
    },
  )
  const verdicts = judged.filter(Boolean).flat().filter(Boolean)
  const confirmed = verdicts.filter(v => v.confirme)
  log(`Relecture ${round} : ${verdicts.length} constat(s) vérifié(s), ${confirmed.length} confirmé(s)`)
  remaining = confirmed
  if (!confirmed.length) break
  if (round === MAX_REVIEW_ROUNDS) {
    log('Constats confirmés restants au dernier tour : laissés à l’utilisateur')
    break
  }

  phase('Correction')
  const fix = await agent(`${SHARED}

Règles de bibliothèque :
${json(libBrief)}

Corrige ces défauts confirmés par une relecture adversariale. Chacun a survécu à deux tentatives de réfutation : corrige la cause, avec un test qui aurait échoué avant quand c'est possible. Si un défaut te paraît malgré tout faux, ne le corrige pas et explique-le dans ecartsAuPlan.

${json(confirmed)}

Termine par la vérification ciblée la plus rapide des fichiers touchés.`,
    { label: `correction:${round}`, phase: 'Correction', schema: STEP_REPORT, model: 'sonnet' })
  if (fix) {
    // A fixer may decline a finding (excluded file, disputed defect): only
    // findings in a file it actually changed count as fixed.
    const changed = new Set(fix.fichiersModifies.map(repoRelative))
    fixed.push(...confirmed.filter(c => changed.has(repoRelative(c.fichier))))
    touched = unique([...touched, ...changed])
  }
  gate = await runGate(touched, `après-correction-${round}`)
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
