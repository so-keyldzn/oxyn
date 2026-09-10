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
| check.svg | Indicateur de case cochée, Recovery `232:9100` (2026-09-10) | 16 × 16 px |
| down.svg | Hugeicons arrow-down-01 | 16 × 16 px |
| logo-dark.svg | Brand / Oxyn mark, sombre bicolore | 32 × 32 px |
| logo-light.svg | Brand / Oxyn mark, clair bicolore | 32 × 32 px |

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


## Actualisation de la palette — 2026-09-07

Lecture fraîche des cadres [sombre 13:291](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv?node-id=13-291)
et [clair 14:603](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv?node-id=14-603),
avec `get_design_context`, `get_variable_defs`, puis lecture seule des alias de
la collection `Oxyn / Colors` (`VariableCollectionId:5:3`). Modes `Dark 5:1`
et `Light 5:2`. La palette précédente zinc est remplacée par graphite/os.
La date UTC précise, les identifiants de variables et les changements détaillés
sont conservés dans `palette_refresh` de [provenance.json](provenance.json).

Les anciennes valeurs ci-dessous sont celles du lot GPUI précédent dans ce
worktree, avant cette actualisation. Les identifiants de source sont ceux du
fichier Figma indiqué ci-dessus.

| Rôle GPUI | Sombre : ancien → nouveau | Clair : ancien → nouveau | Source exacte |
|---|---|---|---|
| `background` | `#09090b` → `#151413` | `#ffffff` → `#eeebea` | `background` · `VariableID:7:18` |
| `surface` | `#18181b` → `#1c1b1a` | `#fafafa` → `#f7f5f3` | `surface` · `VariableID:7:19` |
| `surface_raised` | `#27272a` → `#262422` | `#f4f4f5` → `#e3deda` | `surface_raised` · `VariableID:7:20` |
| `border` | `#3f3f46` → `#443c37` | `#e4e4e7` → `#d5ccc6` | `border` · `VariableID:7:21` |
| `border_focus` | `#fafafa` → `#bf4c22` | `#18181b` → `#bf4c22` | `ring` · `VariableID:127:1138` |
| `text` | `#fafafa` → `#eeebea` | `#18181b` → `#1c1b1a` | `text` · `VariableID:7:22` |
| `text_muted` | `#a1a1aa` → `#b9ada5` | `#71717a` → `#6e625c` | `text_muted` · `VariableID:7:23` |
| `text_faint` | `#71717a` → `#b9ada5` | `#71717a` → `#6e625c` | `text_muted` · `VariableID:7:23` |
| `text_on_accent` | `#18181b` → `#ffffff` | inchangé (`#ffffff`) | `primary_foreground` · `VariableID:127:1137` |
| `accent` | `#fafafa` → `#bf4c22` | `#18181b` → `#bf4c22` | `accent` · `VariableID:7:24` |
| `selection` | `#27272a` → `#262422` | `#f4f4f5` → `#e3deda` | `surface_raised` · `VariableID:7:20` |
| `hover` | `#27272a` → `#262422` | `#f4f4f5` → `#e3deda` | `surface_raised` · `VariableID:7:20` |
| `grid_header` | `#27272a` → `#262422` | `#f4f4f5` → `#e3deda` | `surface_raised` · `VariableID:7:20` |
| `grid_gutter` | `#18181b` → `#1c1b1a` | `#fafafa` → `#f7f5f3` | `surface` · `VariableID:7:19` |
| `grid_line` | `#3f3f46` → `#443c37` | `#e4e4e7` → `#d5ccc6` | `border` · `VariableID:7:21` |
| `null` | `#71717a` → `#b9ada5` | `#71717a` → `#6e625c` | `text_muted` · `VariableID:7:23` |
| `scrim` | `#09090b` → `#151413` (α 0.72) | `#18181b` → `#1c1b1a` (α 0.4) | `background` sombre / `text` clair ; opacité antérieure conservée |
| `grid_stripe` | `#ffffff` → `#eeebea` (α 0.02) | `#18181b` → `#1c1b1a` (α 0.025) | `text` · `VariableID:7:22` |

`hover` et `selection` reprennent le fond `surface_raised` des variantes
`Hover 11:53` et `Selected 11:61` du composant `SidebarMenuButton 11:85`.
Les rôles GPUI sans variable Figma dédiée restent explicitement des adaptations :
`grid_header` → `surface_raised`, `grid_gutter` → `surface`, `grid_line` → `border`,
`grid_stripe` → `text` à l’opacité antérieure ; `text_faint` et `null` →
`text_muted`. Les couleurs d’environnement `success`, `warning`, `danger` sont
inchangées et ont aussi été vérifiées dans la collection. Aucun contour ni
octet des treize SVG existants n’a changé lors de cette actualisation.

### Contradictions et limites observées

- Le code de référence de `WorkspaceContext` dans le cadre clair contient des
  valeurs de repli sombres. Les variables effectives du cadre et sa capture sont
  claires ; elles ont priorité pour la palette GPUI.
- Le composant `Focus 11:69` utilise `text_muted` en bordure tandis que la
  collection définit `ring = #bf4c22`. Le focus global GPUI reprend le jeton
  sémantique `ring`, cohérent avec l’accent.
- Le petit texte secondaire clair `#6e625c` sur `surface_raised #e3deda` donne
  **4,412:1**, sous 4,5:1 : défaut de contraste hérité des valeurs exactes de la
  maquette. Ces valeurs sont conservées et signalées au coordinateur ; une vue
  peut utiliser le texte principal sur une sélection pour éviter ce couple.
- Le logo Figma est désormais le composant `Brand / Oxyn mark 149:22199`, avec
  fragment accent permanent `#bf4c22`. L’extension de périmètre demandée par le
  coordinateur ajoute ses deux exports exacts et `logo(ThemeMode) -> gpui::Img`.

Contrôles numériques : focus sur les trois fonds ≥ **3,146:1** en sombre et
≥ **3,681:1** en clair ; blanc sur accent **4,916:1** ; texte principal ≥
**12,879:1** ; texte secondaire hors le couple signalé ≥ **4,967:1**.
Les métriques, Geist et la sémantique des environnements sont préservées.

### Intégration et contrôles de l’actualisation

Exporter `icons::logo` depuis `lib.rs`, puis remplacer `icon(IconName::Logo)`
par `logo(theme.mode)` aux emplacements de marque. Les Hugeicons continuent
d’utiliser `icon(...).text_color(...)`. Le nouveau logo emploie explicitement
`ImageSource::Resource(Resource::Embedded)` et `UiAssets`, sans accès disque ni
réseau. Ce chemin GPUI conserve les couleurs, convertit les pixels RGBA
prémultipliés en BGRA et rasterise à 2× ; la branche `Image::from_bytes(Svg)`
de GPUI épinglé omet cette conversion dans `Image::to_image_data`.

Les deux exports bicolores gardent la boîte 32 × 32 px, le `viewBox`, les marges
intérieures et les deux tracés identiques entre modes ; le tracé `glyph` est
aussi identique à celui du logo précédent. Les deux rendus ont été inspectés
visuellement après rasterisation ; les empreintes et nœuds des instances sont
consignés dans `palette_refresh.logo_assets`.

Contrôles exécutés : cinq tests existants du thème passent avec
`cargo test -p oxyn-ui theme::tests --locked` ; compilation isolée finale de
`theme.rs` et `icons.rs` contre GPUI avec `rustc --emit=metadata -D warnings`
réussie ; `rustfmt --check`, `git diff --check` et `make socle` réussis
(41/41 cas, avertissement existant de règle tests sans chemin correspondant).
Cargo signale seulement les avertissements amont de compatibilité future de
`block` et `proc-macro-error2`.

La comparaison des jetons, les contrastes et les empreintes des treize SVG
antérieurs ont été vérifiés ; les deux nouveaux logos passent XML, dimensions,
couleurs, identité des tracés et rasterisation. Le couple de contraste à 4,412:1
reste une limite explicite. Aucun contrôle visuel GPUI intégré ni `make qualite`
complet n’est revendiqué par ce worker : ils appartiennent au lot du coordinateur.
