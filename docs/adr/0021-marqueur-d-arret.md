# ADR-0021 — Savoir si Oxyn s'est arrêté normalement, et le dire sans le deviner

**Statut :** accepté · **Date :** 2026-09-10

**Précise :** [ADR-0016](0016-autosauvegarde-bornee.md), sur ce que le workspace
écrit en plus de ses brouillons.

> **Précisé par [ADR-0038](0038-un-plantage-s-annonce-une-fois.md).** Une
> session abandonnée n'est annoncée qu'une fois (`reported_at`), et ⌘Q passe
> par l'arrêt ordonné. Sans ces deux points, la reprise s'affichait après chaque
> fermeture de l'application installée.

## Contexte

[UX-SPEC](../UX-SPEC.md#restauration-après-un-arrêt-brutal) décrit l'écran de
reprise comme celui qui s'affiche « au redémarrage **après un arrêt anormal** ».
Le code ne sait pas faire cette distinction.

`Recovery::load_page` (`crates/oxyn-app/src/recovery.rs`) liste les documents
dont la colonne `documents.is_open` vaut vrai. Ce drapeau ne dit qu'une chose :
ce document n'a pas été fermé **explicitement** par l'utilisateur. Il est posé à
chaque autosauvegarde et retiré par `close_query` seulement ; `QueryConsole::shutdown`
n'y touche pas. Un `⌘Q` ordinaire laisse donc tous les onglets « ouverts », et
le lancement suivant présente l'écran de reprise exactement comme après un
plantage.

C'est le défaut qui use le plus vite un tel écran : montré à chaque démarrage,
il cesse d'être lu, et le jour où une écriture a réellement été interrompue,
l'avertissement qui compte passe avec le bruit. Le message actuel est d'ailleurs
prudent au point de ne rien affirmer — « des copies disponibles », sans prétendre
avoir détecté quoi que ce soit —, ce qui est honnête mais ne rend pas le service
attendu.

Deux contraintes bornent la solution.

**Le dépôt refuse `unsafe`** ([SECURITY](../SECURITY.md#politique-unsafe),
`unsafe_code = "deny"` au workspace). Vérifier qu'un processus identifié par son
pid vit encore demande `kill(pid, 0)` ou une crate qui l'enveloppe ; et un pid
se réutilise, donc le test mentirait tôt ou tard.

**Deux instances d'Oxyn peuvent tourner en même temps** sur le même store. Un
marqueur binaire « une session est ouverte » ferait conclure à un arrêt anormal
à la seconde instance démarrée, alors que la première travaille.

## Décision

Le store porte une table `app_sessions` (migration 6) :

| Colonne | Sens |
|---|---|
| `id` | l'identité de ce lancement |
| `workspace_id` | le workspace ouvert, avec `ON DELETE CASCADE` |
| `started_at` | quand ce lancement a commencé |
| `heartbeat_at` | dernier signe de vie |
| `closed_at` | renseigné **seulement** par un arrêt propre ; `NULL` sinon |

**Un arrêt est anormal quand une session antérieure a `closed_at IS NULL` et un
`heartbeat_at` plus vieux que le seuil d'abandon.** Les deux conditions sont
nécessaires : la première distingue l'arrêt propre du reste, la seconde
distingue une instance morte d'une instance vivante.

Le battement est écrit toutes les **30 secondes**, et une session est réputée
abandonnée après **2 minutes** sans battement. Le rapport de quatre laisse
passer une machine en veille brève ou un système chargé sans conclure à un
plantage. Ces deux valeurs sont des choix de produit, pas des mesures : elles
s'amendent ici, et le battement ne s'écrit **jamais** depuis le thread
d'interface ([I-05](../../CLAUDE.md#i-05)).

**L'arrêt propre renseigne `closed_at` par les mêmes chemins qui attendent déjà
les écritures locales** — `on_app_quit` et `on_window_closed`
(`crates/oxyn-app/src/main.rs`). C'est l'ordre qui compte : la fermeture est
inscrite **après** que la file d'écriture des documents a été vidée. L'inverse
marquerait un arrêt propre sur un travail non écrit.

**Ce qu'Oxyn affiche est ce qu'il a constaté**, et rien de plus. Quand une
session abandonnée est trouvée, l'écran le dit : Oxyn ne s'est pas fermé
normalement. Sinon, l'écran de reprise **n'apparaît pas**, même s'il reste des
copies de travail — celles-ci restent accessibles par la bibliothèque, qui est
faite pour ça. Une écriture au résultat inconnu conserve son avertissement de
réconciliation, qui ne dépend pas de ce marqueur et ne se retente jamais
([I-13](../../CLAUDE.md#i-13)).

**Le pid n'est pas retenu.** Il ne survivrait pas à sa propre réutilisation, et
le vérifier demanderait ce que la politique `unsafe` du dépôt refuse. Le
battement dit la même chose sans mentir : il vieillit.

## Conséquences

- **+** L'écran de reprise redevient un signal : il n'apparaît que lorsqu'il a
  quelque chose à dire, donc il sera lu quand il le dira.
- **+** Deux instances simultanées ne se déclarent pas mutuellement mortes.
- **+** Le marqueur est une ligne de table lisible sans Oxyn
  ([I-11](../../CLAUDE.md#i-11)) : `closed_at IS NULL` se lit à l'œil.
- **−** Une écriture toutes les 30 secondes par instance, même au repos. C'est
  une ligne mise à jour dans un SQLite local ; c'est aussi une écriture disque
  périodique sur une machine portable, et elle n'existait pas avant.
- **−** Un plantage suivi d'un relancement **dans les deux minutes** ne sera pas
  reconnu comme tel : la session précédente paraît encore vivante. L'utilisateur
  ne perd rien — ses copies sont dans la bibliothèque — mais l'écran ne
  s'affichera pas. C'est le prix de ne pas accuser à tort une instance
  concurrente, et c'est le sens choisi pour l'erreur.
- **−** Une migration de plus, donc un format de plus à porter.

**Coût de sortie :** une table et deux points d'appel. Rien d'autre n'en dépend :
les brouillons continuent d'être écrits comme aujourd'hui, et l'écran de reprise
sait déjà fonctionner sans ce marqueur — c'est son état actuel.

**Reconsidérer si** le battement se révèle coûteux à la mesure, ou si Oxyn
acquiert un verrou d'instance pour une autre raison — auquel cas ce verrou dirait
la même chose sans écriture périodique.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Garder `documents.is_open` comme seul signal | Il dit « pas fermé explicitement », pas « plantage » : c'est l'état actuel, et il montre l'écran à chaque `⌘Q` |
| Un drapeau booléen « une session tourne » | Deux instances simultanées se déclareraient mutuellement anormales |
| Retenir le pid et vérifier qu'il vit | Demande `unsafe` ou une crate pour `kill(pid, 0)`, et un pid réutilisé fait mentir le test |
| Un verrou de fichier posé au démarrage | Dit qui tourne **maintenant**, pas comment le lancement précédent s'est terminé — l'information cherchée ne survit pas à la libération du verrou |
| Écrire le marqueur de fermeture avant de vider la file d'écriture | Marquerait un arrêt propre sur du travail non encore écrit : exactement le cas où la reprise doit se déclencher |
