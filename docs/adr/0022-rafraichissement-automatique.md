# ADR-0022 — Ce qui se rafraîchit tout seul, et ce qui ne le fera jamais

**Statut :** proposé · **Date :** 2026-09-10

**Précise :** [ADR-0004](0004-command-bus.md), sur ce que le bus publie en plus
de ce qu'il exécute.

## Contexte

Après un `CREATE TABLE` lancé dans la console, l'explorateur ne bouge pas.
Après un `INSERT`, l'aperçu de la table affichée montre encore l'état d'avant.
Après une exécution, l'historique reste vide tant qu'on n'a pas cliqué. Il faut
cliquer `Refresh` partout, et l'utilisateur qui oublie regarde des données
fausses sans que rien ne le lui dise.

L'audit du code donne trois faits qui commandent la solution.

**Le mécanisme existe déjà et il ne faut pas en écrire un second.**
`Backend::subscribe()` rend un `broadcast::Receiver<ExecEvent>` ; `QueryConsole`
et `Workspace` s'y abonnent déjà, chacun dans une boucle `cx.spawn` qui appelle
son `on_exec_event`. Aucun des deux ne fait d'I/O : ils lisent un tampon déjà en
mémoire et redessinent.

**`Event::CatalogUpdated` existe, et personne ne s'y abonne.** Il n'est publié
que par `Command::RefreshCatalog`, c'est-à-dire quand quelqu'un a déjà demandé
le rafraîchissement. Symétriquement, `CatalogCache::invalidate` existe, sa
documentation dit « à appeler immédiatement après tout DDL émis depuis Oxyn », et
**rien ne l'appelle**. [ARCHITECTURE](../ARCHITECTURE.md) promet ce comportement.
C'est un écart entre le document et le code, pas une fonctionnalité à inventer.

**Aucun événement ne dit ce qui a changé.** `Event::Completed` porte un
`ResultId` et des statistiques. L'instruction est classée — `StatementIntent` —
pour le `PolicyGate`, puis cette classification est jetée. Un abonné ne peut donc
savoir que « une commande de cette connexion vient de finir », jamais « la table
`orders` a changé ».

## Décision

**L'événement porte l'intention, et rien de plus pour l'instant.**
`Event::Completed` gagne `intent: StatementIntent`, que l'exécuteur possède déjà
au moment de publier. C'est assez pour distinguer les trois cas qui comptent —
lecture, écriture, DDL — et cela n'invente aucune structure.

Ce que cela **ne** donne pas : le nom des objets touchés. Un rafraîchissement
est donc, à ce stade, **de la portée d'une connexion**, pas d'une table. Nous
l'assumons plutôt que de faire semblant : extraire les objets référencés d'un
SQL arbitraire est un travail d'analyse à part entière, et le faire à moitié
produirait des invalidations fausses dans les deux sens — c'est-à-dire soit des
vues périmées, soit des relectures inutiles, sans qu'on sache laquelle.

**Ce qui se rafraîchit tout seul, après une exécution qui a réussi :**

| Ce qui a été exécuté | Ce qui se rafraîchit | Pourquoi c'est acceptable |
|---|---|---|
| DDL | le cache de catalogue de **cette** connexion est invalidé, l'arbre se recharge | c'est ce qu'`ARCHITECTURE` promet déjà, et l'introspection est paresseuse : seuls les paliers ouverts sont relus |
| DDL ou écriture | l'aperçu **visible** de cette connexion est relu | une lecture bornée à 200 lignes, sur une table que l'utilisateur regarde en ce moment |
| n'importe quoi | l'historique local, si la bibliothèque est ouverte | une lecture SQLite locale, sans réseau |

**Ce qui ne se rafraîchit jamais tout seul :**

- **rien après une erreur.** [UX-SPEC](../UX-SPEC.md#données-dune-table-sélectionnée)
  le dit déjà : « aucun rafraîchissement automatique ne suit une erreur ». Une
  relecture qui suit un échec masque l'échec ;
- **jamais la requête de l'utilisateur.** Relire un aperçu, c'est réémettre
  `PreviewRelation` — une commande qu'Oxyn compose, bornée et en lecture seule.
  Rejouer l'instruction de la console serait tout autre chose : son coût est
  arbitraire et un `SELECT` peut ne pas être idempotent
  ([PERFORMANCE](../PERFORMANCE.md#budgets-de-mémoire)) ;
- **rien qui n'est pas à l'écran.** Un aperçu dont l'onglet n'est pas affiché
  n'est pas relu : il le sera quand on y reviendra. Le travail invisible est du
  travail que personne n'a demandé ;
- **rien sur une autre connexion.** `ExecEvent` porte sa connexion ; un abonné
  qui n'est pas sur celle-là ignore l'événement.

**Une relecture automatique ne détruit jamais un état en cours.** Si un aperçu
est déjà en train de charger, le rafraîchissement ne l'interrompt pas. Si
l'utilisateur a posé un filtre ou une page, la relecture **conserve la forme
demandée** : le rafraîchissement montre les mêmes lignes à jour, pas un retour
à la première page.

**Le canal est faillible, et le filet est explicite.** Le `broadcast` a une
profondeur de 256 : sous charge, un abonné lent reçoit `RecvError::Lagged` et
perd des événements. Aujourd'hui c'est sans conséquence, parce que les états
terminaux passent par un canal fiable et que le broadcast ne porte que du
confort d'affichage. Dès qu'un rafraîchissement en dépend, un événement perdu
laisserait une vue périmée **pour toujours**. Donc : **sur `Lagged`, l'abonné
rafraîchit comme s'il avait tout manqué.** On ne sait pas ce qui a été perdu ;
la seule réponse honnête est de tout relire.

## Conséquences

- **+** L'utilisateur cesse de regarder des données fausses sans le savoir.
  C'est le défaut que cette décision corrige, et il est silencieux.
- **+** `Refresh` reste, et garde son sens : forcer une relecture même quand
  rien n'a changé côté Oxyn — par exemple quand quelqu'un d'autre a écrit dans
  la base.
- **−** Une écriture qui ne touche pas la table affichée relit quand même son
  aperçu : 200 lignes pour rien. C'est le prix de ne pas connaître la cible, et
  c'est borné.
- **−** Un DDL invalide le catalogue de la connexion entière, donc l'arbre
  relit ses paliers ouverts. Sur une base à dizaines de milliers d'objets, c'est
  ce que le chargement paresseux limite déjà, pas ce que cette décision aggrave.
- **−** Une exécution en boucle — un script qui écrit cent fois — provoquerait
  cent relectures. L'abonné **coalesce** : une relecture en cours absorbe les
  demandes qui arrivent pendant qu'elle tourne.
- **−** Un `Lagged` déclenche une relecture complète. C'est plus coûteux qu'un
  rafraîchissement ciblé, et c'est voulu : la seule alternative est une vue
  fausse.

**Coût de sortie :** un champ d'événement, une invalidation dans l'exécuteur, et
des abonnements dans trois vues. Rien n'est persisté, aucun format ne change.

**Reconsidérer si** l'analyse SQL apprend à nommer les objets qu'une instruction
touche — alors le rafraîchissement deviendrait ciblé, et les deux conséquences
négatives ci-dessus disparaîtraient. C'est la suite naturelle, et elle
n'invalide rien de ce qui est décidé ici.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Un second canal de notification, dédié à l'invalidation | Le bus existe et porte déjà la connexion et la commande ; un second canal serait un second endroit où oublier de publier |
| Interroger périodiquement le serveur pour détecter les changements | Un sondage coûte sur toutes les connexions tout le temps, y compris quand rien ne se passe, et il ne verrait toujours pas ce qui vient d'être écrit ailleurs |
| Rejouer l'instruction de la console pour rafraîchir | Coût arbitraire, et un `SELECT` peut ne pas être idempotent |
| Rafraîchir aussi ce qui n'est pas visible | Du travail que personne n'a demandé, sur des vues que personne ne regarde |
| Ignorer `Lagged`, comme aujourd'hui | Tenable tant que le broadcast ne porte que du confort ; faux dès qu'une vue en dépend pour être juste |
| Attendre de savoir quelle table a changé avant de livrer quoi que ce soit | Laisse le défaut en place pour une amélioration future ; la portée « connexion » est déjà correcte, seulement plus large que nécessaire |
