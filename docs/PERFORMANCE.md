# Budgets de performance

> **Autorité** : les seuils chiffrés au-delà desquels un comportement est un
> défaut, et non une lenteur acceptable.

Invariants concernés : [I-05](../CLAUDE.md#i-05), [I-06](../CLAUDE.md#i-06).
Dérive de [ADR-0002](adr/0002-arrow-result-model.md) pour tout ce qui touche aux
résultats.

> **Statut des chiffres.** Les valeurs ci-dessous restent des **budgets
> décidés**, dérivés des seuils de perception humaine. Une première campagne de
> mesure a eu lieu le **2026-09-10** ; elle est consignée
> [plus bas](#campagne-de-mesure-du-2026-09-10) et n'a amendé aucun budget. Elle
> ne couvre que le code pur : la conversion ligne-à-lot du driver SQLite et
> l'analyse des requêtes. Une seconde campagne, le **2026-09-11**, a mesuré le
> démarrage à froid, le nœud de catalogue en cache et la mémoire au repos — ses
> chiffres sont dans [IMPLEMENTATION-PLAN](IMPLEMENTATION-PLAN.md) et reportés
> dans le tableau ci-dessous. **Le budget de trame, le démarrage à froid et
> l'application au repos ont été mesurés sur l'interface GPUI, retirée le
> 2026-09-18 ([ADR-0029](adr/0029-interface-tauri-shadcn.md)) : ils ne disent
> rien de la webview Tauri, qui n'a pas encore été mesurée** — voir
> [Confrontation aux budgets](#confrontation-aux-budgets). Une troisième mesure, le
> **2026-09-15**, a confronté le budget de mémoire à la mémoire du **processus**
> et non plus à la seule comptabilité du tampon ; elle est consignée
> [plus bas](#mesure-de-mémoire-du-2026-09-15). Un budget
> contredit par une mesure s'amende **par un ADR** — jamais en l'ajustant en
> silence pour faire passer un test.

## Pourquoi des budgets et pas des « bonnes pratiques »

« La latence perçue est une fonctionnalité » ([VISION](VISION.md)) ne veut rien
dire tant qu'aucun nombre ne permet de dire qu'on l'a ratée. Une régression de
performance qui n'a pas de seuil ne se détecte jamais : elle s'accumule par
tranches de 15 ms que personne ne remarque, jusqu'à ce que le produit soit
devenu lent sans qu'aucun commit ne soit coupable.

## Budgets d'interaction

Les seuils viennent de la perception : ~16 ms est la trame à 60 Hz, ~100 ms est
la limite de la réaction « instantanée », ~1 s est celle où l'attention décroche.

| Interaction | Budget | Ce qui se passe au-delà |
|---|---|---|
| Trame d'interface pendant une interaction continue (défilement, saisie, redimensionnement) | **8 ms** p99 | saccade visible ; c'est le symptôme n° 1 d'une violation de [I-05](../CLAUDE.md#i-05) |
| Retour visible après un clic ou une frappe | **100 ms** | l'utilisateur re-clique, croyant avoir raté |
| Premières lignes affichées après lancement d'une requête | **300 ms** après la première réponse du serveur | passé ce délai, l'utilisateur ne fait plus le lien entre son action et le résultat |
| Ouverture de la fenêtre au démarrage à froid | **1 s** | un client natif qui démarre plus lentement qu'un client web perd son argument principal |
| Développement d'un nœud du catalogue déjà en cache | **50 ms** | la navigation dans l'arborescence doit être ressentie comme locale |

Une opération qui ne peut pas tenir son budget ne le rate pas en silence : elle
affiche une progression et reste annulable. **Une opération longue et annulable
est acceptable ; une opération longue et figée ne l'est pas.**

## Budgets de mémoire

| Situation | Budget | Mode de panne |
|---|---|---|
| `ResultBuffer` en mémoire, par résultat | **256 Mo** par défaut, configurable ([ADR-0002](adr/0002-arrow-result-model.md)) ; au-delà, débordement en Arrow IPC relu hors thread UI ([ADR-0012](adr/0012-lecture-pages-resultats.md)) | [I-06](../CLAUDE.md#i-06) : `SELECT *` sur une grande table déclenche l'OOM killer, le processus meurt sans trace, l'utilisateur perd son travail |
| Défilement au-delà du budget mémoire | **une lecture de page disque**, jamais une nouvelle exécution | relancer la requête est doublement faux : le coût est arbitraire, et un `SELECT` peut ne pas être idempotent |
| Résultats conservés sans lecteur | 16 résultats, 256 Mio de lots résidents/cache et 1 Gio d'IPC cumulés ; contrôle après exécution et périodique ([ADR-0017](adr/0017-retention-resultats.md)) | accumulation de résultats inutilisés |
| Cache de catalogue par connexion | borné, avec éviction | dix connexions sur des bases à dizaines de milliers d'objets font grossir la RSS sans plafond |
| Conversations de l'assistant, relecture | **16 tours par page** : au pire 21 Mio lus du disque et 36,3 Mio décodés par appel, quel que soit le fil ; aucune lecture d'un fil d'un seul tenant. Détail du calcul sur `MAX_TURN_PAGE` (`oxyn-store`, 2026-09-16) | [I-06](../CLAUDE.md#i-06) : un fil relu en bloc et envoyé à l'interface alloue jusqu'à 1 Gio ; un fichier écrit par un tiers sans borne rendait l'allocation arbitraire |
| Conversations de l'assistant, relecture d'une branche | **16 échanges par page** : au pire 16,0 Mio lus du disque et 16,0 Mio décodés par appel — la question borne le calcul à elle seule (1 Mio), le reste de l'échange tient en 410 octets lus et 546 décodés. Une branche compte au plus 256 échanges, et les versions d'un échange se listent sans question ni réponse. Détail du calcul sur `MAX_EXCHANGE_PAGE` (`oxyn-store`, 2026-09-18) | [I-06](../CLAUDE.md#i-06) : un fil arborescent relu d'un seul tenant croît avec le nombre de régénérations, que rien ne borne côté appelant |
| Conversations de l'assistant, sur disque | 200 fils, 90 jours d'inactivité et 32 Mio de transcript par workspace, appliqués par `Conversations::prune` ; 512 tours par fil et `stop_reason` ≤ 256 octets tenus par le fichier. Mesure : un fil de 12 échanges pèse 64 Kio de transcript, 72 Kio de fichier (2026-09-16) | un historique d'assistant qui grossit sans fin sur une session de plusieurs mois |
| Définitions DDL en cache par connexion | 16 définitions et 16 Mio de SQL + notes ; éviction des anciennes valeurs, y compris invalidées ([ADR-0018](adr/0018-apercu-ddl.md)) | accumulation de scripts volumineux lors de la navigation |
| Application au repos, une connexion ouverte, aucune requête | stable dans le temps | une croissance au repos est une fuite ; elle se voit sur une session de plusieurs heures, pas dans les tests |

La rétention des lots initiaux et des pages décodées partage le budget du
résultat selon [ADR-0012](adr/0012-lecture-pages-resultats.md). L'index des lots,
les temporaires de décodage et les références tenues par des lecteurs doivent
être comptés dans les mesures du processus ; une assertion sur le cache ne
prouve pas la stabilité RSS.

## Ce qui se mesure, et comment

- **`criterion`** pour les bancs d'essai de code pur : analyse, formatage,
  conversion vers `RecordBatch`, diff de schéma. Ce sont les seules mesures
  reproductibles sur une machine de développement.
- **La conversion ligne-à-lot est un point chaud attendu**, pas une évidence :
  les drivers construits sur des pilotes ligne-à-ligne y passent par chaque
  valeur de chaque ligne ([DRIVER-CONTRACT](DRIVER-CONTRACT.md#3-il-produit-des-recordbatch-arrow-en-flux)).
  C'est le premier endroit à mesurer, et le dernier à optimiser sans mesure.
- **Les instruments du système** (Instruments, `perf`) pour le rendu et
  l'interface. Un banc `criterion` ne voit pas le rendu de la webview : il ne
  mesure rien d'utile sur l'interface.
- **Aucune mesure de latence contre une base réelle n'est un banc d'essai** : le
  réseau et l'état du serveur dominent le signal. Ce qui se mesure, c'est le
  temps passé **dans Oxyn**, pas le temps d'aller-retour.

Le protocole complet est dans [`/benchmark`](../.claude/commands/benchmark.md).

## Campagne de mesure du 2026-09-10

> Cette section consigne **ce qui a été mesuré, et ce qui ne l'a pas été**. Elle
> ne modifie aucun budget : aucun des budgets confrontés n'a été contredit.

### Conditions

| | |
|---|---|
| Machine | Apple M1 Max, 10 cœurs, 64 Gio, macOS 26.2 (25C56) |
| Chaîne | `rustc 1.98.1`, profil `bench` (`lto = "thin"`, `codegen-units = 1`) |
| Instrument | `criterion 0.8.2`, 50 échantillons, échantillonnage plat, 2 s de chauffe |
| Charge | aucune compilation concurrente — ni `rustc` ni `cargo` — et 55 à 71 % de temps CPU libre pendant la mesure |
| Reproductibilité | deux campagnes successives ; écart des médianes ≤ 4 % |

La **charge moyenne** de cette machine est structurellement au-dessus de 20
(session graphique nombreuse) sans que les cœurs soient pris : elle n'est pas un
indicateur exploitable ici, et c'est le temps CPU libre qui a servi de critère.
Il en résulte que **les chiffres valent à ±5 %**. C'est assez pour confronter des
budgets qui se comptent en millisecondes ; ce ne le serait pas pour arbitrer une
optimisation qui promettrait 3 %.

### Banc 1 — conversion ligne-à-lot, driver SQLite

`drivers/oxyn-driver-sqlite/benches/row_to_batch.rs`. Base **en mémoire**, table
à six colonnes (entier, flottant, texte court, texte long, BLOB de 64 octets,
entier à un `NULL` sur sept), remplie par une CTE récursive donc identique d'une
exécution à l'autre.

`ColumnBuilder` est `pub(crate)` et n'a **pas** été rendu public pour le banc.
Le coût de conversion est obtenu par **soustraction** entre deux parcours des
mêmes lignes : `oxyn` par l'API publique du driver, `rusqlite` sur une connexion
brute qui touche chaque valeur sans construire d'Arrow. **La différence est une
estimation, pas une mesure directe** ; elle contient aussi les allers-retours du
canal vers le thread porteur — un par lot, soit ~31 pour 250 000 lignes, donc
négligeable.

| Table | Lignes | Arrow produit | `oxyn` | `rusqlite` | Écart | Écart / valeur |
|---|---|---|---|---|---|---|
| mixte, 6 colonnes | 250 000 | 48,4 Mio | 54,7 ms | 34,6 ms | 20,1 ms | 13,4 ns |
| mixte, 6 colonnes | 1 000 000 | 186,7 Mio | 211,3 ms | 139,1 ms | 72,3 ms | 12,0 ns |
| `INTEGER` | 250 000 | 1,9 Mio | 14,0 ms | 8,7 ms | 5,3 ms | 21,1 ns |
| `REAL` | 250 000 | 1,9 Mio | 16,7 ms | 9,3 ms | 7,3 ms | 29,3 ns |
| `TEXT` court (~11 o) | 250 000 | 4,4 Mio | 17,3 ms | 10,1 ms | 7,2 ms | 29,0 ns |
| `TEXT` long (~80 o) | 250 000 | 20,8 Mio | 18,3 ms | 11,1 ms | 7,2 ms | 28,6 ns |
| `BLOB` 64 o | 250 000 | 17,4 Mio | 18,8 ms | 11,2 ms | 7,6 ms | 30,5 ns |
| `INTEGER` à `NULL` | 250 000 | 2,0 Mio | 14,2 ms | 9,2 ms | 5,0 ms | 20,0 ns |

Médianes. Les tables à une colonne sont une **décomposition indicative** : leur
empreinte est plus petite, et leur coût par valeur porte tout le coût fixe par
ligne au lieu de le partager entre six colonnes.

Ce que ces chiffres établissent :

- **211 ns par ligne de six colonnes**, soit **4,7 millions de lignes par
  seconde** et environ 880 Mio/s de tampons Arrow produits ;
- **le coût par ligne est identique à 250 000 et à 1 000 000 de lignes**
  (219 ns contre 211 ns). Le banc mesure donc l'algorithme et non le cache — à
  186,7 Mio, la table dépasse largement les 24 Mio de cache de niveau système de
  la machine ;
- **la conversion pèse 34 % du temps total du driver**, le reste étant
  l'itération de SQLite elle-même. La couche Oxyn coûte **1,5 fois** le
  parcours brut des mêmes lignes. C'est un point chaud réel, et ce n'est pas une
  pathologie ;
- le coût par valeur ne dépend presque pas de la longueur du texte (29,0 ns à
  11 octets, 28,6 ns à 80) : il est dominé par le travail **par valeur**, pas
  par la copie d'octets.

### Banc 2 — premier lot

Même fichier, groupe `first_batch` : le temps entre `execute` et le premier
`RecordBatch` disponible, c'est-à-dire ce qu'attend la grille avant de pouvoir
peindre. Le premier lot est le plus coûteux, puisque c'est celui pendant lequel
le driver résout le type des colonnes par une passe de sonde.

| Table | Lignes du premier lot | Mesure |
|---|---|---|
| mixte, 250 000 lignes | 8 192 | **2,60 ms** |
| mixte, 1 000 000 lignes | 8 192 | **2,57 ms** |

Le résultat ne dépend pas de la taille de la table : c'est ce que le flux
promet ([I-06](../CLAUDE.md#i-06)), et le banc le constate.

### Banc 3 — analyse des requêtes

`crates/oxyn-query/benches/analysis.rs`, dialecte PostgreSQL. Le texte est un
bloc réaliste — commentaires, identifiant cité, chaîne contenant un
point-virgule, corps `$body$`, une écriture — répété 1, 10 et 50 fois.

| Fonction | 4 instructions (820 o) | 40 instructions (8,2 Kio) | 200 instructions (41 Kio) |
|---|---|---|---|
| `split` | 1,13 µs | 11,5 µs | 55,4 µs |
| `words` | 1,71 µs | 14,6 µs | 66,5 µs |
| `current_statement` (curseur en fin de texte) | 1,44 µs | 11,5 µs | 56,4 µs |
| `format` | 2,23 µs | 22,0 µs | 108,7 µs |
| `classify` | 52,6 µs | 525,7 µs | 2,67 ms |

Tout est **linéaire** en taille de texte : aucun scanner quadratique. `classify`
coûte ~13 µs par instruction, deux ordres de grandeur au-dessus du découpage —
c'est l'analyse syntaxique de `sqlparser`, et c'est attendu.

> **Contrôle de non-régression du 2026-09-15.** Les trois bancs ont été rejoués
> après les correctifs de cette session — neuf défauts de code, dont plusieurs
> dans `oxyn-app` et `oxyn-core`. Les chiffres concordent avec ceux consignés
> ci-dessus : nœud de catalogue à 10 000 relations **6,55 µs** (contre 6,70),
> relecture d'un lot débordé de 8 192 lignes **35,7 µs** (contre 37,3),
> `classify` sur 200 instructions **2,69 ms** (contre 2,67). Les écarts sont
> ceux d'un `--quick` contre une campagne complète, pas un déplacement.
>
> Ce contrôle ne remplace aucune mesure ouverte : il dit seulement qu'aucun de
> ces trois chemins n'a régressé.

### Confrontation aux budgets

> **Trois verdicts portent sur l'interface retirée.** La trame, l'ouverture de
> fenêtre à froid et l'application au repos ont été mesurées le 2026-09-11 sur
> l'interface GPUI, retirée le 2026-09-18
> ([ADR-0029](adr/0029-interface-tauri-shadcn.md)). Ils ne disent rien de la
> webview Tauri, qui n'a pas encore été mesurée
> ([plan](IMPLEMENTATION-PLAN.md#migration-vers-linterface-tauri)). Les autres
> lignes mesurent le cœur, que le changement d'interface n'a pas touché.

| Budget | Verdict | Sur quoi |
|---|---|---|
| Premières lignes affichées — **300 ms** | **confirmé** pour SQLite | 2,6 ms pour le premier lot, quelle que soit la taille de la table : 0,9 % du budget |
| Retour visible après une frappe — **100 ms** | **confirmé pour la part `oxyn-query`** | 56 µs pour l'instruction courante sur un script de 200 instructions. La part interface n'est pas mesurée |
| Trame pendant une interaction — **8 ms** p99 | **tenu** (2026-09-11) : 0 à-coup au repos et à la saisie, 1 sous redimensionnement continu. La saisie en produisait 17 dont un de 50 ms avant [ADR-0024](adr/0024-autosauvegarde-au-repos-de-frappe.md) | un banc `criterion` sur l'interface ne mesure rien : il faut `xcrun xctrace record --template "Animation Hitches" --attach <pid>`, qui s'utilise sans interface graphique. La saisie **a** été couverte depuis, et c'est elle qui a révélé le dépassement corrigé par ADR-0024 : la phrase qui la disait « à couvrir » précédait la mesure. Ce qui reste non couvert est le **défilement d'une grille peuplée** — la relecture d'un lot débordé y coûte 4,5 µs (ligne suivante), mais l'enchaînement complet défilement + rendu n'a pas été observé sous instrument |
| Ouverture de fenêtre à froid — **1 s** | **235–274 ms** (2026-09-11, profil `dev`, trois lancements) | mesurée en horodatant entre le lancement du processus et le `window ready` du journal — aucun instrument spécialisé nécessaire |
| Nœud de catalogue en cache — **50 ms** | **6,70 µs** à 10 000 relations (2026-09-11) | `cargo bench -p oxyn-catalog --bench cached_node` ; la lecture du cache consomme six millionièmes du budget, le goulot d'un nœud lent est donc ailleurs |
| `ResultBuffer` — **256 Mo** puis débordement | **confirmé sur la RSS, à sa valeur réelle** (2026-09-15) : **2 Gio** poussés dans un tampon au budget par défaut de **256 Mo** font croître la RSS de **195 Mio** — sous le budget —, **1,84 Gio** partant sur disque | [mesure détaillée plus bas](#mesure-de-mémoire-du-2026-09-15). La mesure reste **manuelle** — l'automatiser demanderait une exception à [I-03](../CLAUDE.md#i-03) ou à `unsafe_code = "deny"`, arbitrage non tranché |
| Défilement = une lecture de page | **tenu** : **4,5 µs** pour un lot de 512 lignes, **37,3 µs** pour 8 192 (2026-09-14) | Deux moitiés, prouvées séparément. **Que ce soit une lecture** : `page_read_is_local_audited_and_scoped_for_both_actors` relit un lot débordé avec **aucun driver enregistré** — aucune réexécution ne peut s'y glisser. **Ce qu'elle coûte** : `cargo bench -p oxyn-data --bench spilled_page`, soit 0,06 % du budget de trame sur un lot ordinaire. *Réserve* : le fichier de débordement vient d'être écrit, donc le cache de pages du système le sert chaud. C'est le cas réel d'un défilement de va-et-vient ; une relecture après éviction coûterait davantage, et n'est pas mesurée ici |
| Application au repos, stable dans le temps | **82 Mio, stable sur une minute** (2026-09-11) | un signal, pas une preuve : une fuite lente se voit sur une session de plusieurs heures. L'ordre de grandeur, lui, est désormais connu |

**Aucun budget n'a été contredit. Aucun budget n'a été modifié.**

## Mesure de mémoire du 2026-09-15

Jusqu'ici le budget de mémoire n'était vérifié que par la **comptabilité interne
du tampon** — `resident_bytes()` — jamais contre la mémoire réellement tenue par
le processus. Une comptabilité peut être juste et le processus grossir quand
même : c'est exactement le mode de panne que [I-06](../CLAUDE.md#i-06) nomme.

### Conditions

`ResultBuffer` borné à un budget mémoire de **4 Mio**, `max_rows` levé,
débordement disque autorisé. **800 lots de 65 536 entiers `i64`** poussés, soit
**400 Mio** de données produites. Le budget est délibérément petit : ce qui est
éprouvé est le **mécanisme** de débordement, pas la valeur 256 Mo.

### Résultat

| Grandeur | Mesure |
|---|---|
| Croissance de la RSS du processus | **4,25 Mio** (4 456 448 octets) |
| Volume effectivement débordé sur disque | **404 Mio** (423 582 360 octets) |
| `resident_bytes()` en fin de course | sous le budget |

**Contrôle de sensibilité.** Le même scénario, avec un budget de 400 Mio au lieu
de 4 Mio, fait grimper la RSS de **317 Mio** et échouer l'assertion. La mesure
discrimine donc d'un facteur ~75 : elle n'est pas un test qui passe quoi qu'il
arrive.

**Ce que cela établit :** [I-06](../CLAUDE.md#i-06) tient sur la mémoire du
**processus**, et pas seulement sur la comptabilité du tampon. 400 Mio traversent
un tampon de 4 Mio sans que le processus grossisse de plus que son budget.

### La même mesure au budget réel

Le petit budget éprouve le mécanisme ; il ne dit rien de la valeur que le produit
emploie. La mesure a donc été refaite **au budget par défaut**, celui que ce
document pose :

| Grandeur | Mesure |
|---|---|
| Budget du tampon | **256 Mo** (`DEFAULT_MEMORY_BUDGET`, 268 435 456 octets) |
| Données poussées | **2 Gio** (4 096 lots de 65 536 `i64`, 2 147 483 648 octets) |
| Croissance de la RSS du processus | **195 Mio** (204 324 864 octets) — **sous le budget** |
| Volume effectivement débordé sur disque | **1,84 Gio** (1 978 316 104 octets) |
| Durée | 2,1 s |

Huit fois le budget traverse le tampon, et le processus croît de **9,1 %** de ce
qui l'a traversé — moins que le budget lui-même. Le chiffre de 256 Mo n'est donc
plus seulement un budget déclaré : il est **tenu, et mesuré comme tel**.

### Pourquoi ce n'est pas un test permanent

Honnêtement : parce que l'automatiser demanderait une exception à une règle posée
pour un invariant.

- Lire la RSS demande `ps`, donc `std::process::Command::new`, que `clippy.toml`
  interdit au nom de [I-03](../CLAUDE.md#i-03) — un processus enfant hérite de
  l'environnement du parent, secrets compris.
- L'alternative, un allocateur instrumenté, demande `unsafe`, que le workspace
  refuse (`unsafe_code = "deny"`).

**Arbitrage non tranché.** Faut-il une exception étroite à l'un des deux pour
gagner une vérification permanente de [I-06](../CLAUDE.md#i-06), ou la mesure
manuelle et datée suffit-elle ? Ce document ne tranche pas : une exception à une
règle d'invariant se décide par un ADR, pas dans un tableau de mesures.

## La règle qui empêche l'optimisation gratuite

**On ne remplace pas du code clair par du code rapide sans la mesure qui montre
que ça valait la peine.** Un banc d'essai avant, un banc d'essai après, le
chiffre dans le message de commit. Sans ça, la complexité est payée d'avance et
le gain est supposé.

Le corollaire vaut aussi dans l'autre sens : un `.clone()` sur un chemin appelé
une fois par ouverture de fenêtre n'est pas un problème de performance, et le
transformer en emprunt qui contamine cinq signatures est une perte nette.
