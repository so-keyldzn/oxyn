<!-- oxyn-translation source="docs/adr/0048-simple-protocol-for-types-sqlx-cannot-prepare.md" sha256="fd73c91c13d8" -->

> Traduction française de [docs/adr/0048-simple-protocol-for-types-sqlx-cannot-prepare.md](../../../../docs/adr/0048-simple-protocol-for-types-sqlx-cannot-prepare.md). **La version anglaise fait foi.**

# ADR-0048 — Une instruction que sqlx ne sait pas préparer, ou dont le résultat n'a pas de forme binaire, passe par le protocole simple, en texte

**Statut :** accepté · **Date :** 2026-09-28 · **Décideurs :** Nicolas Boromée

## Contexte

Le driver PostgreSQL exécute chaque instruction de l'utilisateur par `sqlx`
0.9.0 (dernière version au 2026-09-28) en protocole **étendu** : Parse,
Describe, Bind avec toutes les colonnes du résultat en binaire, Execute. Trois
familles de types faisaient échouer **toute la requête**, vérifié sur
PostgreSQL 17.11 le 2026-09-28 :

* **Les multi-plages** (`int4multirange`… — 6 types, PostgreSQL 14+) et la
  catégorie interne `Z` (`pg_node_tree`, `pg_ndistinct`, `pg_dependencies`,
  `pg_mcv_list`, les résumés BRIN) : après Describe, `sqlx` interroge
  `pg_type` pour chaque OID qu'il ne connaît pas, et refuse `typtype = 'm'` et
  `typcategory = 'Z'` (`connection/resolve.rs`, `unknown type code 109`,
  `invalid category code 90`). Ni la 0.9.0 ni la branche `main` de `sqlx` ne
  les gèrent.
* **Les types sans fonction de sortie binaire** (`typsend = 0`) : `aclitem`,
  `gtsvector` et leurs tableaux. `sqlx` impose en dur le format binaire pour
  toutes les colonnes (`connection/executor.rs`), et le serveur refuse le
  Bind : `no binary output function available for type aclitem`.

Rien d'exotique : `SELECT * FROM pg_class` porte `relacl` (`aclitem[]`) et
`relpartbound` (`pg_node_tree`), et échouait.

## Décision

1. Quand `prepare` échoue sur la résolution de types de `sqlx`, ou quand le
   résultat décrit a une colonne dont le type n'a pas de sortie binaire
   (`aclitem`, `gtsvector`, leurs tableaux, un domaine sur eux), **et que la
   requête n'a aucun paramètre lié**, le driver exécute le même texte par
   `sqlx::raw_sql`, le protocole **simple**, où le serveur envoie toutes les
   valeurs en texte.
2. Les colonnes du résultat sont alors toutes `Utf8` : le texte imprimé par le
   serveur, marqué `oxyn:fallback = text`, avec le nom du type PostgreSQL (ou
   son OID quand `sqlx` n'a pas pu le résoudre) dans `oxyn:pg_type`. Le schéma
   est connu à la première ligne.
3. Avec des paramètres liés, le protocole simple ne peut pas les porter : le
   driver refuse par une erreur permanente qui nomme la cause et le remède
   (convertir la colonne en `text`).
4. Tout le reste de l'exécution est inchangé : même connexion, même
   `BEGIN READ ONLY` et même contexte de session, même borne de lignes, même
   délai, même annulation côté serveur et même réinitialisation de la
   connexion.

Deux faits rendent cela sûr :

* **Rien n'a été exécuté.** `prepare` n'envoie que Parse et Describe ;
  l'échec survient avant le Bind. Ce n'est pas le rejeu d'une erreur ambiguë
  ([I-13](../../CLAUDE.md#i-13)).
* **C'est une seule instruction.** Le Parse du serveur a accepté le texte, et
  Parse refuse plus d'une commande. Le protocole simple, qui en exécuterait
  plusieurs, reçoit un texte dont on a prouvé qu'il n'en contient qu'une.

## Conséquences

**Positives.** Tout type que le serveur peut envoyer atteint la grille :
multi-plages, requêtes sur le catalogue `pg_class`, `pg_rewrite`,
`pg_statistic_ext_data`.

**Négatives.**

* Dans le repli, toutes les colonnes sont du texte, `int4` compris : pas de tri
  typé ni d'export typé pour ce résultat. Le marquage le dit.
* Un résultat en repli **sans ligne** n'a pas de colonnes : le protocole simple
  donne la liste des colonnes avec les lignes, et `sqlx` ne l'expose pas
  autrement.
* Avec des paramètres liés, ces types échouent encore — avec un message clair
  désormais.
* `PgTypeInfo::kind()` de `sqlx` **panique** sur un type qu'il n'a pas résolu,
  ce que produit précisément le protocole simple : le chemin de repli ne doit
  jamais appeler le décodage typé (`decoding_for`). Un test le tient.

**Coût de sortie.** Faible : une branche dans `session.rs` et une variante de
source dans `cursor.rs`. La retirer ramène les échecs.

**À reconsidérer si** `sqlx` résout `typtype = 'm'` et `typcategory = 'Z'`
et laisse l'appelant choisir le format du résultat colonne par colonne : le
protocole étendu pourrait alors traiter les trois cas avec des colonnes
typées.

## Alternatives écartées

* **Forker `sqlx`** (`[patch.crates-io]`) pour accepter les deux codes et
  choisir le texte colonne par colonne. Corrige la cause, mais maintenir un
  fork de toute la bibliothèque cliente du driver pour trois familles de types
  coûte plus que le repli ; une contribution amont reste souhaitable.
* **Réessayer après l'erreur de Bind du serveur** pour `aclitem`. Dans une
  transaction de l'utilisateur, le Bind échoué l'interrompt : le nouvel essai
  échouerait à son tour, et l'utilisateur perdrait sa transaction. Détecter
  avant le Bind ne coûte rien.
* **Réécrire le SQL de l'utilisateur** (conversions `::text`). Le SQL que
  l'utilisateur écrit part tel quel : c'est la fonctionnalité
  ([I-10](../../CLAUDE.md#i-10)).
