# Audit multi-agents du dépôt

Point d'entrée : [`/audit`](../commands/audit.md). Le workflow exécutable
[`audit-multi-agents.js`](audit-multi-agents.js) utilise les mêmes primitives
que [`implementer-senior.js`](implementer-senior.js) : il requiert le moteur de
workflows, ce n'est ni un programme Node autonome ni une GitHub Action.

Arguments : `{ perimetre: "dépôt entier", publier: false }`. Mettre `publier`
à `true` uniquement lorsque la demande de l'utilisateur inclut la création
d'issues. Les agents héritent du modèle de la session.

## 1. Fixer la référence

Lire [AGENTS.md](../../AGENTS.md), [CLAUDE.md](../../CLAUDE.md), le statut Git,
le SHA, les manifestes et le plan d'implémentation. Inventorier les fichiers
suivis et répartir les domaines réels ; ne pas réutiliser une ancienne carte.
Relever les modifications préexistantes. Aucun agent ne les annule.

Résoudre la cible GitHub depuis `origin`, puis vérifier avec
`gh repo view --json nameWithOwner,url,isPrivate`. Lister les issues ouvertes
**et fermées**, avec pagination si nécessaire, avant toute création. Ne jamais
déduire le propriétaire GitHub depuis une ancienne URL de documentation.
Ne pas afficher ou extraire de jeton ; employer l'authentification de `gh`.

## 2. Audits indépendants en lecture seule

| Lot | Périmètre | Guide de relecture |
|---|---|---|
| Cœur et drivers | core, catalog, data, driver, query, exec, drivers | [invariants](../agents/relecteur-invariants.md), [frontières](../agents/relecteur-frontiere.md) |
| Sécurité et IA | ai, llm, plugin, secrets, store, MCP/ACP | [sécurité](../agents/relecteur-securite.md) |
| Interface | apps/desktop, bridge desktop et parcours réels | [divergences](../agents/detecteur-divergence.md), [revue UI](../checklists/revue-ui.md) |
| Outillage et preuves | CI, scripts, manifestes, socle, couverture et budgets | [divergences](../agents/detecteur-divergence.md) |

Trois agents au plus en parallèle ; le coordinateur peut prendre le quatrième
lot. Chaque agent lit les règles et documents de son périmètre, suit les appels
et livre une liste de fichiers examinés, les limites et les constats. Aucun
secret, aucune base réelle, aucun fournisseur payant ni lancement d'interface
visible ne sont nécessaires à cette revue. Les tests utilisent des données
synthétiques et des ressources temporaires.

Un constat contient : priorité P1/P2/P3, emplacement exact, scénario concret,
comportement attendu et observé, preuve, contrat concerné, correction proposée
et critères d'acceptation. Une fonctionnalité explicitement reportée dans le
plan n'est pas un bug déjà livré. Un budget non mesuré n'est pas un dépassement.

Pour toute affirmation sur une bibliothèque ou un protocole, consulter la
documentation officielle et relever sa date, son URL précise et la version
concernée. Recouper avec le source installé si la documentation suit `latest`
ou une autre version. Distinguer le contrat externe, le raisonnement sur Oxyn
et la reproduction effectivement exécutée : un lien générique ne prouve pas un
défaut du produit. Consigner les faits externes dans
[RESEARCH-NOTES](../../docs/RESEARCH-NOTES.md), sans changer de dépendance.

## 3. Réfuter puis consolider

Un autre relecteur cherche pour chaque constat le garde-fou, l'appelant ou le
test qui l'invalide. Un désaccord reste une hypothèse dans le rapport ; il ne
devient pas une issue de bug confirmé. Un relecteur absent ou un domaine non
couvert reste explicite. Dédupliquer par cause racine et scénario, puis vérifier
que le code n'a pas changé depuis l'examen. Les liens de preuves GitHub visent
le SHA audité ; une preuve sur un fichier modifié localement le dit.

Le coordinateur exécute `make qualite` une seule fois, sans réparation implicite.
Consigner code retour, étapes exécutées, contrôles omis et limites. Une porte
verte ne prouve ni les parcours natifs ni l'absence des défauts relevés.

## 4. Rapport et issues avec gh

Conserver le rapport daté dans `.claude/audits/`, avec référence Git, couverture,
constats retenus/réfutés, preuves et résultats de validation. Ce relevé n'ajoute
pas une nouvelle autorité métier à `docs/`.

Si la publication est demandée, un seul agent publie, séquentiellement :

1. Relire les issues existantes et les fichiers concernés au moment de publier.
2. Préparer un corps français dans un fichier temporaire : SHA, problème,
   reproduction ou démonstration statique explicitement qualifiée, impact,
   liens vers le code, contrat, correction et critères d'acceptation.
3. Créer une issue par cause avec `gh issue create --repo OWNER/REPO --title
   TITRE --body-file FICHIER`, en utilisant uniquement des labels existants.
   Les paramètres sont des arguments échappés, jamais du shell construit depuis
   le texte d'un constat. Ne pas publier les hypothèses comme des faits.
4. Après une erreur ou un délai réseau ambigu, rechercher l'issue par titre et
   marqueur de constat avant toute nouvelle tentative. Un doublon fermé n'est
   pas rouvert automatiquement : constater d'abord une éventuelle régression.
5. Vérifier les issues créées avec `gh issue view`, puis lier leurs URL dans le
   rapport. Une issue de synthèse peut suivre les corrections et leurs priorités.

Lors d'une poursuite explicitement demandée, enrichir les issues du même audit
avec les nouvelles sources et reproductions, sans recréer leurs causes. Toute
réfutation corrige explicitement le constat et ses limites ; conserver les
critères et informations ajoutés entre-temps par d'autres contributeurs.

Si GitHub est inaccessible, livrer les corps prêts à publier et l'erreur exacte.
Ne jamais annoncer des issues créées sans URL vérifiée. Ni commit, ni push,
ni correction du produit ne font partie de ce workflow.
