<!-- oxyn-translation source="docs/adr/0047-english-as-the-repository-language.md" sha256="e3a11e8ad738" -->

> Traduction française de [docs/adr/0047-english-as-the-repository-language.md](../../../../docs/adr/0047-english-as-the-repository-language.md). **La version anglaise fait foi.**

# ADR-0047 — L'anglais est la langue du dépôt ; le français vit dans des miroirs dont l'anglais fait foi

**Statut :** accepté · **Date :** 2026-09-27 · **Décideurs :** Nicolas Boromée

**Amende :** la règle de langue de [CLAUDE.md](../../CLAUDE.md), qui mettait
jusqu'ici le code en anglais et la documentation, les ADR et les messages de
commit en français.

## Contexte

Oxyn devient un dépôt public ouvert aux contributions externes
([CONTRIBUTING.md](../../CONTRIBUTING.md), [CLA.md](../../../../CLA.md)). Le
2026-09-27, ce qu'un contributeur doit lire avant d'écrire du code était en
français :

* `CLAUDE.md` et ses treize invariants, `AGENTS.md`, `README.md` ;
* `.claude/` : 7 règles, 12 commandes, 12 agents, 4 checklists, 3 gabarits,
  environ 2 600 lignes — c'est aussi ce que reçoit la session Claude du
  contributeur ;
* `docs/` : environ 16 400 lignes, 46 ADR ;
* le format de commit, imposé par `.claude/hooks/message_commit.py`.

Seuls le code, `CONTRIBUTING.md` et le CLA étaient en anglais. Un développeur
anglophone pouvait signer le CLA mais pas lire les invariants qu'on lui demande
de respecter. La violation d'un invariant est silencieuse par construction : un
invariant que personne ne peut lire n'est pas tenu.

Le mainteneur travaille en français et veut que la documentation reste lisible
en français.

## Décision

1. **La langue du dépôt est l'anglais** : code, identifiants, commentaires,
   messages d'erreur, documentation, ADR, messages de commit, pull requests,
   issues. Les nouveaux documents et ADR s'écrivent en anglais.
2. **Le socle est traduit maintenant** : `CLAUDE.md`, `AGENTS.md`, `README.md`,
   `docs/README.md`, toute la prose de `.claude/` (règles, commandes, agents,
   checklists, gabarits, workflows), les messages des hooks, les modèles de
   pull request et d'issue.
3. **Les documents d'autorité de `docs/` et les ADR 0001 à 0046 restent en
   français** et font autorité tels quels. L'un d'eux se traduit dans un commit
   dédié qui ne change aucune décision — ADR acceptés compris, puisqu'une
   traduction ne décide rien — avant toute modification substantielle. Une
   petite correction reste dans la langue actuelle du document.
4. **Miroirs français, l'anglais fait foi.** Chaque document traduit a un
   miroir français à `i18n/fr/<chemin>`, où `.claude/` devient `claude/`. Le
   miroir commence par `<!-- oxyn-translation source="<chemin>" sha256="<12 hex>" -->`,
   l'empreinte de la source anglaise qu'il traduit. `verifier_socle.py`, donc
   `make socle` et `make qualite`, refuse un miroir dont la source n'a plus
   cette empreinte, et affiche celle attendue. Un miroir ne décide rien : en
   cas d'écart, l'anglais l'emporte.
5. **Les messages de commit** gardent le format Conventional Commits imposé par
   le hook (`type(portee): sujet`, impératif, minuscule initiale, sans point
   final), en anglais.

La langue des échanges entre le mainteneur et Claude est un réglage personnel,
pas une règle du dépôt : elle sort de `CLAUDE.md`.

## Conséquences

**Positives.**

* Un contributeur anglophone lit les invariants, les règles et les procédures
  avant d'écrire du code ; sa session Claude aussi.
* L'historique, les pull requests et les issues deviennent lisibles par tous.
* Les miroirs ne peuvent pas dériver en silence : un miroir périmé fait échouer
  la porte de qualité.

**Négatives.**

* Deux copies de chaque document du socle. Toute modification d'un document
  anglais qui a un miroir exige une mise à jour française dans la même pull
  request. Un contributeur qui n'écrit pas le français ne peut pas faire passer
  la porte seul : il le signale dans la pull request, et le mainteneur met le
  miroir à jour avant la fusion.
* Les documents d'autorité restent en français pour l'instant : un
  contributeur anglophone lit encore `docs/SECURITY.md` ou
  `docs/DRIVER-CONTRACT.md` en français, ou via un outil de traduction. Les
  ancres vers ces documents restent françaises.
* L'historique est bilingue : français avant le 2026-09-27, anglais après.
* Les identifiants Python des hooks, les noms des cibles `make`, des scripts,
  des commandes et des agents (`make qualite`, `/implementer`, `rustacien`)
  restent en français. Les renommer change les habitudes de chacun et la CI ;
  c'est une décision à part.

**Coût de sortie.** Faible : supprimer `i18n/` et le contrôle de
`verifier_socle.py` laisse un dépôt uniquement anglais ; revenir au français
revient à retraduire le socle.

**À reconsidérer si** la tenue des miroirs bloque régulièrement les pull
requests de contributeurs non francophones, ou si les miroirs cessent d'être
lus — le contrôle ne serait alors plus qu'un coût.

## Alternatives écartées

* **Une simple porte d'entrée anglaise** (README, modèles, un guide qui résume
  les invariants). Deux versions des invariants, dont une non vérifiée : elles
  divergent en quelques semaines, et le contributeur lit la périmée.
* **Tout en anglais maintenant, `docs/` et les 46 ADR compris.** Plusieurs
  jours de traduction pendant lesquels les documents d'autorité continuent de
  changer ; le risque de dérive pendant la traduction l'emporte sur le
  bénéfice. La traduction progressive du point 3 atteint le même état sans ce
  risque.
* **Fichiers bilingues côte à côte (`rust.fr.md`).** Claude Code charge tout
  `*.md` de `.claude/rules`, `.claude/commands` et `.claude/agents` : un fichier
  français à côté de l'anglais chargerait une seconde règle, commande ou agent.
* **Le français fait foi, miroir anglais.** Les contributeurs pour qui ce
  changement est fait liraient une traduction, et ne pourraient pas corriger la
  version qui compte.
