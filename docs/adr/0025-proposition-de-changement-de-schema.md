# ADR-0025 — Une proposition de changement de schéma est du SQL à relire, jamais une écriture

**Statut :** proposé · **Date :** 2026-09-13

**Précise :** [ADR-0018](0018-apercu-ddl.md), sur ce qu'on a le droit
de faire du DDL une fois qu'il est lu.

## Contexte

La planche `229:7637` porte un contrôle `Propose change…` (`229:7663`, 164 px,
dans la barre de structure, à droite de `Refresh structure` et `Copy DDL`). Le
plan le classait jusqu'ici comme « décision de produit » en attente.

**Ce classement était faux, et la planche le dit elle-même.** Son panneau de
définition (`229:7749`) porte, sous le DDL et au-dessus du bouton
`Open DDL in console`, cette phrase :

> Changes require a SQL review naming commerce-prod before execution.

C'est mot pour mot la garantie de [I-02](../../CLAUDE.md#i-02) : une revue qui
**nomme la connexion**, **avant** exécution. La maquette ne demande donc pas
d'arbitrer entre « appliquer » et « proposer ». Elle a déjà tranché : le contrôle
propose, et l'exécution reste un geste séparé, relu.

Trois décisions antérieures cadrent le reste, et aucune n'est à rouvrir :

* [ADR-0018](0018-apercu-ddl.md) sépare déjà la **lecture** du DDL
  de son exécution, et `Open DDL in console` en est le précédent implémenté :
  du texte part vers la console, rien ne s'exécute ;
* [I-01](../../CLAUDE.md#i-01) interdit un second chemin d'exécution. Un contrôle
  qui appliquerait un DDL directement en créerait un, et ce serait celui-là que
  l'IA emprunterait ;
* [I-10](../../CLAUDE.md#i-10) interdit de concaténer un identifiant **reçu**.
  Un DDL de modification en est fait presque entièrement : nom de schéma, de
  table, de colonne, de contrainte — tous viennent du catalogue, donc du serveur.

La contrainte chiffrée qui rend le dernier point concret : `public.customers`
dans la maquette porte 8 colonnes, et une table nommée
`"users"; DROP TABLE audit; --` est légale dans PostgreSQL.

## Décision

`Propose change…` **compose du texte et l'ouvre dans une console. Il n'exécute
rien, et n'emprunte aucun chemin nouveau.**

Le geste emprunte le chemin que `Open DDL in console` emprunte déjà —
`open_library_query` avec `library::OpenQuery::Copy { text, title, origin,
provenance }` (`workspace/definition.rs`, `open_definition_console`) — et rien d'autre. À partir de là,
la proposition est du SQL utilisateur ordinaire : elle traverse `oxyn-query`
pour sa classification, le `PolicyGate` pour son autorisation, et l'approbation
de production existante si la connexion l'est. **Aucune variante de commande
nouvelle**, aucun contournement du bus.

Seul `origin` change : il dit que le texte est un modèle de changement à
compléter, pas une définition lue.

`provenance` reste `None`, et cette version de l'ADR corrige une première
rédaction qui affirmait le contraire. Le dépôt avait déjà tranché la question
pour le cas jumeau — le modèle de requête liée de `metadata.rs` — avec la raison
qui vaut ici mot pour mot : **un modèle composé par Oxyn n'est écrit ni par
l'utilisateur ni par un agent.** La provenance marque qui a écrit un texte
([ADR-0023](0023-fournisseurs-declares-et-provenance.md)) ; composer un squelette
à compléter n'est pas écrire. Trancher autrement aurait introduit deux
comportements différents pour deux modèles voisins, ce qui est précisément la
sorte d'incohérence qu'un ADR doit éviter plutôt que créer.

Chaque identifiant que la proposition insère est cité par
`oxyn_catalog::quote_identifier` avec `QuoteStyle::for_dialect`, jamais
concaténé. Les **valeurs** — un défaut, une expression de contrainte — ne sont
pas liables dans un DDL : elles sont donc reprises **verbatim depuis le
catalogue**, sans reformatage. Réécrire une expression que le serveur a rendue,
c'est en changer le sens sans le dire.

Une première rédaction ajoutait « ou la proposition est refusée ». Ce refus
n'existait pas dans le code, et il n'a plus lieu d'être : tout le modèle étant
commenté (voir juste en dessous), une expression exotique est recopiée sans
pouvoir rien déclencher.

Pour un `Actor::Agent`, `Propose change…` est **indisponible**, pas
« confirmable ». C'est [I-02](../../CLAUDE.md#i-02) au mot : pour un agent, c'est
un refus, pas une confirmation renforcée. Un agent qui veut un changement de
schéma écrit du SQL dans une console comme n'importe qui, et ce SQL est relu.

Cette indisponibilité tient par **absence de chemin** — aucun outil du registre
n'atteint le geste —, ce qui est plus fort qu'un refus. Mais rien ne la
maintenait vraie : `le_changement_de_schema_n_est_pas_un_outil`
(`oxyn-ai/src/tools.rs`) s'en charge désormais, et échoue le jour où un outil de
structure apparaît.

**Tout le modèle est commenté, ligne par ligne** — pas seulement son en-tête.
C'est la propriété qui rend le geste sûr, et elle mérite d'être dite ici :
`--` ne commente que jusqu'au prochain saut de ligne, et un nom de colonne ou une
expression de défaut peut en contenir. Le corps se compose donc nu, puis chaque
ligne physique reçoit son préfixe. L'utilisateur **décommente** l'instruction
qu'il veut ; rien ne part sur un `Run` distrait.

Le texte composé porte **en plus** un commentaire SQL en tête, qui nomme la
connexion et la relation d'origine. `origin` et `provenance` sont des métadonnées
de l'onglet : elles ne survivent pas à un copier-coller vers un ticket ou un
message, et c'est précisément là que la proposition sera relue trois jours plus
tard, par quelqu'un d'autre.

La portée de la première version est celle que la planche montre et pas
davantage : renommer une colonne, changer sa nullabilité, changer son défaut,
retirer une contrainte nommée. **Pas** de `DROP TABLE`, pas de
changement de type, pas de migration de données — un changement de type réécrit
la table et peut échouer à mi-course sur des données réelles, ce qui est un
sujet de migration, pas de panneau de structure.

## Conséquences

* **+** Aucun chemin d'exécution nouveau : la proposition est du SQL, et tout ce
  qui protège le SQL la protège déjà. I-01 tenu par construction plutôt que par
  vigilance.
* **+** La garantie que la maquette écrit — « a SQL review naming commerce-prod
  before execution » — est tenue littéralement, puisque la revue est celle qui
  existe.
* **+** L'utilisateur voit le SQL exact avant qu'il ne parte. Un panneau qui
  appliquerait un changement en montrant un résumé demanderait de faire
  confiance à la traduction ; ici il n'y a pas de traduction à croire.
* **−** Deux gestes au lieu d'un : proposer, puis exécuter. Sur un renommage de
  colonne évident, cela paraîtra lourd, et cela le sera.
* **−** Le SQL composé peut être **édité** avant exécution, y compris en quelque
  chose que le panneau n'aurait jamais proposé. C'est la conséquence assumée de
  le traiter comme du SQL utilisateur ; l'alternative — un texte verrouillé —
  recréerait le second chemin qu'on refuse.
* **−** La portée restreinte laissera de côté le changement de type, qui est
  précisément ce qu'on demande le plus souvent après un renommage.

**Coût de sortie :** faible tant que la décision tient. Le composeur est une
fonction pure — `proposed_change(cache, path, tab, index, dialect, connection)`,
qui **énumère** les opérations légales pour le dialecte plutôt que d'en recevoir
une : c'est l'utilisateur qui choisit dans le texte. Testable sans base et sans
interface ; en sortir demanderait d'ajouter une commande d'exécution, donc de
rouvrir I-01 — ce qui est le vrai coût, et il est délibérément placé là.

**Reconsidérer si** un utilisateur professionnel rapporte que la double étape le
pousse à écrire son DDL à la main sans relire ce que le panneau proposait : la
protection serait alors contournée par son propre poids, ce qui est pire que de
ne pas l'avoir.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Appliquer le changement depuis le panneau, derrière une confirmation | Crée le second chemin d'exécution qu'[I-01](../../CLAUDE.md#i-01) interdit, et une confirmation finit par être cliquée — c'est le raisonnement d'[I-02](../../CLAUDE.md#i-02) sur les agents, qui vaut aussi pour les humains pressés |
| Ouvrir un formulaire de modification riche (type, contraintes, ordre des colonnes) | La portée réelle d'un tel formulaire est une migration. Elle échoue à mi-course sur des données réelles, et un panneau de structure n'a pas où le dire |
| Composer le DDL côté driver plutôt que dans le cœur | Le driver cite déjà les identifiants ; lui confier aussi la **forme** du changement dupliquerait la logique dans chaque driver, et ADR-0003 veut un driver par protocole, pas un générateur de DDL par produit |
| Verrouiller le SQL proposé pour empêcher son édition | Rendrait la console incohérente avec elle-même — un onglet dont le texte ne s'édite pas — et n'empêcherait rien : l'utilisateur retape le SQL à côté |
| Laisser l'agent proposer, avec une confirmation humaine renforcée | [I-02](../../CLAUDE.md#i-02) est explicite : pour un `Actor::Agent`, c'est un refus. Une confirmation renforcée est exactement ce que l'invariant nomme comme insuffisant |
