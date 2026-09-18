# Le socle de pilotage

`docs/` fait autorité sur **le domaine**. `.claude/` porte **la manière de
travailler**. La carte du dépôt et les invariants sont dans
[CLAUDE.md](../CLAUDE.md).

Pour Codex, l'entrée est [AGENTS.md](../AGENTS.md) et les compétences locales
sont décrites dans [.agents/README.md](../.agents/README.md). Les procédures,
règles, profils, gabarits et listes de contrôle restent partagés ici ; les
hooks et permissions de `settings.json` restent propres à Claude Code.

## Les quatre supports, et pourquoi on ne les confond pas

| Support | Nature | Quand |
|---|---|---|
| `docs/` | **fait autorité** — la vérité sur le domaine | quand la contradiction code/doc est un bug |
| `CLAUDE.md` | **carte + invariants**, chargé à chaque session, donc court | quand ça vaut pour tout le dépôt, tout le temps |
| `.claude/rules/` | **conventions**, chargées par `paths:` à la lecture d'un fichier | quand ça ne vaut que pour un répertoire |
| `.claude/hooks/` | **exécuté**, pas lu — un refus, pas un rappel | quand la violation est silencieuse |

La question de tri, pour chaque règle qu'on écrit : **si Claude l'ignore, est-ce
que ça se voit ?** Visible à l'exécution → une ligne de règle suffit. Visible
seulement en production, ou jamais → un hook. Fait sur le domaine → `docs/`.

**Une règle vit à un seul endroit.** Les autres fichiers y renvoient. C'est la
seule chose qui empêche ce socle de pourrir : une règle en trois exemplaires
diverge en deux semaines, et plus personne ne sait laquelle fait foi.

## Commandes

Les gestes qui ont un contrat à respecter. Elles chargent la procédure
explicitement — c'est ce qui **compense l'inertie des règles** à la création
d'un fichier.

| Commande | Objet |
|---|---|
| [`/plan`](commands/plan.md) | trancher avant d'écrire |
| [`/implementer`](commands/implementer.md) | implémenter un changement |
| [`/relire`](commands/relire.md) | relire contre les invariants |
| [`/driver`](commands/driver.md) | implémenter un driver |
| [`/commande`](commands/commande.md) | ajouter une commande au bus |
| [`/ecran`](commands/ecran.md) | ajouter un écran à l'interface Tauri |
| [`/adr`](commands/adr.md) | écrire une décision |
| [`/versions`](commands/versions.md) | re-vérifier les versions externes |
| [`/benchmark`](commands/benchmark.md) | mesurer avant d'optimiser |
| [`/securite`](commands/securite.md) | relecture de sécurité |

## Agents

Deux familles, et la distinction est structurante.

**Ceux qui écrivent** (`memory: project`), un par grand domaine. Ils *invoquent
les commandes* au lieu de redire les invariants, pour qu'une règle corrigée à un
seul endroit profite partout.

| Agent | Domaine |
|---|---|
| [`architecte`](agents/architecte.md) | découpage, traits de frontière, ADR |
| [`rustacien`](agents/rustacien.md) | le cœur : tout ce qui n'est ni driver, ni interface, ni IA |
| [`driveriste`](agents/driveriste.md) | `drivers/oxyn-driver-*`, traits d'`oxyn-driver` |
| [`frontiste`](agents/frontiste.md) | `apps/desktop`, `oxyn-desktop` — tout nouvel écran |
| [`ia-workspace`](agents/ia-workspace.md) | `oxyn-ai` |
| [`documentaliste`](agents/documentaliste.md) | `docs/` |
| [`performance`](agents/performance.md) | mesures et optimisation |

**Ceux qui relisent** — lecture seule, **sans `memory:`**.

| Agent | Ce qu'il cherche |
|---|---|
| [`relecteur-invariants`](agents/relecteur-invariants.md) | les treize invariants |
| [`relecteur-securite`](agents/relecteur-securite.md) | secrets, `unsafe`, surface d'entrée |
| [`relecteur-frontiere`](agents/relecteur-frontiere.md) | les quatre frontières externes |
| [`detecteur-divergence`](agents/detecteur-divergence.md) | code contre `docs/` |

> **Pourquoi aucun relecteur ne porte `memory:`.** La clé active automatiquement
> `Read`, `Write` et `Edit` : elle retirerait à un relecteur sa lecture seule —
> précisément ce qui rend son verdict crédible. `make socle` ne vérifie pas ce
> point ; il se relit à la main quand un agent est ajouté.

> **La mémoire d'un agent porte des pièges d'outillage, jamais des faits sur le
> projet.** Ceux-là appartiennent aux documents d'autorité. Une mémoire qui se
> met à raconter le projet devient une source de vérité concurrente.

## Règles

Huit, à chargement conditionnel. Le tableau de leurs `paths:` est dans
[CLAUDE.md](../CLAUDE.md#règles-chargées-à-la-demande).

> Une règle `paths:` se charge quand Claude *lit* un fichier correspondant, pas
> quand il en crée un : le premier fichier d'un répertoire neuf s'écrit sans
> elle. Ce sont les commandes qui compensent — **`/driver`, `/commande`,
> `/ecran` lisent la règle explicitement**. `make socle` signale en
> avertissement une règle dont aucun fichier ne correspond.

## Hooks

Ce que `CLAUDE.md` ne peut que demander, un hook l'impose. Le protocole et les
quatre faits à ne pas re-découvrir sont dans [hooks/README.md](hooks/README.md).

## Listes de contrôle et gabarits

`checklists/` : [driver](checklists/revue-driver.md) ·
[sécurité](checklists/revue-securite.md) · [interface](checklists/revue-ui.md) ·
[fin de tâche](checklists/fin-de-tache.md).

`templates/` : [ADR](templates/adr.md) ·
[rapport de mesure](templates/rapport-benchmark.md) ·
[rapport de relecture](templates/rapport-relecture.md).

`workflows/` : [l'enchaînement des gestes](workflows/README.md).

## Vérifier le socle lui-même

```bash
make socle
```

Liens morts, règles sans `paths:`, invariants orphelins, hooks non exécutables,
et les tests des hooks. Le socle a un mode de panne propre : il se dégrade en
silence, et devient décoratif sans que personne ne le remarque.

## Étendre

**Ajouter un invariant** — seulement s'il réunit les trois traits : violation
silencieuse, coûteuse, et tentante. Si le scénario de panne concret ne s'écrit
pas, ce n'en est pas un. Il porte une ancre `<a id="i-NN"></a>` et est cité par
au moins un document, sinon `make socle` le signale comme orphelin.

**Ajouter un motif de hook** — avec son cas nominal **et** son faux positif dans
`test_hooks.py`. Un faux positif bloque le travail à chaque tour et le hook finit
désactivé, emportant les vrais positifs.

**Ajouter une règle** — avec un `paths:`, sinon elle se charge à chaque session
comme `CLAUDE.md` et ruine le budget de contexte.

**Ajouter un agent** — une description qui dit *quand le lancer*, c'est elle qui
le fait choisir. S'il relit, pas de `memory:`.
