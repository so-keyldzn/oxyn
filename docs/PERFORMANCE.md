# Budgets de performance

> **Autorité** : les seuils chiffrés au-delà desquels un comportement est un
> défaut, et non une lenteur acceptable.

Invariants concernés : [I-05](../CLAUDE.md#i-05), [I-06](../CLAUDE.md#i-06).
Dérive de [ADR-0002](adr/0002-arrow-result-model.md) pour tout ce qui touche aux
résultats.

> **Statut des chiffres.** Le code existe et des tests vérifient des bornes
> locales ; aucune campagne de performance complète n'a encore été effectuée.
> Les valeurs ci-dessous sont des **budgets décidés**, dérivés des seuils
> de perception humaine, pas des mesures constatées. Elles servent à faire
> échouer un banc d'essai, pas à décrire l'existant. La première campagne de
> mesure doit soit les confirmer, soit les amender **par un ADR** — jamais en
> les ajustant en silence pour faire passer un test.

## Pourquoi des budgets et pas des « bonnes pratiques »

« La latence perçue est une fonctionnalité » ([VISION](VISION.md)) ne veut rien
dire tant qu'aucun nombre ne permet de dire qu'on l'a ratée. Une régression de
performance qui n'a pas de seuil ne se détecte jamais : elle s'accumule par
tranches de 15 ms que personne ne remarque, jusqu'à ce que le produit soit
devenu lent sans qu'aucun commit ne soit coupable.

## Budgets d'interaction

Les seuils viennent de la perception : ~16 ms est la trame à 60 Hz, ~100 ms est
la limite de la réaction « instantanée », ~1 s est celle où l'attention décroche.

| Interaction | Budget | Ce qui se passe au-delà |
|---|---|---|
| Trame d'interface pendant une interaction continue (défilement, saisie, redimensionnement) | **8 ms** p99 | saccade visible ; c'est le symptôme n° 1 d'une violation de [I-05](../CLAUDE.md#i-05) |
| Retour visible après un clic ou une frappe | **100 ms** | l'utilisateur re-clique, croyant avoir raté |
| Premières lignes affichées après lancement d'une requête | **300 ms** après la première réponse du serveur | passé ce délai, l'utilisateur ne fait plus le lien entre son action et le résultat |
| Ouverture de la fenêtre au démarrage à froid | **1 s** | un client natif qui démarre plus lentement qu'un client web perd son argument principal |
| Développement d'un nœud du catalogue déjà en cache | **50 ms** | la navigation dans l'arborescence doit être ressentie comme locale |

Une opération qui ne peut pas tenir son budget ne le rate pas en silence : elle
affiche une progression et reste annulable. **Une opération longue et annulable
est acceptable ; une opération longue et figée ne l'est pas.**

## Budgets de mémoire

| Situation | Budget | Mode de panne |
|---|---|---|
| `ResultBuffer` en mémoire, par résultat | **256 Mo** par défaut, configurable ([ADR-0002](adr/0002-arrow-result-model.md)) ; au-delà, débordement en Arrow IPC relu hors thread UI ([ADR-0012](adr/0012-lecture-pages-resultats.md)) | [I-06](../CLAUDE.md#i-06) : `SELECT *` sur une grande table déclenche l'OOM killer, le processus meurt sans trace, l'utilisateur perd son travail |
| Défilement au-delà du budget mémoire | **une lecture de page disque**, jamais une nouvelle exécution | relancer la requête est doublement faux : le coût est arbitraire, et un `SELECT` peut ne pas être idempotent |
| Résultats conservés sans lecteur | 16 résultats, 256 Mio de lots résidents/cache et 1 Gio d'IPC cumulés ; contrôle après exécution et périodique ([ADR-0017](adr/0017-retention-resultats.md)) | accumulation de résultats inutilisés |
| Cache de catalogue par connexion | borné, avec éviction | dix connexions sur des bases à dizaines de milliers d'objets font grossir la RSS sans plafond |
| Définitions DDL en cache par connexion | 16 définitions et 16 Mio de SQL + notes ; éviction des anciennes valeurs, y compris invalidées ([ADR-0018](adr/0018-apercu-ddl.md)) | accumulation de scripts volumineux lors de la navigation |
| Application au repos, une connexion ouverte, aucune requête | stable dans le temps | une croissance au repos est une fuite ; elle se voit sur une session de plusieurs heures, pas dans les tests |

La rétention des lots initiaux et des pages décodées partage le budget du
résultat selon [ADR-0012](adr/0012-lecture-pages-resultats.md). L'index des lots,
les temporaires de décodage et les références tenues par des lecteurs doivent
être comptés dans les mesures du processus ; une assertion sur le cache ne
prouve pas la stabilité RSS.

## Ce qui se mesure, et comment

- **`criterion`** pour les bancs d'essai de code pur : analyse, formatage,
  conversion vers `RecordBatch`, diff de schéma. Ce sont les seules mesures
  reproductibles sur une machine de développement.
- **La conversion ligne-à-lot est un point chaud attendu**, pas une évidence :
  les drivers construits sur des pilotes ligne-à-ligne y passent par chaque
  valeur de chaque ligne ([DRIVER-CONTRACT](DRIVER-CONTRACT.md#3-il-produit-des-recordbatch-arrow-en-flux)).
  C'est le premier endroit à mesurer, et le dernier à optimiser sans mesure.
- **Les instruments du système** (Instruments, `perf`) pour le rendu et
  l'interface. Un banc `criterion` sur du GPUI ne mesure rien d'utile.
- **Aucune mesure de latence contre une base réelle n'est un banc d'essai** : le
  réseau et l'état du serveur dominent le signal. Ce qui se mesure, c'est le
  temps passé **dans Oxyn**, pas le temps d'aller-retour.

Le protocole complet est dans [`/benchmark`](../.claude/commands/benchmark.md).

## La règle qui empêche l'optimisation gratuite

**On ne remplace pas du code clair par du code rapide sans la mesure qui montre
que ça valait la peine.** Un banc d'essai avant, un banc d'essai après, le
chiffre dans le message de commit. Sans ça, la complexité est payée d'avance et
le gain est supposé.

Le corollaire vaut aussi dans l'autre sens : un `.clone()` sur un chemin appelé
une fois par ouverture de fenêtre n'est pas un problème de performance, et le
transformer en emprunt qui contamine cinq signatures est une perte nette.
