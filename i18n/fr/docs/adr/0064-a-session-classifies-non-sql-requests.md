<!-- oxyn-translation source="docs/adr/0064-a-session-classifies-non-sql-requests.md" sha256="b29861e84768" -->

> Traduction française de [docs/adr/0064-a-session-classifies-non-sql-requests.md](../../../../docs/adr/0064-a-session-classifies-non-sql-requests.md). **La version anglaise fait foi.**

# ADR-0064 — Une session classe les requêtes d'un langage non SQL

**Statut :** accepté · **Date :** 2026-10-22

**Précise :** [ADR-0003](0003-driver-capabilities.md), sur le point suivant :
« les requêtes portent un `QueryLanguage` explicite ; le SQL n'est qu'un cas
parmi d'autres » vaut aussi pour la classification qui garde la production.
ADR-0003 reste en vigueur tel qu'il est écrit.

## Contexte

`oxyn-exec` remplace l'intent qu'un appelant déclare par celui qui se lit dans
le texte, avant que le `PolicyGate` décide
([ARCHITECTURE §8](../ARCHITECTURE.md#8-sécurité-et-garde-fous)). Cette lecture
est `oxyn_query::reclassify`, appelée à deux endroits : `reclassified()` pour
chaque `Command::Execute`, et le chemin d'aperçu pour la requête qu'un driver
compose pour `Command::PreviewRelation` (`crates/oxyn-exec/src/executor.rs`).

`oxyn-query` n'analyse que le SQL. Pour tout autre `QueryLanguage` —
`RedisCommand`, `MongoQuery`, `SearchDsl` et les autres —
`classify_language` renvoie `Unknown`, qui compte comme mutant. Trois drivers
s'écrivent en parallèle pour ces langages (Redis, MongoDB, Elasticsearch), et
avec le code d'aujourd'hui chacun obtient, sur une connexion de production ou
sans environnement :

* une confirmation nominative demandée à l'humain pour un simple `GET` ;
* un refus pour toute requête d'agent, lectures comprises (I-02, I-07) ;
* tout aperçu refusé, quel que soit l'acteur et l'environnement — et avec un
  message qui accuse la syntaxe du filtre de l'aperçu, puisque le chemin
  d'aperçu lit une requête `Unknown` comme une faute de frappe dans le filtre.

Un driver pour ces moteurs est donc inutilisable en production et ne peut
offrir aucun aperçu. Ce qui distingue une lecture d'une écriture dans ces
langages, c'est le moteur, ou le driver qui parle son protocole : une table de
commandes lue une fois à la connexion, par exemple, qu'aucune lecture du texte
depuis l'extérieur du driver ne peut remplacer.

## Décision

`trait Session` (`crates/oxyn-driver/src/traits.rs`) gagne une méthode
synchrone avec un corps par défaut :

```rust
fn classify_intent(&self, request: &ExecRequest) -> Option<StatementIntent> {
    let _ = request;
    None
}
```

* `oxyn-exec` la consulte **seulement** quand `request.language.sql_dialect()`
  vaut `None`. `oxyn-query` reste la seule autorité sur le SQL : une requête SQL
  n'atteint jamais la session.
* Elle est consultée après `oxyn_query::reclassify`, aux deux endroits : dans
  `reclassified()` par le registre des sessions — seulement si la session
  appartient à la connexion de la commande — et dans le chemin d'aperçu sur le
  slot déjà en main. `SessionSlot::classify_intent` prend le verrou du slot avec
  `try_read` et répond `None` si la session se ferme ou si le verrou est tenu :
  rien n'attend sur le chemin du dispatch (I-05).
* `Some(intent)` remplace `Unknown`, avec un risque `None`,
  `transaction_control` et `context_change` à faux. Un driver dont le langage a
  de telles instructions les refuse dans sa console, ou fait passer un
  changement de contexte par `set_context`.
* `None` garde `Unknown`. C'est le défaut : postgres, mysql, sqlite et tout
  driver tiers compilent et se comportent comme avant. Le changement
  d'`oxyn-driver`, la crate Apache-2.0 que les tiers lient
  ([ADR-0044](0044-licence-gpl-et-contrat-apache.md)), n'est pas cassant.
* La méthode ne fait aucune E/S et ne prend aucun verrou qui puisse bloquer.
  Elle répond depuis ce que la session a appris à la connexion.
* Elle est **prudente** : `Some(Read)` seulement pour une requête prouvée en
  lecture seule ; `None` dès qu'il y a un doute.

La règle pour les implémentations vit dans
[DRIVER-CONTRACT §5](../DRIVER-CONTRACT.md#5-il-déclare-ses-capacités-par-session-et-ne-simule-rien).
Une lecture classée ainsi suit toutes les règles d'une lecture SQL : en
production, ou sur une connexion marquée en lecture seule, elle part avec
`limits.read_only` positionné, et la session tient cette borne ou refuse.

## Conséquences

* **+** Un driver non SQL peut servir des lectures en production sans une
  confirmation par `GET`, un agent peut y lire, et les aperçus peuvent
  s'exécuter.
* **+** Aucun driver existant ne change : le défaut est l'ancien comportement,
  et le SQL prend exactement le chemin qu'il prenait.
* **+** La décision reste dans l'exécuteur et le gate : la session ne fournit
  qu'un fait, et le journal enregistre l'intent que le gate a vu.
* **−** La garde de production d'un langage non SQL repose désormais sur la
  parole d'un driver. Un driver qui revendique `Read` à tort — une commande
  qu'il ne connaît pas classée en lecture, un `FLUSHALL` pris pour un `GET` —
  contourne la confirmation nominative et le refus de l'agent, sans qu'aucun
  test d'`oxyn-exec` puisse le remarquer. Le contrat le dit en clair, et chaque
  implémentation doit prouver ses réponses `Read` contre la propre table de
  commandes du moteur.
* **−** La réponse ne porte pas de risque : une écriture non SQL n'est jamais
  signalée comme une suppression non bornée, seulement comme une écriture. La
  confirmation nomme toujours la connexion ; elle ne dit pas combien est touché.
* **−** Une session qui se ferme, ou dont une fermeture tient le verrou, répond
  `None` et envoie une lecture en confirmation. C'est le côté prudent, et cela
  n'arrive que pendant la fermeture de la console.

**Coût de sortie :** faible. Retirer la méthode, c'est supprimer une méthode
par défaut du trait, ses redéfinitions dans les drivers non SQL, et la branche
de `reclassified_request` ; les requêtes non SQL redeviendraient `Unknown`
partout. Les tiers qui la redéfinissent casseraient à la compilation — le seul
coût qu'ajoute un trait publié.

**À reconsidérer si** `oxyn-query` apprend à lire lui-même un langage non SQL
— l'exécuteur aurait alors deux lecteurs du même texte et devrait n'en garder
qu'un — ou si l'on trouve un driver qui répond `Read` pour une requête qui
écrit : la réponse demanderait alors une seconde vérification, par le moteur
lui-même, avant que la lecture parte.

## Alternatives rejetées

| Alternative | Raison du rejet |
|---|---|
| Garder `Unknown` pour toute requête non SQL, côté driver seulement | Sûr, mais rend tout driver non SQL inutilisable en production — une confirmation par `GET`, toute lecture d'agent refusée — et rend les aperçus impossibles ; les utilisateurs marqueraient leurs connexions de production en staging pour travailler, ce qui retire la garde entièrement |
| Apprendre à `oxyn-query` chaque langage non SQL | `oxyn-query` aurait besoin du jeu de commandes de chaque moteur, version et extension, que seul le serveur connecté connaît avec certitude ; la session est le seul endroit qui lui parle |
| Une méthode `async` qui interroge le serveur | Met un aller-retour sur le chemin du dispatch avant chaque décision, et un verrou à travers un `await` à cet endroit (I-05) ; ce dont une session a besoin pour classer peut se lire une fois, à la connexion, quand elle évalue déjà ses capacités (ADR-0003) |
| Laisser la session répondre aussi pour le SQL | Deux lecteurs du SQL divergeraient, et le moins prudent l'emporterait sur la requête qu'il lit mal ; `oxyn-query` tient les pièges testés (`EXPLAIN ANALYZE DELETE`, `WITH … DELETE`) |
| Renvoyer une `Classification` complète (risque, contrôle de transaction, changement de contexte) | Aucun driver non SQL n'en a besoin aujourd'hui ; ces drapeaux pilotent des chemins propres au SQL (verbes de transaction, `USE`) que ces langages traitent par leurs propres refus de console et par `set_context` |
