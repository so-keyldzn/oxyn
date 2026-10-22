<!-- oxyn-translation source="docs/adr/0058-updater-key-signs-in-a-job-that-builds-nothing.md" sha256="84ed446b5446" -->

> Traduction française de [docs/adr/0058-updater-key-signs-in-a-job-that-builds-nothing.md](../../../../docs/adr/0058-updater-key-signs-in-a-job-that-builds-nothing.md). **La version anglaise fait foi.**

# ADR-0058 — La clé de mise à jour signe dans un job qui ne construit rien

**Statut :** proposé · **Date :** 2026-10-22

**Précise :** [ADR-0051](0051-automatic-updates-from-github-releases.md), sur
trois points : où la clé publique factice est refusée (§ 2), où et avec quel
programme les artefacts de mise à jour sont signés (§ 3), et sa conséquence
négative « L'étape de signature fait encore confiance à la CLI Tauri ». Le
reste de l'ADR-0051 — la mise à jour en Rust, le point d'accès,
`requireSignedVersion`, le manifeste écrit en dernier et vérifié par
`minisign -V` — reste en vigueur.

## Contexte

L'ADR-0051 § 3 tenait la clé de mise à jour hors de `tauri build`, mais pas
hors du job qui le lance. Sur `origin/main` à `377d2f1`, le job `paquets` de
`.github/workflows/livraison.yml` nommait l'environnement `release`, détenait
un jeton `contents: write`, lançait `pnpm install` et `tauri build`, puis
remettait la clé à une étape ultérieure **du même job**,
`script/livraison signer`, qui lançait le binaire `@tauri-apps/cli` de
`apps/desktop/node_modules`. Restreindre le secret à cette étape n'isolait
rien : le build exécute chaque `build.rs` et chaque proc-macro des 1 200
paquets de `Cargo.lock` (comptés le 2026-10-22), Vite et ses plugins, et les
scripts d'installation du graphe npm, et n'importe lequel peut réécrire
`node_modules/.bin/tauri` ou `script/livraison` — fichiers que l'étape
ultérieure exécute ensuite avec la clé. Une seule version compromise d'une
dépendance suffisait à lire la clé à laquelle se fie chaque Oxyn installé, et
cette clé n'a pas de révocation
([SECURITY](../SECURITY.md#la-clé-de-signature-des-mises-à-jour)). L'audit du
2026-10-21 l'a relevé comme constat F03, issue #248.

Ce qui borne la correction, vérifié le 2026-10-22
([RESEARCH-NOTES](../RESEARCH-NOTES.md#ci-et-livraison-github)) :

* Tauri publie la CLI comme binaire de release sur GitHub : la release
  `tauri-cli-v2.12.1` porte `cargo-tauri-x86_64-unknown-linux-gnu.tgz`, dont
  le SHA-256 égale l'empreinte que GitHub affiche pour l'asset, construit
  depuis le même commit de tauri que le paquet npm 2.12.1 de
  `apps/desktop/package.json`. Son `signer sign --app-version` écrit le
  commentaire de confiance que vérifie `requireSignedVersion`.
* `actions/upload-artifact` v7.0.2 et `actions/download-artifact` v8.0.2
  transportent des fichiers entre les jobs d'une exécution ; l'envoi range
  chaque fichier relativement au chemin qui précède le premier joker.
* La signature et la notarisation macOS ont lieu **à l'intérieur** du
  bundling de Tauri, avec les identifiants Apple dans l'environnement du
  build : elles ne peuvent sortir du build sans refaire à la main le travail
  du bundler
  ([SECURITY](../SECURITY.md#les-identifiants-de-livraison-apple)).

## Décision

### 1. Cinq jobs, et la clé dans celui qui ne construit rien

| Job | Attend | Environnement | Secrets | Jeton | Exécute du code du build |
|---|---|---|---|---|---|
| `brouillon` | — | — | — | `contents: write` | non |
| `paquets` (macOS, Linux x86_64, Linux aarch64) | `brouillon` | — | identifiants Apple, branche macOS seulement | `contents: read` | **oui** |
| `signature` | `paquets` | `release` | `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | `contents: read` | non |
| `depot` | `signature` | — | — | `contents: write` | non |
| `manifeste` | `depot` | — | — | `contents: write` | non |

Le workflow n'accorde aucune permission de jeton par défaut
(`permissions: {}`). Seul `signature` nomme `release`, donc seul lui peut lire
la clé de mise à jour, et son jeton ne peut pas écrire la release. Il part
d'un checkout neuf du tag, n'installe ni Node, ni pnpm, ni Rust, et exécute
deux programmes sur la clé : `script/livraison` depuis ce checkout, et le
binaire de release de la CLI Tauri du § 3.

### 2. Les paquets traversent comme artefacts, comme données

Chaque branche de `paquets` envoie ses paquets non signés sous
`paquets-<cible>` — `macos-aarch64`, `linux-x86_64`, `linux-aarch64`.
`signature` les télécharge par leur nom exact, signe l'archive macOS et les
deux AppImage, et envoie les trois fichiers `.sig` sous `signatures`, dans le
répertoire de leur plateforme. `depot` télécharge les paquets et les
signatures dans la même arborescence, puis les dépose dans le brouillon avec
`script/livraison deposer`. Dans `signature`, les paquets sont signés, jamais
exécutés.

### 3. Le programme de signature est un binaire de release épinglé par somme de contrôle

`signature` télécharge `cargo-tauri` depuis la release GitHub de Tauri de la
version de la CLI dans `apps/desktop/package.json`, vérifie le SHA-256 de
l'archive — écrit dans le workflow et dans
[RESEARCH-NOTES](../RESEARCH-NOTES.md#ci-et-livraison-github) — avant de la
décompresser, et le nomme dans `$TAURI`. `script/livraison signer` n'a pas de
CLI par défaut : il refuse de s'exécuter sans `$TAURI`, ou avec un chemin sous
`node_modules`.

### 4. La clé publique factice est refusée avant le build

`brouillon` refuse la valeur factice `REPLACE_WITH_UPDATER_PUBLIC_KEY`
(`script/livraison brouillon`), avant que le brouillon n'existe et avant une
heure de build. `script/livraison cle`, la première étape de `signature` à
détenir la clé, refuse un secret vide et la valeur factice une nouvelle fois
avant de rien télécharger ; `manifeste` la refuse toujours avant d'écrire
`latest.json`.

### 5. Tenu par un test du fichier de workflow

Le workflow ne s'exécute que sur un tag `v*`, donc aucune pull request ne
l'exerce. `script/test_livraison_workflow.py`, lancé par `make socle`, lit
`livraison.yml` et refuse un changement du tableau du § 1 — les secrets de
mise à jour ou `release` dans un autre job, une commande de build ou une
action de chaîne d'outils dans `signature`, `contents: write` ou `GH_TOKEN`
hors des trois jobs qui écrivent — ainsi qu'un nom ou un chemin d'artefact qui
diverge entre producteur et consommateur, et une vérification de somme de
contrôle placée après la décompression ou absente de RESEARCH-NOTES.

## Conséquences

* **+** Aucun job qui exécute du code du build ne détient la clé de mise à
  jour, et aucun job qui la détient ne peut écrire la release.
* **+** Le programme de signature vient d'octets épinglés dans le dépôt, pas
  d'un répertoire qu'écrit le build.
* **+** Une régression de la frontière fait échouer `make socle` sur la pull
  request qui l'introduit, pas à la livraison suivante.
* **−** **L'isolation protège la clé, pas les paquets.** Un build compromis
  produit toujours un paquet compromis, et `signature` le signe.
* **−** **Les identifiants Apple restent dans le build macOS**, lisibles par
  son code : un risque accepté, borné et argumenté dans
  [SECURITY](../SECURITY.md#les-identifiants-de-livraison-apple).
* **−** **Le job de signature fait confiance à la release GitHub de Tauri.**
  Un asset malveillant publié sous ce tag avant que la somme de contrôle soit
  relevée aurait été épinglé avec elle ; la somme garantit seulement que les
  octets restent ceux vérifiés le 2026-10-22.
* **−** **La version de la CLI est épinglée deux fois** : dans
  `apps/desktop/package.json` pour le build, et dans l'URL et la somme de
  contrôle du workflow pour la signature. Aucun test n'attrape la montée de
  l'une sans l'autre ; les deux lignes de RESEARCH-NOTES doivent bouger
  ensemble.
* **−** **La topologie est exécutée pour la première fois par une
  livraison.** Le test du § 5 lit le fichier ; il n'exécute pas les actions.
  Les artefacts, chemins et signatures réels sont prouvés par le premier tag
  après cette ADR.
* **−** L'exécution gagne deux jobs et trois allers-retours d'artefacts :
  quelques minutes sur une livraison qui prend déjà environ une heure.

**Coût de sortie :** faible. Un fichier de workflow, les commandes `signer`
et `cle` de `script/livraison` et un fichier de test ; refusionner les jobs
prend un après-midi. Rien dans l'application ne dépend de la façon dont les
signatures ont été produites, seulement de la clé qui les a faites.

**À reconsidérer si** le bundler Tauri sait signer et notariser un bundle
macOS dans une invocation distincte du build (les identifiants Apple
pourraient alors quitter `paquets` eux aussi) ; si la clé de mise à jour passe
dans un service de signature qui ne la remet jamais à un runner ; si Tauri
cesse de publier des binaires de release de la CLI, ou si le plugin cesse de
vérifier des signatures minisign ; si GitHub propose des secrets restreints à
une étape dont il documente l'isolation vis-à-vis des étapes antérieures du
job.

## Alternatives rejetées

| Alternative | Raison du rejet |
|---|---|
| Garder la signature dans `paquets`, la clé restreinte à l'étape de signature (ADR-0051 § 3) | Les étapes antérieures du job écrivaient `node_modules` et pouvaient réécrire les scripts et la CLI qu'exécute cette étape : la restriction ne protégeait de rien face au build |
| `pnpm install` dans le job de signature, pour lancer la CLI qu'épingle le lockfile | Exécute les scripts d'installation de tout le graphe npm dans le job qui détient la clé |
| `cargo install tauri-cli --locked` dans le job de signature | Un build Rust — des centaines de `build.rs` et de proc-macros — dans le job qui détient la clé, l'exposition même que cette ADR supprime |
| Signer directement avec `minisign -S` | Le format du `.sig`, l'encodage de la clé privée de Tauri et le commentaire de confiance que lit `requireSignedVersion` sont ceux de Tauri : les reproduire à la main, c'est une seconde implémentation du contrat que vérifie chaque copie installée, testée contre rien |
| Déplacer la signature Developer ID et la notarisation macOS dans `signature` | Revient à refaire hors de Tauri la signature imbriquée du bundler, le runtime renforcé, la notarisation, l'agrafage et le DMG, dans un job macOS que seule une vraie livraison pourrait exercer ; le risque accepté est dans [SECURITY](../SECURITY.md#les-identifiants-de-livraison-apple) |
| Déplacer les identifiants Apple dans `release` | Le build macOS devrait nommer `release`, et une ligne lui remettrait alors la clé de mise à jour |
