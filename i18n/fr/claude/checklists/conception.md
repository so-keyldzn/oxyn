<!-- oxyn-translation source=".claude/checklists/conception.md" sha256="5204b11dda0d" -->

> Traduction française de [.claude/checklists/conception.md](../../../../.claude/checklists/conception.md). **La version anglaise fait foi.**

# Raisonnement d'ingénierie

Avant de modifier le dépôt, raisonner comme un spécialiste du domaine touché.
Appliquer cette liste même sans plan formel. Adapter la profondeur et
l'explication écrite à la tâche ; une petite modification ne nécessite aucun
nouveau document de planification.

## Comprendre le problème

- [ ] Énoncer le résultat visible par l'utilisateur, les contraintes et le
      critère de fin observable avant de choisir une implémentation.
- [ ] Lire les [profils de spécialité](../README.md#agents), les règles et les
      contrats d'autorité pertinents. Suivre l'implémentation actuelle et ses
      appelants ; un ancien exemple ou un motif familier ne prouve pas qu'il
      convient ici.
- [ ] Distinguer les faits vérifiés des hypothèses. Vérifier les hypothèses qui
      déterminent la correction et dire ce qui reste incertain au lieu
      d'inventer un fait.
- [ ] Pour un bug, identifier sa cause et les conditions qui le révèlent avant
      de choisir une correction ; ne pas seulement masquer le symptôme ou
      supprimer son erreur.

## Choisir une conception adaptée

- [ ] Identifier la responsabilité qui porte le changement, ses entrées, sorties
      et dépendances. Appliquer les [règles d'organisation du code](../../CLAUDE.md#organisation-du-code)
      avant d'étendre un fichier existant ou d'en créer un autre.
- [ ] Examiner ce qui peut déjà être réutilisé. Choisir la conception la plus
      simple qui satisfait les besoins réels et expliquer les compromis
      importants face à une autre solution plausible ; la familiarité seule ne
      constitue pas une justification.
- [ ] Examiner les cas d'échec propres au domaine : réponses serveur malformées
      ou partielles pour un driver ; concurrence, annulation et résultats périmés
      pour l'asynchrone ; confidentialité et frontières de confiance pour l'IA ;
      états de chargement, vides et d'erreur, accès clavier et focus pour
      l'interface ; limites de ressources pour le streaming. Retenir les cas
      applicables, sans imposer une liste rituelle à chaque tâche.
- [ ] Vérifier les contrats et appelants touchés, y compris la compatibilité et
      la récupération lorsque des données ou interfaces persistent au-delà de
      l'opération courante.

## Décider comment le démontrer

- [ ] Choisir des contrôles qui démontrent le comportement attendu et les cas
      d'échec pertinents. Préférer un scénario de régression reproductible pour
      un bug et des résultats observables à des tests qui répètent seulement
      l'implémentation.
- [ ] Étayer les affirmations de performance par des mesures et les budgets
      applicables.
- [ ] Transmettre les hypothèses importantes, les compromis et les limites de
      validation dans la relecture et le rapport final. Terminer avec la
      [liste de fin de tâche](fin-de-tache.md).
