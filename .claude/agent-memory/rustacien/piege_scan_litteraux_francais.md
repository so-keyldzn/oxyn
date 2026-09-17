---
name: piege-scan-litteraux-francais
description: Un grep ligne à ligne rate deux tiers des messages français d'un crate — continuations `\` et chaînes courtes sans accent
metadata:
  type: feedback
---

Pour recenser du texte destiné à l'utilisateur dans du Rust, un `grep` par ligne
sur les accents **ne suffit pas**. Deux angles morts, chacun m'a fait rater des
messages :

- une chaîne coupée par une continuation `\` en fin de ligne : le littéral
  s'étend sur plusieurs lignes, la regex `"..."` d'une seule ligne ne le voit
  pas. Il faut lire le fichier entier en `re.S` ;
- une chaîne courte **sans accent** : `"nom trop long"`, `"le genre est vide"`,
  `"{} lignes / {} octets"`. Il faut une seconde passe sur un lexique de mots
  français fréquents, pas seulement sur `[àâçéèêë…]`.

**Why:** les deux passes ont trouvé chacune une quinzaine de messages que
l'autre manquait ; s'en tenir à la première laisse du français en production
sans que rien n'échoue.

**How to apply:** pour tout inventaire de littéraux (traduction, fuite de
secret, audit de message), écrire le scan au scratchpad avec les deux passes,
couper au premier `\n#[cfg(test)]\nmod ` et **ne pas filtrer** les résultats
multi-lignes à l'affichage — c'est en les masquant que je les ai perdus une
première fois. Vérifier ensuite par `cargo test`, qui débusque les tests
comparant un message.
