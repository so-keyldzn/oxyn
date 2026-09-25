# ADR-0046 — Un workspace de connexion retenu garde ses sessions ouvertes, dans la limite de huit par fenêtre

**Statut :** accepté · **Date :** 2026-09-25

**Précise :** [ADR-0015](0015-consoles-independantes.md), sur le point
suivant : les sessions d'un workspace ne se ferment plus quand une autre
connexion est ouverte dans la fenêtre, mais seulement sur un geste explicite —
`Disconnect`, fermeture d'une console, sortie.

## Contexte

UX-SPEC (« La fenêtre conserve aussi les workspaces de connexion déjà
ouverts ») et IMPLEMENTATION-PLAN (« revenir à une connexion restaure toutes
ses consoles ») promettent qu'un aller-retour A → B → A rend les consoles de A
telles qu'elles étaient. Le code, relevé le 2026-09-25, ne tient qu'un
workspace : `workspace-host.tsx` le remonte par `key={shown.session}`, et
`release-workspace.ts` appelle `backend.disconnect` sur l'ancienne connexion dès
qu'une autre la remplace (issue #17). L'utilisateur a tranché le 2026-09-25 :
on garde les consoles et les résultats.

Garder l'état de l'interface ne suffit pas. Une console, c'est aussi une
session serveur, et cette session porte deux choses que rien d'autre ne
reconstruit :

- **l'état de transaction** ([ADR-0039](0039-etat-de-transaction-d-une-session.md)) :
  fermer une session dont la transaction est ouverte l'annule, écritures non
  validées comprises. UX-SPEC exige pour cela un dialogue qui nomme la
  connexion ; un changement de connexion n'en montre aucun ;
- **le contexte de session** ([ADR-0019](0019-contexte-de-session.md)) :
  schéma courant, base, tables temporaires, variables. Le rétablir au retour,
  c'est exécuter du SQL que l'utilisateur n'a pas demandé — ce que le critère
  de l'issue exclut (« sans exécuter du SQL »), et ce que la table temporaire
  ne permet de toute façon pas.

Côté mémoire, les résultats sont déjà bornés **globalement**, pas par
workspace : le backend tient au plus 16 résultats affichés et 256 Mio entre
eux (`SHOWN_RESULTS`, `SHOWN_BYTES`, `backend/results.rs`), puis la rétention
d'[ADR-0017](0017-retention-resultats.md) évince au-delà de 16 résultats sans
lecteur, 256 Mio résidents et 1 Gio de débordement. Un résultat évincé se lit
« expired » ; il n'est jamais recréé par rejeu. Le nombre de workspaces ne
multiplie donc pas ce budget. Ce qu'il multiplie : les sessions serveur (au
moins deux par workspace, catalogue et première console), le cache de
catalogue par connexion (1 024 scopes, 50 000 objets, PERFORMANCE), et l'arbre
React monté.

## Décision

**Un workspace retenu reste connecté.** Masqué, il garde ses sessions — celle
du catalogue et celle de chaque console —, leurs transactions et leur contexte.
Y revenir n'émet aucune commande : ni `Connect`, ni `SetContext`, ni exécution.
Choisir sur l'écran d'accueil une connexion qui a déjà un workspace le rend
visible au lieu d'en ouvrir un second.

**Seul un geste explicite ferme des sessions** : `Disconnect` sur le
workspace, la fermeture d'une console (avec son dialogue), la sortie
d'Oxyn. `Disconnect` libère tout ce que la connexion tenait : les brouillons
sont écrits d'abord ([ADR-0024](0024-autosauvegarde-au-repos-de-frappe.md)),
puis `Command::Disconnect` ferme toutes ses sessions, puis ses conversations et
ses agents externes s'arrêtent.

**Au plus huit workspaces par fenêtre** (`MAX_RETAINED_WORKSPACES`). Ouvrir
une neuvième connexion est refusé **avant** tout `Connect`, par un message qui
le dit : « 8 connections are open in this window. Disconnect one before
opening another. » Aucun workspace n'est évincé d'office : l'éviction
fermerait une session dont la transaction est peut-être ouverte, sans le
dialogue qui l'annonce. Huit est une borne de garde, pas une mesure : seize
sessions serveur au minimum, et huit caches de catalogue au plus de leur
plafond.

**Une transaction laissée dans un workspace masqué se dit en toutes
lettres**, ajout de l'utilisateur à l'acceptation, le 2026-09-25. La liste des
connexions de l'écran d'accueil — qui est aussi celle des workspaces retenus,
marqués `Open` — nomme la connexion dont une console a signalé en dernier
`Open`, ou `Unknown` sur une session qui déclare `TRANSACTIONS`
([ADR-0039](0039-etat-de-transaction-d-une-session.md)) : « Transaction open in
a console of *billing* ». Un texte, pas une couleur. Chaque workspace publie
les sessions de ses consoles (`features/workspace/pending-transactions.ts`) ;
l'écran d'accueil les croise avec les derniers états reçus, sans interroger le
serveur.

**Un workspace masqué est inerte** : aucun raccourci, aucun dialogue, aucune
entrée dans le registre d'actions ; le texte envoyé à « la console active »
— par l'assistant, l'inspecteur — va au workspace visible, jamais à un masqué.
Le `PolicyGate` ne change pas : il lit la configuration enregistrée par
connexion, qu'un workspace soit visible ou non ([I-02](../../CLAUDE.md#i-02),
[I-04](../../CLAUDE.md#i-04)).

**Les résultats d'un workspace masqué ne sont pas épinglés** au-delà des
budgets existants : ses grilles restent montées et gardent leur fenêtre de
pages, mais les tampons suivent `SHOWN_RESULTS` et ADR-0017. Au retour, une
page d'un résultat évincé se lit « expired » ; rien n'est réexécuté.

Avec [ADR-0043](0043-multi-fenetre.md), la borne et la décision valent par
fenêtre, et la déconnexion d'une fenêtre devient la fermeture de ses propres
sessions, `Backend` émettant `Disconnect` quand plus aucune ne tient la
connexion.

## Conséquences

* **+** A → B → A rend consoles, textes, résultats, transactions et contexte
  sans un aller-retour vers le serveur.
* **+** Aucune transaction n'est annulée par une simple navigation : la seule
  fermeture de session sans dialogue était celle-là.
* **+** Côté backend, la mémoire des résultats reste bornée par les budgets
  déjà mesurés, quel que soit le nombre de workspaces.
* **−** Côté webview, une grille masquée reste montée et garde ses pages
  observées (au plus `MAX_PAGE_BYTES` chacune, en nombre borné) : le tas JS
  croît avec le nombre de consoles, multiplié par huit workspaces au plus.
  Aucun budget de PERFORMANCE ne le couvre encore ; la campagne de mesure dans
  la webview (IMPLEMENTATION-PLAN) doit le chiffrer.
* **−** Une connexion masquée occupe ses sessions côté serveur : elle compte
  dans `max_connections`, et une transaction ouverte y tient ses verrous
  pendant que l'utilisateur travaille ailleurs. L'écran d'accueil nomme la
  connexion concernée, mais ne dit ni quelle console, ni depuis quand.
* **−** Une session masquée peut être coupée par le serveur (délai
  d'inactivité, redémarrage) ; l'utilisateur ne le découvre qu'à la prochaine
  exécution, comme pour une console visible restée longtemps inactive.
* **−** La neuvième connexion demande de fermer une des huit, geste que
  l'utilisateur n'avait pas à faire quand Oxyn en fermait une à sa place.

**Coût de sortie :** revenir à un seul workspace connecté, c'est rétablir la
déconnexion au changement de connexion dans `workspace-host.tsx` et
`release-workspace.ts` — une journée. Le coût réel est ailleurs : les
utilisateurs auront appris qu'une transaction laissée sur A les attend au
retour. Défaire la décision rend silencieuse une annulation qu'ils ne
prévoient plus.

**Reconsidérer si** des mesures sous instrument montrent qu'un workspace
masqué coûte plus que le budget d'une fenêtre (PERFORMANCE), ou si des
utilisateurs se heurtent aux limites de sessions de leur serveur : la réponse
serait alors une déconnexion des workspaces masqués **sans** transaction
ouverte, annoncée, et non un retour au remplacement.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Garder l'interface et fermer les sessions ; rouvrir au retour | annule sans dialogue toute transaction ouverte ; perd le contexte de session, qu'on ne rétablit qu'en exécutant du SQL non demandé |
| Fermer les sessions des workspaces masqués sans transaction ouverte, garder les autres | deux comportements selon un état que l'utilisateur ne voit pas depuis l'écran d'accueil ; le contexte de session et les tables temporaires se perdent quand même |
| Évincer le workspace le moins récent au-delà de la borne | ferme des sessions sans dialogue, éventuellement sur une transaction ouverte : c'est le défaut qu'on corrige |
| Aucune borne | chaque workspace tient au moins deux sessions serveur et un cache de catalogue ; un geste répété les accumule sans plafond |
