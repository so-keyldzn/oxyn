<!-- oxyn-translation source="docs/RELEASE.md" sha256="206305307a3b" -->
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
signé ; l'objet notarisé et agrafé qu'il contient est le `.app`.

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
   et installer les paquets du brouillon. Publier manuellement après validation.

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
