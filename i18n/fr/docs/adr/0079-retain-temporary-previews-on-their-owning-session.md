<!-- oxyn-translation source="docs/adr/0079-retain-temporary-previews-on-their-owning-session.md" sha256="600f2cbdc341" -->

# ADR-0079 — Conserver les aperçus temporaires sur leur session propriétaire

**Statut :** accepté · **Date :** 2026-11-13

**Précise :** [ADR-0078](0078-page-keyless-relations-with-isolated-retained-reads.md), pour l’état qu’une autre session ne peut reproduire, ainsi que [ADR-0019](0019-contexte-de-session.md) / [ADR-0050](0050-mysql-driver-on-mysql-async-prepared-first.md) pour la propriété de la session physique.

## Contexte

Les relations temporaires, bases privées en mémoire et attachements locaux à une session ne peuvent être copiés vers un lecteur isolé sans changer ce qui est lu. Le repli borné actuel perd la pagination. Conserver leur curseur sans coordination bloquerait indéfiniment le worker ou la connexion SQL propriétaire. Le cache de catalogue par connexion et les onglets d’objets ne distinguent pas non plus des objets homonymes appartenant à différentes consoles.

## Décision

`PreviewMode::Original` conserve une lecture sur sa session d’origine. `oxyn-exec` réserve cette session exclusivement pendant la durée du curseur. Les opérations incompatibles y échouent rapidement avec une erreur d’occupation exploitable ; annulation et fermeture restent disponibles. Les autres sessions restent utilisables. Fin du flux, libération explicite, annulation, expiration et fermeture de connexion/session libèrent curseur et réservation. Les budgets existants d’historique, de durée et de curseurs s’appliquent. Une tâche de nettoyage détenue garde réservation et capacité jusqu’à l’acquittement physique de `finish_original_preview` ; la fin du flux et `ReleasePreview` attendent cet acquittement. Aucune matérialisation complète côté client, répétition par OFFSET ni copie de l’état temporaire n’est introduite.

Les opérations ordinaires gardent une réservation partagée pendant leur opération ou la vie de leur curseur. Un aperçu original revendique l’exclusivité avant d’attendre les opérations déjà en cours ; l’annulation et le budget temporel de la commande bornent cette attente. Une seconde lecture originale est refusée rapidement plutôt que mise en attente derrière un curseur au repos. Une lecture de catalogue concurrente ne reste donc pas bloquée derrière l’aperçu suspendu. Les sondages transactionnels répondent unknown pendant cette occupation, jamais idle. Un garde d’admission par connexion exclut les nouvelles ouvertures d’aperçu depuis le nettoyage d’une mutation autorisée jusqu’à sa fin. Les ouvertures gardent une admission partagée jusqu’à la liaison du premier résultat. La déconnexion ferme l’admission, annule les lectures en attente et rejoint le nettoyage. L’arrêt ferme définitivement le registre ; une continuation annulée ne peut rattacher de nouveaux alias de résultat.

`Connect` ouvre une session de catalogue, `OpenConsoleSession` une session d’exécution. Les rôles immuables Catalog/Console/Preview sont indépendants du catalogue préféré d’une connexion : le catalogue d’une autre fenêtre n’est jamais pris pour une console. `RefreshSessionCatalog` valide et lit le propriétaire réellement demandé. L’identité de session traverse actualisation, cache, onglets, magasins d’aperçus et continuations. Les catalogues ont des contenus séparés et partagent un seul budget par connexion de 1 024 scopes et 50 000 objets. L’éviction porte sur propriétaire et scope ; fermer un propriétaire libère ses charges. Remplacer le catalogue préféré installe une génération neuve, sans reprendre son contenu. Publication et fermeture du propriétaire sont ordonnées. Les onglets locaux à une session sont éphémères : restaurer un espace ne doit pas les remplacer par un homonyme persistant. L’utilisateur peut choisir une console ouverte comme source de l’explorateur. La fermer libère ses aperçus et invalide ses onglets. Une source occupée par un aperçu ne se déplace pas vers une autre fenêtre ; utilisation et transfert sont coordonnés. Ces onglets offrent données et métadonnées ; les opérations qui ouvrent une autre session d’exécution restent à faire dans la console propriétaire.

Les drivers SQL choisissent une lecture originale lorsque l’état n’est pas reproductible. Toute session ayant accepté du SQL utilisateur arbitraire conserve aussi ses aperçus sur la source : variables et réglages peuvent modifier les lectures sans créer d’objet temporaire. Une session PostgreSQL garde un seul backend physique plutôt que de répartir l’état temporaire entre connexions d’un pool. Perdre ce backend termine explicitement le contexte local ; une continuation ne le remplace pas silencieusement. Les aperçus PostgreSQL originaux utilisent un curseur non conservable après transaction, des fenêtres FETCH diffusées de 200 lignes et le seuil de lot existant de 1 Mio. La fermeture normale au repos annule seulement la transaction de lecture seule ouverte par l’aperçu. Le nettoyage est borné à dix secondes ; une restauration non prouvée abandonne la connexion physique, et l’utilisation suivante refuse l’état perdu. Redshift suit sa syntaxe DECLARE documentée et ses limites de curseur côté serveur. Une erreur SQL ordinaire pendant la préparation PostgreSQL restaure la transaction d’aperçu avant de renvoyer l’erreur, préservant les données temporaires. Les colonnes et index temporaires MySQL sont lus par SHOW sur la source, pas dans l’homonyme persistant d’information_schema ; une ouverture explicite par nom couvre ce que le serveur n’énumère pas. Une vue persistante confirmée reste lisible avec SELECT seul lorsque SHOW CREATE est refusé faute de SHOW VIEW. PostgreSQL/MySQL ne remplacent jamais silencieusement une connexion physique après du SQL utilisateur susceptible d’avoir créé un état privé. Les autres drivers conservent leur continuation native. Les échantillons restent bornés.

Le front lit les métadonnées initiales avant de réserver la source, libère l’ancien curseur avant de changer de forme, et ferme explicitement un onglet de données sans prendre un démontage React pour une fermeture. La liaison du résultat prouve la navigation conservée même après éviction des métadonnées. Les modèles de lignes liées contrôlent le droit de la fenêtre sur le résultat ainsi que sa connexion.


Turso conserve un flux distinct par instruction ordinaire ; la création explicite d’objets temporaires est refusée plutôt que déclarée réussie pour un état qui disparaît immédiatement. Ce changement n’invente pas de session Hrana utilisateur persistante.

## Conséquences

* **+** Les données temporaires se parcourent sans changer de propriétaire ni réexécuter la requête.
* **+** Deux sessions peuvent posséder des homonymes sans partager colonnes ni lignes.
* **−** La session propriétaire ne peut exécuter un autre travail pendant la lecture originale ; l’interface explique comment la libérer.
* **−** La durée de cinq minutes et le budget d’historique imposent toujours une actualisation lorsqu’ils sont épuisés. Une source fermée ne se reconstruit pas.
* **−** La propriété physique sérielle réduit la concurrence dans une session PostgreSQL ; les consoles indépendantes gardent leurs sessions séparées.

**Coût de sortie :** retirer mode exclusif, réservations, routage des catalogues par session et identité des onglets éphémères. Les aperçus bornés et dédiés restent distincts et inchangés.

**Reconsidérer si** un driver peut suspendre un curseur physique tout en entrelaçant d’autres opérations sans l’invalider, ou si l’occupation de session s’avère inadaptée à l’usage mesuré.

## Alternatives rejetées

| Alternative | Motif |
|---|---|
| Rouvrir une autre session | L’état temporaire ou privé appartient à l’originale |
| Laisser les métadonnées en attente derrière un curseur au repos | L’utilisateur ne distingue pas un blocage d’un serveur lent |
| Annuler la lecture à chaque demande de métadonnées | Une exploration sans rapport détruit la pagination sans geste explicite |
| Partager un cache unique par connexion | Des homonymes temporaires peuvent recevoir les colonnes et lignes d’une autre session |
| Réexécuter des pages sans ordre | Des lignes peuvent être sautées ou répétées |
