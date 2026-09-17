---
name: gpui-spawn-et-runtime-etranger
description: Piège GPUI — deux cx.spawn qui attendent le même runtime Tokio reviennent dans un ordre non garanti, et la réponse la plus ancienne écrase la plus récente ; en test, run_until_parked seul ne suffit jamais
metadata:
  type: feedback
---

Un `cx.spawn` qui `await` une réponse venue du runtime **Tokio** du backend
(`oneshot`, `mpsc`) revient quand ce runtime veut. Deux conséquences distinctes,
et la première n'est pas un problème de test :

**1. Deux lectures qui se chevauchent s'écrasent en silence.** Rien dans GPUI ne
sérialise deux `cx.spawn` sur la même donnée : celui parti en premier peut
atterrir en dernier et remettre l'état comme avant. Le symptôme est un état
correct pendant une fraction de seconde puis faux, sans erreur nulle part.

**Why:** rencontré sur la liste des fournisseurs IA. Le workspace la lit à sa
construction ; l'écran de réglages en redemande une après un enregistrement. La
réponse vide de la première lecture arrivait après la seconde une fois sur six —
un fournisseur enregistré, et une entrée qui n'apparaît jamais.

**How to apply:** toute lecture asynchrone rejouable porte un **jeton** que
l'application vérifie au retour (`if read != self.…read { return; }`). C'est le
patron déjà employé ailleurs dans `oxyn-app` — `console_attempt`,
`preview_active` comparent un `CommandId`. Le test qui l'ancre n'a pas besoin de
provoquer la course : appeler la méthode de réception avec un jeton périmé et
vérifier que l'état ne bouge pas suffit, et il rougit dès qu'on retire le
contrôle.

**2. En test, `run_until_parked()` ne fait pas avancer Tokio.** L'exécuteur de
`gpui::test` est déterministe et n'a aucune horloge commune avec le runtime du
backend ; `run_until_parked` rend la main avant que la réponse existe. Le harnais
du dépôt a `crate::workspace::tests::wait_until(&vue, cx, "ce qui manque", |vue,
cx| …)` : une boucle bornée qui cède du temps réel entre deux trames. À utiliser
dès qu'un test attend quoi que ce soit qui traverse `Backend`. **Ne pas en
réécrire une copie locale** : elle existe dans le module de harnais partagé.

Voir aussi [[gpui-debug-bounds-et-clics]] : un `wait_until` qui expire alors que
la condition est vraie signale souvent l'autre piège, pas celui-ci.
