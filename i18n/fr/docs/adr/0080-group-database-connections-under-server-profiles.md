<!-- oxyn-translation source="docs/adr/0080-group-database-connections-under-server-profiles.md" sha256="45a1022ae9a8" -->

# ADR-0080 — Regrouper les connexions par serveur sans partager leur autorité

**Statut :** accepté · **Date :** 2026-11-13 · **Implémenté :** 2026-11-14

**Précise :** [ADR-0019](0019-contexte-de-session.md) sur la distinction entre
ouvrir une autre base et changer l'espace de noms d'une console, et
[ADR-0046](0046-workspaces-retenus-restent-connectes.md) sur le regroupement des
connexions conservées. L’implémentation multi-base préserve les contrats de
sécurité et de persistance existants ; le [suivi](../implementation-plan/completion.md)
distingue contrôles automatiques et acceptation personnelle.

## Contexte

L'issue [#373](https://github.com/so-keyldzn/oxyn/issues/373) demande de parcourir
les bases sous un serveur. Au commit `6647180`, le driver PostgreSQL ne liste
que sa base connectée et refuse une autre base dans `SetSessionContext`.
MySQL liste déjà les bases comme espaces de noms. La
[matrice des moteurs](../implementation-plan/schema-research-engines.md)
recense les autres hiérarchies.

`ConnectionId` délimite déjà politiques, caches de catalogue, résultats,
conversations et récupération. Le remplacer par une identité serveur
mélangerait des autorisations distinctes. [SECURITY](../SECURITY.md) interdit
aussi de déplacer un secret enregistré vers une destination modifiée, base
comprise. Le besoin est de réduire la duplication de configuration, pas
d'élargir implicitement l'accès.

L'inspection de l'interface à `f456ff1` ajoute une contrainte de propriété :
[`WorkspaceHost`](../../../../apps/desktop/src/features/workspace/workspace-host.tsx)
conserve un workspace par connexion ouverte, tandis que
[`CatalogSidebar`](../../../../apps/desktop/src/features/workspace/catalog-sidebar.tsx)
lie lectures et actions à son `OpenConnection`.
[`useConsoles`](../../../../apps/desktop/src/features/consoles/use-consoles.ts)
peut initialiser un nouveau workspace avec le brouillon SQL précédent, et
[`nameTransfer`](../../../../apps/desktop/src/features/workspace/name-transfer.ts)
insère dans l'éditeur sous le pointeur. Imbriquer simplement des noms de bases
dans l'arbre existant laisserait ces actions liées au mauvais propriétaire.

## Décision

`ServerProfile` regroupe des destinations et porte un `ServerProfileId`
stable, un nom d'affichage et des paramètres de connexion par défaut validés et
non secrets. Il ne possède ni identifiant d'authentification, ni transaction,
ni buffer de résultats, ni niveau de confidentialité IA. `oxyn-core` possède
ce vocabulaire ; `oxyn-store` conserve profils et appartenances dans le format
ouvert existant du workspace.

Une base PostgreSQL ouverte reste une `ConnectionConfig` ordinaire avec son
`ConnectionId` stable, sa destination complète, son environnement, son
marquage lecture seule, son niveau de confidentialité et sa référence de secret
indépendante. Le groupe n'est pas une cible d'exécution. Commandes,
confirmations, chemins d'objets et onglets restaurés sont rattachés à la
connexion enfant avant autorisation. Un nom de base seul n'est jamais une
identité ; les noms affichés peuvent être ambigus.

La première ouverture présente la destination enfant et ses protections. Les
nouveaux enfants utilisent les valeurs prudentes existantes plutôt que les
marquages permissifs d'une connexion sœur. L'authentification suit le parcours
explicite actuel, avec une entrée d'identifiants distincte par enfant. Cette
révision ne copie pas automatiquement le secret d'une connexion sœur et
n'accorde aucun accès d'authentification à l'échelle du serveur. Une telle
autorisation future nécessite sa propre décision de sécurité.

`DiscoverDatabases` est une commande de métadonnées annulable via
`oxyn-exec`. Elle utilise une connexion initiale déjà autorisée sur ce
serveur. Une nouvelle frontière facultative du driver/de la session pour la
découverte retourne `NotSupported` par défaut ; elle rapporte des noms
validés et les indications d'accès connues, jamais des identifiants ni une
promesse de réussite d'ouverture. PostgreSQL possède sa requête de découverte.
L'hôte crée ou choisit un enfant via l'autorisation de connexion existante, puis
utilise `Connect` ; découvrir ne crée aucune session enfant. La sémantique
existante de `CatalogProvider::list_catalogs` ne change pas.

La hiérarchie propre au driver fait toujours autorité : changer d'espace de
noms MySQL reste une opération de contexte de session ; les attachements
SQLite/DuckDB ne deviennent pas des bases distantes. Aucun ATTACH automatique,
connexion implicite au serveur ou introspection de toutes les bases n'est ajouté.

Modifier le profil change les valeurs par défaut des futurs enfants. Appliquer
les changements aux enfants existants passe par une revue explicite des
destinations affectées, puis leur mise à jour ordinaire ; les sessions vivantes
ne sont pas redirigées silencieusement. Détacher un enfant conserve sa connexion
et son historique. Supprimer un groupe ne supprime pas implicitement connexions
enfants ou identifiants ; le travail ouvert suit les règles existantes de
fermeture et de transaction.

Les anciennes connexions migrent comme connexions non regroupées, sans changement
d'ID ni de référence de secret. L'appartenance est un état supplémentaire, pas
une réécriture de l'historique. Les limites de workspaces conservés d'ADR-0046
continuent à compter les enfants ouverts ; lister des bases n'occupe aucune place.

### Contrat des commandes locales de profil

`ListServerProfiles`, `SaveServerProfile`, `RemoveServerProfile` et
`SetServerProfileMembership` opèrent sur le workspace de l'exécuteur.
Toutes sont refusées à un agent, lecture des valeurs par défaut comprise.
Leur intention envers la base est `Read`, comme les autres paramètres locaux :
enregistrer un groupe n'exécute aucun DDL et ne change pas l'autorité d'un enfant.
Les écritures locales restent auditées et possédées jusqu'à leur fin sur le pool
bloquant, y compris pendant l'arrêt du desktop.

L'enregistrement utilise une révision attendue : absente à la création, valeur
précédente exacte à la modification. L'épuisement est refusé avant perte de
précision entière dans JavaScript. L'appartenance compare le groupe précédent
observé avant de le remplacer. Driver et workspace ne changent pas sous un groupe
existant ; une appartenance ne traverse aucune de ces frontières. Supprimer le
groupe retire seulement l'appartenance, en conservant configuration et référence
de secret de l'enfant.

Le core borne un profil à 32 paramètres, 64 octets par clé, 4096 octets par valeur
et 256 octets pour le nom. Le store borne en plus l'encodage à 256 Kio, les
profils d'un workspace à 256 et les membres d'un profil à 1024. Ce sont des
limites d'entrée de l'état local, pas des quotas de préchargement ou de sessions.
L'exécuteur revalide les clés face aux champs non secrets déclarés du driver,
relecture comprise ; le store applique indépendamment sa garde existante des
clés de secrets. Un état corrompu ou excessif échoue explicitement, sans liste
partielle présentée comme complète ni reproduction des valeurs fournies.

### Contrat des commandes de découverte et création d'enfant

La frontière optionnelle de découverte appartient au `CatalogProvider` de la
session : son implémentation PostgreSQL possède déjà les lectures de métadonnées
réservées et annulables. `DiscoverDatabases` est une lecture réservée à l'humain
sur sa connexion source. Les résultats sont bornés à 1024 noms de 1024 octets
maximum, avec refus du dépassement plutôt que troncature silencieuse.
`CreateDatabaseConnection` applique la politique de `CreateConnection` : création
interdite à l'agent, approbation explicite en production nommant la nouvelle
destination. Persistance de l'enfant et appartenance sont atomiques. La comparaison
stricte des paramètres hors base/utilisateur évite de deviner l'équivalence de destinations
ou déplacer un secret source. L'ouverture de session reste l'opération `Connect`
existante, autorisée séparément.

### Propriété et activation dans l'interface

Un explorateur commun appartient à la navigation de la fenêtre. La zone de
contenu conserve un workspace par connexion enfant, avec ses onglets, résultats,
inspecteur et assistant. La première révision ne mélange pas les onglets de
plusieurs bases dans une barre commune. Activer une base déjà ouverte montre
son workspace conservé sans rouvrir les sessions ni modifier un brouillon.

Sélection, développement et activation sont des gestes distincts. Un clic
sélectionne un nœud ; son chevron le développe. Entrée, double-clic ou
`Open workspace` active explicitement une base ; ouvrir un de ses objets
active cette base avant d'ouvrir l'objet. En-tête, environnement, lecture seule
et assistant décrivent toujours le contenu actif, pas le nœud sélectionné.
Une base inactive peut rester connectée, travailler ou garder une transaction.

La navigation porte des cibles typées : profil, base découverte, connexion
enfant, objet avec connexion enfant et adresse complète. Actions d'objets,
résultats de recherche, menus, actualisation, nouvelles consoles et ouvertures
de carte portent ce propriétaire ; l'hôte le valide.
`ConnectionSummary.location` reste un libellé, jamais une valeur parsée
pour reconstruire la cible.

Chaque base a son état de découverte/ouverture/erreur et son annulation.
Ouvrir une autre base ne bloque pas tout l'explorateur. Les réservations
d'ouverture par fenêtre comptent dans la limite existante ; les activations
répétées se rejoignent sur le même enfant. Une ouverture annulée qui finit
tardivement ne libère que ses propres ressources, pas une session déjà conservée
par une autre opération. Les états distinguent découverte, sauvegarde, ouverture,
connexion, activité, annulation et erreurs locales. La revue de fermeture
ordinaire traite la déconnexion en attente ; une découverte échouée n’est pas
un serveur vide.

### Routage du texte et contexte

Dans cette nouvelle navigation groupée, une base nouvellement ouverte démarre
avec une console vide. Cela précise la copie automatique décrite dans la première
navigation d’UX-SPEC ; le parcours distinct de l’accueil garde sa copie existante.
Une future copie explicite entre bases devra nommer source et destination et
créer une copie non exécutée ; elle ne fait pas partie de l’activation implicite.

Les insertions nomment connexion et génération de console/session destinataires ;
changer de base active ne peut les rediriger. Déposer un objet sur l'éditeur
d'une autre base n'insère rien automatiquement : proposer
`Open a console on the source database` ou Annuler. La même discipline
s'applique aux demandes de l'assistant et de l'inspecteur. Un sélecteur d'espace
de noms ne change que le contexte confirmé de sa console ; il n'active pas
une autre connexion enfant PostgreSQL.

### Configuration, modification et déconnexion du groupe

La configuration choisit explicitement une connexion initiale déjà autorisée,
ou en crée une par le parcours ordinaire avant la découverte. Enregistrer des
valeurs par défaut n'authentifie pas le profil. Ouvrir un enfant découvert
présente destination et protections et demande ses identifiants indépendants.

Les menus de profil, base et objet ont des opérations distinctes.
`Disconnect group…` présente chaque enfant affecté, ses consoles, travail
non enregistré, opérations en cours et état de transaction rapporté, inconnu
compris. Les transactions non résolues bloquent la fin jusqu'à leur traitement
par la revue existante par session ; aucun commit ni rollback implicite.
Annuler la revue ne ferme rien. Les fermetures terminées sont rapportées
individuellement si une suivante échoue. La fermeture suit la propriété
multi-fenêtre existante, pas un arrêt global.

Déconnecter l'enfant actif active un autre enfant conservé s'il existe, sinon
affiche une vue neutre de connexion. Supprimer le profil reste distinct de
déconnecter, détacher et de toute opération DDL sur une base.

## Conséquences

* **+** Le regroupement réutilise la sécurité et la récupération par connexion.
* **+** Les hiérarchies des moteurs et les identités des anciens workspaces restent utilisables.
* **−** Ouvrir une autre base authentifiée peut demander de saisir ses identifiants.
* **−** Modifier un profil ne synchronise pas immédiatement tous ses enfants ;
  la revue reste volontairement visible et les sessions indépendantes consomment des ressources.
* **−** Voir les onglets d'une autre base demande de changer de workspace ;
  une barre commune sort volontairement de cette révision. L'explorateur exige
  son propre état de navigation et un suivi asynchrone par enfant.

**Coût de sortie :** retirer appartenance et interface de regroupement en
conservant les connexions enfants ordinaires. Supprimer les identités par base
exigerait au contraire de migrer politiques, résultats, documents et références
de conversations ; c'est l'alternative coûteuse que ce découpage évite.

**À reconsidérer si** les usages mesurés ne tolèrent pas la saisie par base,
ou si un driver ne peut préserver sa hiérarchie native dans ce regroupement.
Partager un identifiant exige de définir destination autorisée exacte,
révocation et comportement des sessions vivantes avant de modifier cette décision.

## Alternatives rejetées

| Alternative | Motif du rejet |
|---|---|
| Une connexion mutable pour tout le serveur PostgreSQL | Changer de base pourrait modifier le sens du travail en attente et mélanger identités de politique/cache |
| Copier automatiquement les identifiants entre enfants | Élargit leur destination sans l'autorisation exigée par SECURITY |
| Ouvrir chaque base découverte | Consomme des sessions et effectue du travail non demandé avant le choix utilisateur |
| Remplacer toutes les hiérarchies par le modèle PostgreSQL | Fait régresser les moteurs exposant déjà plusieurs espaces de noms sur une connexion |
| Regrouper automatiquement les anciennes connexions à la migration | Risque de modifier historique et propriété des secrets pour une fonction visuelle |
| Ajouter des bases tout en gardant un gestionnaire de catalogue lié au workspace | Menus et lectures asynchrones pourraient utiliser la base active plutôt que celle du nœud |
| Mélanger immédiatement les onglets de toutes les bases | Élargit les changements de propriété à l'exécution, l'inspecteur, l'assistant et la fermeture/récupération |
| Copier automatiquement le brouillon précédent à l'ouverture groupée | Préremplit du texte pour une autre destination lors d'une navigation sans geste explicite de copie |
