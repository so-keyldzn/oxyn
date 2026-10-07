<!-- oxyn-translation source="docs/adr/0049-agents-declared-as-markdown-files.md" sha256="6fe0ce9fed8c" -->

> Traduction française de [docs/adr/0049-agents-declared-as-markdown-files.md](../../../../docs/adr/0049-agents-declared-as-markdown-files.md). **La version anglaise fait foi.**

# ADR-0049 — Un agent est un fichier Markdown à en-tête YAML, composé avec un fragment de dialecte et un fragment de destinataire, rempli seulement par une liste fermée de variables

**Statut :** accepté (2026-10-06) · **Date :** 2026-09-29 · **Amendée le :** 2026-10-06, 2026-10-07 ·
**Décideurs :** Nicolas Boromée

**Précise :** [ARCHITECTURE §7.3](../ARCHITECTURE.md#73-runtime-dagents), sur un
point : la forme que prend un `AgentSpec` sur disque. Qu'un agent soit une
configuration, et non une implémentation, reste inchangé.

> **Amendée le 2026-10-06, avant acceptation.** La première version avait un
> seul axe : un agent, ciblé par dialecte, dont tout le prompt était son
> fichier. Un prompt système dépend désormais de **trois** axes — le **rôle**
> de l'agent, le **dialecte** de la connexion, et le **destinataire** qui le
> lit (un protocole de provider ou un agent externe). L'amendement ajoute le
> champ `recipients` (§ 2), la variable `{{recipient}}` (§ 3), les fichiers
> de fragments de dialecte et de destinataire (§ 8), l'ordre de composition
> fixe (§ 9), la surface Rust et IPC que toute implémentation suit (§ 10),
> ainsi que les conséquences et les alternatives rejetées correspondantes.
> L'ADR étant encore `proposed`, elle est amendée sur place ; rien de la
> première version n'est retiré, sauf la phrase « le corps est le prompt
> système, tel quel », devenue « le corps est la partie **rôle** du prompt
> système ».

> **Amendée le 2026-10-07, après acceptation — l'agent par défaut.** Le
> défaut d'une nouvelle conversation est l'**agent SQL, quel que soit
> l'ordre d'affichage**. Le § 4 trie le sélecteur par `name`, si bien que
> Schema est listé avant SQL ; SQL reste celui sélectionné dans un fil vide,
> et une conversation qui ne nomme aucun agent l'exécute côté backend. La
> clause du § 2 « sinon le premier proposé dans l'ordre du § 4 » est
> retirée : le défaut ne suit jamais l'ordre d'affichage. Le reste des § 2
> et § 4 tient tel qu'écrit.

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

**Ajouté le 2026-10-06.** Un prompt par agent reste un seul prompt pour
tous les lecteurs. Le même texte part vers l'API d'Anthropic, vers un modèle
de 7 milliards de paramètres derrière Ollama (`openai_compatible`), et vers
Claude Code ou Codex par ACP, qui voient les outils d'Oxyn sous leur propre
préfixe MCP et n'ont plus de shell à eux une fois confinés
([ADR-0032](0032-agent-externe-confine-au-lancement.md)). Ce qu'un modèle
doit savoir du dialecte — la citation des identifiants, `LIMIT` contre `TOP`
ou `FETCH`, le fait que `EXPLAIN ANALYZE` exécute l'instruction, le DDL
transactionnel ou non — est le même pour l'agent SQL et l'agent Schema, et
serait sinon recopié dans chaque fichier de rôle, dialecte par dialecte.

## Décision

### 1. Le format : Markdown avec un en-tête YAML

Un agent est un fichier UTF-8, `<nom>.md` :

```markdown
---
id: 0199a3c0-0000-7000-8000-000000000001
name: SQL
description: Writes, fixes and explains queries on the open connection.
applies_to: []
recipients: []
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
* Le **corps**, après le `---` fermant, est la partie **rôle** du prompt
  système : ce à quoi sert l'agent, quelle que soit la base et quel que soit
  le lecteur. Le reste du prompt vient des fragments du § 8, dans l'ordre du
  § 9. Oxyn ne le rend jamais en HTML ; le Markdown n'est qu'une commodité
  pour l'auteur et pour le modèle.
* Le parseur est configuré strict, et cette configuration fait partie de la
  décision : `Budget { max_anchors: 0, max_aliases: 0, .. }`,
  `merge_keys: MergeKeyPolicy::Error`,
  `duplicate_keys: DuplicateKeyPolicy::Error`, `strict_booleans: true`,
  `reject_unsupported_tags: true`, et la fonctionnalité `include` de la
  crate désactivée. Un fichier d'agent n'a aucun usage des références entre
  nœuds, et chacun de ces points est soit un levier d'épuisement des
  ressources, soit une façon pour une clé de dire autre chose que ce qu'on
  lit.
* Un fichier est refusé, avec son nom et la ligne dans le message — jamais
  son contenu au-delà —, quand : il dépasse **64 Kio** (vérifié avant la
  lecture : le plafond de taille de `serde-saphyr` s'applique aux lecteurs,
  pas aux chaînes) ; il n'est pas en UTF-8 valide ; l'en-tête manque ou n'est
  pas la première chose du fichier ; le YAML enfreint un des réglages
  ci-dessus ; `validate()` échoue ; le prompt nomme une variable inconnue
  (§ 3) ; `applies_to` nomme un dialecte inconnu, ou `recipients` un
  destinataire inconnu (§ 2).

### 2. Le ciblage : `applies_to` et `recipients`

`applies_to` est une liste de noms de `SqlDialect`, tels que
`SqlDialect::as_str` les écrit : `ansi`, `postgres`, `mysql`, `sqlite`,
`sqlserver`, `oracle`, `clickhouse`, `duckdb`, `snowflake`, `bigquery`,
`redshift`.

`recipients` est une liste de clés de destinataire, telles que
`Recipient::as_str` les écrit (§ 8) : `anthropic`, `openai`, `gemini`,
`openai_compatible`, `claude-code`, `codex`, `external`.

Pour les deux, une liste vide vaut « tous ». Un agent n'est **proposé** pour
une cible que si les deux listes sont vides ou contiennent le dialecte et le
destinataire de la cible (`AgentSpec::offered_for`). Le sélecteur d'agents
montre les agents proposés ; le défaut est l'agent SQL quand il est proposé,
sinon le premier proposé dans l'ordre du § 4.

Le ciblage restreint ce qui est proposé. Il n'accorde rien : les outils, le
`PolicyGate` et le niveau de la connexion sont les mêmes quel que soit
l'agent.

### 3. La liste fermée des variables

Le prompt — corps de rôle comme fragments — peut contenir des marqueurs
`{{nom}}`, et seulement ceux-ci :

| Variable | Valeur | Source |
|---|---|---|
| `{{dialect}}` | `SqlDialect::as_str` de la connexion | le driver de la connexion |
| `{{driver}}` | le nom du driver | le driver |
| `{{environment}}` | `local`, `development`, `staging` ou `production` | le marquage de la connexion |
| `{{recipient}}` | `Recipient::as_str` de la destination (§ 8) | la destination choisie dans le panneau |

Ces quatre valeurs sont tenues par Oxyn, dans un ensemble fermé ; aucune
n'est écrite par le serveur, par la base ou par le modèle. Un marqueur hors
de cette liste rend le fichier **invalide** ; il n'est jamais remplacé par
une chaîne vide. La liste ne s'étend que par une nouvelle ADR, et seulement
avec une valeur qui n'est ni un contenu de la base, ni une réponse du
serveur, ni un secret. `{{recipient}}` a été ajoutée par l'amendement du
2026-10-06 selon cette règle : elle nomme un protocole ou un preset, jamais
une adresse, un modèle ou une clé.

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
   `id` ne change pas (`0199a3c0-0000-7000-8000-000000000001` et `…0002`),
   puisque le journal d'audit l'enregistre.
2. **Les agents de l'utilisateur** — seconde étape, même format — vivent
   dans `agents/` du répertoire de données (`ProjectDirs::data_dir()`, à côté
   du store local). Ils sont lus au lancement et quand l'utilisateur les
   recharge ; un fichier invalide est listé avec son erreur et n'est pas
   proposé, il ne bloque jamais les autres. Leur `id` ne doit pas entrer en
   collision avec celui d'un agent livré : une collision est refusée, pour
   qu'un fichier ne puisse pas prendre l'identité d'audit d'un agent livré.
   Un fichier utilisateur fournit un **rôle** ; il ne remplace jamais un
   fragment de dialecte ou de destinataire (§ 8).
3. **Les agents de plugins** (phase 4, [PLUGIN-CONTRACT](../PLUGIN-CONTRACT.md))
   utilisent le même format et la même validation ; le manifeste porte le
   fichier.

Ordre dans le sélecteur : livrés, puis utilisateur, puis plugins, chacun par
`name`.

### 5. Ce qu'un fichier ne peut jamais porter

Inchangé depuis `spec.rs`, et désormais tenu par `deny_unknown_fields` : ni
connexion ni session, ni niveau de confidentialité, ni adresse, ni clé, ni
choix de modèle. `tools` restreint le registre ; il n'y ajoute jamais rien.
`recipients` restreint là où un agent est proposé ; il ne choisit jamais la
destination.

### 6. Une conversation, un agent

* Le sélecteur se trouve dans l'en-tête du panneau de l'assistant. L'agent
  se choisit **par conversation** : choisir un autre agent ouvre une
  **nouvelle conversation** ; la conversation en cours reste dans
  l'historique, inchangée. Le **rôle** d'une conversation ne change donc
  jamais sous elle.
* La conversation se souvient de son agent : `ai_conversations` reçoit une
  colonne `agent_id TEXT` nullable (une migration additive
  `ALTER TABLE … ADD COLUMN` d'`oxyn-store`). Une conversation écrite avant
  se lit comme l'agent SQL, qui est celui avec lequel elle a tourné.
* Reprise, une conversation tourne avec l'agent enregistré, rendu à nouveau
  avec la cible actuelle de la connexion. Si cet agent n'existe plus — un
  fichier utilisateur supprimé ou devenu invalide —, la conversation se
  rouvre avec l'agent SQL, et le panneau **le dit**, en nommant l'agent
  manquant. Elle ne tourne jamais en silence sous un autre prompt que celui
  qu'elle affiche.
* Changer de **destination** entre deux questions garde la conversation et
  son agent, et change le fragment de destinataire (§ 9). Cela ne place pas
  un historique sous un prompt avec lequel il n'a pas été écrit : un
  changement de destination n'envoie déjà aucun échange précédent au nouveau
  destinataire
  ([UX-SPEC](../UX-SPEC.md#qui-répond-se-choisit-dans-le-panneau)), qui part
  donc d'un contexte neuf avec son propre prompt rendu. Chaque échange
  enregistre sa destination, si bien que le prompt qui l'a produit se nomme :
  agent × dialecte × destinataire. Une destination pour laquelle l'agent de
  la conversation n'est pas proposé est montrée désactivée, avec sa raison ;
  elle ne change pas l'agent.

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

### 8. Trois axes, trois sortes de fichiers

Chaque fichier ci-dessous est en UTF-8, plafonné à **64 Kio**, embarqué avec
`include_str!`, et vérifié par un test à la compilation : taille, encodage,
et appartenance de chaque marqueur au § 3.

| Axe | Fichier | En-tête | Porte |
|---|---|---|---|
| Rôle | `crates/oxyn-ai/agents/<role>.md` | oui (§ 1) | ce à quoi sert l'agent ; `applies_to`, `recipients`, outils, contexte, tours |
| Dialecte | `crates/oxyn-ai/prompts/dialects/<dialect>.md` | **aucun** | ce qu'un modèle doit savoir pour écrire un SQL correct dans ce dialecte |
| Destinataire | `crates/oxyn-ai/prompts/recipients/<recipient>.md` | **aucun** | comment ce destinataire se conduit dans Oxyn |

**Fragments de dialecte.** Un fichier par clé de `SqlDialect::as_str`. Il
dit : comment les identifiants sont cités ; comment un résultat est borné
(`LIMIT`, `TOP`, `FETCH FIRST`) ; la forme d'`EXPLAIN`, et que la forme qui
analyse (`EXPLAIN ANALYZE` et ses équivalents) **exécute** l'instruction
([I-07](../../CLAUDE.md#i-07)) ; si le DDL est transactionnel ; les vues du
catalogue ; les pièges courants. De vrais fragments sont écrits au moins pour
`ansi`, `postgres`, `redshift`, `mysql`, `sqlite` et `duckdb` — les
dialectes pour lesquels Oxyn a ou prévoit un driver. **Un dialecte sans
fichier utilise `ansi.md`** : le dénominateur commun est un défaut prudent,
un fragment absent laisserait le modèle deviner.

**Fragments de destinataire.** Un fichier par destinataire, et chaque
destinataire a le sien — un fichier manquant fait échouer le test de
compilation, il n'y a pas de repli. Il dit à quoi ressemblent les outils
**tels que ce destinataire les voit**, le format de sortie (blocs `sql`
clôturés, le bloc `erd` de
[UX-SPEC](../UX-SPEC.md#un-bloc-erd-se-dessine-depuis-le-catalogue-pas-depuis-la-réponse)),
et ce que le destinataire ne doit pas tenter :

| Clé | Destinataire | À quoi sert son fragment |
|---|---|---|
| `anthropic` | `AiProviderKind::Anthropic` | les outils d'Oxyn comme appels d'outils natifs |
| `openai` | `AiProviderKind::OpenAi` | de même, pour le *function calling* d'OpenAI |
| `gemini` | `AiProviderKind::Gemini` | de même, pour les *function declarations* de Gemini |
| `openai_compatible` | `AiProviderKind::OpenAiCompatible` | un style **plus court et plus explicite** : Ollama, LM Studio, llama.cpp servent souvent de petits modèles locaux |
| `claude-code` | le preset `claude-code` | les outils d'Oxyn (`describe_schema`, `execute_query`, `request_sample`) sous son propre préfixe MCP ; **aucun outil shell ou fichier à lui**, confiné ([ADR-0032](0032-agent-externe-confine-au-lancement.md)) |
| `codex` | le preset `codex` | de même, pour Codex |
| `external` | un agent externe déclaré à la main | seuls les outils MCP d'Oxyn sont faits pour servir ; il peut avoir des outils shell ou fichier qu'Oxyn ne voit pas, et ne doit pas s'en servir |

Le fragment `external` est une demande, pas une clôture : un agent déclaré à
la main n'est **pas confiné**, et Oxyn ne peut pas faire respecter ce que le
fragment demande (Conséquences).

Les fragments sont **livrés seulement**. Un agent utilisateur ou de plugin
fournit un rôle ; il n'ajoute, ne remplace ni ne retire aucun fragment de
dialecte ou de destinataire. Ce qu'Oxyn dit à tout modèle sur
`EXPLAIN ANALYZE` ou sur ses propres outils est le texte d'Oxyn, relu dans
le dépôt.

### 9. L'ordre de composition est fixe

```text
rendered = role body + "\n\n" + dialect fragment + "\n\n" + recipient fragment
```

puis la substitution du § 3, appliquée une fois à tout le texte. L'ordre est
fixe et rien d'autre n'est concaténé : ni catalogue, ni échantillon, ni
bannière du serveur, ni état de la conversation. Le contenu de la base
n'atteint le modèle que par `ContextBuilder` ([I-04](../../CLAUDE.md#i-04)),
dans un message à part, jamais dans le prompt rendu. Comme la substitution
ne connaît que quatre valeurs tenues par Oxyn, une valeur ne peut pas
introduire un marqueur à elle.

`render_system_prompt` est la **seule** façon de produire un prompt système.
Un `format!` qui en construit un ailleurs est un défaut, comme le serait un
second point de passage.

### 10. La surface que toute implémentation suit

Dans `oxyn-ai` :

```rust
pub enum Recipient { Provider(AiProviderKind), External(ExternalAgentKind) }
pub enum ExternalAgentKind { ClaudeCode, Codex, Other }

pub struct PromptTarget {
    pub dialect: SqlDialect,
    pub driver: DriverId,
    pub environment: Environment,
    pub recipient: Recipient,
}

impl AgentSpec {
    pub fn offered_for(&self, target: &PromptTarget) -> bool;
}

pub fn parse_agent_file(name: &str, text: &str) -> Result<AgentSpec, AgentFileError>;
pub fn render_system_prompt(spec: &AgentSpec, target: &PromptTarget) -> Result<String, AgentFileError>;
pub fn shipped_agents() -> Vec<AgentSpec>;
```

* `Recipient::as_str` donne les clés de fichier du § 8. `ExternalAgentKind`
  se lit depuis l'id du preset (`claude-code`, `codex`) ; un agent externe
  sans preset est `Other`, clé `external`.
* `AgentSpec` reçoit `applies_to: Vec<SqlDialect>` et
  `recipients: Vec<Recipient>`, tous deux `#[serde(default)]`, vides
  signifiant « tous ».
* `AgentFileError` porte le nom du fichier et la ligne, jamais le contenu
  du fichier au-delà.

Entre `oxyn-desktop` et `apps/desktop` :

* `ai_list_agents { connectionId, destination? } -> AgentOption[]`, avec
  `AgentOption = { id, name, description, origin: "shipped" | "user", error: string | null, disabledDestinations }`.
  Seuls les agents proposés pour cette connexion reviennent ; un fichier
  utilisateur invalide revient avec `error` renseigné, et n'est pas
  sélectionnable.
* Démarrer une conversation prend un `agentId` optionnel ; absent, c'est
  l'agent SQL.
* Un résumé de conversation et une transcription portent
  `agentId: string | null` et `missingAgent: { name: string } | null` —
  renseigné quand l'agent enregistré n'existe plus et que l'agent SQL l'a
  remplacé (§ 6).

> **Amendé le 2026-10-06, par l'implémentation de cette section.** Trois
> détails que la première rédaction laissait ouverts :
>
> * `destination` est optionnel, un `DestinationChoice` tel qu'une question
>   le prend. Fourni, la liste est celle des agents proposés pour cette
>   destination. Absent — le panneau demande avant qu'une destination soit
>   choisie —, ce sont les agents proposés pour au moins une destination
>   déclarée.
> * `disabledDestinations: { kind: "provider" | "agent", id: string }[]`
>   nomme les destinations déclarées pour lesquelles un agent n'est pas
>   proposé : le panneau les montre désactivées tant qu'une conversation
>   exécute cet agent (§ 6). Elle est vide pour une entrée en erreur. Une
>   question ou un démarrage d'agent externe vers une telle destination est
>   refusé par le backend avant que rien ne démarre, quoi que montre le
>   panneau.
> * `missingAgent.name` porte l'**id** de l'agent enregistré : le store
>   garde l'id et non le nom, et un fichier disparu ne peut plus dire comment
>   il s'appelait. `agentId` est l'agent que la conversation exécute
>   maintenant — l'id de l'agent SQL après ce repli.

## Conséquences

* **+** Écrire ou ajuster un agent, c'est éditer du texte : ni Rust ni
  recompilation pour un agent utilisateur, un diff d'un fichier pour un
  agent livré.
* **+** Un agent par base devient un fichier par base, proposé seulement là
  où il s'applique.
* **+** Ce que tout modèle doit savoir d'un dialecte s'écrit une fois, dans
  un fragment, et chaque rôle en profite. Ce qu'un destinataire doit savoir
  des outils d'Oxyn s'écrit une fois par destinataire.
* **+** Un petit modèle local derrière `openai_compatible` reçoit un prompt
  écrit pour lui, et un agent externe confiné apprend ce qu'il a et n'a pas,
  au lieu de chercher un shell qu'il n'a plus.
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
  ([SECURITY](../SECURITY.md#surface-dentrée), « Fichiers de workspace ») : un fichier écrit par
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
* **−** Le fragment `external` demande à un agent déclaré à la main de ne pas
  se servir de ses propres outils shell ou fichier ; Oxyn **ne peut pas le
  faire respecter**, puisqu'un tel agent n'est pas confiné
  ([ADR-0032](0032-agent-externe-confine-au-lancement.md)). La mention
  permanente du panneau pour un agent non confiné reste la vérité, pas le
  fragment.
* **−** Le prompt qu'un modèle a lu n'est plus un fichier : c'en est trois,
  et un échange s'explique en nommant l'agent, le dialecte et le
  destinataire. Qui relit un changement de prompt relit la composition, pas
  un fichier.
* **−** Le repli `ansi` n'est juste pour aucun dialecte en particulier : tant
  qu'un dialecte n'a pas son fragment, ses modèles reçoivent le dénominateur
  commun.
* **−** Quatre variables paraîtront peu. Toute demande d'une cinquième passe
  par une ADR, à dessein.

**Coût de sortie :** faible pour les agents livrés — environ une journée pour
remettre les prompts en littéraux Rust ; le type `AgentSpec` ne change pas.
Les fragments ajoutent une journée : leur texte retourne dans les littéraux
de rôle, une copie par rôle. Plus élevé une fois les agents utilisateur en
place : leurs fichiers appartiennent à l'utilisateur, et abandonner le
format demanderait un convertisseur ; la colonne `agent_id` reste, lisible,
quel que soit le format. Ce qui le borne : l'en-tête est la forme serde
d'`AgentSpec` elle-même, donc tout autre format serde (TOML, JSON) lit les
mêmes données.

**À reconsidérer si** un fournisseur demande des réglages par agent qui ne
tiennent pas dans un en-tête plat (sorties structurées, schémas d'outils
écrits par l'auteur de l'agent), si les agents utilisateur servent surtout
à contourner les refus du `PolicyGate` — ce qui plaiderait pour les signer
plutôt que de les lire librement —, ou si un fragment de destinataire se
révèle demander un texte différent pour deux modèles du même protocole, ce
qui plaiderait pour un axe de famille de modèles plutôt qu'un fragment plus
long.

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
| Un fichier complet par rôle × dialecte × destinataire | Combinatoire : 2 rôles livrés × 11 dialectes × 7 destinataires font 154 fichiers, presque tous copies les uns des autres ; une correction de l'avertissement sur `EXPLAIN ANALYZE` se ferait en des dizaines d'endroits, et serait oubliée dans l'un. |
| Du texte propre à un destinataire ou à un dialecte dans le fichier de rôle, derrière des conditions | Demande des conditions dans le prompt, c'est-à-dire le moteur de templates rejeté plus haut ; et chaque rôle porterait sa propre copie de la connaissance de dialecte et de destinataire. |
| Les consignes de destinataire écrites en Rust, dans chaque adaptateur de provider d'`oxyn-llm` | Retour au texte de prompt dans des littéraux, hors de la relecture des prompts, et absent du chemin des agents externes, qui n'a pas d'adaptateur. |
| Un destinataire désigné par nom de modèle plutôt que par protocole ou preset | La liste des modèles est ouverte et change chaque mois ; une clé fermée est ce que la règle du § 3 permet. `openai_compatible` approche « souvent un petit modèle local », et la clause de réexamen couvre le jour où cela ne suffit plus. |
| Aucun fragment de dialecte quand un dialecte n'a pas de fichier | Le modèle n'aurait aucune consigne SQL ; `ansi` est prudent et toujours présent. |
| Des fichiers utilisateur ou de plugin qui remplacent un fragment | Un fichier utilisateur pourrait retirer l'avertissement qu'`EXPLAIN ANALYZE` exécute, ou dire à un agent externe qu'il a un shell. Les fragments sont le texte d'Oxyn ; un corps de rôle peut toujours ajouter ses propres consignes. |
| Concaténer le prompt rendu avec le contexte du schéma | Met du contenu de la base dans le prompt système, hors de la clôture de `ContextBuilder` (I-04) ; le contexte reste un message à part. |
