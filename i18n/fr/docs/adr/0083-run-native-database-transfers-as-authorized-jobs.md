<!-- oxyn-translation source="docs/adr/0083-run-native-database-transfers-as-authorized-jobs.md" sha256="68ae3ba4d5c9" -->

# ADR-0083 — Exécuter les transferts natifs comme tâches autorisées et possédées

**Statut :** proposé · **Date :** 2026-11-13

**Précise :** [ADR-0004](0004-command-bus.md) sur les commandes de transfert
natif et [ADR-0035](0035-ecritures-locales-de-l-ordonnanceur-sur-le-pool-bloquant.md)
sur les I/O possédées et l'issue auditée. Cette proposition définit une frontière,
pas l'autorisation d'exécuter un outil ni l'affirmation que l'authentification
est résolue.

## Contexte

L'issue [#374](https://github.com/so-keyldzn/oxyn/issues/374) demande sauvegarde
et restauration natives PostgreSQL. La commande `Export` écrit des résultats
conservés ; elle n'exporte pas une base. Un lanceur de processus Tauri brut
créerait un second chemin hors politique de connexion, annulation et audit.

Un transfert peut survivre à son panneau, émettre des diagnostics sensibles,
créer un fichier partiel ou modifier une base avant une erreur client.
L'authentification vers un exécutable externe traverse aussi une frontière
absente des connexions ordinaires. Les
[parcours de recherche](../implementation-plan/schema-research-journeys.md)
distinguent réussite d'export, réussite de restauration et résultats inconnus.

## Décision

`ExportDatabase` et `RestoreDatabase` sont des commandes typées distinctes
d'`oxyn-core`. `oxyn-exec` possède politique, revérifications d'autorisation,
audit, identité de connexion et cycle de vie. La première révision réserve les
deux commandes à l'humain, avec refus par la porte et non simple masquage d'un
outil IA. Lecture seule et confirmation de production restent gouvernées par
[SECURITY](../SECURITY.md) ; restaurer est explicitement mutant.

Une nouvelle crate `oxyn-transfer` possède la frontière des outils natifs :
interface étroite `TransferRunner`, demandes/événements validés et adaptateur
PostgreSQL. Elle ne décide aucune politique et ne dépend ni de Tauri,
ni d'`oxyn-store`, ni d'`oxyn-secrets`. L'exécuteur dépend de cette frontière
et l'appelle seulement après autorisation ; `oxyn-desktop` fournit le runner
et les dialogues natifs. Composition des arguments PostgreSQL et propriété
des processus vivent dans des modules dédiés de cette crate, pas dans l'interface
ou le chemin d'exécution SQL du driver réseau. Aucun lanceur shell général
n'est exposé.

Un exécutable est une sélection configurée par l'humain et possédée par l'hôte.
Son identité et son invocation prise en charge sont vérifiées avant usage ;
un chemin fourni par la webview ne donne pas à lui seul le droit d'exécuter.
Les commandes nomment une sélection de l'hôte, une connexion cible et des
options structurées, pas un exécutable accompagné d'arguments arbitraires.
La demande revue fixe destination effective, fichiers source/destination,
options et identité de l'outil ; un changement significatif exige une nouvelle revue.
La revue identifie la provenance de l'archive et les effets connus des options
choisies. Elle ne certifie pas la sûreté du contenu et ne prétend pas énumérer
tous ses effets ; restaurer reste une opération privilégiée explicite de l'humain.

L'adaptateur invoque l'exécutable sans shell, avec un environnement construit
explicitement et sans configuration ambiante de service de connexion. Cible
réseau et protection du transport correspondent à la connexion revue. Les
options pouvant rediriger vers une cible nommée dans l'archive sont refusées
tant que cette cible effective ne peut être autorisée indépendamment. Partager
un protocole ne prouve pas qu'une variante PostgreSQL accepte ces outils.

L'authentification refuse en cas d'incertitude. Cet ADR n'autorise ni mot de
passe dans les arguments, ni environnement hérité, ni fichier temporaire de
mot de passe en clair. Il ne choisit pas le transport du mot de passe et ne
s'appuie pas sur les fichiers d'identifiants ambiants de l'utilisateur. Un mode
n'est proposé qu'après preuve, sur la plateforme, du rattachement à la
destination, du confinement du secret et du nettoyage. Le transport non résolu
est un prérequis d'implémentation du plan, pas un repli silencieux. Aucune
nouvelle exception à SECURITY n'est accordée.

`oxyn-exec` garde la propriété pendant démarrage de l'enfant, drainage des
sorties, annulation, terminaison et récupération de son état de sortie.
Fermer l'onglet de progression demande l'annulation et conserve la tâche
jusqu'au nettoyage ; le masquer n'annule pas. Fermer la fenêtre propriétaire
participe à la revue de fermeture existante. Le nettoyage couvre les descendants
par une implémentation propre à la plateforme ; abandonner un handle n'est pas
une preuve de terminaison. Aucune reprise ou répétition automatique de
restauration après timeout, perte du processus ou redémarrage.

La sortie de l'outil est non fiable. Les diagnostics bornés retirent contrôles
de terminal et contenus sensibles avant affichage, stockage ou journalisation ;
le SQL ou les données brutes réémis par une erreur ne sont pas transmis. Si une
extraction sûre n'est pas disponible, rapporter une erreur structurée sans
stderr brut. La progression décrit étapes ou comptes observés, pas un
pourcentage deviné.

L'export écrit dans une destination temporaire possédée et ne publie le fichier
final qu'après réussite du processus et finalisation. Une destination existante
exige un accord explicite d'écrasement ; une interruption ne peut présenter un
fichier partiel comme réussi. La restauration distingue refus, échec avant
démarrage, réussite, résultat partiel et inconnu. Une écriture interrompue reste
non réconciliée jusqu'à inspection. Métadonnées et aperçus concernés deviennent
périmés après toute restauration ayant pu modifier la cible, pas seulement
après une réussite annoncée.

L'enregistrement persistant de tâche contient identité, options non secrètes
revues, horodatages et issue expurgée ; aucun identifiant d'authentification ni
journal brut. Après redémarrage, les tâches inachevées sont montrées comme
interrompues, jamais rejouées.

## Conséquences

* **+** Les outils natifs suivent la même autorité d'exécution que les autres écritures.
* **+** Fichiers partiels, restaurations ambiguës et échecs de nettoyage restent observables.
* **−** Authentification et arrêt des descendants exigent des preuves par plateforme ;
  les modes non pris en charge restent indisponibles.
* **−** Les options structurées offrent moins de liberté qu'un terminal sans restriction,
  et les diagnostics sûrs peuvent donner moins de détails que la sortie brute.

**Coût de sortie :** retirer runner et adaptateur, conserver les enregistrements
historiques lisibles et désactiver les deux commandes. Un autre adaptateur peut
réutiliser les contrats de politique, audit et interface. Cela introduit une
frontière de crate dédiée, mais aucune dépendance externe de gestion de processus
n'est choisie ici.

**À reconsidérer si** aucun mode d'authentification utile ne peut respecter
SECURITY sur les plateformes cibles, ou si une bibliothèque native offre une
compatibilité de formats équivalente et une annulation plus sûre. Toute exception
de sécurité ou nouvelle bibliothèque nécessite une décision revue avant usage.

## Alternatives rejetées

| Alternative | Motif du rejet |
|---|---|
| Lancer les outils directement depuis une commande Tauri | Contourne bus, politique et audit |
| Réutiliser l'export de résultats comme sauvegarde | Confond un buffer de résultats avec une opération sur toute la base |
| Exposer des options de commande libres | Peut rediriger la cible ou modifier les effets hors de la demande revue |
| Assimiler sortie ou annulation du processus à un rollback | N'établit pas ce que la destination a appliqué |
| Écrire un fichier temporaire de mot de passe comme détail technique | Contredit l'interdiction actuelle des secrets en clair sur disque |
| Partager un lanceur générique avec les fournisseurs IA | Couple des frontières de confiance distinctes et encourage une autorité trop large |
