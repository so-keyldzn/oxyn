---
paths:
  - "drivers/oxyn-driver-*/**"
  - "crates/oxyn-driver/**"
---

# Drivers — conventions

Le contrat fait autorité : [DRIVER-CONTRACT](../../docs/DRIVER-CONTRACT.md). Il
n'est pas résumé ici. Cette règle porte ce qui se rate à l'écriture.

## Avant d'écrire une ligne

Passer par [`/driver`](../commands/driver.md). La commande charge le contrat, la
liste de contrôle et la question qui vient avant toutes les autres : **est-ce un
nouveau protocole, ou un produit qui parle un protocole déjà implémenté ?**
Redshift ≡ PostgreSQL, MariaDB ≡ MySQL, OpenSearch ≡ Elasticsearch
([ADR-0003](../../docs/adr/0003-driver-capabilities.md)). Une crate en trop, ce
sont deux décodeurs de protocole à maintenir pour un dialecte.

## Les quatre pièges

**Le lot se dimensionne en octets, pas en lignes.** Mille lignes portant chacune
un BLOB d'un mégaoctet font un gigaoctet. Un `batch_size` en nombre de lignes
marche sur les tables de démonstration et déclenche l'OOM sur les vraies.

**L'annulation doit atteindre le serveur.** `pg_cancel_backend`, `KILL QUERY`,
`sqlite3_interrupt`. Abandonner le futur ne libère ni la connexion ni le verrou :
au dixième onglet fermé, la base refuse les connexions et l'utilisateur conclut
qu'Oxyn a cassé sa production. Un driver qui ne sait pas annuler côté serveur le
**déclare** dans ses capacités.

**Les capacités s'évaluent par session, pas par driver.** La version du serveur,
ses extensions et les droits du compte changent ce qui est disponible. Le même
driver PostgreSQL parle à une base 12 et à une base 17.

**L'erreur ambiguë ne se retente pas** ([I-13](../../CLAUDE.md#i-13)). Un délai
dépassé pendant une écriture n'est pas transitoire. C'est le cas le plus tentant
à traiter par une boucle de retry, et celui qui crée des doublons invisibles.

## Types

La table de correspondance va **dans les deux sens** et documente ses pertes.
Ce qui se perd le plus souvent, et le plus silencieusement :

- un `NUMERIC` PostgreSQL sans précision ne tient dans aucun type flottant ;
  le convertir en `f64` corrompt des montants ;
- un `timestamp` sans fuseau ne se voit **jamais** attribuer un fuseau à la
  lecture ([DRIVER-CONTRACT](../../docs/DRIVER-CONTRACT.md#7-il-traite-les-fuseaux-et-les-types-temporels-comme-des-données-pas-comme-du-texte)) ;
- un `u64` MySQL au-delà de 2^53 ne survit pas à un passage par un flottant ;
- un type inconnu se rend en octets bruts **avec son identifiant de type**,
  jamais en chaîne « best effort ».

## Interdits

| Interdit | Pourquoi |
|---|---|
| Dépendre d'`oxyn-desktop`, d'`oxyn-ai`, ou d'un autre driver | inverse le sens des dépendances. **`oxyn-core` est au contraire la dépendance attendue** — c'est le vocabulaire commun, et les deux drivers livrés en dépendent ([DRIVER-CONTRACT](../../docs/DRIVER-CONTRACT.md)) |
| Lire une variable d'environnement, écrire un fichier | un driver reçoit sa configuration |
| Retenter tout seul | la politique de reprise appartient à l'appelant, seul à savoir si l'opération est rejouable |
| `SET`/`USE` non déclaré | change en silence le sens des requêtes suivantes de l'utilisateur |
| Journaliser une valeur liée | [I-03](../../CLAUDE.md#i-03) |

## Vérifier

[Liste de contrôle](../checklists/revue-driver.md), puis `make qualite`.
Les deux tests qui ne se contournent pas : **l'annulation qui prouve l'arrêt
côté serveur**, et **le flux sur un volume qui ne tiendrait pas en mémoire**.
