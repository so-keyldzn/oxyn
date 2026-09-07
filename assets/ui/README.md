# Ressources visuelles de l’interface

Exports exacts du [design Figma Oxyn](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv?node-id=13-291),
vérifiés le 2026-09-07 sur les écrans sombre `13:291` et clair `14:603`.
Les octets téléchargés sont conservés sans réécriture. Les nœuds sources,
dimensions et sommes SHA-256 sont consignés dans [provenance.json](provenance.json).

| Fichier | Glyphe source | Boîte SVG et GPUI |
|---|---|---|
| database.svg | Hugeicons database-02 | 16 × 16 px |
| table.svg | Hugeicons grid-table | 16 × 16 px |
| search.svg | Hugeicons search-01 | 16 × 16 px |
| plus.svg | Hugeicons add-01 | 16 × 16 px |
| panel.svg | Hugeicons sidebar-left | 16 × 16 px |
| settings.svg | Hugeicons settings-01 | 16 × 16 px |
| book.svg | Hugeicons book-open-01 | 16 × 16 px |
| terminal.svg | Hugeicons code | 16 × 16 px |
| history.svg | Hugeicons transaction-history | 16 × 16 px |
| folder.svg | Hugeicons folder-01 | 16 × 16 px |
| chevron.svg | Hugeicons arrow-right-01 | 16 × 16 px |
| down.svg | Hugeicons arrow-down-01 | 16 × 16 px |
| logo.svg | Oxyn official mark | 32 × 32 px |

## Rendu

`IconName` et `icon` se trouvent dans `crates/oxyn-ui/src/icons.rs`.
`Application::with_assets(UiAssets)` installe les fichiers inclus à la compilation ;
aucune lecture disque ni réseau n’a lieu dans le rendu ou dans la source d’assets.
L’appelant fournit la couleur du thème avec `text_color` : GPUI rasterise le SVG
en masque alpha. Les mêmes contours conviennent ainsi aux deux apparences.
Les marges intérieures des exports sont conservées, notamment les 5,5 px autour
de la masse du logo dans sa boîte de 32 px. Aucun recadrage ni redessin.

## Sources et droits

Les douze icônes appartiennent à **Hugeicons Stroke Rounded**, selon les
annotations des composants Figma. Leur dépôt source est
[hugeicons/hugeicons-static](https://github.com/hugeicons/hugeicons-static/tree/f9dbcca8d72cc2777a0ccd873c274d9bf7a153e6).
La [copie du README amont](HUGEICONS-UPSTREAM-README.txt) conserve la notice
d’utilisation consultée : utilisation telle quelle, sans autorisation de modifier
ou redistribuer le jeu d’icônes. Aucune licence MIT n’est attribuée à ces fichiers.
Ils sont intégrés comme ressources de l’interface Oxyn, pas comme bibliothèque
d’icônes indépendante. Les modalités de redistribution du dépôt restent à
vérifier avant une publication ; ce lot ne publie rien.

Le logo est la marque du projet Oxyn, exportée depuis Figma ; les règles de
géométrie restent celles de [la marque](../brand/README.md).

La police et sa licence sont décrites dans [assets/fonts](../fonts/README.md).
