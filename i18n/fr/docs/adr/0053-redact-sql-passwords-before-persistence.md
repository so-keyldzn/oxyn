<!-- oxyn-translation source="docs/adr/0053-redact-sql-passwords-before-persistence.md" sha256="3e487fd71024" -->

# ADR-0053 — Caviarder les littéraux de mot de passe SQL avant la persistance

**Statut :** accepté · **Date :** 2026-10-03

## Contexte

Les instructions d'administration de base de données peuvent porter un mot de
passe sous forme de littéral chaîne SQL. Les commandes PostgreSQL
`CREATE`/`ALTER ROLE` et `USER`, ainsi que MySQL `CREATE`/`ALTER USER` et
`SET PASSWORD`, placent donc un identifiant directement dans le texte de
l'instruction plutôt que dans une valeur liée.

Oxyn persiste le texte des instructions dans `query_history`,
`audit_journal`, qui est en ajout seul, et les enregistrements d'appels d'outil
des conversations. Conserver ce texte mot pour mot placerait un mot de passe
dans le fichier du workspace, contrairement à I-03 et à
[SECURITY](../SECURITY.md#ce-qui-ne-touche-jamais-le-disque-en-clair). Réécrire
les lignes existantes d'`audit_journal` contredirait le contrat d'ajout seul du
journal et détruirait après coup les octets historiques exacts.

## Décision

`oxyn-query::redact_password_literals` utilise le scanner lexical existant et
sensible au dialecte pour remplacer par `'<redacted>'` le littéral chaîne des
clauses `PASSWORD`, `IDENTIFIED BY` et `IDENTIFIED WITH … BY`. Tous les autres
octets sont préservés, y compris la mise en forme et les commentaires.

Le dialecte MySQL couvre aussi les modificateurs d'utilisateur MariaDB,
l'authentification dans `GRANT` et les chaînes d'authentification par plugin.
Les commentaires exécutables MySQL sont scannés comme du SQL ; les commentaires
ordinaires restent inchangés.

Les références grammaticales ont été vérifiées le 2026-10-03 et sont consignées
dans [RESEARCH-NOTES](../RESEARCH-NOTES.md#sql-password-literals-before-persistence).
Les noms de compte entre guillemets dans `SET PASSWORD FOR` sont préservés ; le
littéral après l'affectation est caviardé. Les fragments de littéraux adjacents
et l'ancien mot de passe dans `REPLACE` le sont également. Lorsqu'un
enregistrement ne porte ni dialecte ni mode SQL, le scanner combine prudemment
les interprétations lexicales prises en charge ; des guillemets ambigus peuvent
donc entraîner un caviardage plus large que strictement nécessaire.

La `Command` et l'`ExecRequest` d'origine restent inchangées et sont envoyées au
driver exactement comme saisies. `HistoryRecord`, `JournalRecord` et les
valeurs `ToolCallRecord` sérialisées ne reçoivent que la copie caviardée à leurs
frontières de persistance. L'historique stocké indique au lecteur que les
littéraux de mot de passe sont caviardés.

Les mentions de requêtes enregistrées passent par la même fonction à la porte
du contexte IA, avant leur troncature. Cette modification couvre les clauses
de mot de passe reconnues, pas les secrets arbitraires dans les commentaires,
le SQL dynamique, les brouillons de l'éditeur ou le texte libre des conversations.

La règle ne s'applique qu'aux nouvelles écritures. Les lignes existantes
d'`audit_journal` ne sont ni réécrites, ni supprimées, ni migrées : l'ajout seul
signifie que même une correction de sécurité ne crée pas de voie privilégiée
capable de modifier la piste d'audit. Les utilisateurs susceptibles d'avoir
exécuté du SQL contenant un identifiant avant cette décision doivent renouveler
ces identifiants et protéger ou remplacer le fichier de workspace concerné.

## Conséquences

* **+** Les nouvelles lignes d'historique, de journal et d'appels d'outil ne conservent pas les littéraux de mot de passe SQL reconnus.
* **+** La sémantique d'exécution ne change pas, car le driver reçoit la requête d'origine.
* **+** Le journal d'audit reste strictement en ajout seul, sans exception de maintenance qui pourrait ensuite effacer des preuves.
* **−** Une instruction stockée n'est plus identique octet pour octet à l'instruction contenant l'identifiant qui a été exécutée.
* **−** Les identifiants précédemment persistés restent dans les anciens fichiers de workspace et sauvegardes ; la remédiation exige le renouvellement des identifiants et une gestion des fichiers hors d'Oxyn.
* **−** Une nouvelle grammaire de mot de passe propre à une base de données exige un test lexical explicite avant d'être couverte.

**Coût de sortie :** supprimer la règle exige de modifier les trois frontières
de persistance, l'indication de l'historique et le contrat de sécurité.
Récupérer les littéraux déjà caviardés est impossible par conception.

**À reconsidérer si** un futur format de workspace peut séparer
cryptographiquement les preuves d'audit contenant des secrets tout en restant
lisible ouvertement selon I-11, ou si un dialecte de base de données introduit
une syntaxe d'identifiant que la règle lexicale ne peut pas reconnaître sans
ambiguïté.

## Alternatives rejetées

| Alternative | Motif du rejet |
|---|---|
| Réécrire les lignes existantes du journal | Cela viole le contrat d'ajout seul de l'audit et crée une voie de modification dont l'abus serait impossible à distinguer de la remédiation. |
| Caviarder avec des expressions régulières | Les commentaires SQL, identifiants entre guillemets, échappements et lots feraient à la fois caviarder des littéraux ordinaires et manquer des syntaxes d'identifiant valides. |
| Caviarder la commande avant l'exécution | Cela modifierait le mot de passe envoyé au serveur et casserait l'opération de l'utilisateur. |
| Ne plus persister aucun texte d'instruction | L'historique et l'audit ne répondraient plus à ce qui a été exécuté, et le SQL sans rapport n'est pas secret par défaut. |
