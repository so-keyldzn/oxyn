<!-- oxyn-translation source=".claude/checklists/conception.md" sha256="e767e3bd475e" -->

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
- [ ] Avant d'écrire, repérer les comportements équivalents et identifier ce que
      le changement rend obsolète, selon les règles d'organisation du code
      ci-dessus. Choisir la conception la plus simple qui satisfait les besoins
      réels et expliquer les compromis importants face à une autre solution
      plausible ; la familiarité seule ne constitue pas une justification.
- [ ] Examiner les cas d'échec propres au domaine : réponses serveur malformées
      ou partielles pour un driver ; concurrence, annulation et résultats périmés
      pour l'asynchrone ; confidentialité et frontières de confiance pour l'IA ;
      états de chargement, vides et d'erreur, accès clavier et focus pour
      l'interface ; limites de ressources pour le streaming. Retenir les cas
      applicables, sans imposer une liste rituelle à chaque tâche.
- [ ] Vérifier les contrats et appelants touchés, y compris la compatibilité et
      la récupération lorsque des données ou interfaces persistent au-delà de
      l'opération courante.

## Évaluer les conséquences

- [ ] Retracer des scénarios concrets de défaillance liés au changement : fuite,
      perte ou corruption de données, modification non autorisée ou résultats
      partiels pris pour un succès complet. Identifier les personnes, données et
      travaux ultérieurs affectés ; considérer les scénarios applicables, pas
      tous les dangers hypothétiques.
- [ ] Évaluer la gravité, l'étendue, la détectabilité, la réversibilité et
      l'effort de récupération. Inclure le travail utilisateur perdu,
      l'interruption, la remise en état et la perte de confiance. Distinguer les
      conséquences démontrées des hypothèses ; ne chiffrer qu'avec des éléments
      probants, sans inventer de probabilités ni d'estimations de coût.
- [ ] Vérifier les états d'échec contre le contrat d'autorité : une erreur
      absorbée ou une valeur de repli ne doit pas présenter un travail échoué,
      partiel ou incertain comme un succès complet. Vérifier comment l'appelant
      ou l'utilisateur détecte le problème et quelle récupération est possible,
      en respectant le [traitement des erreurs ambiguës](../../CLAUDE.md#i-13).
- [ ] Vérifier que des diagnostics utiles et les chemins de récupération
      préservent la [gestion des secrets](../../CLAUDE.md#i-03) et la
      [frontière de confidentialité de l'IA](../../CLAUDE.md#i-04) lorsque cela
      s'applique. Suivre le [contrat de sécurité](../../docs/SECURITY.md), y
      compris en signalant une défaillance ; exposer des données sensibles
      n'est pas un raccourci de débogage.

## Décider comment le démontrer

- [ ] Choisir des contrôles qui démontrent le comportement attendu et les cas
      d'échec pertinents. Préférer un scénario de régression reproductible pour
      un bug et des résultats observables à des tests qui répètent seulement
      l'implémentation.
- [ ] Vérifier en priorité les conséquences graves et les défaillances
      difficiles à détecter. Montrer comment ces scénarios sont contrôlés ou
      préciser la lacune de validation ; cette priorité ne rend jamais un
      invariant facultatif.
- [ ] Étayer les affirmations de performance par des mesures et les budgets
      applicables.
- [ ] Transmettre les hypothèses importantes, les compromis et les limites de
      validation dans la relecture et le rapport final. Terminer avec la
      [liste de fin de tâche](fin-de-tache.md).
