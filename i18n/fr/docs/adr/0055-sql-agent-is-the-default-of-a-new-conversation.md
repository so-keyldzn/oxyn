<!-- oxyn-translation source="docs/adr/0055-sql-agent-is-the-default-of-a-new-conversation.md" sha256="8f92503ffb59" -->

> Traduction française de [docs/adr/0055-sql-agent-is-the-default-of-a-new-conversation.md](../../../../docs/adr/0055-sql-agent-is-the-default-of-a-new-conversation.md). **La version anglaise fait foi.**

# ADR-0055 — L'agent SQL est le défaut d'une nouvelle conversation, quel que soit l'ordre d'affichage

**Statut :** accepté (2026-10-07) · **Date :** 2026-10-07 ·
**Décideurs :** Nicolas Boromée

**Précise :** [ADR-0049](0049-agents-declared-as-markdown-files.md), § 2 et
§ 4, sur un point : l'agent qu'exécute une nouvelle conversation quand
l'utilisateur n'en choisit aucun. Le ciblage, l'ordre du sélecteur et tout le
reste de l'ADR-0049 restent en vigueur.

## Contexte

L'ADR-0049 § 2 dit : « le défaut est l'agent SQL quand il est proposé, sinon
le premier proposé dans l'ordre du § 4 », et le § 4 ordonne le sélecteur
« livrés, puis utilisateur, puis plugin, chacun par `name` ». Avec les deux
agents livrés, `Schema` se trie avant `SQL`.

Le code livré en 0.0.6 (#200, #203) ne suit pas l'ordre d'affichage :

- le backend exécute l'agent SQL pour une conversation qui ne nomme aucun
  agent (`requested_agent(None)` dans `crates/oxyn-desktop/src/backend/ai/agents.rs`) ;
- le sélecteur (`assistant-agent-picker.tsx`) montre `SQL_AGENT_ID`
  sélectionné quand le fil n'a pas d'`agentId`, tout en listant `Schema` en
  premier.

Une revue Codex sur #200 et #202 a lu le § 2 comme « le premier agent par
nom » et a signalé l'écart. L'utilisateur a décidé le 2026-10-07 que SQL
reste le défaut.

## Décision

1. Une nouvelle conversation, et un fil vide envoyé sans `agentId`,
   exécutent l'**agent SQL** (`sql_agent()`, id
   `0199a3c0-0000-7000-8000-000000000001`), quel que soit l'ordre qu'affiche
   le sélecteur.
2. Le sélecteur montre l'agent SQL **sélectionné** dans un fil vide. La liste
   garde l'ordre de l'ADR-0049 § 4 : ordre d'affichage et défaut sont
   distincts.
3. Le repli « sinon le premier proposé dans l'ordre du § 4 » de l'ADR-0049
   § 2 ne décide plus du défaut : le défaut ne suit jamais l'ordre
   d'affichage. Une conversation qui a enregistré un agent le garde
   (ADR-0049 § 6) ; celle dont l'agent a disparu se replie sur SQL, comme
   avant.

## Conséquences

* **+** Le défaut est le rôle généraliste autour duquel le panneau de
  l'assistant a été construit : écrire, corriger et expliquer des requêtes
  sur la connexion ouverte.
* **+** Ajouter un agent — livré, utilisateur ou plugin — ne change jamais ce
  qu'exécute une nouvelle conversation. Avec un défaut alphabétique, un
  fichier utilisateur nommé `Analytics` prendrait en silence chaque nouveau
  fil.
* **+** Backend et front s'accordent sur une constante, déjà livrée en
  0.0.6 : aucune migration, aucune donnée stockée modifiée.
* **−** L'agent sélectionné n'est pas la première ligne de la liste, ce qui
  peut surprendre un utilisateur qui attend que la première option soit le
  défaut.
* **−** L'agent SQL est traité à part, par son id. Aujourd'hui il est
  proposé pour toute cible (`agents/sql.md` a `applies_to` et `recipients`
  vides, et un fichier utilisateur ne peut pas prendre son id). Si une
  modification ultérieure le restreignait, une nouvelle conversation sur une
  cible pour laquelle il n'est pas proposé serait refusée (`not_offered`) au
  lieu de se replier sur un autre agent.

**Coût de sortie :** faible. Une fonction côté backend (`requested_agent`) et
une sélection dans le sélecteur ; aucun format persistant n'en dépend.

**À reconsidérer si** l'utilisateur peut choisir son propre agent par défaut,
ou si un agent livré autre que SQL devient le rôle principal du panneau.

## Alternatives rejetées

| Alternative | Raison du rejet |
|---|---|
| Le premier agent proposé par nom, comme se lit l'ADR-0049 § 2 | Une question d'affichage déciderait du comportement ; chaque nouvel agent pourrait changer le défaut sans que personne ne l'ait choisi |
| Trier SQL en premier dans le sélecteur pour que « premier proposé » et « défaut » coïncident | Rompt l'ordre de l'ADR-0049 § 4 pour une entrée, et lie encore le défaut à l'affichage |
| Amender l'ADR-0049 sur place | L'ADR-0049 a été acceptée le 2026-10-06 ; une ADR acceptée ne se réécrit pas (`.claude/rules/documentation.md`) |
