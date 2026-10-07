<!-- oxyn-translation source="docs/adr/0054-bundle-sqlite-vec-in-the-sqlite-driver.md" sha256="49cbccb6f6b2" -->

> Traduction française de [docs/adr/0054-bundle-sqlite-vec-in-the-sqlite-driver.md](../../../../docs/adr/0054-bundle-sqlite-vec-in-the-sqlite-driver.md). **La version anglaise fait foi.**

# ADR-0054 — Embarquer sqlite-vec dans le driver SQLite, à activer par connexion

**Statut :** accepté (2026-10-07) · **Date :** 2026-10-06

## Contexte

Les outils vectoriels stockent leurs embeddings dans SQLite grâce à l'extension
sqlite-vec : une base d'utilisateur créée par `semantiq` contient `chunks_vec`,
une `CREATE VIRTUAL TABLE … USING vec0(…)`. L'ouvrir dans Oxyn échoue avec
`no such module: vec0`. Le moteur a besoin du module pour lire une table
virtuelle, pas seulement pour y écrire : la table est visible dans le catalogue
et on ne peut rien en lire. [VISION](../VISION.md) met les bases vectorielles
dans le périmètre.

Les extensions SQLite existent sous deux formes : une bibliothèque partagée
chargée à l'exécution (`load_extension`), ou du code C compilé dans le binaire
et enregistré sur la connexion. Les faits ci-dessous ont été vérifiés le
2026-10-06 et sont consignés dans
[RESEARCH-NOTES](../RESEARCH-NOTES.md#sqlite-vec--vérifié-le-2026-10-06) :

- la crate `sqlite-vec`, **0.1.9** (dernière stable, 2026-03-31), double
  licence MIT/Apache-2.0, acceptée par `deny.toml` ; elle livre l'amalgamation
  C et un `build.rs` qui la compile avec `cc` et `SQLITE_CORE`, si bien qu'elle
  appelle le SQLite 3.50.2 que le `bundled` de `rusqlite` 0.37 lie déjà — un
  seul moteur, pas deux ;
- 96 Kio de code et 1 Kio de données dans l'archive statique, 84 480 octets une
  fois liés dans le binaire release dépouillé, 8 s pour le build release de la
  crate sur un portable arm64, aucun drapeau SIMD (code scalaire portable) ;
- ses 42 `assert()` restent actifs en release : `build.rs` ne définit pas
  `NDEBUG`, et certains assertent des tailles lues dans les tables fantômes du
  fichier. Un fichier hostile qui en atteint un **interrompt le processus**,
  sans panique, avec le travail non sauvegardé perdu ;
- `rusqlite` 0.37 n'a aucune API sûre pour enregistrer une extension liée
  statiquement, et le workspace refuse `unsafe_code`
  ([SECURITY](../SECURITY.md#politique-unsafe)) : lever ce refus pour un module
  exige cet ADR.

## Décision

**sqlite-vec est compilée dans `oxyn-driver-sqlite`, et ne tourne que sur une
connexion où l'utilisateur l'a activée.** Le workspace épingle exactement
`sqlite-vec = "=0.1.9"` — l'`unsafe` ci-dessous repose sur sa signature C et
son `build.rs` — et `Cargo.lock` porte la somme de contrôle.

**L'interrupteur est un paramètre de connexion, désactivé par défaut.** Le
driver SQLite déclare un second champ, `sqlite_vec`
(`SqliteDriver::VECTOR_EXTENSION`), de genre `Bool`, valeur par défaut
`"false"`, libellé « Enable sqlite-vec (vec0 vector tables) ». Sa description
dit ce qu'il exécute et quand cela s'applique : « Runs the bundled sqlite-vec C
extension on this file. Enable it only for files you trust. A change applies
to sessions opened afterwards: disconnect to apply it to open ones. » Il est
persisté avec les autres paramètres non
secrets, en JSON lisible dans le workspace ([I-11](../../CLAUDE.md#i-11)) ; une
connexion enregistrée avant l'existence du champ n'a pas cette clé et se lit
désactivée. Seul `"true"` l'active ; une autre valeur non vide est une erreur de
configuration plutôt qu'une désactivation silencieuse. Un `Duplicate` ne le
copie pas : une copie est une nouvelle connexion, souvent vers un autre fichier.

**Enregistrée à l'ouverture, ou pas du tout.** Quand l'interrupteur est activé,
`worker::open` appelle `vector_extension::register` juste après
`sqlite3_open_v2`, avant le pragma `query_only`, pour toute session de la
connexion : lecture-écriture, lecture seule, aperçus, fichier et mémoire
partagée. `SELECT name FROM pragma_module_list` liste alors `vec0` et
`vec_each`. Quand il est désactivé, rien de sqlite-vec n'est enregistré : une
table `vec0` répond `no such module: vec0`, et aucune instruction sur une
session ouverte ainsi ne peut atteindre le code C de l'extension. Un
changement s'applique, comme tout autre paramètre de connexion, aux sessions
ouvertes après lui (`Backend::update_connection` n'en ferme aucune : une
console peut tenir une transaction ouverte). Les sessions déjà ouvertes gardent
ce avec quoi elles ont été ouvertes jusqu'à `Disconnect`, ce que la description
dit — désactiver l'interrupteur sur un fichier auquel on ne fait plus confiance
demande donc un `Disconnect` pour atteindre la session du catalogue et les
consoles ouvertes. Un échec d'enregistrement fait échouer
l'ouverture avec une erreur de driver **permanente** — elle échoue de même à
chaque tentative — portant le message de sqlite-vec, qui nomme une fonction ou
un module, jamais un chemin ni une valeur.

**Seul l'humain le change.** L'interrupteur décide si du code C embarqué
analyse un fichier : c'est de la configuration de connexion, modifiée dans le
formulaire de connexion. Aucun outil IA ne produit `CreateConnection` ni
`UpdateConnection`, et le `PolicyGate` refuse les deux à un `Actor::Agent` dans
tout environnement — un refus, pas une approbation
([I-07](../../CLAUDE.md#i-07), [I-02](../../CLAUDE.md#i-02)).

**Seule `sqlite3_vec_init` est appelée.** Elle enregistre les fonctions
scalaires `vec_*` et les modules `vec0` et `vec_each`.
`sqlite3_vec_numpy_init`, dont `vec_npy_file` lit n'importe quel fichier par
son chemin, n'est pas enregistrée : une base ou un agent pourrait sinon faire
lire au driver le fichier de son choix.

**L'enregistrement se fait par connexion, pas pour tout le processus.**
`sqlite3_auto_extension` équiperait aussi les connexions d'`oxyn-store` et
d'`oxyn-desktop`, qui n'en ont pas l'usage, et ne saurait pas suivre un
interrupteur par connexion.

**`unsafe` est autorisé dans une fonction et nulle part ailleurs.**
`drivers/oxyn-driver-sqlite/src/vector_extension.rs` porte l'unique
`#[allow(unsafe_code)]` de la crate, sur `register`, avec trois blocs, chacun
précédé de son `// SAFETY:` :

1. la crate amont déclare le point d'entrée comme `fn()` ; il est transmuté
   vers sa signature C, `int (*)(sqlite3*, char**, const sqlite3_api_routines*)`.
   La version épinglée le garantit ; une montée de version le relit ;
2. l'appel reçoit le handle vivant de la connexion empruntée, sur le thread
   worker qui la possède, un `pzErrMsg` valide (sqlite-vec l'écrit sans tester
   le pointeur nul) et un `pApi` nul, ignoré sous `SQLITE_CORE` ;
3. le message d'erreur, alloué par `sqlite3_mprintf`, est copié puis libéré une
   fois avec `sqlite3_free`.

**Ce que renvoie une lecture.** Une colonne vecteur est un BLOB pour SQLite —
`float[N]` vaut N `f32` petit-boutistes, `int8[N]` N octets, `bit[N]` N/8
octets — et la décision de type existante de
[`convert`](../../../../drivers/oxyn-driver-sqlite/src/convert.rs) en fait une
colonne Arrow `Binary` ([ADR-0002](0002-arrow-result-model.md)). Le driver ne
réinterprète pas les octets en liste de flottants : le blob ne porte pas son
type d'élément, et l'utilisateur qui veut du texte écrit
`vec_to_json(embedding)`. Rien n'est décodé en Rust, donc un vecteur malformé
ne peut atteindre aucune indexation de tranche ([I-09](../../CLAUDE.md#i-09)) ;
les contrôles de l'extension renvoient une erreur du moteur, que le driver
classe comme pour toute instruction. Les tests le prouvent pour un blob
tronqué, la mauvaise dimension, un texte qui n'est pas un vecteur, un vecteur
de requête KNN malformé et un chunk tronqué dans une table fantôme.

**Une requête KNN reste du SQL utilisateur.** `WHERE embedding MATCH ? AND k =
10` part tel qu'écrit ; la décision lecture/écriture reste celle du moteur
(`sqlite3_stmt_readonly`), qui la classe en lecture, et un `INSERT` dans une
table `vec0` en écriture. Aucun nouveau chemin ne contourne le command bus
([I-01](../../CLAUDE.md#i-01)).

**Une écriture arrêtée est ambiguë quel que soit son code.** sqlite-vec
transforme le `SQLITE_INTERRUPT` de ses instructions internes en simple
`SQLITE_ERROR` (« Could not find latest chunk »), si bien que le code de
résultat ne distingue plus une écriture arrêtée d'une écriture refusée.
`WorkerHandle::await_verdict` classe donc `Ambiguous` toute erreur de driver
d'une requête interrompue non déclarée en lecture seule
([I-13](../../CLAUDE.md#i-13)) — une lecture KNN envoyée avec des limites
d'écriture comprise, ce qui est imprécis mais jamais retenté. Le test du worker
`an_interrupted_write_failing_under_another_code_is_ambiguous` prouve la règle
de façon déterministe ; `a_stopped_write_into_vec0_is_ambiguous` reproduit le
cas sur l'extension.

Afficher les tables virtuelles et la disponibilité de leur module dans le
catalogue est un changement distinct ; cette décision rend seulement `vec0`
disponible là où l'utilisateur l'a demandé.

## Conséquences

* **+** Un fichier pour lequel l'utilisateur n'a rien activé ne peut jamais
  atteindre le code C de sqlite-vec : les risques d'arrêt et de sécurité
  mémoire ci-dessous sont cantonnés aux fichiers que l'utilisateur a choisi de
  croire, connexion par connexion.
* **+** Sur une telle connexion, une table `vec0` s'ouvre, s'aperçoit et répond
  aux requêtes KNN comme n'importe quelle table, sans rien installer.
* **+** Aucun code natif n'entre à l'exécution : ce qui tourne est ce qui a été
  construit, signé et livré.
* **+** La surface `unsafe` est une fonction d'une vingtaine de lignes, sans
  arithmétique de pointeurs ; le reste de la crate continue d'appeler du code
  sûr.
* **−** Une étape de plus pour l'utilisateur qui ouvre une base vectorielle :
  la première tentative échoue avec `no such module: vec0` — expliquer cette
  erreur relève du changement distinct du catalogue — et la lecture exige
  l'interrupteur, puis un `Disconnect` et un nouveau `Connect`.
* **−** L'interrupteur fait confiance à une connexion, pas strictement à un
  fichier : le module est enregistré sur le handle de la base, si bien qu'un
  fichier amené par `ATTACH` sur cette connexion atteint lui aussi sqlite-vec.
  `ATTACH` est du SQL utilisateur, et du DDL pour le `PolicyGate` quand un agent
  le propose (refusé en production, soumis à approbation ailleurs).
* **−** Désactiver l'interrupteur n'atteint pas les sessions déjà ouvertes :
  jusqu'à `Disconnect`, elles gardent sqlite-vec enregistrée. Les fermer à
  l'enregistrement a été rejeté plus bas.
* **−** 320 Ko de source C tiers tournent dans le processus d'Oxyn sur des
  données qui viennent du fichier ouvert, une fois l'interrupteur activé :
  `vec0` lit ses tables fantômes. Une erreur mémoire y échappe à Rust ; le
  modèle de menace de [SECURITY](../SECURITY.md) — l'attaquant, ce sont les
  données que l'utilisateur ouvre — s'y applique. Les tests de chunk corrompu
  couvrent les contrôles de taille des blobs `vectors` et `rowids`, pas
  l'extension.
* **−** Sur une connexion où l'interrupteur est activé, un fichier hostile qui
  atteint l'un des `assert()` actifs **interrompt le processus**. Un chemin est
  plausible et n'a pas été reproduit : `vec0_metadata_filter_text` reçoit un
  identifiant de chunk tronqué en `int` et ne fait qu'asserter la taille du
  blob `rowids` qu'il ouvre ensuite. Définir `NDEBUG` changerait cet arrêt en
  lecture non contrôlée : ce n'est pas un correctif.
* **−** L'affirmation « pas d'`unsafe` » du workspace a désormais une
  exception, que [SECURITY](../SECURITY.md#politique-unsafe) nomme.
* **−** sqlite-vec est en pré-1.0 et maintenue par une personne ; le format
  disque de `vec0` peut changer d'une version à l'autre, et une base écrite par
  une sqlite-vec plus récente peut ne pas s'ouvrir avec celle embarquée.
* **−** Les écritures dans une table `vec0` deviennent possibles aussi, avec
  les mêmes confirmations que toute écriture ([I-02](../../CLAUDE.md#i-02)).
* **−** 84 480 octets de plus dans l'`oxyn-desktop` release dépouillé (macOS
  arm64, mesuré contre le même arbre sans l'extension), et une compilation C à
  chaque build à froid (8 s, en parallèle du reste).

**Coût de sortie :** faible. Retirer la dépendance, `vector_extension.rs`, le
champ `sqlite_vec` et un appel dans `worker::open` ramène à l'état précédent ;
un paramètre `sqlite_vec` enregistré deviendrait alors une clé inconnue, et les
utilisateurs perdent la lecture des tables `vec0`. Rien d'autre de ce qu'ils
ont enregistré n'en dépend.

**À reconsidérer si** un avis de sécurité mémoire touche sqlite-vec sans
correctif rapide, ou si un `assert()` est montré atteignable depuis les données
du fichier ; si l'amont cesse de publier des versions stables pendant un an ;
si `rusqlite` gagne une API d'enregistrement sûre (l'`unsafe` disparaît
alors) ; si les asserts de sqlite-vec deviennent de vrais contrôles en amont,
ce qui rendrait la question d'une activation par défaut à nouveau légitime ;
ou si une deuxième extension est demandée — la question devient alors une liste
d'extensions validées, pas ce cas isolé.

## Alternatives rejetées

| Alternative | Raison du rejet |
|---|---|
| Enregistrer sqlite-vec sur **chaque** connexion qu'ouvre le driver (premier jet de cet ADR) | Tout fichier SQLite ouvert dans Oxyn — un téléchargement, un fichier déposé sur la fenêtre, une base partagée — atteindrait le code C de sqlite-vec à sa première lecture de `vec0`, et ses `assert()` actifs peuvent interrompre le processus sur un fichier hostile. Le coût de l'interrupteur est une case à cocher ; le coût de son absence est un plantage avec le travail non sauvegardé perdu, sur un fichier que l'utilisateur n'a jamais choisi de croire |
| Fermer les sessions ouvertes de la connexion à l'enregistrement de l'interrupteur | Une console peut tenir une transaction ouverte et des brouillons non enregistrés : la fermer annule le travail de l'utilisateur sur un changement de réglage, la perte même que l'interrupteur existe pour éviter. Aucun autre paramètre de connexion ne le fait ; `Disconnect` est l'acte explicite qui écrit d'abord les brouillons, puis ferme |
| L'activer automatiquement quand le fichier contient une table `vec0` | Le fichier déciderait lui-même que le code C tourne sur lui : exactement le cas hostile, un fichier construit pour atteindre un assert |
| Une préférence globale au lieu d'un interrupteur par connexion | La confiance porte sur un fichier, pas sur la machine : le réglage choisi pour son propre index vectoriel s'appliquerait à la prochaine base téléchargée |
| Charger un `.dylib`/`.so` désigné par l'utilisateur (`load_extension`) | Du code natif arbitraire exécuté avec les droits de l'utilisateur, depuis un chemin qu'une base partagée, un fichier de workspace ou un agent pourrait suggérer. Ni signature, ni revue, rien dont Oxyn puisse répondre — et la fonctionnalité `load_extension` resterait active sur chaque connexion |
| Charger l'extension depuis un fichier livré par Oxyn à côté du binaire | Le même code qu'en l'embarquant, plus un fichier à signer par plateforme et un chemin à protéger du remplacement ; rien de gagné |
| Compiler l'amalgamation C amont avec notre propre `build.rs` | Le même code, mais la validation, l'épinglage de version et le suivi de licence passent dans le dépôt au lieu de `Cargo.lock` et `cargo-deny` |
| `PRAGMA trusted_schema = OFF` sur chaque connexion, pour que les vues et déclencheurs du fichier ne puissent pas piloter `vec0` | sqlite-vec ne marque son module ni inoffensif ni direct seulement : une vue sur une table `vec0` — ce qu'un outil vectoriel écrit pour joindre les chunks à leur texte — cesserait de fonctionner. Le code C atteint par une vue est celui qu'atteint de toute façon un aperçu direct ; l'interrupteur cantonne déjà les deux aux fichiers de confiance |
| `sqlite3_auto_extension` pour tout le processus | Équipe des connexions qui n'en ont pas besoin (`oxyn-store`, `oxyn-desktop`), ne sait pas suivre un interrupteur par connexion, et fait dépendre l'enregistrement de l'ordre de première ouverture |
| Lire les tables `vec0` sans le module, par leurs tables fantômes | Duplique en Rust le format disque de l'extension, casse à son prochain changement, et ne sait toujours pas répondre à une requête KNN |
| Décoder les vecteurs en `List<Float32>` Arrow | Le blob ne dit pas son type d'élément (`float`, `int8`, `bit`) hors de la déclaration de la table ; deviner rendrait faux deux cas sur trois, et décoder mettrait de l'arithmétique de tranche sur des octets contrôlés par le fichier |
| La branche alpha 0.1.10 | Pré-version ; rien de ce qu'elle apporte n'est nécessaire pour lire des tables existantes |
