# ADR-0016 — Sérialiser les écritures d'un document dans une file bornée

**Statut :** accepté · **Date :** 2026-09-10

**Précise :** [ADR-0014](0014-documents-et-historique.md), pour la sauvegarde
automatique et la concurrence entre instances.

## Contexte

Envoyer un texte de 1 Mio à chaque frappe sans borne de file accumulerait des
copies pendant un blocage du store. Faire attendre la sauvegarde sur le thread
UI violerait I-05. Une sauvegarde automatique ne doit ni modifier la copie nommée,
ni remplacer le travail d'un autre éditeur ayant ouvert le même document.

## Décision

Un `DocumentWriter` côté backend sérialise les écritures d'une console : une
opération active, au plus un dernier brouillon en attente, une sauvegarde nommée
et une fermeture. Les brouillons intermédiaires sont remplacés par le plus
récent. Une sauvegarde explicite conserve son instantané jusqu'à son traitement.
La fermeture retire les brouillons non engagés et interdit de nouveaux envois.

Les écritures utilisent une révision attendue, vérifiée dans la transaction
SQLite avant mutation. Un conflit ou un état illisible arrête la file ; la vue
conserve son texte et propose une copie séparée. Aucun SQL utilisateur n'est
exécuté. Les anciennes commandes sans révision attendue restent lisibles pour
compatibilité, sans être utilisées par l'autosauvegarde.

Le compteur de fermeture appartient au backend et couvre toute la file active,
pas seulement son opération courante. La disparition de la vue ne perd donc pas
le dernier brouillon déjà soumis. Un document fermé avant sa première écriture
reçoit un marqueur sans texte afin de refuser toute écriture tardive.

## Conséquences

- **+** La mémoire en attente est bornée par document.
- **+** Les brouillons restent distincts des copies explicitement sauvegardées.
- **+** Le contrôle de révision empêche un éditeur périmé d'écraser un autre.
- **−** Une erreur de concurrence nécessite une décision de copie ou de reprise.
- **−** La fermeture et les sauvegardes partagent un ordonnanceur local.

**Coût de sortie :** remplacer la file et ses acquittements dans `oxyn-app` et
le contrôle de révision dans `oxyn-store`, sans changer les drivers ni Arrow.

**Reconsidérer si** une édition collaborative exige une fusion, ou si les
scripts doivent dépasser la borne de 1 Mio décidée dans ADR-0014.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Une tâche portant le texte à chaque frappe | File de copies non bornée |
| Un délai détenu uniquement par la vue | Dernière frappe perdue si la vue disparaît avant son déclenchement |
| Révision croissante sans révision attendue | Un éditeur périmé peut finir par dépasser le compteur d'un autre |
