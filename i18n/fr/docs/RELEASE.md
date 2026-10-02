<!-- oxyn-translation source="docs/RELEASE.md" sha256="7637c334bfa3" -->
# Livraisons GitHub

GitHub Actions construit les paquets ; le mainteneur publie le brouillon
obtenu après relecture. `.github/workflows/livraison.yml` démarre au push d'un
tag `v*`, ou manuellement avec un tag existant. Le tag doit être exactement
`v` suivi de la version de `crates/oxyn-desktop/tauri.conf.json`. Les deux jobs
extraient ce tag précis, même si le lancement manuel sélectionne une autre branche.

`macos-latest` produit un DMG Apple Silicon ; `ubuntu-24.04` produit les paquets
DEB, RPM et AppImage. macOS Intel et Windows ne font pas partie de cette matrice.
`make desktop PROFIL=release` est le point d'entrée du build. Le workflow
installe le binaire épinglé de `cargo-about` après vérification de son empreinte,
pour inclure les mentions obligatoires des licences tierces.

Le workflow construit aussi ce dont les copies installées se mettent à jour
([ADR-0051](adr/0051-automatic-updates-from-github-releases.md)) : les jobs de
paquets lancent `make desktop PROFIL=release MISE_A_JOUR=1`, qui signe les
artefacts de mise à jour — `Oxyn_<version>_aarch64.app.tar.gz` et l'AppImage,
chacun avec son `.sig` — et un dernier job, `manifeste`, en écrit
`latest.json`.

## Configuration Apple

Le job macOS exige les sept **secrets Actions du dépôt** ci-dessous. Une valeur
manquante l'arrête avant l'installation des dépendances ; il ne produit jamais
une livraison non signée par défaut. Linux ne reçoit pas les secrets Apple.

| Secret | Valeur |
|---|---|
| `APPLE_CERTIFICATE` | Base64 d'un export `.p12` protégé par mot de passe contenant le certificat Developer ID Application **et sa clé privée correspondante** |
| `APPLE_CERTIFICATE_PASSWORD` | Mot de passe choisi pour l'export `.p12` |
| `APPLE_SIGNING_IDENTITY` | Identité complète : `Developer ID Application: <nom> (<identifiant d'équipe>)` |
| `APPLE_API_KEY` | Identifiant de la clé API d’équipe App Store Connect dédiée |
| `APPLE_API_ISSUER` | Identifiant de l’émetteur affiché au-dessus du tableau des clés d’équipe |
| `APPLE_API_PRIVATE_KEY` | Contenu de la clé privée `.p8` téléchargée |
| `APPLE_TEAM_ID` | Identifiant d'équipe de dix caractères figurant dans l'adhésion |

Utiliser les [secrets Actions du dépôt](https://github.com/so-keyldzn/oxyn/settings/secrets/actions).
Saisir soi-même les identifiants, inspection de l'écran par l'agent suspendue.
Ne jamais les mettre dans une conversation, les fichiers du dépôt, les arguments
de commande, les journaux ou le presse-papiers. Ne pas exporter de clé privée
dans le worktree.

Dans Trousseaux d'accès, exporter l'identité correspondante depuis **Mes
certificats** en `.p12` protégé par mot de passe. Si sa clé privée manque,
télécharger le `.cer` public depuis Apple ne la récupère pas : utiliser le Mac
qui a créé le certificat, ou créer un nouveau certificat Developer ID Application
avec une nouvelle CSR. Ne pas révoquer de certificat existant pour libérer une
place sans vérifier ses autres utilisateurs. Ce mode de distribution ne
nécessite pas de profil de provisionnement.

Avec une session GitHub CLI autorisée à gérer les secrets de ce dépôt, le
mainteneur peut envoyer directement le Base64 à GitHub sans presse-papiers ni
fichier texte intermédiaire. Exécuter personnellement, en remplaçant seulement
le chemin du fichier :

```sh
set -o pipefail
openssl base64 -A -in /absolute/path/outside-the-repo/developer-id.p12 |
  gh secret set APPLE_CERTIFICATE --repo so-keyldzn/oxyn
gh secret set APPLE_CERTIFICATE_PASSWORD --repo so-keyldzn/oxyn
gh secret set APPLE_API_PRIVATE_KEY --repo so-keyldzn/oxyn < /absolute/path/outside-the-repo/AuthKey.p8
```

La commande du mot de passe d’export demande une saisie privée ; la clé API est lue directement depuis son fichier. Un `403` lors de la
lecture ou de l'écriture des secrets indique un accès insuffisant du jeton CLI ;
il ne prouve pas leur absence. Utiliser le navigateur ou une session CLI
autorisée séparément ; ne pas coller de jeton GitHub dans une conversation.

Créer la clé API d’équipe dans App Store Connect → Users and Access → Integrations,
avec le rôle Developer. Télécharger sa clé privée une seule fois et la transférer
directement vers GitHub. Les clés d’équipe couvrent les apps du compte ; réserver
cette clé à la livraison automatisée et la révoquer lors de son remplacement.

`script/apple-release build` écrit la clé API dans un fichier en mode 0600,
dans un répertoire temporaire privé hors du workspace, transmet seulement son
chemin à Tauri et le supprime à la sortie normale, y compris après un échec.
La clé API brute est retirée de l’environnement du processus enfant.
Tauri importe le certificat dans un trousseau temporaire, signe avec le runtime
renforcé, soumet l'application à Apple et lui agrafe le ticket accepté avant de
créer le DMG signé. Son trousseau temporaire est supprimé lors du nettoyage
normal ; ces jobs utilisent des runners GitHub jetables, jamais persistants.
`script/apple-release verify` vérifie ensuite les deux signatures et l'équipe
configurée, le runtime renforcé de l'application, le ticket agrafé et
l'évaluation Gatekeeper. Un échec empêche l'envoi du paquet macOS. Le DMG est
signé ; l'objet notarisé et agrafé qu'il contient est le `.app`. L'archive de
mise à jour est faite par le bundler à partir de ce même `.app`, après
l'agrafage ; `verify` l'extrait et soumet le `.app` qu'elle contient aux mêmes
contrôles `codesign`, `stapler` et `spctl`.

## Clé de signature des mises à jour

Un Oxyn installé n'accepte une mise à jour que si son archive porte une
signature minisign de la clé dont la moitié publique est compilée dans
l'application (`plugins.updater.pubkey` dans
`crates/oxyn-desktop/tauri.conf.json`). GitHub ne détient jamais la clé privée
en clair : elle vit dans deux secrets Actions et dans deux sauvegardes hors
ligne, nulle part ailleurs ([I-03](../CLAUDE.md#i-03)). La générer, la ranger
et la renouveler reviennent au seul mainteneur ; rien n'en est délégué à un
agent.

| Secret | Valeur |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | Contenu du fichier de clé privée écrit par `tauri signer generate` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Le mot de passe choisi à la génération |

Les jobs de paquets ne les passent qu'aux étapes de vérification de la clé et
de build. `script/livraison cle` refuse un secret vide ou la clé publique de
substitution avant une heure de build, et nomme les variables, jamais leurs
valeurs.

**Générer**, une fois, hors du dépôt, avec un mot de passe :

```sh
pnpm --dir apps/desktop tauri signer generate -w /absolute/path/outside-the-repo/oxyn-updater.key
```

La commande demande le mot de passe, écrit la clé privée à ce chemin et la clé
publique à côté, suffixée de `.pub`. Ensuite :

1. Coller la clé **publique** (`oxyn-updater.key.pub`, une ligne de Base64)
   dans `plugins.updater.pubkey` et la committer par une pull request.
2. Envoyer les deux secrets à GitHub depuis le fichier et depuis une invite,
   jamais par le presse-papiers, une conversation ou un argument de commande :

   ```sh
   gh secret set TAURI_SIGNING_PRIVATE_KEY --repo so-keyldzn/oxyn < /absolute/path/outside-the-repo/oxyn-updater.key
   gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --repo so-keyldzn/oxyn
   ```

3. Copier le fichier de clé privée sur **deux sauvegardes hors ligne**, sur
   des supports différents, le mot de passe rangé à part (un gestionnaire de
   mots de passe), puis supprimer la copie de travail. Un secret GitHub ne se
   relit pas : si les deux sauvegardes sont perdues, la clé l'est aussi.

**Renouveler** tant que l'ancienne clé existe. La release N, signée avec
l'ancienne clé, porte la nouvelle clé publique ; la release N+1 est signée
avec la nouvelle. Les utilisateurs qui sautent N restent sur l'ancienne clé :
ils n'obtiennent N+1 qu'après être passés par N, donc N reste publiée. Alors
seulement, remplacer les deux secrets et les sauvegardes.

**Perte.** Sans la clé privée, aucune copie installée ne peut plus être mise à
jour. Générer une nouvelle clé, publier une release signée avec elle, et
annoncer que chaque utilisateur doit la télécharger à la main une fois.

**Compromission.** Qui détient la clé et le mot de passe peut signer une
archive que toute copie installée installera, s'il peut aussi servir un
manifeste. Renouveler aussitôt si l'ancienne clé est encore en main ; sinon,
traiter comme une perte. Retirer les secrets, et confronter les `.sig` des
releases publiées à leurs archives.

## Lancer et publier

1. Intégrer le workflow et les scripts relus dans `main`. Exécuter `make qualite`
   sur les sources choisies et examiner l'exécution qualité GitHub correspondante.
2. Régler la version de livraison de façon cohérente dans le workspace et la
   configuration Tauri, committer les sources choisies, puis pousser le tag
   correspondant. Le push d'un tag déclenche automatiquement la livraison ;
   les push de branches ordinaires ne créent pas de release.
3. Autre possibilité : **Actions → livraison → Run workflow**, en indiquant
   ce tag existant, ou :

   ```sh
   gh workflow run livraison.yml --repo so-keyldzn/oxyn -f tag=v0.0.1
   ```

   Le workflow manuel doit d'abord exister sur la branche par défaut. Remplacer
   la version de l'exemple lorsque la version configurée change.
4. Attendre les deux jobs de paquets, examiner les contrôles puis télécharger
   et installer les paquets du brouillon.
5. Attendre le job `manifeste`. Il démarre une fois les deux jobs de paquets
   au vert, sans autre secret que `GH_TOKEN` : il télécharge les `.sig` du
   brouillon, exige exactement un `*_aarch64.app.tar.gz` et une `*.AppImage`,
   chacun avec sa signature, et dépose `latest.json` avec les clés
   `darwin-aarch64` et `linux-x86_64-appimage`. C'est le seul job qui dépose
   `latest.json`, et il refuse un brouillon qui en contient déjà un.
6. **Ne publier le brouillon que lorsque les deux jobs de paquets et
   `manifeste` sont au vert et que `latest.json` est joint.** C'est la
   publication qui propose la mise à jour :
   `https://github.com/so-keyldzn/oxyn/releases/latest/download/latest.json`
   ne sert que la dernière release publiée et non préliminaire. Une release
   publiée sans `latest.json` fait échouer la vérification de chaque copie
   installée jusqu'à la suivante.

Les relances ne remplacent jamais les fichiers. Retirer manuellement un fichier
précis d'un **brouillon** avant de le reconstruire. Une release publiée est
refusée avant création/envoi ; son état est vérifié autour de chaque transfert.
Si la publication survient pendant un transfert, ce fichier peut arriver ; le
workflow s'arrête sans le supprimer. Garder la release en brouillon jusqu'à la
fin de tous les jobs.

## Limites de validation

`make socle` exécute les tests simulés de livraison GitHub et de refus Apple ;
il ne contacte pas Apple et ne prouve pas une signature. Le premier build
GitHub signé réussi, la vérification du ticket et l'installation sur un Mac
vierge constituent les preuves de bout en bout. Sources et contrats vérifiés :
[RESEARCH-NOTES](RESEARCH-NOTES.md#contrats-de-livraison-apple--vérifiés-le-2026-10-01).

Les mêmes tests couvrent `manifeste` et `cle` ; ils ne prouvent pas qu'un Oxyn
installé accepte le résultat. Il y faut deux étapes :

1. **Répétition locale**, avec la feature cargo `update-rehearsal`, jamais
   activée en CI : une clé jetable, des bundles 0.0.1 et 0.0.2 servis sur
   `127.0.0.1`. Vérifier le téléchargement, l'installation au ⌘Q,
   l'installation en quittant depuis le Dock, « Restart now », l'absence
   d'écran de récupération après la relance, un `.sig` falsifié qui finit en
   erreur de signature, et une vérification hors ligne qui échoue en silence.
2. **Releases réelles.** Publier v0.0.2, installée à la main (v0.0.1 n'a pas
   de mise à jour automatique), puis v0.0.3. Vérifier que v0.0.2 se met à jour
   seule sur un Mac vierge et en AppImage, et qu'une installation `.deb`
   affiche « Updates are managed by your package manager ».

Contrats de mise à jour :
[RESEARCH-NOTES](RESEARCH-NOTES.md#contrats-de-mise-à-jour-tauri--vérifiés-le-2026-10-02).
