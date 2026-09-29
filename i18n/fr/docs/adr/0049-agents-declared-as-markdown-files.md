<!-- oxyn-translation source="docs/adr/0049-agents-declared-as-markdown-files.md" sha256="42aedacc905a" -->

> Traduction française de [docs/adr/0049-agents-declared-as-markdown-files.md](../../../../docs/adr/0049-agents-declared-as-markdown-files.md). **La version anglaise fait foi.**

# ADR-0049 — Un agent est un fichier Markdown à en-tête YAML, ciblé par dialecte, rempli seulement par une liste fermée de variables

**Statut :** proposé · **Date :** 2026-09-29 · **Décideurs :** Nicolas Boromée

**Précise :** [ARCHITECTURE §7.3](../ARCHITECTURE.md#73-runtime-dagents), sur un
point : la forme que prend un `AgentSpec` sur disque. Qu'un agent soit une
configuration, et non une implémentation, reste inchangé.

## Contexte

ARCHITECTURE §7.3 dit qu'un agent est une configuration : un `AgentSpec`,
avec un prompt système, un sous-ensemble d'outils, une politique de contexte
et une limite de tours. Elle dit qu'en ajouter un ne demande aucun code Rust.
Le code ne le suit pas encore :

* les deux agents livrés, `sql_agent` et `schema_agent`
  (`crates/oxyn-ai/src/builtin.rs`), sont des **fonctions Rust** dont les
  prompts sont des littéraux `concat!` d'une trentaine de lignes chacun.
  Changer un mot d'un prompt, c'est une modification Rust, une recompilation
  et la relecture d'un fichier `.rs` ;
* la conversation utilise toujours `sql_agent()`, en dur
  (`crates/oxyn-desktop/src/backend/ai/conversation.rs`). `schema_agent` est
  déclaré, testé, et jamais proposé ;
* `AgentSpec` n'a aucun champ qui dise pour quelles bases un agent est
  écrit. Un agent qui connaît PostgreSQL (`EXPLAIN (ANALYZE, BUFFERS)`,
  `pg_stat_statements`, `LATERAL`) et un agent qui connaît SQLite
  (`EXPLAIN QUERY PLAN`, pas d'`ALTER COLUMN`) ne peuvent être que le même ;
* sept agents de la vision restent à écrire (`REMAINING_AGENTS`), et des
  agents par base sont demandés. Écrits en fonctions Rust, chacun est une
  modification de code pour ce qui est du texte.

`AgentSpec` est déjà `Serialize + Deserialize`, et déjà validé comme une
entrée non fiable (`AgentSpec::validate` : outil inconnu refusé, tours
plafonnés à `MAX_TURNS_CEILING` = 64). Ce qui manque : un format de fichier,
un champ de ciblage, et un endroit où lire les fichiers.

## Décision

### 1. Le format : Markdown avec un en-tête YAML

Un agent est un fichier UTF-8, `<nom>.md` :

```markdown
---
id: 0199a3c0-0000-7000-8000-000000000001
name: SQL
description: Writes, fixes and explains queries on the open connection.
applies_to: []
tools: [execute_query, describe_schema, request_sample]
max_turns: 8
context:
  max_relations: 30
---
You help a data professional write and fix queries against the
{{dialect}} database they have open. …
```

* L'**en-tête** porte tous les champs d'`AgentSpec` sauf le prompt, sous les
  mêmes noms ; `tools` est `allowed_tools`. Il est lu avec `serde-saphyr`
  1.3.0 (MIT OR Apache-2.0, MSRV 1.89, vérifié le 2026-09-29 —
  [RESEARCH-NOTES](../RESEARCH-NOTES.md#lecture-du-yaml-des-fichiers-dagents--vérifié-le-2026-09-29)),
  avec `deny_unknown_fields` sur la structure de l'en-tête : une clé mal
  orthographiée est une erreur, pas un réglage ignoré en silence.
* Le **corps**, après le `---` fermant, est le prompt système, tel quel.
  Oxyn ne le rend jamais en HTML ; le Markdown n'est qu'une commodité pour
  l'auteur et pour le modèle.
* Le parseur est configuré strict, et cette configuration fait partie de la
  décision : `Budget { max_anchors: 0, max_aliases: 0, .. }`,
  `merge_keys: MergeKeyPolicy::Error`,
  `duplicate_keys: DuplicateKeyPolicy::Error`, `strict_booleans: true`,
  `reject_unsupported_tags: true`, et la fonctionnalité `include` de la
  crate désactivée. Un fichier d'agent n'a aucun usage des références entre
  nœuds, et chacun de ces points est soit un levier d'épuisement des
  ressources, soit une façon pour une clé de dire autre chose que ce qu'on
  lit.
* Un fichier est refusé, avec son nom et la ligne dans le message, quand :
  il dépasse **64 Kio** (vérifié avant la lecture : le plafond de taille de
  `serde-saphyr` s'applique aux lecteurs, pas aux chaînes) ; l'en-tête
  manque ou n'est pas la première chose du fichier ; le YAML enfreint un des
  réglages ci-dessus ; `validate()` échoue ; le prompt nomme une variable
  inconnue (§ 3) ; `applies_to` nomme un dialecte inconnu (§ 2).

### 2. Le ciblage : `applies_to`

`applies_to` est une liste de noms de `SqlDialect`, tels que
`SqlDialect::as_str` les écrit (`postgres`, `sqlite`, `mysql`, `duckdb`…).
Une liste vide vaut pour toute connexion. Un agent n'est **proposé** pour une
connexion que si la liste est vide ou contient le dialecte de la connexion.
Le sélecteur d'agents montre les agents proposés ; le défaut est le premier
proposé, dans l'ordre du § 4.

Le ciblage restreint ce qui est proposé. Il n'accorde rien : les outils, le
`PolicyGate` et le niveau de la connexion sont les mêmes quel que soit
l'agent.

### 3. La liste fermée des variables

Le prompt peut contenir des marqueurs `{{nom}}`, et seulement ceux-ci :

| Variable | Valeur | Source |
|---|---|---|
| `{{dialect}}` | `SqlDialect::as_str` de la connexion | le driver de la connexion |
| `{{driver}}` | le nom du driver | le driver |
| `{{environment}}` | `local`, `development`, `staging` ou `production` | le marquage de la connexion |

Ces trois valeurs sont tenues par Oxyn, dans un ensemble fermé ; aucune n'est
écrite par le serveur ou par la base. Un marqueur hors de cette liste rend le
fichier **invalide** ; il n'est jamais remplacé par une chaîne vide. La liste
ne s'étend que par une nouvelle ADR, et seulement avec une valeur qui n'est
ni un contenu de la base, ni une réponse du serveur, ni un secret.

**Le contenu de la base n'est jamais une variable** : pas de `{{tables}}`,
`{{schema}}`, `{{comments}}`, `{{sample}}`. La structure et les échantillons
n'atteignent le modèle que par `ContextBuilder`, qui applique le niveau de la
connexion et clôture ce que la base a écrit
([I-04](../../CLAUDE.md#i-04), [AI-PROVIDERS](../AI-PROVIDERS.md)). Une
variable de template serait un second chemin, non clôturé, vers le prompt, et
un commentaire de colonne y arriverait comme une instruction du prompt
système. Le produit et la version du serveur ne sont pas non plus une
variable, pour la même raison : c'est une réponse du serveur, et
`ContextBuilder` les envoie déjà, bornés, quand la `ContextPolicy` de l'agent
a `include_server_info`.

### 4. D'où viennent les agents

1. **Les agents livrés** vivent dans `crates/oxyn-ai/agents/*.md` et sont
   embarqués avec `include_str!`. Un test les lit et les valide à la
   compilation (`shipped_agents_are_valid`, qui existe déjà) : un fichier
   livré cassé n'atteint jamais un utilisateur. `sql_agent()` et
   `schema_agent()` deviennent de simples lecteurs de leur fichier ; leur
   `id` ne change pas, puisque le journal d'audit l'enregistre.
2. **Les agents de l'utilisateur** — seconde étape, même format — vivent
   dans `agents/` du répertoire de données (`ProjectDirs::data_dir()`, à côté
   du store local). Ils sont lus au lancement et quand l'utilisateur les
   recharge ; un fichier invalide est listé avec son erreur et n'est pas
   proposé, il ne bloque jamais les autres. Leur `id` ne doit pas entrer en
   collision avec celui d'un agent livré : une collision est refusée, pour
   qu'un fichier ne puisse pas prendre l'identité d'audit d'un agent livré.
3. **Les agents de plugins** (phase 4, [PLUGIN-CONTRACT](../PLUGIN-CONTRACT.md))
   utilisent le même format et la même validation ; le manifeste porte le
   fichier.

Ordre dans le sélecteur : livrés, puis utilisateur, puis plugins, chacun par
`name`.

### 5. Ce qu'un fichier ne peut jamais porter

Inchangé depuis `spec.rs`, et désormais tenu par `deny_unknown_fields` : ni
connexion ni session, ni niveau de confidentialité, ni adresse, ni clé, ni
choix de modèle. `tools` restreint le registre ; il n'y ajoute jamais rien.

### 6. Une conversation, un agent

* Le sélecteur se trouve dans l'en-tête du panneau de l'assistant. L'agent
  se choisit **par conversation** : choisir un autre agent ouvre une
  **nouvelle conversation** ; la conversation en cours reste dans
  l'historique, inchangée. Le prompt système d'une conversation ne change
  donc jamais sous elle.
* La conversation se souvient de son agent : `ai_conversations` reçoit une
  colonne `agent_id` nullable (une migration `ALTER TABLE … ADD COLUMN`
  d'`oxyn-store`). Une conversation écrite avant se lit comme l'agent SQL,
  qui est celui avec lequel elle a tourné.
* Reprise, une conversation tourne avec l'agent enregistré, rendu à nouveau
  depuis son fichier avec les variables actuelles de la connexion. Si cet
  agent n'existe plus — un fichier utilisateur supprimé ou devenu invalide —,
  la conversation se rouvre avec l'agent SQL, et le panneau **le dit**, en
  nommant l'agent manquant. Elle ne tourne jamais en silence sous un autre
  prompt que celui qu'elle affiche.

### 7. Les agents externes reçoivent aussi le prompt de l'agent

L'agent choisi s'applique aux deux destinations, provider et agent externe
([ADR-0026](0026-agents-externes-acp.md)) :

* **Provider :** le prompt rendu est le message système, comme aujourd'hui.
* **Agent externe :** ACP n'a pas de message système ; l'agent a le sien. Le
  prompt rendu est placé dans le texte d'**ouverture** de la session, par
  `AgentPrompt::with_schema` — le seul constructeur qui en ouvre une —, comme
  un bloc écrit par Oxyn avant le schéma, jamais mêlé à lui. Les questions
  suivantes ne le répètent pas : le processus se souvient
  ([ADR-0027](0027-porte-unique-pour-les-deux-destinations.md)). Aucun
  nouveau constructeur public d'`AgentPrompt` n'est ajouté.
* `tools` restreint les **outils MCP** qu'Oxyn expose à cette session
  ([ADR-0030](0030-outils-oxyn-exposes-a-un-agent-externe.md)) exactement
  comme il restreint la liste d'outils du provider. `context` fixe la
  `ContextPolicy` du schéma d'ouverture. `max_turns` ne s'applique pas : un
  agent externe mène sa propre boucle, et Oxyn la borne par ses budgets
  existants, pas par ce champ.
* Le prompt atteint l'agent externe comme des **instructions dans un message
  utilisateur**, sous le prompt système propre à l'agent. Il y pèse moins que
  sur un provider, et le sélecteur ne peut pas promettre davantage : le
  confinement de l'agent externe
  ([ADR-0032](0032-agent-externe-confine-au-lancement.md)) et le
  `PolicyGate` restent les garanties, pas le prompt.

## Conséquences

* **+** Écrire ou ajuster un agent, c'est éditer du texte : ni Rust ni
  recompilation pour un agent utilisateur, un diff d'un fichier pour un
  agent livré.
* **+** Un agent par base devient un fichier par base, proposé seulement là
  où il s'applique.
* **+** Un agent utilisateur est lisible et transportable sans Oxyn
  ([I-11](../../CLAUDE.md#i-11)).
* **+** Les sept agents restants et les agents de plugins arrivent dans un
  format qui existe déjà, avec la même validation.
* **−** Une dépendance de plus, `serde-saphyr`, et la surface du YAML : le
  typage implicite (par défaut, `serde-saphyr` lit `no`, `off` comme `false`
  dans un champ booléen — d'où `strict_booleans`), les erreurs
  d'indentation. `deny_unknown_fields` et les champs typés en rattrapent
  l'essentiel ; le reste apparaît comme une erreur de validation.
* **−** Les agents utilisateur sont une **nouvelle surface d'entrée**
  ([SECURITY](../SECURITY.md), « Workspace files ») : un fichier écrit par
  quelqu'un d'autre peut porter un prompt qui pousse le modèle à mal se
  conduire. Il ne peut obtenir ni un outil, ni un niveau, ni une écriture que
  le `PolicyGate` refuserait, mais il peut rendre le modèle insistant. Le
  sélecteur signale donc les agents utilisateur comme tels, et l'étape qui
  les lit demande une relecture sécurité avant sa fusion.
* **−** Une erreur de prompt dans un fichier livré est trouvée par un test,
  pas par le compilateur : les littéraux `concat!` étaient au moins vérifiés
  comme chaînes.
* **−** Une colonne persistée de plus, `ai_conversations.agent_id` : une
  conversation dépend désormais d'un fichier qui peut disparaître, d'où le
  repli du § 6.
* **−** Sur un agent externe, le prompt de l'agent est un conseil que
  l'agent externe peut mettre en balance avec son propre prompt système. Le
  même `.md` peut s'y montrer plus lâche que sur un provider.
* **−** Trois variables paraîtront peu. Toute demande d'une quatrième passe
  par une ADR, à dessein.

**Coût de sortie :** faible pour les agents livrés — environ une journée pour
remettre les prompts en littéraux Rust ; le type `AgentSpec` ne change pas.
Plus élevé une fois les agents utilisateur en place : leurs fichiers
appartiennent à l'utilisateur, et abandonner le format demanderait un
convertisseur ; la colonne `agent_id` reste, lisible, quel que soit le
format. Ce qui le borne : l'en-tête est la forme serde d'`AgentSpec`
elle-même, donc tout autre format serde (TOML, JSON) lit les mêmes données.

**À reconsidérer si** un fournisseur demande des réglages par agent qui ne
tiennent pas dans un en-tête plat (sorties structurées, schémas d'outils
écrits par l'auteur de l'agent), ou si les agents utilisateur servent surtout
à contourner les refus du `PolicyGate` — ce qui plaiderait pour les signer
plutôt que de les lire librement.

## Alternatives rejetées

| Alternative | Raison du rejet |
|---|---|
| Garder les agents en fonctions Rust | Contredit ARCHITECTURE §7.3 ; chaque changement de prompt est un changement de code, et les agents par base le multiplient. |
| En-tête TOML (`+++`), avec la crate `toml` déjà dans le workspace | Aucune dépendance nouvelle, et le prompt reste le corps dans les deux cas ; mais l'en-tête YAML est la convention que les auteurs d'agents et de skills connaissent déjà, et un fichier dans une autre convention est une chose de plus à apprendre sans rien gagner dans ce qu'il peut dire. Gardé comme repli si `serde-saphyr` est abandonnée : les données sont les mêmes. |
| Un moteur de templates complet (`minijinja`, `tera`) | Des conditions et des boucles dans un prompt le rendent intestable fichier par fichier, et un moteur capable de lire n'importe quelle valeur invite à lui passer du contenu de la base — le chemin que le § 3 interdit. |
| Des variables ouvertes remplies depuis le catalogue (`{{tables}}`) | Contourne `ContextBuilder`, donc le niveau et la clôture (I-04) ; un commentaire de la base devient une instruction du prompt système. |
| Une variable `{{server_version}}` | Une réponse du serveur dans le prompt système, hors de la clôture ; le contexte la porte déjà, bornée (`include_server_info`). |
| Un fichier JSON par agent | Lisible sans Oxyn, mais un prompt de trente lignes en une chaîne JSON échappée est illisible et impossible à relire. |
| Changer d'agent dans une conversation en cours | Le prompt système changerait sous un historique existant, et un tour ne pourrait plus être rattaché au prompt qui l'a produit. |
| Ne pas persister l'agent d'une conversation | Une conversation reprise tournerait en silence sous l'agent sélectionné à ce moment, avec un historique écrit sous un autre prompt. |
| Des agents pour les providers seulement | Laisse le chemin des agents externes sans la connaissance par base qui est l'objet de la fonctionnalité ; le texte d'ouverture porte déjà des instructions écrites par Oxyn, le prompt y a sa place. |
| `applies_to` sur le nom du driver plutôt que le dialecte | Redshift parle le dialecte PostgreSQL par le driver PostgreSQL ([ADR-0003](0003-driver-capabilities.md)) ; c'est pour le dialecte que le prompt est écrit. `{{driver}}` reste disponible dans le texte. |
