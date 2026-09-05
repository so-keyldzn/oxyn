# Revue d'interface

Comportements dans `docs/UX-SPEC.md`, budgets dans `docs/PERFORMANCE.md`.

## Chemin d'exécution

- [ ] La vue produit des `Command` ; elle n'appelle aucun driver, même « en
      attendant »
- [ ] Aucun `block_on`, `blocking_*`, ni I/O sur le thread UI

## Les cinq états

- [ ] Initial — explique quoi faire
- [ ] En cours — progression **et moyen d'annuler**, jamais un simple gel
- [ ] Peuplé
- [ ] **Vide** — visiblement distinct d'une erreur. C'est celui qu'on oublie, et
      le premier que voit un nouvel utilisateur
- [ ] Erreur — ce qui a échoué, si c'est retentable, l'action suivante

## Annulation

- [ ] Toute opération dépassant 300 ms est annulable
- [ ] L'annulation atteint le **serveur** — sinon le bouton ment et laisse une
      connexion prise

## Capacités

- [ ] Aucune surface ne suppose des tables, un schéma ou du SQL sans vérifier les
      capacités de la session
- [ ] Ce qui est indisponible est **expliqué**, pas masqué sans raison ni échoué
      sans message

## Écritures

- [ ] Aucun affichage optimiste sur une opération qui écrit
- [ ] Sur `production` : confirmation nommant la connexion, SQL exact, estimation
      des lignes touchées
- [ ] Le bouton par défaut n'est jamais l'action destructrice

## Grille et résultats

- [ ] Lecture directe des `RecordBatch`, sans conversion en lignes
- [ ] Le défilement au-delà du budget mémoire lit une page disque et **ne relance
      jamais la requête**
- [ ] Aucun décodage de gros lot sur le thread UI

## Erreurs affichées

- [ ] Le message du serveur est montré, code compris — pas une paraphrase
- [ ] Aucun identifiant de connexion, aucune valeur liée, y compris dans un
      panneau de débogage

## Accessibilité — bloquant

- [ ] Toute vue nouvelle est atteignable au clavier
- [ ] Le focus est visible
- [ ] L'ordre de tabulation suit l'ordre de lecture
- [ ] Aucune information portée par la seule couleur

Ce n'est pas une finition : `docs/adr/0001-ui-toolkit.md` identifie
l'accessibilité comme un risque structurel de GPUI. Rattraper après coup coûte
une réécriture.

## Performance

- [ ] Budgets mesurés avec les instruments du système, **pas** avec `criterion`
- [ ] Un budget qui ne peut pas être tenu est amendé **par un ADR**, jamais en
      silence
