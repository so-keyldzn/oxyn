# ADR-0019 — Un contexte de session déclaré, jamais posé en silence

**Statut :** accepté · **Date :** 2026-09-10

**Précise :** [ADR-0015](0015-consoles-independantes.md), sur ce qu'une session
porte en plus de sa transaction.

## Contexte

La barre de console de la maquette (`191:2003`, 260 px) affiche
`commerce-prod / public` : la connexion **et** le schéma dans lequel travaille
la console. Rien dans le produit ne porte aujourd'hui cette seconde moitié.

L'état relevé dans le code :

- le trait `Session` (`crates/oxyn-driver/src/traits.rs:119`) n'a aucune méthode
  de contexte ; `Driver::connect` reçoit un `ConnectionConfig` qui ne porte ni
  schéma ni `search_path` ;
- aucune capacité de `Capabilities` (`crates/oxyn-core/src/capabilities.rs:37`)
  ne décrit un contexte modifiable ; `SCHEMAS` ne dit que « la source expose des
  schémas nommés », ce qui est une capacité d'introspection ;
- `CatalogPath` (`crates/oxyn-catalog/src/path.rs:229`) désigne déjà un schéma
  par son palier `namespace`, mais par nœud du catalogue, jamais comme un état ;
- le driver PostgreSQL ne pose aucun `SET` (`session.rs`), et le driver SQLite
  documente explicitement son refus de tout `PRAGMA` implicite
  (`session.rs:28-35`).

Deux contraintes bornent la solution. D'abord un interdit de contrat :
« modifier l'état de session du serveur sans le déclarer » figure dans le
tableau des refus de [DRIVER-CONTRACT](../DRIVER-CONTRACT.md), parce qu'un
`SET search_path` invisible change le sens des requêtes suivantes de
l'utilisateur. Ensuite l'isolation d'[ADR-0015](0015-consoles-independantes.md) :
chaque console a sa session, mais **le catalogue et l'aperçu de table partagent
la session « préférée »** de la connexion (`crates/oxyn-exec/src/executor.rs`,
`SessionSlot::read_catalog`). Un contexte posé sur cette session-là déplacerait
donc aussi ce que l'explorateur montre.

## Décision

Le contexte de session est une **opération déclarée**, pas un effet de bord.

- Le contexte désigne un emplacement par les deux paliers déjà validés du
  catalogue, `catalog` et `namespace`, sans relation. Il ne porte aucun texte
  SQL. Comme `Command::PreviewRelation`, la commande les transporte en
  `Option<String>` : `oxyn-core` ne dépend pas d'`oxyn-catalog`, et inverser ce
  sens pour un type de commande serait payer une dépendance de crate pour un
  confort de signature ([ARCHITECTURE](../ARCHITECTURE.md#le-sens-des-dépendances)).
  Le type `SessionContext` du contrat vit dans `oxyn-driver`, qui connaît déjà
  `CatalogPath` ; l'exécuteur le construit à la frontière, comme il le fait
  aujourd'hui pour l'aperçu.
- `Command::SetSessionContext { connection, session, catalog, namespace }`
  traverse le `PolicyGate` comme toute autre commande ([I-01](../../CLAUDE.md#i-01)). L'UI
  ne l'applique pas elle-même et n'affiche rien avant la réponse du serveur.
- Sa `StatementIntent` est `Read` : elle n'écrit aucune donnée, et la classer
  `Ddl` la ferait refuser sur une connexion marquée en lecture seule, où changer
  de schéma **pour lire ailleurs** est précisément l'usage. En revanche elle est
  **refusée à `Actor::Agent`**, comme `GRANT` et pour la même raison : un agent
  n'a aucun usage légitime de déplacer le contexte sous les pieds de l'humain,
  et l'effet survit à la commande — le `DELETE` que l'humain écrira ensuite
  frapperait un autre schéma que celui qu'il croit viser
  ([I-07](../../CLAUDE.md#i-07)). C'est un refus, pas une confirmation.
- Le trait `Session` gagne `async fn set_context(&self, context: &SessionContext,
  cancel: &CancelToken) -> Result<()>`, de défaut `NotSupported`, et
  `fn context(&self) -> Option<SessionContext>` qui rend ce que le serveur a
  confirmé — une valeur et non une référence, parce qu'une implémentation garde
  son contexte derrière un verrou : `set_context` prend `&self`. Une capacité `SESSION_CONTEXT` déclare le support ; le sélecteur
  n'existe pas pour un driver qui ne la porte pas
  ([ADR-0003](0003-driver-capabilities.md)).
- Le driver PostgreSQL l'implémente par un `SET search_path` dont l'identifiant
  est **cité par le driver**, jamais concaténé
  ([I-10](../../CLAUDE.md#i-10)). Le geste est visible : il apparaît dans la
  barre d'état et dans l'historique comme l'opération qu'il est.
- Ce `SET` est posé **à chaque exécution, sur la connexion que cette exécution
  emprunte**, et non une fois pour toutes. La raison est mesurable : une session
  PostgreSQL d'Oxyn est un bassin de quatre connexions
  (`MAX_CONNECTIONS`, `drivers/oxyn-driver-postgres/src/options.rs`), et
  `search_path` est un état **par connexion**. Un `SET` posé une fois vaudrait
  pour la connexion qui l'a reçu et pour aucune autre : une requête sur deux
  résoudrait dans un autre schéma, sans rien pour le signaler. C'est le pire
  résultat possible — un contrôle qui a l'air de marcher. `execute` tient déjà
  une seule connexion du début à la fin, ce qui rend l'application déterministe.
  Revenir au défaut du serveur pose de même un `SET search_path TO DEFAULT` :
  ne rien poser laisserait la connexion sur son état précédent.
- Symétriquement, **la connexion revient au bassin dans l'état où elle en est
  sortie** : ce qui a été posé pour une exécution est défait avant de la rendre.
  Sans cela, le contexte d'une console voyagerait avec la connexion vers tout ce
  qui l'emprunte ensuite — et l'introspection en dépend, ce qui n'est pas
  évident : `pg_get_indexdef`, `pg_get_constraintdef`, `pg_get_expr` et
  `format_type` qualifient leur texte **relativement au `search_path`**. Un même
  objet se verrait décrit différemment d'une lecture à l'autre, selon la
  connexion tirée. Une connexion qu'on ne sait pas remettre au défaut est fermée
  plutôt que rendue.
- `set_context` **vérifie l'existence** de l'emplacement avant de le retenir,
  par une requête à valeur liée. PostgreSQL accepte en silence un
  `SET search_path` vers un schéma inexistant ; sans cette vérification, Oxyn
  afficherait un contexte que le serveur n'applique pas.
- Le palier `catalog` est refusé par PostgreSQL quand il désigne une autre base
  que celle de la connexion : une session PostgreSQL ne change pas de base, et
  prétendre le contraire serait le faux contrôle que cet ADR cherche à éviter.
- SQLite ne le déclare pas. Ses schémas (`main`, `temp`, bases attachées) se
  qualifient dans le SQL, et `ATTACH` reste une instruction de l'utilisateur.
- **Le SQL de l'utilisateur n'est jamais réécrit.** Le contexte change ce que le
  serveur résout, pas le texte soumis. Aucune qualification n'est ajoutée à un
  identifiant écrit à la main.
- La session réservée au catalogue et à l'aperçu **n'accepte pas** de changement
  de contexte : l'explorateur montre l'arbre qualifié, pas une vue dépendante
  d'un état. Changer le contexte d'une console ne déplace donc pas le catalogue.

## Conséquences

- **+** Le sens d'un `SELECT` non qualifié devient explicable : une seule
  opération, visible, l'a changé, et la barre dit laquelle.
- **+** Le contexte suit la session, donc la console. Deux onglets de la même
  connexion peuvent travailler dans deux schémas sans se gêner.
- **+** Aucune réécriture de SQL, donc aucune classe de bugs de requalification.
- **−** Une capacité et une méthode de plus dans un contrat que
  [PLUGIN-CONTRACT](../PLUGIN-CONTRACT.md) devra porter en phase 4.
- **−** Le contexte de la console et la portée du catalogue peuvent diverger :
  l'explorateur montre `public`, la console travaille dans `analytics`. C'est un
  écart visible, assumé, et préférable à un catalogue qui bouge sous les pieds
  de l'utilisateur.
- **−** Le changement traverse le réseau et peut échouer. Il a donc les cinq
  états d'une opération distante, y compris l'annulation.
- **−** Un aller-retour de plus par exécution tant qu'un contexte est déclaré,
  sur une session PostgreSQL. C'est le prix du bassin de connexions. Si une
  mesure montre que ce coût pèse, l'alternative est de porter `search_path`
  comme option de connexion et de recréer le bassin au changement — plus rapide,
  mais incapable de préserver une transaction ouverte.
- **−** Après toute exécution inscriptible, le driver PostgreSQL remet `search_path` au défaut et `standard_conforming_strings` à `on`, qu'il impose aussi à l'ouverture (valeur que suppose le découpeur) : un `SET` tapé en console ne suit pas la connexion rendue au bassin, et le schéma d'une console ne passe que par ce contexte.

**Coût de sortie :** retirer une commande, une capacité et deux méthodes de
trait, plus le sélecteur. Rien n'est persisté dans un format de workspace tant
que le contexte n'est pas mémorisé entre lancements, ce que cet ADR ne décide
pas — cela borne le coût à du code.

**Reconsidérer si** un driver ne sait exprimer son contexte que par une
requalification du texte, ou si les utilisateurs demandent que le catalogue
suive le contexte de la console active. Le second cas est un choix d'interface,
et se tranchera sans rouvrir le contrat.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Poser un `SET search_path` à l'ouverture de chaque session, sans le dire | Exactement l'état de session invisible que le contrat de driver refuse |
| Qualifier les identifiants du SQL de l'utilisateur avant l'envoi | Demande d'analyser puis de réécrire du SQL arbitraire ; une requalification fausse exécute autre chose que ce qui est écrit |
| Faire du contexte une propriété de la **connexion** | Deux consoles de la même connexion se le partageraient : changer de schéma dans un onglet déplacerait l'autre sans prévenir |
| N'utiliser le sélecteur que pour filtrer le catalogue | Ne répond pas à la question posée par la maquette : ce que résout un `SELECT` non qualifié |
| Ouvrir une session neuve à chaque changement de contexte | Perd la transaction et le travail en cours de la console, pour une opération que l'utilisateur croit anodine |
