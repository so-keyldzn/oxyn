# ADR-0018 — Le DDL inspecté reste une métadonnée, préparée séparément de son exécution

**Statut :** proposé · **Date :** 2026-09-10

## Contexte

La maquette `193:2433` montre un panneau DDL de 424 px, les index associés,
Copy DDL et Open DDL in console. Un texte de création n'est ni une lecture de
données ni une autorisation de l'appliquer. SQLite conserve ses déclarations ;
PostgreSQL expose principalement leurs composants et des fonctions de rendu.
Une définition peut aussi dépasser la limite de 1 Mio des documents éditables.

## Décision

`CatalogProvider::relation_definition` est une lecture annulable, conditionnée
à `OBJECT_DEFINITION`. Elle passe par `CatalogRefreshScope::Definition`.
`RelationDefinition` porte SQL, provenance et notes de portée. Le driver et le
bus valident la taille avant publication atomique ; une erreur ou une
annulation conserve le cache précédent. Le rendu Debug omet le SQL.

SQLite lit objet, index et triggers dans un même curseur de `sqlite_schema` et
qualifie leurs noms de déclaration. PostgreSQL reconstruit les statements à
partir d'une lecture de catalogue cohérente. Les objets non reconstructibles
sont refusés explicitement ; aucun exemple Figma ne remplace une réponse réelle.
La portée est la création de l'objet et de ses éléments associés, sans données,
privilèges ni dépendances externes. La provenance et les limites restent visibles.

L'aperçu utilise un éditeur en lecture seule, remplacé à chaque nouvelle
définition pour ne pas accumuler d'anciens textes dans l'historique d'annulation.
Son chargement a une identité et une annulation indépendantes des autres
onglets de métadonnées. Les retours d'un objet quitté sont ignorés.

Copy DDL copie le texte affiché. Open DDL in console crée une nouvelle console
via le trajet existant de copie de document ; il préserve les consoles
existantes et n'exécute rien. Toute exécution ultérieure traverse la politique
normale et ses confirmations. Après un échec de rafraîchissement, le texte
précédent est marqué comme potentiellement périmé.

L'aperçu est borné à 1 Mio, ses notes à 32 entrées et 64 Kio. Le cache de chaque
connexion conserve au plus 16 définitions et 16 Mio cumulés de SQL et notes.
Les anciennes définitions, y compris invalidées, sont évincées sans toucher
aux autres métadonnées ; la définition nouvellement publiée est conservée. La poignée de 8 px
redimensionne le panneau entre 320 et 640 px ; Début restaure 424 px. La largeur
est conservée dans le workspace ouvert. Le mode compact donne accès au DDL
par son sous-onglet, sans nouvelle lecture lors du seul redimensionnement.

## Conséquences

- La consultation et la préparation du SQL n'introduisent aucun chemin
  d'exécution parallèle au bus.
- Un texte trop grand est refusé au lieu d'être copié partiellement.
- La reconstruction PostgreSQL exige des validations contre le moteur ; elle
  ne constitue pas un export intégral de base.
- La largeur du panneau n'est pas encore un réglage persisté entre lancements.

**Coût de sortie :** changer le format public impose une évolution du contrat,
du cache et du lecteur ; le SQL reste récupérable comme texte. L'éditeur et la
géométrie sont confinés à `oxyn-app`/`oxyn-ui`.

**Reconsidérer si** la génération devient un export de schéma complet, si une
définition doit être modifiée sur place, ou si le plafond du document évolue.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Copier le SQL d'exemple de la maquette | Ne décrit pas la base connectée |
| Remplacer l'éditeur actif | Risque de perdre le brouillon de l'utilisateur |
| Exécuter à l'ouverture de la console | Confond préparation et autorisation |
| Donner une reconstruction partielle sans indication | Présente une omission comme une définition complète |

Les sources et vérifications externes figurent dans
[RESEARCH-NOTES](../RESEARCH-NOTES.md).
