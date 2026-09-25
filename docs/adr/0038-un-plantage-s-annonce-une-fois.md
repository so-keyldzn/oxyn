# ADR-0038 — Un plantage s'annonce une fois, et ⌘Q passe par l'arrêt ordonné

**Statut :** accepté · **Date :** 2026-09-25

**Précise :** [ADR-0021](0021-marqueur-d-arret.md), sur ce qu'il advient d'une
session abandonnée une fois constatée, et sur les chemins de fermeture qui
inscrivent `closed_at`.

## Contexte

L'application installée affichait l'écran de reprise — « Oxyn did not close
normally » — après **chaque** fermeture ordinaire : exactement le défaut
qu'[ADR-0021](0021-marqueur-d-arret.md) devait supprimer. La base d'un
workspace réel, lue le 2026-09-25, portait 17 sessions dont 16 sans
`closed_at`. Deux causes, indépendantes.

**Une session abandonnée l'était pour toujours.** ADR-0021 définit l'arrêt
anormal comme « une session antérieure sans fermeture au battement vieilli »,
sans dire ce qu'elle devient après avoir été annoncée. Rien ne la soldait : un
seul plantage — ici un lancement du 2026-09-10 — rendait anormaux tous les
lancements suivants, fermetures propres comprises.

**⌘Q et le menu Quit n'inscrivaient jamais la fermeture.** L'élément Quit
prédéfini de Tauri envoie `terminate:` à l'application. tao 0.35.3 traite
`applicationWillTerminate` mais pas `applicationShouldTerminate` : macOS
termine la boucle d'événements par un `RunEvent::Exit`, sans `ExitRequested`,
et rien ne peut retenir la sortie le temps de vider les brouillons et
d'inscrire la fermeture. Seul le bouton de fermeture de la fenêtre, qui passe
par `CloseRequested`, l'inscrivait : c'est la seule session close de la base.

## Décision

**Le lancement qui constate un abandon le marque annoncé.** La migration 17
ajoute `app_sessions.reported_at`. `Sessions::begin` le renseigne sur les
sessions abandonnées et non encore annoncées, dans la transaction qui inscrit
le nouveau lancement ; seule une session abandonnée **et non annoncée** fait
conclure à un arrêt anormal. `closed_at` reste `NULL` : la ligne dit toujours
que ce lancement ne s'est pas fermé proprement. Si le lancement qui annonce
plante à son tour, il devient lui-même une session abandonnée non annoncée, et
le suivant propose de nouveau la reprise.

**Sur macOS, Oxyn remplace le Quit prédéfini par le sien.** Le menu par défaut
de Tauri est reconstruit à l'identique, sauf le dernier élément du menu de
l'application : un élément `oxyn-quit` (`CmdOrCtrl+Q`) qui déclenche l'arrêt
ordonné du bouton de fermeture (`crates/oxyn-desktop/src/commands/recovery.rs`).
Ailleurs, Tauri ne pose pas de menu, et la fermeture de la fenêtre reste la
sortie.

**Chaque abandon laisse une ligne de journal.** `info` quand la fermeture est
inscrite ; `warn` quand aucune webview n'est abonnée, quand le vidage des
brouillons n'est pas confirmé, quand la fermeture n'est pas inscrite et quand
l'application sort sans l'arrêt ordonné. Le journal est vidé sur disque avant
la sortie, dans une attente bornée à une seconde.

## Conséquences

* **+** L'écran de reprise redevient un signal : il ne suit plus une fermeture
  par ⌘Q, par le menu ou par le bouton de la fenêtre.
* **+** Le plantage reste lisible sans Oxyn ([I-11](../../CLAUDE.md#i-11)) :
  `closed_at IS NULL` le dit, `reported_at` dit quand il a été annoncé.
* **+** Un prochain diagnostic part d'un journal qui dit comment la fermeture
  s'est passée, et non d'un journal muet.
* **−** Le Quit du Dock et la fermeture de session macOS passent encore par
  `terminate:` : la fermeture n'y est pas inscrite, et le lancement suivant
  propose la reprise. C'est le sens prudent, et le journal le dit.
* **−** Oxyn porte une copie du menu d'application de Tauri : une évolution du
  menu par défaut ne lui parviendra pas d'elle-même.
* **−** Une migration de plus. Au premier lancement qui l'applique, les
  sessions déjà abandonnées sont annoncées une dernière fois.

**Coût de sortie :** une colonne, une clause de requête et un menu. Retirer le
menu rend ⌘Q à `terminate:` ; retirer `reported_at` rend la reprise
permanente après un plantage.

**Reconsidérer si** tao expose `applicationShouldTerminate` : la sortie par
`terminate:` deviendrait alors retenable, pour le Dock comme pour le menu, et
le Quit propre à Oxyn n'aurait plus de raison d'être.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Solder une session abandonnée en renseignant son `closed_at` | Récrirait un plantage en arrêt propre, dans la table même qui doit les distinguer |
| Effacer une session abandonnée une fois annoncée | Efface la seule trace lisible sans Oxyn qu'un plantage a eu lieu |
| Inscrire la fermeture de façon synchrone dans `RunEvent::Exit` | Les brouillons ne peuvent plus y être vidés (la webview répond par le thread principal qu'on bloquerait) : ce serait marquer un arrêt propre sur un travail peut-être non écrit |
