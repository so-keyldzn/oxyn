# ADR-0040 — Une sortie que macOS ne laisse pas retenir inscrit sa fermeture

**Statut :** accepté · **Date :** 2026-09-25

**Précise :** [ADR-0038](0038-un-plantage-s-annonce-une-fois.md), sur
l'alternative « inscrire la fermeture dans `RunEvent::Exit` », qu'il écartait.

## Contexte

[ADR-0038](0038-un-plantage-s-annonce-une-fois.md) fait passer ⌘Q et le menu
Quit par l'arrêt ordonné. Il laisse une limite : **le Quit du Dock et la
fermeture de session macOS** envoient encore `terminate:` à l'application. La
fermeture n'y est pas inscrite, et le lancement suivant propose la reprise
après une sortie ordinaire. C'est le défaut même qu'ADR-0038 corrigeait, sur
un chemin de plus.

Le seul point où ce chemin peut être retenu est `applicationShouldTerminate:`,
sur le délégué d'application. tao ne l'enregistre pas, ni la version épinglée
(0.35.3) ni la dernière publiée (0.37.0), et Tauri 2.11.5 n'ajoute rien. tao
traite seulement `applicationWillTerminate:`, qui produit `RunEvent::Exit` sur
le thread principal. macOS termine le processus dès que ce rappel rend la main.
Sources datées dans [RESEARCH-NOTES](../RESEARCH-NOTES.md#interface-tauri-et-front).

Ajouter la méthode au délégué de tao demanderait du `unsafe` dans
`oxyn-desktop`, que le workspace refuse ([SECURITY](../SECURITY.md#politique-unsafe)).

ADR-0038 écartait l'inscription dans `RunEvent::Exit` pour une raison :
les brouillons ne peuvent plus y être vidés, parce que la webview répond par le
thread principal que ce rappel occupe. Or l'arrêt ordonné accepte déjà ce
cas : si la webview ne confirme pas le vidage dans les 2 s, la fermeture est
inscrite quand même, parce qu'un brouillon est écrit au plus tard 250 ms après
la dernière frappe ([ADR-0024](0024-autosauvegarde-au-repos-de-frappe.md)). Ce que
`RunEvent::Exit` perd est donc ce que l'arrêt ordonné tolère déjà : au plus
les 250 dernières millisecondes de frappe.

## Décision

**Dans `RunEvent::Exit`, si l'arrêt ordonné n'a pas eu lieu, Oxyn inscrit la
fermeture après les écritures locales, dans une attente bornée à 2 s.**

- `Backend::close_on_forced_exit` (`crates/oxyn-desktop/src/backend/recovery.rs`)
  marque l'arrêt comme commencé, puis lance sur le runtime la même
  `close_session_after_local_writes` que l'arrêt ordonné. La lecture et
  l'écriture du store se font sur le pool bloquant.
- Le thread principal attend la fin de cette tâche sur un canal, au plus
  `FORCED_EXIT_GRACE` (2 s, `crates/oxyn-desktop/src/commands/recovery.rs`). La
  fenêtre est déjà partie : cette attente ne fige rien
  ([I-05](../../CLAUDE.md#i-05)), comme le vidage du journal déjà fait au même
  endroit.
- Les brouillons ne sont pas vidés. Aucune demande n'est envoyée à la webview.
- Si une écriture locale est encore en cours au terme de l'attente, la
  fermeture **peut rester non inscrite**. La tâche n'est pas annulée : elle
  l'inscrit si l'écriture finit avant que macOS ne tue le processus, et
  toujours après elle. Sinon, le lancement suivant propose la reprise, et le
  journal le dit (`warn`).
- Les instructions en cours sur les serveurs ne sont pas annulées sur ce
  chemin. Rien ne change sur ce point : elles ne l'étaient pas avant.

## Conséquences

* **+** Le Quit du Dock et la fermeture de session ne font plus proposer la
  reprise après une sortie ordinaire.
* **+** Aucun `unsafe`, aucune dépendance à un comportement privé de tao : le
  correctif ne tient qu'à `RunEvent::Exit`, que Tauri documente.
* **+** La règle d'ADR-0021 tient : la fermeture n'est jamais écrite par-dessus
  une écriture locale en cours.
* **−** Sur ce chemin, les 250 dernières millisecondes de frappe peuvent être
  perdues sans que la reprise soit proposée. C'est la tolérance de l'arrêt
  ordonné face à une webview muette, étendue à un cas où la webview n'est pas
  même interrogée.
* **−** Le thread principal peut attendre jusqu'à 2 s en plus de la seconde du
  journal. macOS peut forcer la fin d'une fermeture de session plus lente.
  Dans ce cas, la fermeture n'est pas inscrite, ce qui est le sens prudent.
* **−** Une instruction serveur en cours n'est pas annulée : elle continue
  jusqu'à ce que le serveur constate la coupure.
* **−** Une écriture n'est comptée qu'au début de sa commande Tauri. Une
  écriture partie de la webview mais pas encore lancée échappe à l'attente,
  pendant quelques millisecondes. L'arrêt ordonné a la même fenêtre.

**Coût de sortie :** une méthode et une branche de `on_run_event`. Les retirer
rend ce chemin à l'état décrit par ADR-0038 : fermeture non inscrite, reprise
proposée.

**Reconsidérer si** tao enregistre `applicationShouldTerminate:`. La sortie
par `terminate:` deviendrait alors retenable et passerait par l'arrêt ordonné
complet, brouillons vidés. Ce chemin et le Quit propre à Oxyn n'auraient plus
de raison d'être.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Ajouter `applicationShouldTerminate:` à la classe du délégué de tao (`class_addMethod`) | Exige du `unsafe` dans `oxyn-desktop`, et lever ce refus pour un seul appel est une décision d'architecture disproportionnée. Le correctif dépendrait aussi du nom d'une classe privée de tao |
| Garder la limite (reprise proposée après le Quit du Dock) | L'écran de reprise cesse d'être un signal pour qui quitte depuis le Dock, soit le défaut qu'ADR-0038 corrigeait |
| Inscrire la fermeture sans attendre les écritures locales | Marquerait un arrêt propre par-dessus une écriture peut-être non faite, ce qu'ADR-0021 interdit |
| Attendre sans borne | Une écriture bloquée retiendrait la fermeture de session macOS, qui finirait par tuer le processus |
