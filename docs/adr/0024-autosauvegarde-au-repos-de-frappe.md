# ADR-0024 — Le brouillon s'écrit quand la frappe s'arrête, pas à chaque touche

**Statut :** proposé · **Date :** 2026-09-11

**Précise :** [ADR-0016](0016-autosauvegarde-bornee.md), sur le moment où un
brouillon quitte le fil d'interface.

## Contexte

Une mesure du 2026-09-11 a trouvé le seul dépassement de budget de tout le
produit. `xcrun xctrace`, gabarit `Animation Hitches`, attaché au processus :

| Condition | À-coups en ~14 s | Pire |
|---|---|---|
| Au repos | 0 | — |
| Redimensionnement continu | 1 | 10 ms |
| **Saisie continue** | **17** | **50 ms** |

Le budget de [PERFORMANCE](../PERFORMANCE.md#budgets-dinteraction) est de **8 ms
p99** pendant une interaction continue, et la saisie est nommément l'une des
trois interactions visées. Cinquante millisecondes, c'est six fois le budget, sur
le geste le plus fréquent du produit.

La cause est dans le code, pas dans le rendu. À chaque `EditorEvent::Changed`,
`QueryConsole::document_changed` :

1. appelle `editor.text()`, qui est un `self.lines.join("\n")` — donc une
   **copie complète du document** ;
2. construit un `QueryDocumentUpdate` entier, texte compris ;
3. le valide ;
4. l'inscrit dans la file d'écriture.

Les trois premières étapes sont sur le fil d'interface. `MAX_QUERY_DOCUMENT_BYTES`
vaut 1 Mio : à la borne, **chaque caractère tapé recopie un mégaoctet** avant que
la file n'ait quoi que ce soit à faire.

Ce qui n'est **pas** en cause, et qu'il fallait écarter : le rendu de l'éditeur
est virtualisé par `uniform_list` et ne dessine que les lignes visibles ;
`oxyn_query::current_statement` tient en 55 µs et n'est appelé qu'à la
soumission. ADR-0016 a bien borné la **file** — une opération active, au plus un
brouillon en attente — mais n'a rien dit du coût payé **avant** d'y entrer.

## Décision

**Une frappe marque le brouillon sale ; elle ne le copie pas.** La copie, la
validation et l'envoi n'ont lieu qu'après **250 ms sans frappe**, ou
immédiatement quand la console perd le focus, se ferme, ou qu'une exécution est
lancée.

Le délai est relancé à chaque touche : une frappe continue n'écrit donc rien tant
qu'elle dure, et écrit une fois quand elle s'arrête. Le fil d'interface ne porte
plus qu'un drapeau et un minuteur.

**250 ms, et pourquoi ce chiffre.** Il est inférieur au budget de 300 ms de
« retour visible après une frappe » : un utilisateur qui s'arrête de taper ne
peut pas percevoir le retard de l'écriture, puisqu'il est plus court que le délai
au-delà duquel il cesserait de faire le lien avec son action. Il est aussi plus
long qu'un intervalle entre deux touches d'une frappe rapide — autour de 100 ms
—, ce qui est la condition pour qu'une phrase tapée d'un trait ne produise
**qu'une** écriture. Ce n'est pas une mesure : c'est un choix de produit, et il
s'amende ici.

**Ce que la fenêtre de 250 ms coûte, énoncé sans détour.** Un arrêt brutal dans
cet intervalle perd jusqu'à 250 ms de frappe — quelques caractères. Avant cette
décision, il n'en perdait aucun. C'est le prix, et il est payé sciemment : un
éditeur qui saccade à chaque touche sur un document long est un défaut que
l'utilisateur subit à chaque seconde, là où la perte de quelques caractères
suppose un plantage.

**Les trois échappées sont immédiates**, et ce sont elles qui bornent la perte :
perte de focus, fermeture de console, lancement d'une exécution. Les deux
premières couvrent le geste ordinaire — on quitte un onglet, on va ailleurs ; la
troisième garantit que ce qui s'exécute est ce qui est écrit.

**Ce qui ne change pas** : la file d'ADR-0016, ses révisions attendues, son
compteur de fermeture, et le fait qu'un brouillon suspendu par une fermeture
annulée se reprend. Cette décision ne touche que le moment où l'on entre dans la
file.

## Conséquences

- **+** Le fil d'interface ne copie plus le document à chaque touche : c'est la
  cause mesurée du seul dépassement de budget du produit.
- **+** Une phrase tapée d'un trait produit une écriture SQLite au lieu d'une par
  caractère. Sur une machine portable, c'est aussi une écriture disque de moins
  par frappe.
- **+** Le coût cesse de dépendre de la **taille** du document. Aujourd'hui, plus
  un brouillon est long, plus chaque touche coûte cher — la dégradation est donc
  invisible sur un document court et sévère sur celui qu'on travaille depuis une
  heure.
- **−** Jusqu'à 250 ms de frappe perdus sur un arrêt brutal. L'écran de reprise
  d'[ADR-0021](0021-marqueur-d-arret.md) restaure alors un texte à quelques
  caractères près de ce qui était à l'écran.
- **−** Un minuteur de plus dans la console, donc un état de plus à défaire
  correctement à la fermeture — et un test à écrire pour ça, sans quoi une
  écriture arrive après la fermeture qu'elle était censée précéder.
- **−** Le délai est un réglage qui n'en est pas un : il n'est exposé nulle part,
  et le changer demande de revenir ici.

**Coût de sortie :** retirer le minuteur et rappeler la copie directement dans
`document_changed`. La file, les révisions et la reprise ne dépendent pas de
cette décision.

**Reconsidérer si** une mesure montre que la copie n'est plus le coût dominant —
par exemple si `TextBuffer` cesse d'être un `Vec<String>` joint à la demande —,
ou si l'usage montre que 250 ms perdus sont de trop, auquel cas la réponse est un
délai plus court, pas la suppression du minuteur.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Laisser la file lire le texte au moment d'écrire | La file tourne sur le runtime Tokio et ne peut pas lire une entité GPUI ; il faudrait un canal inverse vers le fil d'interface, donc le même coût déplacé |
| Copier seulement les lignes modifiées | Demande un format de brouillon incrémental, donc une migration et un format de plus à porter — pour un gain qu'un anti-rebond obtient sans rien changer au stockage |
| Écrire à chaque touche mais hors du fil d'interface | La copie elle-même est le coût, et elle ne peut avoir lieu que là où vit l'éditeur |
| Un anti-rebond par nombre de caractères plutôt que par temps | Une pause de dix secondes après trois caractères n'écrirait rien : le critère qui compte est l'arrêt, pas le volume |
| Allonger le délai à 1 s pour écrire encore moins | Fait perdre une seconde de frappe sur un plantage, pour un gain nul : la frappe est déjà coalescée à 250 ms |
