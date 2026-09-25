# ADR-0039 — Une session rend l'état de transaction qu'elle a constaté, et la console ne montre que celui-là

**Statut :** proposé · **Date :** 2026-09-25

**Précise :** [ADR-0003](0003-driver-capabilities.md), sur un point : une
capacité dit ce qu'une session **sait faire** ; rien ne disait encore ce qu'elle
**est en train de faire**. Cet ADR ajoute au trait `Session` une lecture d'état,
la seule dont la console ait besoin : une transaction est-elle ouverte ?

## Contexte

Aujourd'hui, un seul moteur peut garder une transaction ouverte dans une
console : **SQLite**. Sa session déclare `Capabilities::TRANSACTIONS` et
`MULTIPLE_STATEMENTS` (`drivers/oxyn-driver-sqlite/src/driver.rs`), et un
`BEGIN` tapé dans la console part tel quel. **PostgreSQL** ne la déclare pas :
une session s'appuie sur un bassin et emprunte une connexion par exécution, donc
le driver refuse `BEGIN`, `COMMIT`, `ROLLBACK` et leurs synonymes **avant
l'envoi** (`drivers/oxyn-driver-postgres/src/session.rs`, `TRANSACTIONS_REFUSED` ;
`transaction_text.rs`), plutôt que de laisser un `ROLLBACK` « réussir » sur une
autre connexion que celle de l'écriture. Les méthodes `Session::begin`, `commit`
et `rollback` n'ont, elles, aucun appelant hors des tests.

Une transaction SQLite ouverte dans une console ne se voit nulle part. Trois
conséquences, toutes silencieuses :

1. **La fermeture annule.** « Si un objet `sqlite3` est détruit pendant qu'une
   transaction est ouverte, la transaction est automatiquement annulée »
   ([`sqlite3_close`](https://www.sqlite.org/c3ref/close.html), vérifié le
   2026-09-25). Le dialogue de fermeture d'une console
   ([UX-SPEC](../UX-SPEC.md#consoles-indépendantes)) ne se déclenche que pour du
   SQL non sauvegardé ou une opération en cours : fermer un onglet dont le texte
   est sauvegardé jette sans un mot les écritures non validées.
2. **Le verrou reste posé.** Une transaction d'écriture ouverte garde le verrou
   d'écriture du fichier ; une **écriture** de la console voisine, qui a sa
   propre session ([ADR-0015](0015-consoles-independantes.md)), échoue sur
   `database is locked` — immédiatement, aucun `busy_timeout` n'étant posé —,
   sans que rien ne dise d'où vient le verrou.
3. **La fin peut être implicite.** « Si certaines erreurs surviennent sur une
   instruction d'une transaction multi-instructions (dont `SQLITE_FULL`,
   `SQLITE_IOERR`, `SQLITE_NOMEM`, `SQLITE_BUSY` et `SQLITE_INTERRUPT`), la
   transaction peut être annulée automatiquement. La seule façon de savoir si
   SQLite l'a fait est d'appeler cette fonction »
   ([`sqlite3_get_autocommit`](https://www.sqlite.org/c3ref/get_autocommit.html),
   vérifié le 2026-09-25). `SQLITE_INTERRUPT`, c'est le bouton **Stop**. Un
   affichage déduit du texte soumis — « on a vu passer `BEGIN` » — mentirait
   donc précisément après une annulation ou une erreur.

Deux faits du code contraignent **où** et **quand** l'état se lit :

* **Le thread de travail SQLite répond avant d'avoir fini.** Une tâche envoie sa
  réponse de l'intérieur (`worker.rs`, `call` ; `stream.rs`, écriture sans
  colonnes), puis le thread clôt la tâche (`Interrupter::end`). Sur Stop,
  `await_reply` rend `Cancelled` dès que le jeton se déclenche, pendant que le
  thread est encore dans `sqlite3_step` — c'est-à-dire **avant** l'annulation
  d'office qu'il va provoquer. Une valeur rangée par le thread et lue par
  l'exécuteur à ce moment-là est périmée ; pire, plus rien ne la republie. Le
  thread traite en revanche ses tâches **dans l'ordre de soumission** : une
  tâche soumise après une autre s'exécute après sa fin.
* **Toutes les fins d'exécution ne produisent pas d'événement terminal.** Dans
  `Executor` (`crates/oxyn-exec/src/executor.rs`), un échec de `slot.execute`
  rend l'erreur à l'appelant sans publier `Event::Failed` ; seul le drainage
  publie `Completed` ou `Failed`. Or une écriture SQLite s'exécute en entier
  pendant `slot.execute` : ses erreurs, `SQLITE_BUSY` compris, et le Stop
  pendant l'écriture passent par ce chemin. Enfin, un futur abandonné publie
  `Cancelled` depuis `AbandonGuard::drop` (`abandon.rs`), qui n'a pas la session.

`rusqlite` 0.37.0, la version de `Cargo.lock`, expose `Connection::is_autocommit`
(`src/lib.rs`, lu dans les sources installées le 2026-09-25). Côté PostgreSQL,
l'état est porté par chaque `ReadyForQuery` sous trois valeurs — `Idle`,
`Transaction`, `Error` —, mais `sqlx-postgres` 0.9.0 n'en expose rien :
`PgConnection::in_transaction` est `pub(crate)` et confond `Error` avec `Idle`
(`src/connection/mod.rs`). La méthode publique `Connection::is_in_transaction`
de `sqlx-core` 0.9.0 ne répond pas non plus : elle compte les transactions
ouvertes **par sqlx** (`transaction_depth`), et un `BEGIN` tapé ne la change pas.

La question vient du lot consoles (P30 de l'audit du 2026-09-24) ; l'utilisateur
a demandé le 2026-09-25 qu'elle passe par un ADR avant tout code, parce qu'elle
touche le trait de frontière `Session` — donc, à terme, l'interface WIT des
drivers en plugin ([PLUGIN-CONTRACT](../PLUGIN-CONTRACT.md#ce-que-ce-contrat-impose-aux-traits-daujourdhui)).

## Décision

### 1. Un type d'état dans `oxyn-core`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TransactionState {
    /// No transaction block: each statement commits on its own.
    Idle,
    /// A transaction block is open on this session.
    Open,
    /// The session does not know, or does not say.
    Unknown,
}
```

Il vit dans `oxyn-core` parce qu'un `Event` le porte (§ 3) et qu'`oxyn-core` ne
dépend pas d'`oxyn-driver`. Il ne porte aucune valeur de la base : le
`derive(Debug)` ne contrevient pas à [I-03](../../CLAUDE.md#i-03).

`Unknown` n'est **jamais** présenté comme `Idle`. C'est la règle du contexte de
session ([ADR-0019](0019-contexte-de-session.md)) : l'interface montre ce que
la session a constaté, pas ce qu'on suppose.

### 2. Une méthode asynchrone du trait `Session`, avec défaut

```rust
/// The transaction state, once every operation already submitted on this
/// session has ended.
async fn transaction_state(&self, cancel: &CancelToken) -> TransactionState {
    let _ = cancel;
    TransactionState::Unknown
}
```

* **Ordonnée après ce qui précède.** La méthode rend l'état constaté **après**
  la fin de toute opération déjà soumise à la session — exécution, `begin`,
  `commit`, `rollback`, `set_context` —, qu'elle ait réussi, échoué ou été
  interrompue. C'est ce qui rend la lecture juste après un Stop : l'annulation
  d'office est constatée, pas devancée.
* **Sans aller-retour réseau.** Elle attend la fin des opérations, elle
  n'interroge pas le serveur. Si `cancel` se déclenche avant, ou si la session
  ne peut plus répondre, elle rend `Unknown` : jamais une erreur, jamais `Idle`
  par défaut.
* **Appelée une fois le curseur lâché.** Le thread SQLite reste pris par un flux
  tant que son curseur vit (`stream.rs`) : une lecture soumise pendant ce temps
  attendrait sans borne. L'appelant draine ou lâche le curseur d'abord.
* **Jamais déduite du texte soumis.** La troisième conséquence du contexte
  l'interdit.
* **SQLite** soumet à son thread de travail une tâche qui lit
  `Connection::is_autocommit`. L'ordre de soumission du thread fait le reste :
  la tâche ne s'exécute qu'une fois l'exécution précédente revenue de
  `sqlite3_step`, interruption comprise. Le coût est un passage de fil local.
* **PostgreSQL** garde le défaut `Unknown` tant qu'il ne déclare pas
  `TRANSACTIONS`. Il n'y a pas de transaction manuelle à montrer, et le refus
  d'un `BEGIN` dit déjà que chaque instruction est validée seule.
* **Contrat.** Une session qui déclare `TRANSACTIONS` redéfinit la méthode.
  Aucune suite de contrat commune n'existe dans `oxyn-driver` ; la mise en œuvre
  ajoute donc à chaque driver qui déclare `TRANSACTIONS` — SQLite aujourd'hui —
  des tests qui vérifient `Idle` à l'ouverture, `Open` après un `BEGIN` exécuté
  et après `begin`, `Idle` après `COMMIT`, `ROLLBACK`, `commit` et `rollback`,
  et `Idle` après une interruption pendant une écriture qui a déclenché
  l'annulation d'office. [`revue-driver.md`](../../.claude/checklists/revue-driver.md)
  en fait une exigence pour tout driver qui déclare la capacité. Même logique
  que le garde de `begin` : ne pas savoir est acceptable, laisser croire ne l'est
  pas ([DRIVER-CONTRACT §5](../DRIVER-CONTRACT.md#5-il-déclare-ses-capacités-par-session-et-ne-simule-rien)).

### 3. L'exécuteur publie l'état sur toute fin d'exécution qu'il maîtrise

L'état appartient à la **session**, pas à la commande :
`Event::TransactionState { session, state }`. Le front range, pour chaque
session, la dernière valeur reçue ; il ne la déduit jamais de la réponse d'une
commande.

* **Toutes les sorties.** Pour une session qui déclare `TRANSACTIONS`,
  l'exécuteur appelle `transaction_state` à **chaque** sortie de l'exécution
  d'une instruction — succès, échec du drainage, échec anticipé de
  `slot.execute`, annulation —, en un point de sortie unique plutôt que branche
  par branche, et publie l'événement. C'est l'échec et l'annulation qui
  referment une transaction SQLite en silence : un seul chemin oublié, et c'est
  celui-là.
* **Avant le terminal.** Quand la sortie produit un événement terminal
  (`Completed`, `Failed`, `Cancelled`), l'état est publié **avant** lui :
  `Event::is_terminal` promet qu'après un terminal plus rien n'arrive pour cette
  exécution.
* **Avant `settle`, garde armée.** L'appel ajoute un `.await` sur le chemin de
  sortie. Il se fait **avant** `guard.settle()` : un futur abandonné pendant
  cette attente laisse alors `AbandonGuard` publier `Cancelled`. Placé après,
  l'abandon ne publierait ni `Cancelled` ni l'événement normal, et la console
  resterait « en cours ». La règle existante — aucun `.await` entre `settle` et
  l'envoi du terminal — vaut ici comme au point de sortie de `dispatch`.
* **Un jeton propre.** L'appel ne reçoit ni le jeton fils de l'exécution ni
  celui de l'onglet : déjà déclenchés après un Stop ou un délai dépassé, ils
  feraient rendre `Unknown` à chaque fois, et le dialogue de fermeture
  s'ouvrirait après chaque Stop. Il reçoit un jeton borné par la fermeture de la
  session.
* **L'abandon.** `AbandonGuard::drop` ne peut pas lire la session. Un
  `Cancelled` qui n'est précédé d'aucun `TransactionState` pour son exécution —
  les événements d'une exécution arrivent dans l'ordre — fait passer la session
  à `Unknown` côté front.
* **Le pont.** `ExecutionEventKind::of` (`crates/oxyn-desktop/src/ipc.rs`) se
  termine par un `_ => return None` qui jette en silence toute variante qu'il ne
  nomme pas : la nouvelle variante y est nommée, et le schéma de validation du
  front l'apprend dans le même commit
  ([ADR-0031](0031-validation-des-reponses-ipc.md)).

### 4. L'état initial vient de l'ouverture

`ConsoleSession` (`crates/oxyn-desktop/src/backend/consoles.rs`) porte l'état
lu par `transaction_state` à l'ouverture de la console. Une connexion SQLite
neuve est en autocommit, et cela se **constate** : la console démarre à `Idle`,
sans repère ni dialogue superflu. `console_session`, aujourd'hui synchrone,
devient asynchrone pour ses deux appelants.

### 5. La console le montre à côté de son contexte

* **`Open`** : la barre de la console porte `Transaction open`, en texte, à côté
  du sélecteur `<connexion> / <schéma>`. La couleur seule ne porte pas
  l'information ([UX-SPEC, repères permanents](../UX-SPEC.md#repères-permanents)) ;
  la pilule d'environnement reste où elle est, dans la barre supérieure.
* **`Unknown`** sur une session qui déclare `TRANSACTIONS` : `Transaction state
  unknown`. Rien ne dit « aucune transaction » sans que la session l'ait
  constaté.
* **`Idle`**, ou une session sans `TRANSACTIONS` : rien. Aucun repère éteint ne
  laisse croire la transaction possible là où elle ne l'est pas — même règle que
  le sélecteur de contexte.
* **Rien d'optimiste.** L'affichage change à l'événement, jamais à la soumission
  d'un `BEGIN` ou d'un `COMMIT`. Pendant une exécution, il garde la dernière
  valeur constatée.
* **Fermeture de la console.** Une console dont l'état est `Open`, ou `Unknown`
  sur une session qui déclare `TRANSACTIONS`, ne se ferme pas sans le dialogue
  existant. C'est un **changement** de ce dialogue : il nomme aujourd'hui la
  console ; il nomme alors aussi la connexion, et dit que la transaction
  ouverte sera **annulée**. Le focus reste sur `Cancel`.

Cet ADR n'ajoute **aucun** bouton `Commit` ni `Rollback` : ce seraient des
écritures émises par Oxyn, qui passeraient par le bus et le `PolicyGate`
([I-01](../../CLAUDE.md#i-01), [I-02](../../CLAUDE.md#i-02)) et méritent leur
propre décision. L'utilisateur valide ou annule en tapant `COMMIT` ou `ROLLBACK`,
comme aujourd'hui.

### 6. Ce que la mise en œuvre met à jour, dans le même commit

[ARCHITECTURE §4.1](../ARCHITECTURE.md) (la liste des méthodes du trait),
[DRIVER-CONTRACT §5](../DRIVER-CONTRACT.md) (la méthode et son contrat),
[UX-SPEC](../UX-SPEC.md) (le § 5 ci-dessus, dans « Consoles indépendantes » et
la barre de console) et `revue-driver.md`. Pas avant : tant que cet ADR est
`proposé`, ces documents décrivent le code tel qu'il est.

## Conséquences

* **+** Une transaction SQLite ouverte se voit dans sa console, et **fermer la
  console** ne la jette plus sans prévenir.
* **+** L'état affiché est celui que le moteur rapporte après la fin effective
  de l'opération, y compris après un Stop ou une erreur qui a refermé la
  transaction.
* **+** Le trait reste traversable par WIT : une fonction qui rend une
  énumération, sans rappel ni état partagé implicite
  ([PLUGIN-CONTRACT](../PLUGIN-CONTRACT.md#ce-que-ce-contrat-impose-aux-traits-daujourdhui)).
  Elle est asynchrone, comme `execute` déjà.
* **+** Le jour où PostgreSQL épingle une connexion par session et déclare
  `TRANSACTIONS` (le `TODO(phase 1)` de `variant.rs`), l'écran est prêt : seul
  le driver change.
* **−** Les autres chemins de fermeture annulent encore sans prévenir :
  `Command::Disconnect`, la libération d'un workspace de connexion, ⌘Q, et le
  Quit du menu macOS, que rien ne peut retenir
  ([ADR-0038](0038-un-plantage-s-annonce-une-fois.md)). Cet ADR ne couvre que la
  fermeture d'une console.
* **−** Une méthode de plus sur le trait de frontière, et un appel de plus à
  chaque fin d'exécution sur une session transactionnelle — un passage de fil
  pour SQLite. Un driver qui la redéfinit doit respecter l'ordre « après tout ce
  qui a été soumis » ; une valeur simplement rangée par le driver et lue sans
  attendre recréerait la course décrite au contexte.
* **−** L'état est celui de la **fin de la dernière opération**. Un futur driver
  réseau dont la connexion est tuée côté serveur resterait affiché `Open`
  jusqu'à l'exécution suivante.
* **−** Aujourd'hui, l'effort ne sert qu'à SQLite ; pour PostgreSQL, la
  fonctionnalité reste invisible jusqu'à l'épinglage.
* **−** L'état d'échec de PostgreSQL (`Error` dans `ReadyForQuery` : toute
  instruction est refusée jusqu'au `ROLLBACK`) n'a pas de variante. Il en
  faudra une, `Aborted`, quand PostgreSQL déclarera `TRANSACTIONS` ; le
  `#[non_exhaustive]` le permet côté Rust, mais `ipc.rs` et le schéma du front
  devront l'apprendre dans le même commit.

**Coût de sortie :** faible. Retirer la méthode, dont le défaut rend `Unknown`,
ne casse aucun driver ; retirer l'événement, le champ de `ConsoleSession` et le
repère touche `oxyn-exec`, `ipc.rs`, le schéma du front et un composant. Ce qui
borne ce coût : l'état ne sert à **aucune** décision de sécurité — le
`PolicyGate` ne le consulte pas.

**Reconsidérer si** PostgreSQL épingle une connexion par session : il faudra
alors ajouter `Aborted`, et vérifier que `sqlx` expose l'état de
`ReadyForQuery` — sinon, le driver devra le reconstruire, et l'argument
« jamais déduit du texte » se reposera. Reconsidérer aussi si le `PolicyGate`
doit un jour tenir compte d'une transaction ouverte (par exemple pour refuser
qu'un `Actor::Agent` écrive dans une transaction ouverte par l'utilisateur) :
l'état deviendrait une donnée de sécurité, et une valeur publiée pour
l'affichage ne suffirait plus. Reconsidérer enfin si les fermetures hors
console (déconnexion, workspace, ⌘Q) doivent être retenues à leur tour.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| **Méthode synchrone qui lit une valeur rangée par le driver** — la proposition initiale du lot consoles | Le thread SQLite répond avant de finir sa tâche, et Stop rend la main pendant `sqlite3_step` : la valeur lue à la fin d'une exécution est celle d'avant l'annulation d'office, et rien ne la republie. La rendre juste imposerait `Unknown` pendant toute tâche en cours, plus un second signal pour publier l'état final — la méthode asynchrone ordonnée obtient la même chose par la file du thread |
| **État porté par la réponse d'exécution** — un champ d'`ExecStats` rempli par `Cursor::stats`, donc dans `Event::Completed` | Ne couvre que le succès. `Failed` et `Cancelled` ne portent pas de statistiques, l'échec anticipé de `slot.execute` ne publie aucun événement, et ce sont précisément l'erreur et le Stop qui referment une transaction SQLite. `begin`, `commit` et `rollback` ne produisent pas de curseur. Enfin, `ExecStats` décrit le coût d'une exécution, pas l'état d'une session : l'y mettre ferait voyager l'état de la console dans l'historique et les résultats retenus, où il serait périmé à la lecture |
| **État porté par le résultat de la commande** — l'`Outcome` du succès et la réponse d'erreur IPC | L'erreur est une `OxynError` qui traverse toutes les crates : y joindre l'état d'une session mêle deux choses qu'un message d'erreur ne doit pas porter. L'abandon ne produit pas de résultat. Et l'état appartient à la session, qui survit à la commande : le lire dans la réponse d'une commande laisse la question de l'ordre entre réponses et événements, qui voyagent par deux canaux distincts |
| **Ne rien exposer tant que PostgreSQL refuse les transactions manuelles** | SQLite garde déjà des transactions ouvertes, et la fermeture d'une console les annule sans rien dire. Attendre PostgreSQL laisse ce défaut en place pour le seul moteur qu'il concerne aujourd'hui |
| **Déduire l'état du texte soumis**, côté front ou dans `oxyn-exec` | `sqlite3_get_autocommit` dit que seul l'appel au moteur révèle une annulation automatique après erreur ou interruption. L'affichage mentirait au moment où il compte |
| **Interroger le serveur** (une requête qui renvoie l'état) | Un aller-retour réseau après chaque exécution. Pour PostgreSQL sur bassin, la réponse porterait sur une connexion d'emprunt, pas sur la session ; pour SQLite, il n'y a pas de serveur, et le moteur donne la réponse sans requête |
| **Défaut `Idle` plutôt qu'`Unknown`** | Un driver qui n'a rien implémenté affirmerait qu'aucune transaction n'est ouverte. C'est le « faire semblant » que DRIVER-CONTRACT §5 interdit |
| **Montrer l'état pour toute session, `Unknown` compris** | Un repère « état inconnu » sur chaque console PostgreSQL, où aucune transaction manuelle n'est possible, n'apprend rien et habitue à ignorer le repère — celui qui comptera sur SQLite |
