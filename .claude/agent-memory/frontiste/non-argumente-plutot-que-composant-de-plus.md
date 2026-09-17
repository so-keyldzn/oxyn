---
name: non-argumente-plutot-que-composant-de-plus
description: Sur une liste de manques d'interface, un « celui-là ne sert à rien ici » chiffré est préféré à un composant de plus
metadata:
  type: feedback
---

Quand on me donne une liste de composants à écrire, rendre un **refus argumenté**
pour ceux que le dépôt couvre déjà ou que la donnée ne permet pas, plutôt que
de livrer la liste entière.

**Why:** demandé explicitement — « je préfère un “celui-là ne sert à rien ici”
argumenté qu'un composant de plus ». Le dépôt paie chaque composant en stories,
en axe et en relecture ; un doublon de geste crée en plus deux chemins pour une
seule action, dont un seul sera relu.

**How to apply:** avant d'écrire, lire les composants voisins et chercher si le
**geste** existe déjà sous un autre nom. Le refus doit nommer le fichier qui
couvre déjà le besoin, ou la donnée backend qui manque — jamais « ça me semble
redondant ». Corollaire : si un composant demande une donnée que le backend ne
rend pas, l'exposer en prop et dire laquelle, au lieu de la calculer dans la
webview (ce que [front.md] interdit de toute façon).

**Le corollaire le plus coûteux : ne pas dessiner une prop sans source
vérifiée.** Réclamer la forme réelle (fichier et lignes) *avant* d'écrire les
types. Vécu : j'ai dessiné un plan d'agent avec des états « échoué » et
« abandonné », un `detail` d'erreur, un identifiant d'étape et un compteur de
révisions — aucun n'existe dans le protocole, et le compteur aurait **menti**,
comptant des avancements ordinaires comme des remplacements. Une surface sans
source finit vide ou remplie par une invention côté front.

Et quand une surface n'aura **jamais** de source, la supprimer et l'écrire dans
le rapport comme « écarté faute de source » : c'est une information utile, pas
un échec.

Deux règles de collaboration qui vont avec, dans ce dépôt à plusieurs agents :
ne pas modifier un fichier possédé par un autre agent, et **signaler** une
contradiction entre une consigne et un ADR au lieu de trancher (CLAUDE.md :
« signaler, ne pas trancher seul »).
