---
name: piege-annulation-fenetre-deterministe
description: sqlite3_interrupt vise la connexion et son drapeau s'efface au démarrage d'instruction ; pg_cancel_backend vise un processus réutilisé par le bassin — et comment rendre ces fenêtres reproductibles en test
metadata:
  type: reference
---

Relevés le 2026-09-16 en corrigeant deux annulations mal ciblées.

## SQLite (source 3.50.2 embarquée par libsqlite3-sys 0.35.0)

`sqlite3_interrupt` pose `db->u1.isInterrupted` sur la **connexion**. Il est remis
à 0 dans `sqlite3Step` et `sqlite3RunParser` seulement si `nVdbeActive == 0`.
Donc : une interruption qui arrive pendant le `step` de la requête **suivante**
la tue ; une interruption posée entre deux instructions ou pendant une
préparation **se perd**. Il faut vérifier sous verrou que la tâche visée est
bien celle en cours, et garder un drapeau propre à la tâche.

Test déterministe : une tâche qui lit **une** ligne (instruction active), signale,
attend un feu vert sur un canal std, puis lit le reste. L'état « instruction
active » est alors garanti au moment où le test interrompt.

## PostgreSQL

`pg_cancel_backend(pid)` vise un processus ; le bassin sqlx rend la même
connexion — même pid — à l'emprunt suivant. Seul qui **tient** la connexion peut
annuler sans risque. `PoolConnection` revient au bassin de façon asynchrone
(tâche lancée au drop) : quelques ms.

Outils de test qui ont marché :
* **verrous consultatifs** tenus par une connexion de contrôle
  (`pg_advisory_xact_lock($1)` dans la requête testée), attente sur
  `pg_locks WHERE locktype='advisory' AND NOT granted AND classid=0 AND objid=($1::bigint)::oid` ;
* bassin de **deux** connexions : une seule inactive ⇒ sqlx la rend à coup sûr ;
* `pg_stat_activity.wait_event = 'ClientWrite'` prouve que le client ne lit plus
  sa socket (contre-pression côté tâche de flux) ;
* une assertion « le processus a disparu » est propre à une conception qui ferme
  la connexion : pour une preuve valable dans les deux sens d'une mutation,
  préférer « `state <> 'active'` ».

## Bloquer `execute` entre le `SET` et la préparation (2026-09-17)

Préparer (Parse) prend `ACCESS SHARE` sur les tables citées : une connexion de
contrôle en `BEGIN; LOCK TABLE t IN ACCESS EXCLUSIVE MODE` bloque `execute`
**après** `SET search_path` et `BEGIN READ ONLY`, visible dans `pg_locks`
(`NOT granted`, jointure `pg_class`). `SET` et `BEGIN` eux-mêmes ne bloquent
jamais : pas de barrière serveur pour leur propre aller-retour.

sqlx-core 0.9.0 : `PoolConnection::close_on_drop` ne se relève pas ; `Drop` sans
ce drapeau rend la connexion telle quelle. Pour « fermer sauf si remise au
défaut », une garde qui ne l'appelle que dans son propre `Drop`.

Protocole : `ROLLBACK`/`COMMIT` hors transaction ne sont **pas** des erreurs —
`WARNING: there is no transaction in progress`, invisible côté sqlx. Sur un
bassin, `BEGIN; INSERT; ROLLBACK` exécutés séparément « réussissent » et
l'`INSERT` reste validé (vérifié 17.11, 2026-09-17). Un texte ouvre un bloc de
transaction seulement par `BEGIN` ou `START TRANSACTION` en tête ; sqlx-postgres
0.9.0 garde `in_transaction` en `pub(crate)`.

Piège de mutation : un `SET` correctif posé par le curseur après chaque
exécution **masque** l'absence d'un réglage posé à l'ouverture. Tester le réglage
d'ouverture sur une session neuve, avant toute exécution inscriptible.

Voir [[outil-cluster-postgres-jetable]].
