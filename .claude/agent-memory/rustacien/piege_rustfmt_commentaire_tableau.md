---
name: piege-rustfmt-commentaire-tableau
description: rustfmt recolle un commentaire d'une ligne placé avant un élément de tableau court en fin de ligne de l'élément précédent — le commentaire change de propriétaire
metadata:
  type: feedback
---

Dans un tableau d'éléments courts (`["PATH", "HOME", …]`), rustfmt passe en
disposition « mixte » : un commentaire `//` **d'une seule ligne** posé au-dessus
d'un élément est remonté en fin de ligne de l'élément **précédent**
(`"LANG", // Where a launcher unpacks…` au lieu d'annoncer `"TMPDIR"`). Un
commentaire de fin de ligne sur le dernier élément fait pire : rustfmt fusionne
les deux derniers éléments sur une ligne.

**Why:** c'est ainsi qu'un commentaire de `ALWAYS_PASSED` (spawn.rs) s'est
retrouvé accroché à la mauvaise variable, et qu'une revue l'a signalé. Rien
n'échoue : le format est « correct », le sens est faux.

**How to apply:** tout commentaire au-dessus d'un élément de tableau court tient
sur **deux lignes au moins** (rustfmt ne déplace pas un bloc multi-ligne).
Vérifier avec `rustfmt --edition 2024 --check <fichier>` après l'édition, pas
seulement à la fin.
