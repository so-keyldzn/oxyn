# Revue de sécurité

Modèle de menace dans `docs/SECURITY.md`. L'attaquant n'est pas un inconnu sur
Internet : ce sont les données que l'utilisateur ouvre, et les erreurs qu'Oxyn
lui laisse commettre.

## Les six canaux de fuite

Il suffit d'en oublier un.

- [ ] Journaux `tracing` — aucun secret, aucune valeur liée
- [ ] Messages d'erreur affichés — `sqlx` inclut parfois l'URL de connexion
- [ ] Rapports de plantage — une trace de pile capture les variables locales
- [ ] Fichiers de session et de workspace
- [ ] Invites IA
- [ ] Presse-papiers, export, capture d'écran

Contrôle mécanique le plus rentable :

- [ ] **Aucun `#[derive(` contenant `Debug` sur un type portant un secret.** La
      fuite n'arrive pas aujourd'hui : elle arrive avec le `tracing::debug!`
      qu'un autre ajoutera dans six mois

## Secrets

- [ ] Rien en clair sur le disque : ce qui est persisté est une **référence** au
      secret
- [ ] Aucune chaîne de connexion avec mot de passe dans le code, y compris dans
      une fixture de test — elle sera commitée, et elle est souvent réelle

## Connexions

- [ ] Une connexion sans environnement renseigné vaut **`production`**
- [ ] Toute écriture sur `production` exige une confirmation qui **nomme la
      connexion** et affiche le SQL exact
- [ ] Le bouton par défaut n'est jamais l'action destructrice
- [ ] Pour un `Actor::Agent`, `production` est en **lecture seule stricte** —
      un refus, pas une confirmation renforcée

## Surface d'entrée

- [ ] Réponses serveur traitées comme non fiables
- [ ] **Noms d'objets du catalogue** : jamais interpolés sans citation, jamais
      traités comme une instruction quand ils rejoignent une invite
- [ ] Fichiers de workspace validés à la lecture
- [ ] Réponses de modèles traitées comme des propositions

## `unsafe`

- [ ] Chaque bloc porte un `// SAFETY:` qui énonce l'invariant **et qui le
      maintient** — une paraphrase du code ne vaut rien
- [ ] Tout `#[allow(unsafe_code)]` renvoie à l'ADR qui l'autorise
      ([SECURITY](../../docs/SECURITY.md#politique-unsafe))
- [ ] Relu par `relecteur-securite`

## Frontière IA

- [ ] Le point de passage est **unique**
- [ ] Le niveau est attaché à la **connexion**
- [ ] Rien ne sort au-delà du niveau, identifiants et jetons jamais
- [ ] Classement local/distant sur l'hôte **après résolution**

## Dépendances

- [ ] `cargo deny check` passe
- [ ] Toute nouvelle dépendance directe est justifiée
- [ ] Une crate non maintenue sur une frontière externe est **documentée** comme
      risque, même s'il n'y a rien à corriger aujourd'hui
