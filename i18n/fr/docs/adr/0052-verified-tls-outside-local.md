<!-- oxyn-translation source="docs/adr/0052-verified-tls-outside-local.md" sha256="3e3d3a4ca4df" -->

> Traduction française de [docs/adr/0052-verified-tls-outside-local.md](../../../../docs/adr/0052-verified-tls-outside-local.md). **La version anglaise fait foi.**

# ADR-0052 — PostgreSQL et MySQL exigent TLS vérifié hors de Local

**Statut :** accepté · **Date :** 2026-10-03

**Précise :** [ADR-0050](0050-mysql-driver-on-mysql-async-prepared-first.md),
décision 6 : sa politique TLS devient la règle commune à PostgreSQL et MySQL.
Le reste de l'ADR-0050 reste en vigueur.

## Contexte

L'issue [#149](https://github.com/so-keyldzn/oxyn/issues/149) relève deux
politiques pour un même environnement de connexion : MySQL utilise
`verify-full` par défaut et refuse les modes plus faibles hors de Local,
tandis que PostgreSQL utilise `prefer` et accepte tous les modes en production.
Chiffrer sans vérifier l'identité du serveur ne suffit pas face à un
intermédiaire actif ; la distinction entre les modes PostgreSQL est sourcée
dans [RESEARCH-NOTES](../RESEARCH-NOTES.md#modes-tls-postgresql--vérifiés-le-2026-10-03).

## Décision

**Une seule règle TLS s'applique à `oxyn-driver-postgres` et `oxyn-driver-mysql` :**

- Le défaut est `verify-full`. La chaîne de certificats et le nom d'hôte du
  serveur doivent être vérifiés.
- Seul `Environment::Local` autorise un mode explicitement plus faible.
  PostgreSQL refuse ailleurs `disable`, `allow`, `prefer`, `require` et
  `verify-ca` ; MySQL y refuse aussi chaque mode pris en charge plus faible
  que `verify-full`.
- Un environnement non renseigné vaut production, comme le précise déjà
  [SECURITY](../SECURITY.md#marquage-des-connexions). Ni un nom d'hôte de boucle
  locale, ni un marquage développement ou staging n'accordent l'exception.
- `ConnectSpec::from_config` refuse un mode plus faible avant toute ouverture
  réseau, avec `OxynError::Config` et le message MySQL existant qui nomme
  `Local` et `verify-full`. Les configurations enregistrées et les DSN collés
  passent par cette même validation ; aucune migration silencieuse ni
  nouvelle tentative avec une sécurité affaiblie n'a lieu.
- Les métadonnées du formulaire du driver proposent `verify-full` par défaut.
  Un serveur de test local jetable sans TLS doit être marqué Local et choisir
  explicitement un mode plus faible.

Cet ADR est la source de la politique commune ; SECURITY y renvoie. La
décision historique propre à MySQL est conservée, sans être réécrite.

## Conséquences

* **+** Choisir un autre protocole de base de données ne change plus la
  protection des identifiants et des requêtes pour un même environnement.
* **+** Un champ omis conserve la protection ; l'utilisateur doit choisir
  explicitement l'exception Local.
* **−** Les configurations PostgreSQL existantes hors de Local utilisant un
  mode plus faible ne se connectent plus tant qu'elles n'utilisent pas
  `verify-full` avec un certificat vérifiable et un nom d'hôte correspondant.
  Seule une base réellement locale et jetable doit être marquée Local pour
  conserver un mode plus faible.
* **−** Les URL de test locales qui dépendaient de l'ancien défaut doivent
  nommer leur mode TLS. Le seul environnement Local ne désactive pas la
  vérification.

**Coût de sortie :** faible dans le code (deux constructeurs d'options et les
défauts de leurs formulaires), mais important dans le contrat de sécurité :
assouplir la règle changerait la protection attendue pour les connexions déjà
enregistrées. Cela exige un nouvel ADR et une couverture de non-régression
dans les deux drivers.

**Réexaminer si** Oxyn introduit un autre mécanisme explicite de vérification
de l'identité du serveur, avec une protection équivalente et une migration
testée. Des échecs de connexion à un serveur non vérifié ne justifient pas à
eux seuls d'affaiblir le défaut.

## Alternatives rejetées

| Alternative | Motif du rejet |
|---|---|
| Garder le défaut PostgreSQL `prefer` | fait dépendre la sécurité de l'environnement du protocole choisi |
| Accepter `require` ou `verify-ca` hors de Local | ne satisfait pas la vérification commune du certificat et du nom d'hôte |
| Déduire Local de `localhost` ou autoriser des exceptions développement/staging | le nom d'hôte et les marquages hors production n'établissent pas la limite explicite d'une base locale jetable |
| Réécrire automatiquement les modes enregistrés ou se rabattre après une erreur TLS | masque une décision de sécurité et modifie la configuration sans consentement |
