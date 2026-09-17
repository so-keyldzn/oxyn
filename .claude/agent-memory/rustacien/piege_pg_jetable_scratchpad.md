---
name: piege-pg-jetable-scratchpad
description: Lancer un PostgreSQL jetable pour vérifier un comportement du serveur — socket Unix trop long dans le scratchpad, psql inutilisable pour tester le protocole étendu sur plusieurs commandes
metadata:
  type: feedback
---

Pour vérifier un comportement réel du serveur, Postgres.app fournit `initdb` et
`pg_ctl`, dans `/Applications/Postgres.app/Contents/Versions/latest/bin`.

**Premier piège : le socket Unix.** Le chemin du scratchpad de session dépasse
103 octets, et `pg_ctl start` meurt avec « Unix-domain socket path is too long ».
Il faut lancer le serveur en TCP seul :
`-o "-p 5499 -k '' -c listen_addresses=127.0.0.1"`.

**Second piège : `psql` ne sait pas tester le protocole étendu sur un texte à
plusieurs commandes.** Son lexer, `psqlscan.l`, découpe au `;` avant l'envoi.
`\bind \g` ne voit donc jamais `SELECT 1; DROP …` en un seul message Parse.
Un client filaire minimal en Python (`socket` et `struct` ; Startup, puis
Parse/Bind/Execute/Sync en authentification `trust`) fait l'affaire en une
cinquantaine de lignes.

**Why:** ces deux pièges m'ont coûté deux essais pendant la vérification de la
fin de commentaire `--` au `\r`.

**How to apply:** dès qu'une question porte sur ce que le serveur exécute
réellement, notamment la différence entre protocole simple et étendu. Penser à
arrêter le serveur avec `pg_ctl stop -m fast`.
