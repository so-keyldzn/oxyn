<!-- oxyn-translation source="docs/adr/0066-a-session-classifies-the-risk-of-non-sql-requests.md" sha256="5ac76b042882" -->

> Traduction française de [docs/adr/0066-a-session-classifies-the-risk-of-non-sql-requests.md](../../../../docs/adr/0066-a-session-classifies-the-risk-of-non-sql-requests.md). **La version anglaise fait foi.**

# ADR-0066 — Une session classe le risque des requêtes non SQL qu'elle classe comme écritures

**Statut :** accepté · **Date :** 2026-10-23

**Précise :** [ADR-0064](0064-a-session-classifies-non-sql-requests.md), sur le
point suivant : « `Some(intent)` remplace `Unknown`, avec un risque `None` ».
Une écriture, un DDL ou un grant prouvés prennent désormais le risque que leur
session répond. ADR-0064 reste en vigueur tel qu'il est écrit pour tout le
reste — l'intention, la frontière du SQL, l'absence d'E/S, le contrôle de
transaction et le changement de contexte.

## Contexte

Pour le SQL, `oxyn-query` calcule un `MutationRisk` à côté de l'intention :
`UnboundedUpdate`, `UnboundedDelete`, `Truncate`, `DropObject`, les deux formes
de `COPY`. Le `PolicyGate` vérifie le risque avant toute autre chose, et un
risque demande une approbation **quels que soient l'acteur et
l'environnement** (`crates/oxyn-core/src/policy.rs`) : `DELETE FROM users` sur
staging attend un humain, avec le motif « DELETE without a WHERE clause: every
row ».

ADR-0064 a permis à une session non SQL de prouver une intention, et a fixé le
risque d'une intention prouvée à `MutationRisk::None` — il l'énonçait comme une
conséquence négative : « une écriture non SQL n'est jamais signalée comme une
suppression non bornée, seulement comme une écriture ». La relecture du driver
MongoDB (pull request #330) a donné à cette conséquence sa forme concrète. Le
classifieur du driver sait que
`{"delete":"users","deletes":[{"q":{},"limit":0}]}` supprime tous les documents
et que `{"drop":"users"}` supprime la collection, mais n'a aucun moyen de le
dire :

* sur staging ou en développement, la suppression de toute une collection par
  un humain s'exécute en un clic, là où `DELETE FROM users` demande une
  approbation ;
* la suppression de toute une collection par un agent sur staging est demandée
  sous le générique « an agent is requesting a write operation » au lieu du
  motif qui dit combien est touché ;
* sur production la confirmation nomme la connexion, mais pas la portée.

Redis (`FLUSHALL`, `FLUSHDB`) et Elasticsearch (`DELETE /index`,
`_delete_by_query` avec `match_all`) ont le même besoin, et leurs drivers
s'écrivent en ce moment.

## Décision

`trait Session` (`crates/oxyn-driver/src/traits.rs`) gagne une seconde méthode
synchrone avec un corps par défaut, sœur de `classify_intent` :

```rust
fn classify_risk(&self, request: &ExecRequest) -> Option<MutationRisk> {
    let _ = request;
    None
}
```

* `oxyn-exec` la consulte dans `reclassified_request`
  (`crates/oxyn-exec/src/executor.rs`), par `SessionSlot::classify_risk`, qui
  prend le verrou du slot par `try_read` comme `classify_intent`. Il le fait
  **seulement** pour une requête dont le langage n'a pas de dialecte SQL, et
  **seulement** quand `classify_intent` a répondu `Write`, `Ddl` ou `Grant`. Un
  `Read` prouvé ne reçoit jamais de risque : la session ne pourrait que
  l'inventer. Une requête restée `Unknown`, ou répondue `Unknown`, garde son
  traitement actuel — elle compte déjà comme mutante.
* La réponse remplace `MutationRisk::None`, le risque qu'ADR-0064 donne à une
  intention prouvée et celui qu'`oxyn-query` donne à tout texte non SQL. Elle
  ne peut donc que relever le risque : `None`, ou `Some(MutationRisk::None)`,
  laisse la requête exactement comme avant que cette méthode existe.
* Le `PolicyGate`, le flux d'approbation et le journal sont inchangés. Le risque
  voyage dans l'`ExecRequest` comme un risque SQL : le gate le lit, le journal
  l'enregistre, et une commande approuvée part avec lui.
* La méthode est **prudente dans l'autre sens** que `classify_intent` : le
  risque est répondu dès que la requête peut toucher tous les documents, clés
  ou index de sa cible, pas seulement quand c'est prouvé. Un `Read` à tort
  contourne la garde de production ; un risque manqué contourne une
  approbation. Les deux sont silencieux.
* La règle pour les implémenteurs vit dans
  [DRIVER-CONTRACT §5](../DRIVER-CONTRACT.md#5-il-déclare-ses-capacités-par-session-et-ne-simule-rien).

Les variantes de `MutationRisk` sont réutilisées telles quelles : un filtre de
suppression vide est `UnboundedDelete`, le drop d'une collection ou d'un index
est `DropObject`, le vidage de toutes les clés revient au driver, qui le range
sous la variante la plus proche. Cette décision n'ajoute aucune variante.

## Conséquences

* **+** La suppression ou le drop d'une collection entière en non SQL demande
  l'approbation que demande son équivalent SQL, sur tout environnement, pour un
  humain comme pour un agent.
* **+** Aucune session existante ne change : le défaut répond `None`, qui est le
  comportement d'ADR-0064. La modification d'`oxyn-driver`, la crate Apache-2.0
  que les tiers lient ([ADR-0044](0044-licence-gpl-et-contrat-apache.md)), n'est
  pas cassante.
* **+** La décision reste au gate : la session fournit un fait, par le type même
  qu'utilise le SQL, et rien en aval n'a besoin de savoir d'où il vient.
* **−** L'approbation d'une requête non SQL destructrice repose désormais sur la
  parole du driver. Un driver qui manque une forme — un filtre qui correspond à
  tout sans être vide, `{"q":{"_id":{"$exists":true}}}` — la laisse passer comme
  une simple écriture, et aucun test d'`oxyn-exec` ne peut le remarquer.
* **−** Les motifs que montre le gate sont rédigés pour le SQL
  (`MutationRisk::reason`) : une suppression MongoDB est annoncée comme
  « DELETE without a WHERE clause: every row ». C'est compréhensible, pas exact.
* **−** Une session qui se ferme entre les deux appels répond une intention et
  aucun risque ; dans cette fenêtre, une écriture destructrice est demandée
  comme une simple écriture. Cela n'arrive que pendant la fermeture de la
  console, et l'écriture passe toujours par le gate.

**Coût de sortie :** faible. Retirer la méthode, c'est supprimer une méthode
par défaut du trait, ses redéfinitions dans les drivers non SQL,
`SessionSlot::classify_risk` et le `match` de `reclassified_request` ; les
écritures non SQL redeviendraient sans risque. Les tiers qui la redéfinissent
casseraient à la compilation.

**À reconsidérer si** un langage non SQL a besoin de dire ce que `MutationRisk`
ne sait pas dire — un vidage, un script côté serveur, une réindexation — et
qu'un driver se met à le ranger sous une variante dont le motif dit quelque
chose de faux ; ou si l'on trouve `classify_intent` et `classify_risk` en
désaccord dans un driver assez souvent pour qu'une méthode unique renvoyant les
deux soit la forme la plus sûre.

## Alternatives rejetées

| Alternative | Raison du rejet |
|---|---|
| Faire renvoyer à `classify_intent` une paire `(StatementIntent, MutationRisk)`, ou une `Classification` complète | Casse toute redéfinition écrite d'après ADR-0064, dans un trait publié, pour un fait que la moitié des requêtes ne porte jamais ; une méthode sœur avec un défaut ne coûte rien aux sessions qui n'ont pas de risque à dire |
| Consulter le risque pour toute requête non SQL, lectures comprises | Une lecture avec un risque compte comme mutante (`ExecRequest::is_mutating`) : une session qui répondrait un risque sur une lecture en ferait une écriture, ce qui est la question de `classify_intent`, et deux méthodes décideraient alors de l'intention |
| Laisser la session abaisser un risque calculé par l'exécuteur | L'exécuteur n'en calcule aucun pour une requête non SQL aujourd'hui ; autoriser un abaissement laisserait un futur lecteur de texte non SQL être contredit par le moins prudent des deux |
| Ajouter dès maintenant des variantes non SQL à `MutationRisk` (`FlushAll`, `DropIndex`) | `MutationRisk` est dans `oxyn-core`, dont les motifs sont affichés et journalisés ; une variante vaut d'être ajoutée quand un driver en a besoin et sait nommer son motif, pas en avance sur les drivers |
| Apprendre au gate les formes destructrices de chaque langage | Le gate analyserait les requêtes MongoDB, Redis et Elasticsearch, ce qu'ADR-0064 a rejeté pour l'intention pour la même raison : seule la session parle le langage |
