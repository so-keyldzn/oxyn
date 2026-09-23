---
name: piege-edit-sequence-u-decodee
description: Edit/Write décodent une séquence `\uXXXX` écrite dans le texte — un `r#"… …"#` devient un vrai U+2028 dans le fichier
metadata:
  type: feedback
---

Une séquence `\u` suivie de quatre chiffres hexadécimaux, écrite dans `old_string`/`new_string`/`content`, arrive **décodée** dans le fichier : `r#""a b""#` devient une chaîne brute contenant le vrai caractère U+2028 (invisible, il ressemble à une espace). `\n`, `\"` passent tels quels ; seul `\u` est touché. Les formes `'\u{2028}'` (avec accolades) passent intactes.

**Why:** constaté en écrivant un test d'échappement de U+2028 dans `oxyn-ai` : l'assertion attendait ` ` échappé et comparait en fait au caractère brut. Rien ne l'a signalé, le test échouait sans raison visible.

**How to apply:** pour attendre une séquence `\uXXXX` dans un test Rust, la construire par une fonction (`format!("\\u{code:04x}")`) plutôt que de l'écrire dans un littéral brut. Après une édition qui en contient, vérifier par un script Python qui cherche `' ' in ligne`, pas par `grep` (qui ne voit rien).
