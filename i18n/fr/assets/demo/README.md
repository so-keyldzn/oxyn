<!-- oxyn-translation source="assets/demo/README.md" sha256="43362c0308b8" -->

> Traduction française de [assets/demo/README.md](../../../../assets/demo/README.md). **La version anglaise fait foi.**

# Galerie de l’interface et démonstrations

Captures natives macOS enregistrées le 2026-10-03 avec **Oxyn 0.0.3**, un
workspace temporaire et le jeu SQLite fictif de
[seed.sql](../../../../assets/demo/seed.sql). Les noms d’entreprises, les commandes
et le chiffre d’affaires sont fictifs. Aucune base externe ni aucun fournisseur
IA n’intervient.

## Captures

### Console SQL

Une requête de chiffre d’affaires joint les clients et les commandes payées,
regroupe par pays et affiche les résultats dans la grille. La requête exacte
figure dans [revenue.sql](../../../../assets/demo/revenue.sql).

![Éditeur SQL et résultats de chiffre d’affaires](../../../../assets/demo/screenshots/sql-workspace.jpg)

### Exploration des tables

Le catalogue et l’onglet Data affichent les commandes de démonstration.

![Commandes dans l’explorateur de tables](../../../../assets/demo/screenshots/table-explorer.jpg)

### Inspection du schéma

L’onglet Structure affiche les colonnes lues dans le catalogue SQLite.

![Colonnes des commandes dans l’onglet Structure](../../../../assets/demo/screenshots/table-structure.jpg)

## Vidéos

| Démonstration | Durée | Lecture | Contenu |
|---|---|---|---|
| Explorer une base | 31 s | [MP4](../../../../assets/demo/videos/explore-database.mp4) | Ouvrir les commandes, parcourir les données, inspecter Structure, DDL et Indexes. |
| Exécuter une requête de chiffre d’affaires | 21 s | [MP4](../../../../assets/demo/videos/run-query.mp4) | Saisir le SQL, l’exécuter explicitement et inspecter la grille de résultats. |

Les vidéos sont des enregistrements d’écran H.264 sans son (1920 × 1286, 30 images/s).
Les pauses ont été raccourcies ; les interactions et les résultats sont réels.
Les captures conservent leurs 2560 × 1640 pixels d’origine. Téléchargez les liens MP4
si votre lecteur Markdown ne propose pas de lecture. Ces captures illustrent
ces parcours locaux précis ; elles ne valident pas les autres drivers ni l’IA.

## Provenance des captures

L’application installée n’est pas une compilation vérifiée de ce checkout.
L’exécutable installé d’origine a pour SHA-256
`1d85505c37e9daceb4c845458602057ab6fbbeac8785fb6e4f48b4f0292e1d99`.

Pour isoler les captures, une copie jetable de l’application installée utilisait
un identifiant de bundle distinct et une signature locale ad hoc. Le code et les
ressources web étaient inchangés ; l’empreinte ci-dessus désigne l’exécutable
avant cette nouvelle signature.

## Reproduire les scènes

Depuis la racine du dépôt, créez une base jetable neuve :

```sh
demo_dir=$(mktemp -d /tmp/oxyn-demo.XXXXXX)
sqlite3 "$demo_dir/studio.sqlite" < assets/demo/seed.sql
printf '%s\n' "$demo_dir/studio.sqlite"
open -n /Applications/Oxyn.app --args --temporary-workspace
```

Ces enregistrements utilisent l’application macOS installée. Pour essayer le même
parcours avec les sources du checkout, utilisez plutôt `make desktop-dev`.

Dans l’application temporaire :

1. Créez une connexion SQLite nommée **Studio Commerce** avec le chemin affiché.
   Choisissez l’environnement **Development** et activez la lecture seule.
2. Connectez-vous, développez le catalogue et ouvrez **orders**. Capturez les
   onglets **Data** et **Structure** ; incluez **DDL** dans la vidéo d’exploration.
3. Ouvrez une console SQL, saisissez [revenue.sql](../../../../assets/demo/revenue.sql),
   sélectionnez explicitement le SQL, puis cliquez sur **Run**. Capturez l’éditeur et les résultats terminés.

Capturez uniquement la fenêtre de démonstration. Excluez les autres fenêtres,
notifications, chemins personnels et identifiants du cadre. N’enregistrez pas
un workspace existant. Conservez les pixels d’origine pour les captures et
encodez les vidéos en MP4 H.264 sans piste audio. Vérifiez visuellement le
résultat avant de remplacer ces fichiers. La base et les enregistrements bruts
restent des fichiers de travail locaux, hors des ressources du dépôt.
