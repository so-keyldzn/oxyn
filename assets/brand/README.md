# Marque Oxyn

> **Autorité** : la géométrie du symbole, les jetons de couleur et le fichier à
> employer selon le contexte. Toute production d'un visuel Oxyn passe par ce
> document ; aucun symbole n'est redessiné à la main.

## Le symbole

**Binôme — une masse, une coupe.** Un bloc plein à courbure continue, entaillé
d'un seul trait de scie vertical qui ne traverse pas : la masse reste **une**
pièce, et le fragment à droite de l'entaille porte l'accent. L'utilisateur et
l'agent, même matière, côte à côte — ce que promet
[VISION](../../docs/VISION.md) : les agents travaillent *aux côtés* de
l'utilisateur, jamais à sa place.

La coupe ne traverse jamais. Une fente traversante produit deux panneaux
juxtaposés, c'est-à-dire le glyphe système `sidebar.trailing` (bouton
« afficher l'inspecteur », Split View) : à 32 et 16 px, le dessin est le même.
Le pont de matière sous l'entaille est ce qui distingue la marque d'un
contrôle d'interface.

## Géométrie

Canevas 1024, vue frontale, aucune rotation.

| Paramètre | Valeur |
|---|---|
| Masse | 672 centrée (176–848), soit 65,6 % du côté |
| Rayon extérieur | 168, raccord G1 sur les angles |
| Fente | largeur 64, à x 576–640 |
| Coupe | depuis le haut, arrêt à y 704 (79 % de la hauteur) |
| Fond de fente | demi-cercle r 32 |
| Congés d'embouchure | 44 |
| Pont | 144 (21 % de la hauteur) |
| Pièces | 400 / 208, soit 1,92 : 1 |

La fente est calée sur la grille de 64 pour tomber sur une colonne entière :
1 px à 16 px, 2 px à 32 px, 4 px à 64 px. **Ne pas déplacer la fente** : tout
décalage recrée des colonnes grises en dessous de 64 px.

Matière minimale 144, fente 64, tous rayons ≥ 32 — au-dessus des seuils que
demande macOS 26 pour le rendu en couches.

## Couleurs

| Jeton | Valeur | Emploi |
|---|---|---|
| Graphite | `#262422` → `#1C1B1A` | tuile, mode light |
| Graphite profond | `#1E1D1C` → `#151413` | tuile, mode dark |
| Os | `#EEEBEA` | glyphe sur tuile sombre |
| Oxyde | `#9A3B1E` → `#7F3416` | tuile de la palette D |
| Oxyde profond | `#7F3416` | glyphe sur tuile claire |
| **Accent** | **`#BF4C22`** | le fragment, partout |

Un seul accent pour tout le système. Il tient le rapport de contraste de 3:1
sur chaque tuile : 3,15 sur le haut du graphite light, 3,50 en bas, 3,42 et
3,74 en dark, et 4,14 contre l'os, ce qui garde les deux pièces distinctes.

La valeur précédente `#9A3B1E` ne tenait que 2,22:1 sur la tuile light, le mode
le plus courant : le binôme ne se lisait pas là où l'icône est le plus vue.

## Quel fichier employer

| Contexte | Fichier |
|---|---|
| Icône d'application macOS | `Oxyn.icns`, ou `svg/oxyn-appicon-light.svg` |
| Dock, App Store, vitrine | `svg/oxyn-appicon-D.svg` — mono, une seule matière |
| Mode Tinted de macOS 26 | `svg/oxyn-appicon-tinted.svg` |
| 16 et 32 px | `svg/oxyn-appicon-{16,32}.svg` — redessinés sur la grille pixel |
| Sur fond clair | `svg/oxyn-mark.svg` |
| Sur fond sombre | `svg/oxyn-mark-white.svg` |
| Site, documentation | `svg/oxyn-mark-accent.svg` |
| Favicon | `favicon.svg` |

Les tailles 16 et 32 px ne sont **pas** des réductions du 1024 : elles sont
redessinées sur la grille pixel. Une réduction du 1024 laisse une frange grise
d'un pixel sur le bord de la masse.

## Ce qui n'est jamais fait

La géométrie ne change pas : le `<path id="glyph">` est identique, à l'octet
près, dans les huit fichiers 1024 ; seules les couleurs varient. Le symbole
n'est ni incliné, ni déformé, ni posé sur une plaque à l'intérieur de la tuile.
Aucun effet n'est cuit dans le tracé — ombre, biseau, flou : macOS 26 les
fournit par couche.

## Régénérer

Les rendus dérivent des SVG de `svg/`. Toute reprise part de là, jamais d'un
PNG.

```bash
rsvg-convert -w 1024 -h 1024 svg/oxyn-appicon-light.svg -o png/oxyn-appicon-light-1024.png
iconutil -c icns Oxyn.iconset -o Oxyn.icns
```

`oxyn-sheet-corrige.png` est la planche de contrôle : les cinq tuiles, les
trois marques, les tailles réelles et les deux bandes Dock.
