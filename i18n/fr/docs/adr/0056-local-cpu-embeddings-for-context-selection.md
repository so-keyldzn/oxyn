<!-- oxyn-translation source="docs/adr/0056-local-cpu-embeddings-for-context-selection.md" sha256="3484cd5b6214" -->

> Traduction française de [docs/adr/0056-local-cpu-embeddings-for-context-selection.md](../../../../docs/adr/0056-local-cpu-embeddings-for-context-selection.md). **La version anglaise fait foi.**

# ADR-0056 — Des embeddings locaux sur CPU classent le contexte IA, le lexical d'abord

**Statut :** proposé · **Date :** 2026-10-07

## Contexte

Avant qu'une question atteigne un modèle, `ContextBuilder::build` garde au
plus 24 relations du catalogue
([AI-PROVIDERS](../AI-PROVIDERS.md#ce-que-lia-voit-du-schéma-et-quand-cest-lu)).
Sa première étape, la sélection, est `oxyn_catalog::search()` : un score
lexical sur les noms, les champs et les commentaires, puis l'ordre des chemins
quand aucun terme ne correspond. Une question posée en français sur des tables
nommées en anglais (« quels clients ont commandé hier ? » face à `customers`
et `orders`) ne correspond à rien, et les 24 relations gardées sont les 24
premières dans l'ordre alphabétique. Sur une base de 5 000 tables, c'est tout
l'échec que nomme l'[ADR-0006](0006-ai-privacy-tiers.md) : un élagage faux
produit des requêtes fausses.

Classer par le sens demande des embeddings de texte. L'utilisateur qui l'a
demandé n'a ni Ollama ni clé d'API : un endpoint d'embeddings derrière
`OpenAiCompatibleProvider` ne marcherait pas pour lui, ni pour quiconque n'a
rien installé à côté d'Oxyn. Les embeddings doivent donc être calculés dans le
processus, sur le CPU, sans rien à installer.

Un spike l'a mesuré le 2026-10-07 (Apple Silicon arm64, 10 cœurs, build
release ; consigné dans
[RESEARCH-NOTES](../RESEARCH-NOTES.md#embeddings-locaux--vérifié-le-2026-10-07)) :

- le modèle `ibm-granite/granite-embedding-97m-multilingual-r2` (Apache-2.0,
  ModernBERT, 97 441 152 paramètres en bf16, multilingue, publié le
  2026-04-20) se convertit depuis son export ONNX avec burn-onnx 0.22.0
  **sans opérateur manquant**, et sa sortie sous Burn égale celle
  d'onnxruntime à un écart maximal de 4,2e-7 (cosinus 1,0) ;
- **latence** environ 14 ms pour une phrase, environ 440 ms pour 256 noms de
  tables ;
- **mémoire** : RSS maximal de 1,15 Go en chargeant le safetensors publié,
  environ 750 Mo en chargeant un fichier `.bpk` de burn-store en f32 ; le
  tokenizer chargé seul, 286 Mo ;
- **qualité** : cosinus entre une question et la table pertinente de 0,909 en
  anglais et 0,865 en français, contre 0,773 au plus pour les tables non
  pertinentes. L'écart est d'environ 0,1 : assez pour réordonner, pas assez
  pour s'y fier seul ;
- **build** : 198 crates, aucune `-sys`, toutes sous des licences que
  `deny.toml` accepte déjà — la seule MPL-2.0, `colored`, comprise. Une
  compilation à froid de la crate d'inférence prend 80 s de temps réel, dont
  45 s de burn-flex sur le chemin critique ; le binaire de démonstration pèse
  11,7 Mo, 8,6 Mo dépouillé ;
- le tokenizer pur Rust (`tokenizers` avec `fancy-regex`) donne les mêmes
  identifiants que la build Oniguruma par défaut sur 17 cas multilingues.

La crate `oxyn-embed`, mesurée le même jour sur la même machine en release,
confirme ces ordres de grandeur : un chargement à froid en 0,67 s (hachage de
415 Mo, mappage, échauffement), une question en 18 ms, 256 noms de tables en
0,63 s (environ 3,5 fois plus lent en profil dev : 2,2 s) ; **795 Mo** une
fois chargé ; le téléchargement réel et la conversion en 14 s, **avec un pic à
1,45 Go**.

## Décision

**Oxyn calcule des embeddings de texte localement, sur le CPU, dans une
nouvelle crate `oxyn-embed`, et ne s'en sert que pour classer les relations
que garde le contexte IA : le lexical d'abord, le sémantique pour départager
et pour ordonner ce que le score lexical manque.** L'option est désactivée
par défaut.

### La crate et sa place

`crates/oxyn-embed` porte un seul sujet : un modèle d'embeddings épinglé — son
téléchargement, sa vérification, sa conversion et son inférence sur CPU. Parmi
les crates d'Oxyn, elle ne dépend que d'`oxyn-core` (pour `CancelToken`). Sa
surface publique : `ModelStore` (le répertoire du modèle : `status`,
`download`, `remove`), `Embedder` (`load`, `embed`), `OnDemandEmbedder`
(chargé à la première demande, libéré après inactivité), `Embedding` (384
`f32`, normalisés L2), `cosine`, et les constantes de `pinned`.

**`oxyn-desktop` est la seule crate qui dépend d'`oxyn-embed`.** `oxyn-ai`
n'en dépend pas : `ContextBuilder` reçoit des scores par relation — une table
de `CatalogPath` vers `f32`, sans aucun texte — calculés par `oxyn-desktop`.
La passerelle d'[I-04](../../CLAUDE.md#i-04) reste le seul endroit qui
rend un contexte, et un score n'apporte aucun mot à un prompt. Il décide en
revanche **quelles** relations sont rendues, et peut en augmenter le nombre —
voir les règles de classement plus bas. `oxyn-ai`, ses tests et le chemin d'un agent
externe continuent de compiler sans Burn, et retirer l'étape sémantique ne
touche les tests d'aucune des deux crates. La sélection que lit le complément
du catalogue (`wanted_relations`) prend les mêmes scores, si bien que ce qui
est décrit et ce qui est rendu restent un seul calcul.

### Le modèle, épinglé par ses octets

| | Valeur |
|---|---|
| Dépôt | `ibm-granite/granite-embedding-97m-multilingual-r2` |
| Révision | `835ad14087e140460703cf0fae09f97d469d65c2` (un commit, pas une branche) |
| `model.safetensors` | 194 889 568 octets, SHA-256 `f3ea88b230492811046145513710e76b4cc8c2ad49e8708da0e7247e548903be` |
| `tokenizer.json` | 25 301 672 octets, SHA-256 `4f2842d568e2724370aec203652a42ac783c7937f8347a1a2cc7506d71f1582f` |
| `onnx/model.onnx` | 390 004 608 octets, SHA-256 `68e592b160673d30250824c1116bc6ab33f70efb22b97c9e1d7ce1e69c1c9d70` — **source de génération seulement, jamais téléchargée par Oxyn** |
| Pooling | le jeton `[CLS]`, puis une normalisation L2 ; 384 dimensions |
| Troncature | 512 jetons, `[CLS]` et `[SEP]` compris |

`oxyn_embed::pinned::MODEL_ID` nomme le modèle et sa révision. Tout vecteur
gardé par un appelant est rangé à côté de cette valeur et jeté quand elle
change : les vecteurs de deux modèles ne sont pas comparables.

### Le moteur : Burn, avec le code généré commité

Le workspace épingle `burn`, `burn-flex` et `burn-store` exactement à
`=0.22.0`, et `tokenizers` exactement à `=0.23.2` :

- `burn` avec `default-features = false`, `std` et `flex` ;
- `burn-flex`, le backend CPU pur Rust, déclaré **directement** avec `std`,
  `simd` et `rayon`. Par `burn/flex` seul, il n'a que `std` et tourne sur un
  cœur sans SIMD ; la déclaration directe est ce dont l'unification des
  features a besoin ;
- `burn-store` avec `default-features = false`, `std`, `safetensors` et
  `memmap` ;
- `tokenizers` avec `default-features = false` et `fancy-regex` : pas
  d'Oniguruma, pas de C à compiler.

Le code Rust du modèle est **généré une fois** par burn-onnx 0.22.0 depuis
l'export ONNX épinglé et commité sous `crates/oxyn-embed/src/generated/` :
`model.rs` (le graphe), `weights_map.rs` (les clés du safetensors vers ses
paramètres) et `residual.bpk`, les 43 paramètres que le safetensors ne porte
pas — fréquences rotatives, biais de LayerNorm, scalaires que l'export ONNX a
repliés en constantes. Il n'y a pas de `build.rs` : burn-onnx, sa pile
protobuf et le fichier ONNX de 390 Mo restent hors de toute compilation.
`crates/oxyn-embed/codegen/` régénère les trois ; rien dans `generated/` n'est
modifié à la main.

**Un test verrouille la sortie.** Sans `residual.bpk`, le modèle tourne encore
et renvoie des vecteurs silencieusement faux. Les tests comparent les
embeddings de phrases multilingues fixes à des valeurs de référence calculées
par le modèle amont hors d'Oxyn, si bien qu'une régénération, une montée de
Burn ou une constante oubliée fait échouer la compilation au lieu de dégrader
chaque classement.

**Une exemption de lints limitée à un module.** Le code généré contient 69
`unwrap()` et quelque 250 conversions `as` (comptés le 2026-10-07). Ils
dépendent des formes des tenseurs, jamais de valeurs venues d'un serveur ou
d'un fichier : `Embedder::embed` ne fournit que des entrées rectangulaires
`[batch, length]` avec `batch ≥ 1` et `2 ≤ length ≤ 512`, et les tests
exécutent les deux bornes de cet intervalle. Un seul `#[allow]` est posé sur
`mod model;` dans `generated/mod.rs`, et il nomme exactement les trois lints
que le graphe déclenche : `clippy::unwrap_used`, `clippy::unnecessary_cast`
et `clippy::too_many_arguments`. Le graphe contient **69** `.unwrap()` —
`cargo clippy -p oxyn-embed --lib` signale 69 sites distincts une fois
l'exemption retirée (2026-10-07). Le 138 écrit dans `generated/mod.rs` est le
compte de `--all-targets` : `model.rs` est alors compilé deux fois, comme
bibliothèque et comme harnais de test de la bibliothèque, et chaque site est
signalé une fois par compilation. Jamais un groupe comme `clippy::all`, qui ferait taire aussi
`disallowed_methods` et `disallowed_types` — les murs que le workspace refuse
exprès. `weights_map.rs` n'en déclenche aucun et n'a pas d'exemption ; le reste
de la crate garde les lints du workspace, [I-09](../../CLAUDE.md#i-09)
compris. Aucun `unsafe` n'est écrit : le
mappage mémoire vit dans burn-store.

### Désactivée par défaut, activée par l'humain

L'option est la préférence du workspace `semantic_ranking`, un booléen de
`WorkspacePreferences` ([ADR-0013](0013-preferences-workspace.md)),
**désactivée par défaut**. Elle est ajoutée selon la règle de l'ADR-0013 — un
nouveau champ sans changement de version —, si bien qu'un contenu écrit avant
elle se lit comme désactivé et ne lance jamais de téléchargement. La politique
par défaut réserve son écriture à `Actor::Human` : aucun agent ne l'active,
donc aucun agent ne lance un téléchargement. Son écran est dans
[UX-SPEC](../UX-SPEC.md#classement-sémantique). Tant qu'elle est désactivée, rien
d'`oxyn-embed` ne touche au réseau, au disque ni à la mémoire, et la sélection
est exactement celle d'aujourd'hui.

**Le téléchargement est lancé par une commande, pas par la préférence.** Les
réglages envoient `enable_semantic_ranking`, qui enregistre la préférence et
lance `ModelStore::download` en arrière-plan, avec un suivi de progression et
une annulation. Un `true` lu sur disque au démarrage ne lance rien : il décide
seulement si un modèle déjà présent est utilisé. `disable_semantic_ranking`
l'enregistre à désactivée, annule un téléchargement, décharge le modèle et
supprime son répertoire (`ModelStore::remove`).

### Le téléchargement : deux sources, un seul ensemble d'octets acceptés

`model.safetensors` et `tokenizer.json`, 220 191 240 octets à eux deux, sont
récupérés depuis :

1. `https://huggingface.co/<repository>/resolve/<revision>/<file>` ;
2. si cela échoue ou renvoie des octets qui échouent à la vérification :
   `https://github.com/so-keyldzn/oxyn/releases/download/embedding-model-835ad140/<file>`,
   une release de ce dépôt qui porte les mêmes fichiers, octet pour octet.

**Un fichier est accepté sur sa taille et son SHA-256 épinglés, jamais sur sa
source.** Il est écrit en flux dans `<name>.part`, haché à mesure qu'il
arrive, coupé dès qu'il dépasse sa taille épinglée, et renommé en place
seulement quand taille et somme correspondent ; tout échec supprime le
`.part`. HTTPS seulement, redirections comprises (5 au plus), TLS vérifié
([ADR-0052](0052-verified-tls-outside-local.md)), 10 s pour établir la
connexion et 60 s de silence au plus entre deux morceaux. La requête sortante
ne porte aucune donnée de l'utilisateur : une URL fixe et l'agent utilisateur
`oxyn/<version>`. Le proxy système est respecté, contrairement aux transports
IA, qui le désactivent : ce qu'ils envoient, ce sont les données de
l'utilisateur, alors que cette requête n'en envoie aucune, et les octets
qu'elle reçoit sont acceptés sur leur somme seule — qui les relaie peut les
retenir, pas les altérer. Le `reqwest` du workspace active pour cela sa
feature `system-proxy` : sans elle, reqwest ne lit que les variables
`HTTP(S)_PROXY`, dont une application lancée depuis le Finder n'hérite pas. La
feature s'applique à tous les clients du graphe, si bien qu'un client qui ne
doit pas suivre de proxy le dit : le seul client d'`oxyn-llm` continue
d'appeler `no_proxy()`. Le système de mise à jour (`tauri-plugin-updater`,
même `reqwest`) suit désormais lui aussi le proxy système ; ce qu'il installe
reste accepté sur sa signature minisign, pas sur son chemin
([ADR-0051](0051-automatic-updates-from-github-releases.md)).

**Un seul téléchargement à la fois, entre processus.** Le téléchargement tient
un verrou exclusif sur le fichier `.lock` du répertoire du modèle
(`File::try_lock`) ; un second processus Oxyn qui le demande est refusé avec
`EmbedError::DownloadInProgress`, puisqu'attendre est la solution — les
fichiers qu'il écrit sont les mêmes fichiers. Le safetensors est haché de
nouveau juste avant que la conversion l'analyse, et supprimé s'il est
endommagé. `tokenizer.json` est lu une fois : les octets hachés sont les
octets analysés.

**Les erreurs portent des textes fixes.** Les messages de burn-store nomment
le chemin complet d'un fichier, et une erreur du tokenizer peut citer le
texte : les erreurs de conversion, de chargement et du tokenizer
d'`oxyn-embed` les remplacent par des phrases fixes, et le message
qu'`oxyn-desktop` affiche dans les réglages et écrit au journal est une phrase
fixe par variante d'`EmbedError`, complétée seulement d'un nom de fichier
épinglé, d'une action ou d'un type d'erreur d'E/S. Des tests donnent à chaque
variante un chemin et un marqueur et vérifient qu'aucun des deux ne ressort.

La release `embedding-model-835ad140` est publiée comme **pré-version** : la
« latest release » de GitHub est la plus récente qui n'est ni une pré-version
ni un brouillon, et le système de mise à jour lit
`releases/latest/download/latest.json`
([ADR-0051](0051-automatic-updates-from-github-releases.md)). Publiée comme une
release ordinaire, elle deviendrait « latest » et arrêterait toutes les mises
à jour. Elle a été publiée le 2026-10-07 et porte, à côté des deux fichiers,
`LICENSE-Apache-2.0.txt` et `NOTICE.md` (l'attribution d'IBM, la révision et
les sommes) ; « latest » est resté `v0.0.7`.

Les fichiers vivent sous `<data dir>/models/granite-embedding-97m-multilingual-r2-835ad140/`,
`models/` se trouvant à côté du fichier du store local. L'espace de travail
temporaire de `make desktop-dev` utilise à la place
`models-temporary-workspace/` dans le même répertoire de données de
l'utilisateur : partager `models/` permettait à une session de développement
qui désactive l'option de supprimer le modèle de l'Oxyn installé. Ce n'est pas
le répertoire temporaire du système, où d'autres comptes peuvent écrire sous
Linux, et il persiste d'un lancement de développement à l'autre.
Après le téléchargement, le safetensors en bf16 est élargi en f32 et écrit en
`model.bpk` (389 816 832 octets), qui se charge par mappage mémoire ; le
safetensors n'est supprimé qu'une fois le fichier converti vérifié. Ce qui
reste sur disque, c'est `model.bpk` et `tokenizer.json`, environ 415 Mo.
Téléchargement et conversion prennent ensemble 14 s, avec un pic de RSS à
**1,45 Go**, une seule fois.

La somme de `model.bpk` est elle aussi une constante —
`d5ac67b8e7e85e63ba433faebbe3e5537732ab7e27dde9cf3422710c6abce719` —, si bien
qu'un fichier endommagé plus tard est refusé comme un fichier téléchargé. Cela
demande de contourner burn-store d'un pas : `BurnpackStore` enregistre le
`ParamId` de chaque paramètre, un `u64` aléatoire tiré à la construction du
modèle, si bien que deux conversions des mêmes poids différeraient. La
conversion écrit par `burn_pack::Writer` avec chaque `param_id` mis à `None` ;
les identifiants ne servent qu'à reprendre un entraînement, et
l'élargissement depuis le bf16 est exact, si bien que le fichier ne dépend que
des poids. Aucun fichier n'est analysé avant que sa taille et sa somme aient
été vérifiées.

**Un risque accepté : `model.bpk` est vérifié, puis mappé.** Entre la
vérification et le mappage mémoire, un processus du même utilisateur qui
remplace le fichier fait charger son contenu sans vérification ; un processus
qui le tronque sur place pendant qu'il est mappé fait tuer Oxyn par `SIGBUS` à
la prochaine lecture d'une page manquante. Ni l'un ni l'autre n'est possible
sans accès en écriture au répertoire de données, qui contient déjà tout ce
qu'Oxyn garde. Renommer par-dessus le fichier ou le supprimer — ce que fait
Oxyn lui-même — laisse le mappage intact. Lire les 390 Mo en mémoire fermerait
la fenêtre au prix de l'avantage mémoire du mappage ; le risque est écrit à
côté de la vérification, dans `embedder.rs`.

### Chargé à la demande, libéré après cinq minutes

`OnDemandEmbedder` charge le modèle à la première demande et le libère **cinq
minutes** après sa dernière utilisation. Chaque appel bloque — des
millisecondes sur un modèle chargé, 0,67 s pour un chargement à froid — et
s'exécute sur le pool bloquant, jamais sur le thread d'interface ni sur un
thread d'exécuteur async ([I-05](../../CLAUDE.md#i-05)). burn-flex et
`tokenizers` utilisent le pool global de rayon.

### Ce qui est transformé en vecteur, et comment cela classe

- la question : le texte que reçoit `ContextBuilder::focused_on` ;
- une relation : son nom qualifié et son commentaire, tels que les porte le
  résumé du catalogue — connus dès qu'un schéma est listé, avant qu'aucun
  champ soit lu, si bien que le vecteur ne change pas quand les champs se
  chargent.

Un commentaire est coupé à 2 048 octets avant d'être transformé : le modèle
lit 512 jetons au plus. Les vecteurs des relations sont gardés en mémoire
seulement, dans `oxyn-desktop`, indexés par un **SHA-256 de `MODEL_ID` et du
texte transformé** : le cache ne garde aucun nom de table ni commentaire, le
texte ne vit que le temps d'une question, et la borne ne dépend pas de la
longueur des commentaires. Ils ne sont jamais écrits sur disque, jamais
journalisés, jamais envoyés. Au plus 50 000 vecteurs — le plafond du cache de
catalogue — en deux générations : 50 000 × 1 536 octets plus 2 × 32 768
compartiments de 41 octets, environ **79,5 Mo**.

Le classement, dans `oxyn-ai` :

1. les mentions `@` d'abord, comme aujourd'hui ;
2. les relations qui ont un score lexical, selon ce score ; **les scores égaux
   selon le cosinus**, puis selon le chemin ;
3. les relations sans score lexical, **selon le cosinus**, puis selon le
   chemin, jusqu'à `ContextPolicy::max_relations`.

**Les scores peuvent élargir la sélection, pas seulement la réordonner.** Sans
scores, une question qui correspond à certains noms garde ces correspondances
et rien d'autre (l'ordre des chemins ne décide que quand rien ne correspond).
Avec des scores, la règle 3 complète les correspondances avec les relations
que la recherche a manquées, jusqu'à `max_relations` — 24. Une plus grande
part du schéma est alors décrite et envoyée, toujours sous le niveau de la
connexion et dans le budget de jetons ; les réglages le disent
([UX-SPEC](../UX-SPEC.md#classement-sémantique)). Un test d'`oxyn-ai` fixe les
deux nombres et vérifie que le complément du catalogue veut la même sélection.

Pas de seuil de cosinus : une faible correspondance lexicale passe toujours
devant une forte correspondance sémantique. Avec un écart de 0,1 entre tables
pertinentes et non pertinentes, un seuil serait un nombre réglé qui tient pour
les cas mesurés et échoue en silence ailleurs ; l'ordre seul se dégrade
proprement.

**L'étape sémantique ne fait jamais échouer une question et ne la fait jamais
attendre longtemps.** Elle est bornée à **2 secondes** par question,
chargement compris, et transforme les relations manquantes par lots, en
vérifiant l'échéance entre deux lots. Ce qui n'est pas calculé à temps se
classe comme aujourd'hui, après ce qui l'est ; ce qui est calculé reste en
cache pour la question suivante. Tant que le modèle est en téléchargement,
absent, corrompu ou en échec de chargement, le classement est celui
d'aujourd'hui, et le panneau dit que le classement sémantique est
indisponible et pourquoi.

## Conséquences

* **+** Une question dans une langue trouve des tables nommées dans une autre,
  et une question qui ne nomme aucune table obtient les plus proches au lieu
  des premières dans l'ordre alphabétique : les 24 relations gardées cessent
  d'être arbitraires.
* **+** Rien ne quitte la machine pour le calculer : pas de fournisseur, pas
  de clé, pas d'installation. Le niveau de confidentialité est intact sous
  toutes ses valeurs.
* **+** Les correspondances lexicales gardent leur ordre et leur place : un
  vecteur faux ne peut rétrograder rien de ce qui correspondait par le nom.
* **−** **Une plus grande part du schéma peut sortir.** Là où la sélection
  lexicale ne gardait que ses correspondances, les relations trouvées par le
  sens la complètent jusqu'à 24 : un fournisseur peut lire la description de
  tables que la question n'a jamais nommées. Le niveau et le budget de jetons
  bornent toujours ce qui sort ; c'est le nombre de relations décrites qui
  augmente.
* **+** `oxyn-ai` ne dépend pas de Burn : ses tests et le chemin des agents
  externes gardent leur temps de compilation, et la passerelle reste une seule
  fonction.
* **−** **795 Mo de RAM tant que le modèle est chargé**, cinq minutes
  au plus après la dernière question. Sur une machine de 8 Go qui tient déjà
  de gros résultats, c'est la différence entre tenir et swapper.
* **−** **Burn est en 0.x et casse son API entre versions mineures.** Chaque
  montée régénère `model.rs`, `weights_map.rs` et `residual.bpk`, et la somme
  de `model.bpk` change avec le format de burn-store — chaque utilisateur
  convertit alors de nouveau. Les quatre épinglages exacts existent pour que
  cela n'arrive jamais par `cargo update`.
* **−** **Oublier une constante résiduelle donne des vecteurs faux, pas une
  erreur.** Seul le test de sortie de référence l'attrape ; une régénération
  qui met à jour les valeurs attendues de ce test au lieu d'enquêter le
  neutralise.
* **−** Un module de code généré exempté des lints du workspace : 69
  `unwrap()` que personne ne relit ligne à ligne, tenus seulement par
  l'argument des formes ci-dessus et par les tests aux deux bornes.
* **−** burn-flex et `tokenizers` partagent le pool **global** de rayon : un
  lot d'embeddings dispute tous les cœurs à ce qui s'en sert d'autre dans le
  processus, et peut faire attendre une autre tâche.
* **−** **La disponibilité de Hugging Face.** La release de secours couvre une
  panne ou un retrait en amont, pas un réseau qui bloque les deux hôtes :
  derrière un tel réseau, l'option ne peut pas être activée. Le secours est à
  nous de le garder : une release supprimée est un 404 que l'erreur de
  téléchargement nomme.
* **−** **Sans ses features, burn-flex tourne sur un cœur, sans SIMD**,
  plusieurs fois plus lentement — et rien n'échoue. Seule la déclaration
  directe dans le `Cargo.toml` racine, commentée, les garde.
* **−** **+80 s de compilation à froid** en CI et pour toute compilation
  d'`oxyn-desktop` à partir de zéro ; 45 s d'entre elles sur le chemin
  critique.
* **−** Environ 415 Mo de disque pour l'utilisateur qui active l'option, un
  téléchargement de 220 Mo au premier usage, et **1,45 Go de RSS au pic** de
  ce téléchargement et de cette conversion — au-dessus du 1 Go du modèle
  chargé, une seule fois.
* **−** La somme de `model.bpk` ne tient que parce que la conversion contourne
  `BurnpackStore` et efface chaque `ParamId` : une montée de Burn qui change
  `burn_pack::Writer` ou la collecte des paramètres peut rendre le fichier non
  reproductible, et la conversion de chaque utilisateur est alors refusée
  jusqu'à ce que la constante soit régénérée.
* **−** Mesuré sous macOS arm64 seulement. La release compile aussi Linux
  x86_64 et arm64, où les chemins SIMD de burn-flex et la latence ne sont pas
  mesurés.

**Coût de sortie :** faible, et borné par le sens des dépendances. Retirer la
fonctionnalité supprime `crates/oxyn-embed`, la préférence, les scores passés
à `ContextBuilder` et le cache en mémoire d'`oxyn-desktop` ; le classement
lexical continue de fonctionner tel quel, puisqu'il n'a jamais dépendu des
scores. Les utilisateurs gardent un répertoire d'environ 415 Mo sous
`models/` qu'une release de retrait devrait supprimer. Changer de moteur (pour
candle, ou pour un endpoint distant) remplace `Embedder` derrière les mêmes
scores ; changer de modèle change `pinned`, le code généré et `MODEL_ID`, et
chaque vecteur en cache est jeté par sa clé.

**À réexaminer si** Burn atteint la 1.0 ou cesse de casser son API entre
versions mineures (le coût de régénération tombe) ; si un modèle plus petit
égale la qualité de celui-ci sur les paires question/table mesurées ; si la
mémoire résidente mesurée dans `oxyn-desktop` dépasse **1 Go** ; si les
utilisateurs disposent d'un endpoint d'embeddings là où ils travaillent (le
chemin distant devient alors un complément qui vaut d'être écrit) ; si
candle, mesuré sur les mêmes paires, égale la sortie et la mémoire sans code
généré ; ou si les opérateurs de ModernBERT cessent de se convertir —
`MongoDB/mdbr-leaf-mt` est alors le repli, en anglais seulement.

## Alternatives rejetées

| Alternative | Raison du rejet |
|---|---|
| Un endpoint d'embeddings (`/v1/embeddings`) par `OpenAiCompatibleProvider`, comme première étape | Ne marche pas pour l'utilisateur qui l'a demandé, qui n'a ni Ollama ni clé, ni pour quiconque n'a rien installé à côté d'Oxyn. Il reste un complément pour plus tard, pas une exclusion : un vecteur distant tiendrait derrière les mêmes scores par relation |
| `ort` (onnxruntime) | Pas de 2.0 stable sur crates.io (2.0.0-rc.13, 2026-07-28) ; dépend d'`ort-sys`, et sa feature par défaut `download-binaries` récupère un onnxruntime précompilé — un binaire C++ dans le processus, téléchargé à la compilation. La sortie de Burn égale déjà celle d'onnxruntime à 4,2e-7 sans lui |
| candle (`candle-transformers` 0.11.0, 2026-06-26) | Non mesuré par le spike. Sur le papier, c'est l'alternative la plus forte : son `models/modernbert.rs` implémente les deux bases RoPE et l'attention locale/globale depuis la configuration, si bien qu'il n'aurait besoin ni de code généré ni de constantes résiduelles. Burn est le moteur dont la sortie, la mémoire et la compilation ont été mesurées ; candle est le premier à mesurer sur les mêmes paires, nommé dans la condition de réexamen |
| Livrer les poids dans l'installeur | 220 Mo de plus à chaque téléchargement et chaque mise à jour, pour une option désactivée par défaut |
| Héberger nous-mêmes un `.bpk` converti | 390 Mo en f32 au lieu de 195 Mo en bf16 — deux fois le téléchargement —, un fichier lié au format de burn-store qui change à chaque montée de Burn, et un hébergement qui n'est qu'à nous au lieu d'un secours |
| `microsoft/harrier-oss-v1-270m` (MIT) | 268 M de paramètres en bf16, environ 1 Go une fois élargis en f32 ; pas d'export ONNX dans son dépôt (vérifié le 2026-10-07) |
| `voyageai/voyage-4-nano` (Apache-2.0) | 346 M de paramètres, 3,6 fois ce modèle, sans gain mesuré sur la question que tranche cet ADR |
| `jinaai/jina-embeddings-v5-*` | CC-BY-NC-4.0 : non commerciale, incompatible avec les fonctionnalités payantes qu'Oxyn prévoit |
| `google/embeddinggemma-300m` | La licence Gemma, avec ses propres restrictions d'usage, au lieu d'une licence OSI sur laquelle `deny.toml` puisse raisonner ; 303 M de paramètres |
| `MongoDB/mdbr-leaf-mt` (Apache-2.0, 22,6 M de paramètres) | Anglais seulement : la question en français du contexte est exactement ce qu'il manquerait. Gardé comme repli si ModernBERT cesse de se convertir |
| Un seuil de cosinus qui admet une relation sur le sens seul | Un écart d'environ 0,1 entre tables pertinentes et non pertinentes fait de tout seuil un nombre réglé ; l'ordre se dégrade proprement là où un seuil échoue en silence |
| `oxyn-ai` dépendant d'`oxyn-embed` et transformant en vecteurs dans `ContextBuilder::build` | Burn dans chaque compilation et chaque test d'`oxyn-ai`, et une passerelle qui bloque des secondes sur un chargement de modèle. Passer des scores garde la passerelle une fonction pure de ce qu'elle reçoit |
| Un `build.rs` qui exécute burn-onnx à la compilation | burn-onnx, sa pile protobuf et un fichier ONNX de 390 Mo dans chaque compilation, pour du code qui ne change que quand le modèle ou Burn change |
