# ADR-0035 — Les écritures locales de l'ordonnanceur passent par le pool bloquant, en opérations possédées

**Statut :** accepté · **Date :** 2026-09-24

**Précise :** [ADR-0012](0012-lecture-pages-resultats.md), qui déplace la lecture
de pages hors rendu par le même geste (`spawn_blocking`, pas de runtime propre) —
cette décision l'étend aux écritures ; [ADR-0004](0004-command-bus.md), sur la
séquence « journaliser avant et après », dont le déroulement change ici sans que
la séquence elle-même soit remise en cause.

## Contexte

`Executor::local_worker` (`crates/oxyn-exec/src/executor.rs:1541`) soumet déjà
les lectures — pages de résultat, historique, connexions — au pool bloquant de
Tokio de l'application : `tokio::runtime::Handle::try_current()`, jamais
`Runtime::new`, puis `spawn_blocking(move || …).await`, l'échec de jointure
mappé en `OxynError::Internal`. C'est le geste qu'[ADR-0012](0012-lecture-pages-resultats.md)
a établi pour les lectures.

Un TODO daté du 2026-09-10 (`executor.rs:43`) marquait ce qui restait à faire :
les accès au Store *locaux* qui ne sont pas des lectures de page — sauvegarde et
suppression d'une connexion, lecture de sa configuration avant connexion,
résolution des identifiants dans le trousseau (`self.credentials.resolve`), et
l'écriture d'export (`export_result`, qui fait `File::create` puis `export(...)`
en synchrone) — ainsi que les écritures d'audit — décision de politique
(`journal_decision`), issue d'exécution (`journal_result`), début et fin
d'historique (`history_start`, `history_finish`) — tournaient toutes en ligne
sur le worker qui exécute `Executor::dispatch`. C'est le worker partagé du
runtime Tokio multi-thread décrit dans
[ARCHITECTURE §9](../ARCHITECTURE.md#9-modèle-dexécution-et-de-threads) : le
même qui porte les drivers, le réseau et les appels LLM. Une écriture
synchrone qui y bloque retarde toute autre commande en vol sur ce fil, ce
qu'[I-05](../../CLAUDE.md#i-05) interdit.

Le déplacement n'est pas mécanique pour les écritures d'audit. `Executor::run`
(`executor.rs:552-574`) arme un `OutcomeGuard` (`crates/oxyn-exec/src/abandon.rs`)
juste avant d'exécuter la commande et appelle `guard.settle()`
(`executor.rs:569`) **avant** `journal_result` et `history_finish`
(`executor.rs:571-572`). Le guard existe précisément pour qu'un futur abandonné
entre l'exécution et l'écriture de son issue — fermeture de fenêtre, `select!`
perdant, arrêt du runtime — laisse quand même une trace au journal, classée
`Ambiguous` ([I-13](../../CLAUDE.md#i-13)) plutôt qu'un trou silencieux. Faire de
`journal_result`/`history_finish` un simple `spawn_blocking(...).await` sans y
penser aurait déplacé la fenêtre d'abandon *après* `settle()`, là où plus rien
ne la couvre. La même question se pose, de façon moins grave, pour
`journal_decision` sur une décision `Allow` : entre l'autorisation rendue par
`self.policy.authorize` et l'écriture de la décision, un abandon ne doit
laisser ni une commande exécutée sans trace, ni une trace pour une commande
jamais exécutée.

`Executor` ne construit pas de runtime : il n'utilise que le pool bloquant que
`main.rs` confie à Tauri ([ARCHITECTURE §9](../ARCHITECTURE.md#9-modèle-dexécution-et-de-threads)).
Un appel à `spawn_blocking` suppose donc un runtime Tokio courant — absent, par
exemple, lors de l'arrêt de l'application. `journal_abandoned_off_runtime`
(`executor.rs:1999-2020`) traite déjà ce cas : `Handle::try_current()` réussi
part sur `spawn_blocking`, sinon l'écriture se fait en ligne, faute de fil de
pool à qui la confier. C'est le précédent que les écritures d'audit reprennent.

## Décision

Chaque écriture locale que le worker de dispatch exécutait jusqu'ici en ligne
devient une **opération possédée** : ses données sont construites en mémoire
sur le worker — clonées ou déplacées, jamais empruntées à `&self` ni à la pile
de `dispatch` — puis exécutées par `spawn_blocking(move || …).await`, avec le
même mappage d'échec de jointure que `local_worker` déjà établi par
[ADR-0012](0012-lecture-pages-resultats.md). Sont concernées : `save_connection`,
`delete_connection`, `connection_config` (donc `connect`), `export_result`, et
les quatre écritures d'audit — `journal_decision`, `journal_result`,
`history_start`, `history_finish`.

**Le cache des connexions suit le disque dans la même opération.** Le registre
`connections` de l'ordonnanceur fournit l'environnement soumis au `PolicyGate`
([I-02](../../CLAUDE.md#i-02)). `save_connection`, `delete_connection` et
`connection_config` le mettent donc à jour **dans la tâche du pool**, juste
après l'écriture ou la lecture du Store, sous un verrou dédié
(`connection_writes`) que prennent aussi `register_connection`,
`forget_connection` et `load_connections`. Mis à jour au retour de l'`.await`,
le cache pourrait garder `development` quand le disque dit `production` : deux
mises à jour concurrentes y écrivent dans un autre ordre que sur le disque, et
un futur abandonné après l'écriture ne le met jamais à jour. Les lecteurs du
cache ne prennent pas ce verrou.

**Sans runtime Tokio courant**, une opération de commande (connexion, export)
échoue par une erreur de configuration, comme `local_worker` le fait déjà ; une
écriture d'audit s'exécute en ligne, comme `journal_abandoned_off_runtime` le
fait déjà — l'audit ne doit jamais se perdre faute de pool, même à l'arrêt.

Pour une décision `Allow`, l'`OutcomeGuard` est armé dès que
`self.policy.authorize` rend `Allow`, **avant** l'écriture de la décision, sans
`.await` entre les deux : aucun abandon ne peut donc survenir entre
l'autorisation et l'armement du guard. La décision `Allow` et l'inscription « en
cours » à l'historique forment une seule opération possédée, soumise au pool
avant toute exécution.

Le refus (`Deny`) et l'inscription de son historique forment une opération.
Les deux écritures qu'elle contient restent tentées indépendamment — l'échec de
l'une n'empêche pas l'autre, comme aujourd'hui.

La décision `RequireApproval` est une opération à part : `approvals.submit` et
l'événement `Event::ApprovalRequested` ne partent qu'après son succès.

L'issue d'une exécution — `journal_result` et `history_finish` — forme une
seule opération, soumise au pool **immédiatement après** `guard.settle()`, sans
`.await` intercalé : la fenêtre qu'`OutcomeGuard` couvre reste donc exactement
celle qu'elle couvre aujourd'hui, entre le début de l'exécution et la mise en
file de cette opération.

Chaque écriture est soumise à `spawn_blocking` avant tout point de suspension
qui la suit dans le code appelant. Aucune tâche n'est détachée : la poignée
(`JoinHandle`) est toujours attendue par un `.await` du chemin normal.

La résolution des identifiants dans `connect()` (`self.credentials.resolve`,
trousseau système) suit le même geste : `Arc::clone(&self.credentials)` et un
clone possédé de la configuration de connexion partent dans la fermeture. Les
`Credentials` qu'elle rend ne sont jamais journalisés, ni formatés en `Debug`
([I-03](../../CLAUDE.md#i-03)) — le filtrage déjà en place avant l'écriture
Store (`executor.rs:1634-1637`) reste inchangé par ce déplacement.

## Conséquences

* **+** Le worker qui exécute `Executor::dispatch` ne bloque plus sur aucune
  I/O locale : le TODO du 2026-09-10 est résolu dans son intégralité, y compris
  les écritures d'audit qu'il ne nommait pas littéralement.
* **+** Le geste est celui, déjà revu et en production, d'[ADR-0012](0012-lecture-pages-resultats.md) :
  aucun nouveau patron à apprendre, aucune nouvelle classe d'erreur.
* **+** Aucune tâche détachée : chaque opération garde sa poignée jusqu'à son
  `.await`, conformément à [rust.md](../../.claude/rules/rust.md#async).
* **−** (a) Un abandon pendant l'écriture de la décision `Allow` laisse une
  issue « abandonnée, issue inconnue » (`Ambiguous`, jamais retentée), même si
  rien n'a été exécuté. C'est la vérité vue de l'ordonnanceur — il ne peut pas
  savoir, à cet instant, s'il a été abandonné avant ou après avoir atteint le
  serveur — jamais un trou dans le journal.
* **−** (b) Un abandon du futur de `dispatch_as` pendant l'écriture d'une
  décision `RequireApproval` laisse au journal une ligne `RequireApproval` sans
  demande en attente : `approvals.submit` n'est jamais atteint,
  `Event::ApprovalRequested` n'est jamais publié, et personne ne peut
  l'approuver. Rien n'est exécuté et rien n'est ambigu — I-13 n'est pas en jeu,
  puisqu'aucune commande n'a pu atteindre le serveur sans une demande en
  attente pour la porter. L'état visible est celui d'une demande rejetée
  (`Executor::reject`) ou expirée (`PendingApprovals::sweep`), qu'aucune ligne
  de journal ne suit déjà aujourd'hui. C'est une limite assumée, pas un trou
  d'issue.
* **−** (c) L'ordre d'insertion des lignes du journal peut différer de leur
  `ts` : plusieurs fils du pool bloquant écrivent, sérialisés seulement par le
  verrou interne du Store, pas par l'ordre d'émission.
* **−** (d) Une opération du pool pas encore démarrée au moment où le runtime
  s'arrête peut ne jamais tourner — situation déjà vraie de tout usage de
  `spawn_blocking`, désormais plus fréquente puisque plus d'écritures l'empruntent.
* **−** (e) Chaque commande fait deux allers au pool bloquant de plus (décision,
  issue), sans chiffre de latence mesuré à ce jour. Toute optimisation future
  qui regrouperait ces allers doit s'appuyer sur une mesure, pas une supposition
  ([PERFORMANCE](../PERFORMANCE.md#la-règle-qui-empêche-loptimisation-gratuite)).
* **−** Restent en ligne sur le worker, hors périmètre de cette décision :
  l'éviction des résultats retenus après une exécution (`prune_results`, qui
  supprime des fichiers de débordement) et `load_connections`, appelé à
  l'assemblage du backend, avant que la fenêtre n'existe.

**Coût de sortie :** modéré, confiné à `crates/oxyn-exec/src/executor.rs` et
`crates/oxyn-exec/src/abandon.rs` — aucun schéma, aucune nouvelle `Command`,
aucun IPC touché. Revenir aux écritures en ligne rouvre I-05 sur l'ensemble des
gestionnaires concernés.

**Reconsidérer si**
* une mesure montre que les allers au pool dominent la latence des petites
  commandes ;
* l'ordre d'insertion du journal devient une exigence (nécessiterait un
  écrivain d'audit dédié, à file ordonnée) ;
* une demande d'accord perdue sur abandon (conséquence b) devient un problème
  observé, appelant une réconciliation des décisions `RequireApproval` sans
  suite ;
* le Store cesse d'être synchrone ;
* une écriture du cache des connexions apparaît hors du verrou
  `connection_writes` : c'est la fenêtre d'I-02 qui se rouvre, pas une
  simplification.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Garder les écritures d'audit en ligne sur le worker de dispatch (option A) | Ne résout pas le TODO ni I-05 pour la partie la plus fréquente des écritures : chaque commande journalise au moins deux fois. |
| Un fil écrivain dédié, alimenté par un canal | Résoudrait la conséquence (c) sur l'ordre, mais introduit une file, un protocole d'arrêt propre et une latence de livraison que rien ici ne justifie sans mesure préalable. |
| `tokio::spawn` détaché pour les écritures | Personne ne tient la poignée : viole [rust.md §Async](../../.claude/rules/rust.md#async) et fait disparaître silencieusement une écriture si le runtime s'arrête avant qu'elle ne tourne. |
| Un `spawn_blocking` par écriture élémentaire, y compris décision + historique séparés | Multiplie les allers au pool sans réduire la fenêtre d'abandon utile ; regrouper en une opération par étape logique (décision+historique « en cours », issue+historique « fin ») ne coûte rien de plus et réduit le nombre d'allers. |
| Écrire la décision `RequireApproval` en ligne, pour garder une fenêtre d'abandon nulle à cet endroit | Rouvre I-05 sur ce seul cas, pour une fenêtre déjà couverte par un état visible et sans conséquence ambiguë (b). |
| Mettre la demande d'approbation en attente *avant* d'écrire sa décision, en la retirant en cas d'échec d'écriture | Une approbation concurrente entre les deux ferait exécuter une commande dont la décision ne s'est jamais écrite au journal. |
