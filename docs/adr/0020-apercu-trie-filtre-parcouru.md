# ADR-0020 — Aperçu : un tri qu'Oxyn compose, un prédicat que l'utilisateur écrit, une page déterministe

**Statut :** proposé · **Date :** 2026-09-10

**Précise :** [ADR-0012](0012-lecture-pages-resultats.md), sur ce qui distingue
une page de résultat d'une page de table.

> **Précisé par [ADR-0028](0028-pas-dordre-par-defaut-pas-de-page-sans-ordre-total.md)
> sur un point, et il est important.** Le remède proposé ici — « sans tri
> demandé, le driver ordonne par la clé primaire seule » — **n'a pas été mis en
> œuvre**. L'argument qui le motive, lui, est retenu : un `OFFSET` sur un ordre
> non garanti duplique et omet des lignes en silence. Ce que le code fait à la
> place est **plus strict** — aucun ordre imposé, et aucune page offerte tant
> que l'ordre n'est pas total.
>
> Lire ADR-0028 **avant** de « corriger » `pagination_from` ou de composer un
> `ORDER BY` par défaut : les deux passages de cet ADR qui décrivent un tri
> imposé sont périmés, et les suivre réintroduirait la panne silencieuse que cet
> ADR existe pour empêcher.

## Contexte

Sélectionner une table ouvre l'onglet Data et lit au plus 200 lignes
(`PREVIEW_ROWS`, `crates/oxyn-app/src/workspace/preview.rs`). Le driver compose
`SELECT … FROM … LIMIT n`, cite les identifiants, et l'exécuteur impose la
lecture seule. [UX-SPEC](../UX-SPEC.md#données-dune-table-sélectionnée) est
explicite sur ce qui manque : « l'ordre des lignes n'est pas garanti et l'aperçu
ne compte pas la table entière ».

Trois faits relevés dans le code bornent la solution.

**La grille n'a aucun tri.** `crates/oxyn-ui/src/data_grid.rs` lit les
`RecordBatch` dans leur ordre d'arrivée ; ses seuls réglages de colonne sont la
visibilité et la largeur. Il n'y a donc rien à réconcilier entre un tri client
existant et un tri serveur — mais aussi rien sur quoi s'appuyer.

**`ReadResultPage` ne pagine pas ce qu'on croit.** [ADR-0012](0012-lecture-pages-resultats.md)
le dit : « c'est une lecture locale […] elle ne contacte pas le serveur, ne
compose pas de SQL et ne réexécute rien ». Elle relit un lot Arrow **déjà reçu**
et débordé sur disque. Paginer une *table* est l'opération inverse : une
nouvelle exécution, avec un `OFFSET` ou un curseur. Confondre les deux donnerait
soit un bouton « page suivante » qui remontre les mêmes lignes, soit un
défilement qui relance des requêtes — ce que la règle de la grille interdit.

**`oxyn-core` ne peut ni nommer `CatalogPath` ni composer de SQL.**
`Command::PreviewRelation` transporte déjà ses paliers en `Option<String>` pour
cette raison.

Enfin, le catalogue sait quelles colonnes forment la clé primaire
(`Relation::primary_key`, `crates/oxyn-catalog/src/model.rs:712`). C'est ce qui
rend une pagination honnête possible.

## Décision

**Les deux moitiés de la demande ne se ressemblent pas, et c'est la maquette qui
le dit.** Le relevé Figma du 2026-09-10 montre, sous la barre Data, une
« Filter toolbar » (`190:1618`, 1272 × 32) faite d'un champ `272:10667` portant
le préfixe littéral **`WHERE`**, une zone de saisie de 1048 px et un bouton
`Apply`, puis d'un bouton `Sort` de 84 px. Le filtre est donc un **prédicat que
l'utilisateur écrit**, pas un constructeur colonne/opérateur/valeur.

Ce n'est pas une entorse à [I-10](../../CLAUDE.md#i-10), c'est sa lettre : ce que
l'invariant interdit, c'est qu'Oxyn **concatène un identifiant reçu** ; il dit
aussi que « le SQL que *l'utilisateur écrit* part tel quel — c'est la
fonctionnalité ». Un prédicat tapé par un professionnel appartient à la seconde
catégorie, comme le texte d'une console.

**Le tri, lui, reste structuré.** `PreviewSort { column, descending }` : la
colonne est un identifiant que le driver cite. C'est Oxyn qui compose ce
fragment-là, donc c'est Oxyn qui répond de ce qu'il contient. Une colonne que la
relation ne déclare pas est refusée plutôt que transmise au serveur.

**`PreviewShape` porte les trois : `sort`, `predicate: Option<String>` et
`offset`.** Un prédicat vide ou fait d'espaces vaut « aucun filtre » — composer
un `WHERE` sans condition produirait une erreur de syntaxe là où l'utilisateur
croit avoir tout effacé. C'est la seule normalisation appliquée à son texte.

**Le prédicat n'est pas une porte ouverte pour autant.** Le texte final est
reclassifié par `oxyn-query` et refusé s'il devient mutant — l'exécuteur le fait
déjà pour tout aperçu —, la session est tenue en lecture seule **par le
serveur**, et la borne de lignes s'applique. Un `;` suivi d'une écriture ne
franchit aucun de ces trois-là.

**Deux capacités, `PREVIEW_SORT` et `PREVIEW_FILTER`.** Un moteur qui ne les
déclare pas n'affiche pas ces contrôles
([ADR-0003](0003-driver-capabilities.md)). Ce n'est pas une précaution
théorique : le produit vise aussi les familles clé-valeur, où ordonner une
lecture n'a pas de sens.

**Une page suivante est une exécution, et elle n'est offerte que si l'ordre est
déterministe.** Un `OFFSET` sur un ordre non garanti rend des lignes en double
et en omet d'autres, sans que rien ne le signale — c'est la panne silencieuse
que cet ADR refuse. Donc :

- si l'utilisateur a demandé un tri, le driver **le complète** par la clé
  primaire déclarée au catalogue, pour lever les ex æquo ;
- sans tri demandé, le driver ordonne par la clé primaire seule ;
- si la relation n'a pas de clé unique connue, **la pagination n'est pas
  proposée** et l'interface dit pourquoi. L'aperçu reste borné à sa première
  page, ce qu'il est déjà aujourd'hui.

**`ReadResultPage` reste inchangé, et les deux notions ne se rejoignent nulle
part.** Faire défiler les lignes reçues ne déclenche jamais de requête ; demander
la page suivante de la table est un geste explicite, qui produit un nouveau
résultat avec sa propre identité.

## Conséquences

- **+** Un aperçu devient utilisable sur une vraie table : trouver une ligne
  n'oblige plus à écrire du SQL dans la console.
- **+** Le prédicat est du SQL, donc il dit tout ce que le SQL dit :
  `a IS NOT NULL AND (b > c)` s'écrit, là où trois menus ne l'auraient pas
  exprimé.
- **−** Ce prédicat est aussi du SQL que l'utilisateur peut écrire faux. Le
  message d'erreur du serveur le lui dira — son public le lit — mais l'aperçu
  n'a plus la propriété « ne peut pas échouer pour une raison de syntaxe ».
- **+** La pagination ne ment pas : elle existe quand elle est correcte, et son
  absence est expliquée.
- **−** Chaque page est une exécution : elle coûte au serveur, et les données
  peuvent avoir changé entre deux pages. L'interface doit le dire plutôt que de
  laisser croire à un instantané.
- **−** `OFFSET` est linéaire : la centième page coûte cent fois la première. Ce
  n'est pas un défaut d'Oxyn, mais c'est Oxyn qui le rendra visible.
- **−** Deux capacités et une signature de plus dans un contrat que
  [PLUGIN-CONTRACT](../PLUGIN-CONTRACT.md) devra porter en phase 4.
- **−** Un tri imposé par défaut sur la clé primaire change ce que l'utilisateur
  voit en premier par rapport à aujourd'hui. C'est un ordre arbitraire remplacé
  par un ordre déterministe, mais c'est un changement visible.

**Coût de sortie :** retirer un champ de forme, deux capacités et une traduction
par driver. Rien n'est persisté dans un format de workspace — le tri
et le filtre d'un aperçu ne survivent pas à la fermeture de l'onglet, ce que cet
ADR ne cherche pas à changer —, ce qui borne le coût à du code.

**Reconsidérer si** un driver ne sait pas exprimer un `OFFSET` stable et impose
un curseur opaque, ou si la mesure montre que la pagination par `OFFSET` est
inutilisable sur les tailles de table réelles. Le second cas conduirait à une
pagination par clé (`WHERE clé > dernière valeur vue`), qui est plus rapide mais
suppose exactement ce que la présente décision exige déjà : un ordre unique.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Trier et filtrer dans la grille, en mémoire | Ne porterait que sur les lignes déjà lues : l'utilisateur croirait chercher dans la table et chercherait dans 200 lignes |
| Un constructeur de filtres structuré — colonne, opérateur, valeur liée | Plus sûr sur le papier, mais ce n'est pas ce que la maquette dessine, et le public d'Oxyn écrit du SQL toute la journée. Un constructeur l'obligerait à exprimer en trois menus ce qu'il tape en cinq secondes, et ne saurait pas dire `a IS NOT NULL AND (b > c)` |
| Laisser Oxyn composer un `WHERE` à partir de valeurs qu'il concatène | Là, oui, l'invariant s'applique : ce serait du SQL composé par le produit à partir de données reçues ([I-10](../../CLAUDE.md#i-10)) |
| Réutiliser `ReadResultPage` pour la page suivante | Elle relit un tampon déjà reçu ; elle ne contacte pas le serveur et ne peut donc pas rendre des lignes qui n'ont jamais été lues |
| Paginer sans ordre déterministe | `OFFSET` sans `ORDER BY` stable duplique et omet des lignes sans rien signaler — un résultat faux qui a l'air juste |
| Une seule capacité pour le tri et le filtre | Un moteur peut savoir ordonner sans savoir filtrer, et l'inverse ; un drapeau unique forcerait à refuser les deux pour n'en manquer qu'un |
