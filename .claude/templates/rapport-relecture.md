# Rapport de relecture — <périmètre>

**Date :** AAAA-MM-JJ · **Périmètre :** `<chemins ou plage de commits>`

## Verdict

<Une phrase. S'il n'y a rien à signaler, cette section suffit et le rapport
s'arrête ici. Ne pas inventer de remarques pour justifier l'exécution : un
rapport qui trouve toujours quelque chose finit par n'être plus lu, et c'est
alors que le vrai problème passe.>

## Points, par gravité décroissante

### 1. <titre court> — bloquant / à corriger / à considérer

| | |
|---|---|
| Où | `chemin/fichier.rs:42` |
| Ce qui est enfreint | I-NN, ou le document d'autorité et sa section |
| Scénario de panne | *Ce qui arrive à un utilisateur réel. Pas la règle récitée.* |
| Correction | … |

<Répéter par point.>

## Ordre de gravité

1. invariant enfreint — bloquant
2. divergence code / documentation — bloquant, c'est un bug par définition
3. contrat non tenu
4. budget dépassé sans mesure justifiant l'écart
5. le reste

## Ce dont je ne suis pas sûr

<Les doutes, nommés. Un doute signalé vaut mieux qu'une certitude fabriquée.>

## Ce qui n'a pas été relu

<Le périmètre non couvert, et pourquoi. Un lecteur suppose que tout a été vu.>
