<!-- oxyn-translation source="docs/adr/0084-load-visible-schema-scopes-on-demand.md" sha256="98e1092f4249" -->

# ADR-0084 — Charger à la demande les périmètres de schéma visibles

**Statut :** proposé · **Date :** 2026-11-15

**Précise :** [ADR-0082](0082-project-schema-maps-from-provenanced-catalog-metadata.md)
sur le geste autorisant la lecture, et
[ADR-0081](0081-load-catalog-facets-independently-with-explicit-prefetch.md)
sur la présentation du préchargement global facultatif.

## Contexte

Choisir une base laisse encore la carte derrière un bouton Charger. Le dialogue
de métadonnées place le préchargement à côté des préférences, suggérant une
préparation obligatoire. Facettes et complétion utilisent déjà des lectures
bornées à la demande. Ces gestes rendent l’interface incohérente sans apporter
d’autorité utile supplémentaire pour les métadonnées ordinaires.

## Décision

Ouvrir une carte visible ou choisir son périmètre natif exprime la demande
humaine de le voir. Le contrôleur appelle immédiatement LoadSchemaMap borné sur
la connexion/session originale. Les montages répétés sont regroupés ; changer
le périmètre annule la requête exacte et rejette les réponses tardives. Une carte
masquée ne lance pas de nouvelle lecture. Un enfant natif unique peut être suivi
automatiquement ; une ambiguïté exige un choix, sans namespace deviné. Aucun
parcours de toutes les bases n’est ajouté.

Actualisation, annulation, expansion explicite des voisins et couverture partielle
restent visibles. Filtres, déplacement, zoom et sélection restent locaux. Aucune
source éphémère fermée ou fichier supprimé n’est rouvert. L’échantillonnage
documentaire conserve sa confirmation et ses bornes ; choisir un périmètre ne
lit aucun échantillon et n’appelle pas l’IA.

Les préférences décrivent la lecture à la demande. Le réchauffement global
facultatif du cache est replié sous Avancé, sans prérequis pour ouvrir une vue.
Enregistrer, déplier Avancé ou ouvrir les réglages ne lance aucun préchargement.
Bornes et annulation backend existantes sont conservées.

Les actions source utilisent la sidebar shadcn et des Hugeicons sémantiques.
Carte, transfert et résultat ont des icônes distinctes. Tokens du thème,
clavier, propriété des sources et confirmations sûres restent applicables.

## Conséquences

- **+** Choisir ce que l’on veut voir affiche directement les métadonnées disponibles.
- **+** Comportement commun aux moteurs selon leurs périmètres/capacités.
- **−** Un changement de périmètre visible déclenche une lecture bornée ; état de
  chargement, annulation et absence de boucle de réessai sont indispensables.
- **−** Ambiguïtés et inférence exigent encore choix ou consentement ; les bornes
  peuvent produire une carte partielle.

**Coût de sortie :** rétablir le geste Charger dans contrôleur/vue et tests, sans
changer persistance ou contrat de driver.

**Réexaminer si** les mesures dépassent les budgets ou si la recette montre des
sélections accidentelles coûteuses. Ne pas relever les bornes ni échantillonner
automatiquement pour contourner le problème.

## Alternatives rejetées

| Alternative | Motif |
|---|---|
| Précharger toutes les bases connectées | Travail non borné sans rapport avec la vue |
| Échantillonner automatiquement | Coût/confidentialité modifiés sans consentement |
| Garder sélection puis Charger | Conserve la friction signalée |
| Supprimer le backend en masquant ses contrôles seulement | Contrats/appelants incohérents |
