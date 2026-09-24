# ADR-0037 — Une décision critique se confirme dans un dialogue natif de l'hôte, jamais dans la webview

**Statut :** proposé · **Date :** 2026-09-25

**Précise :** [ADR-0029](0029-interface-tauri-shadcn.md), dont les Conséquences
nomment la surface nouvelle — « une XSS dans la webview atteint les commandes
Tauri » — sans dire ce qui reste hors de sa portée. Cet ADR le dit pour trois
décisions.

**Complète :** [ADR-0034](0034-echantillon-pour-toute-destination.md), dont
le § 4 fait d'`ai_answer_sample` « un geste de l'utilisateur dans l'écran » :
c'est vrai de ce que l'agent peut envoyer, pas de ce qu'un script dans la
webview peut appeler.

## Contexte

Une confirmation dessinée dans la webview protège contre un clic malheureux,
pas contre un script. Tout ce que la webview peut cliquer, un script injecté
dans la webview peut l'appeler directement, sans rien dessiner. Le 2026-09-24,
trois décisions reposaient pourtant sur un bouton de la webview :

| Décision | Ce qui l'accorde aujourd'hui |
|---|---|
| Écriture ou DDL sur une connexion `production` ([I-02](../../CLAUDE.md#i-02)) | `decide(command, approved: true)` — `crates/oxyn-desktop/src/commands.rs`, puis `Backend::decide`, qui appelle `executor.approve("human", …)` sans autre vérification |
| Modification ou suppression d'une connexion `production` | `decide_connection_change(command, approved: true)` — `commands/settings.rs` |
| Sortie de lignes vers un agent | `ai_answer_sample(connection, request, columns)` pour une demande de l'agent ; le champ `sample` d'`AskRequest` pour un échantillon épinglé |
| Changement d'environnement ou de niveau de confidentialité | `update_connection`, puis `decide_connection_change` quand la connexion est ou devient `production` — le `PolicyGate` retient le plus contraignant des deux marquages (`crates/oxyn-core/src/policy.rs`) ; **aucune** confirmation sinon : un changement de niveau sur une connexion `development` passe sans approbation |

Un script qui obtient l'identifiant d'une commande en attente — il transite par
la webview, c'est là qu'on l'affiche — l'approuve en un appel. Un script qui
lit le catalogue approuve un échantillon qu'aucun agent n'a demandé à
l'utilisateur. Et le dernier cas n'a même pas besoin d'identifiant : passer une
connexion de `production` à `development`, puis écrire, supprime la
confirmation d'[I-02](../../CLAUDE.md#i-02) en trois appels, dont le premier
rend lui-même l'identifiant à approuver ; passer `Metadata`
à `Sampled` rouvre la sortie de lignes que l'utilisateur avait fermée.

Le dépôt a déjà la réponse, pour un cas : `ai_save_external_agent`
(`commands/ai.rs`) ne déclare un agent qu'après un dialogue du plugin
`tauri-plugin-dialog`, dessiné par l'hôte. Le commentaire en dit la raison :
« a script injected in the webview can call this command, it cannot click a
window it does not draw ». Cet ADR étend ce modèle, et rien que lui, aux
décisions dont l'effet est irréversible ou fait sortir des données.

**Fait externe, vérifié dans la source de `tauri-plugin-dialog` 2.7.3 (version
de `Cargo.lock`), `src/lib.rs`, `MessageDialogBuilder::show`, le 2026-09-25
([I-12](../../CLAUDE.md#i-12)).** Le rappel reçoit `true` pour `Ok`, `Yes`, ou
un bouton personnalisé **dont le libellé égale** celui du bouton de
confirmation ; toute autre issue — Annuler, fermeture — donne `false`. Deux
libellés identiques feraient donc d'Annuler une confirmation.

## Décision

### 1. Ce qui est critique, et rien d'autre

Trois familles de décisions passent par un dialogue natif :

1. **une écriture ou un DDL sur une connexion `production`** — toute
   approbation, par `decide`, `decide_connection` ou
   `decide_connection_change`, d'une commande
   mutante dont la connexion est `production` **au moment de l'approbation**,
   quel que soit le motif pour lequel le `PolicyGate` l'a retenue — un `DROP`
   ou un `DELETE` sans `WHERE` est retenu pour « unbounded mutation » avant de
   l'être pour la production, et c'est le pire cas. L'environnement est celui
   que le gate retient, le plus contraignant de l'annoncé et de l'enregistré,
   lu par **son** calcul et non recopié : une commande retenue sur
   `development` dont la connexion est passée en `production` depuis relève
   du dialogue. La création d'une connexion `production`, que le gate retient
   comme DDL et que `decide_connection` approuve, en relève aussi. Pour un
   `Actor::Agent`, rien ne change : c'est un refus, et aucun dialogue ne
   s'ouvre ([I-02](../../CLAUDE.md#i-02)) ;
2. **toute sortie de lignes approuvée vers un agent** — l'échantillon épinglé,
   au moment où la question qui le porte part, et l'échantillon demandé par
   l'outil `request_sample`, au moment où l'utilisateur a coché ses colonnes
   ([ADR-0034](0034-echantillon-pour-toute-destination.md)) ;
3. **tout changement d'environnement ou de niveau de confidentialité d'une
   connexion**, dans les deux sens et quel que soit l'environnement de départ.
   Le critère est le changement, pas sa direction : décider qu'un sens est sûr,
   c'est écrire une seconde règle de niveau que personne ne relira. Le
   `PolicyGate` ne retient un tel changement que si l'un des deux marquages
   est `production` : un changement de niveau sur `development` passe sans
   approbation. Le `Backend` compare donc l'édition à la configuration
   enregistrée et ouvre le dialogue **avant** d'envoyer la commande, qu'elle
   soit ensuite retenue ou non. Si elle l'est — la connexion est ou devient
   `production` —, son approbation relève en plus de la famille 1 et ouvre son
   propre dialogue : deux dialogues pour ce cas rare, plutôt qu'une
   approbation accordée sans passer par `decide_connection_change`.

La déclaration d'un agent externe a déjà son dialogue natif, décidé par
[ADR-0026](0026-agents-externes-acp.md) : elle reste hors de ces trois
familles, et ne fait que rejoindre le port du § 4.

Le reste garde la confirmation dans la webview : écriture hors `production`,
approbation d'une commande d'agent hors `production`, renommage, suppression
d'une connexion hors `production`. **Refuser** n'est jamais critique : un
`approved: false` ne passe par aucun dialogue.

### 2. Le dialogue nomme la connexion depuis le backend

Le texte du dialogue est composé en Rust à partir de ce que le **backend**
tient — la configuration enregistrée, la commande retenue par l'exécuteur, la
demande d'échantillon en attente —, jamais à partir d'un argument venu de la
webview. Un script peut choisir quoi faire approuver ; il ne peut pas choisir
ce que le dialogue en dit.

Chaque dialogue porte :

* **la connexion** : son nom tel qu'enregistré, son environnement en toutes
  lettres (`PRODUCTION`), et l'adresse non secrète que l'écran de connexion
  affiche déjà (hôte, port, base, ou chemin de fichier) — deux connexions du
  même nom se distinguent ainsi. Aucun champ marqué secret n'y figure
  ([I-03](../../CLAUDE.md#i-03)) ;
* **l'objet** :
  * pour une écriture, d'abord ce que le backend a classé — l'intention et le
    motif du `PolicyGate` —, puis le texte de l'instruction tel que
    l'exécuteur le tient : en entier jusqu'à **1 000 caractères**, au-delà son
    début et sa fin (500 caractères chacun) séparés par « … N more characters
    … ». La coupe se dit, elle ne se devine pas ; et montrer la fin empêche
    qu'un préambule anodin de 1 000 caractères cache seul ce qui suit ;
  * pour un échantillon, la relation, les colonnes cochées une à une, le
    nombre de lignes au plus, et le destinataire tel que la configuration
    enregistrée le décrit — hôte du point d'accès pour un fournisseur, commande
    pour un agent externe, et si les lignes quittent la machine. Pas son seul
    libellé : un script peut déclarer un fournisseur sous le libellé d'un
    autre ;
  * pour une édition de connexion, **chaque** champ modifié, ancienne et
    nouvelle valeur, et si le secret est conservé ou ressaisi : l'édition se
    confirme entière, elle se montre entière ;
  * pour une suppression, la connexion seule — c'est tout l'objet ;
* **aucune chaîne telle quelle** : tout ce qui vient d'une entrée non fiable —
  nom de connexion (fichier de workspace), instruction, noms de relation et de
  colonnes (catalogue), libellé d'un destinataire
  ([SECURITY](../SECURITY.md#surface-dentrée), points 2 et 3) — y passe par
  la même fonction d'échappement : caractères de contrôle et marques de
  direction bidirectionnelle rendus visibles, longueur bornée ;
* **deux boutons de libellés distincts**, dont celui de confirmation dit
  l'effet (« Write to production », « Send rows », « Change marking ») et
  l'autre « Cancel ». Les libellés sont des constantes, pas des paramètres :
  deux libellés égaux feraient d'Annuler une confirmation (Contexte).

### 3. Fermer, c'est refuser, et un refus épuise la décision

Tout ce qui n'est pas le bouton de confirmation vaut refus : Annuler, Échap, la
fermeture de la fenêtre du dialogue, l'arrêt de l'application, un dialogue qui
n'a pas pu s'ouvrir, un canal de réponse abandonné.

Un refus **consomme** ce qu'il refuse, comme un refus dans l'écran :

* la commande retenue est rejetée (`executor.reject`) et ne reste pas en
  attente — sinon un script rappellerait `decide` jusqu'au clic réflexe ;
* la demande d'échantillon est déclinée, et l'échange est épuisé comme
  l'impose [ADR-0034 § 3](0034-echantillon-pour-toute-destination.md) ; une
  réponse du dialogue arrivée après l'expiration de cinq minutes ne lit rien ;
* la question qui portait un échantillon épinglé ne part pas ;
* le changement de marquage n'est pas enregistré, et rien d'autre de la même
  édition ne l'est — une édition est confirmée entière ou pas du tout.

**Une confirmation trop rapide est un refus.** Sous macOS, le bouton de
confirmation est le premier ajouté à l'`NSAlert`, donc celui qu'Entrée
déclenche, et le plugin ne permet pas d'en désigner un autre (source de `rfd`
0.16.0, `src/backend/macos/message_dialog.rs`, vérifiée le 2026-09-25). Un
script qui ouvre le dialogue pendant que l'utilisateur tape — Cmd+Entrée pour
exécuter — obtiendrait la frappe suivante. Une confirmation reçue moins d'**une
seconde** après l'ouverture vaut donc refus, et consomme la décision comme
tout refus.

**Une échéance pour tout dialogue : cinq minutes**, la borne qu'[ADR-0034](0034-echantillon-pour-toute-destination.md)
fixe déjà à la demande d'échantillon. Le plugin ne peut pas fermer un dialogue
par programme ; passé ce délai, la décision est consommée comme refusée, le
dialogue suivant peut s'ouvrir, et la réponse tardive du dialogue resté à
l'écran est ignorée — une écriture approuvée le lendemain serait une écriture
que personne n'a vue partir.

**Un seul dialogue critique à la fois.** Une décision critique qui arrive
pendant qu'un dialogue est ouvert est refusée sur-le-champ, avec un message
qui le dit ; elle n'attend pas son tour. Une file permettrait à un script
d'empiler des dialogues que l'utilisateur fermerait par réflexe, le dernier
étant le bon. Ce refus-là **ne consomme pas** la décision : sinon un script
ferait échouer la décision légitime de l'utilisateur en ouvrant un dialogue
juste avant lui.

**La webview n'ouvre jamais de dialogue de message.** La garantie repose sur
ce que seul le backend compose un dialogue d'apparence native :
`capabilities/main.json` n'accorde ni `dialog:allow-message`, ni
`dialog:allow-ask`, ni `dialog:allow-confirm`, et ne les accordera pas. Un
script qui les obtiendrait dessinerait des dialogues de même apparence, au
texte de son choix.

### 4. Le dialogue passe par un port, et se teste sans fenêtre

`oxyn-desktop` porte un trait `HostConfirm` — une seule méthode asynchrone qui
prend une `Confirmation` (titre, corps, libellé de confirmation, sévérité) et
rend un booléen. Deux implémentations, c'est une frontière avec l'hôte et non
une indirection ([CLAUDE.md](../../CLAUDE.md#organisation-du-code)) :

* **`NativeDialog`**, sur le plugin `tauri-plugin-dialog`, comme
  `ai_save_external_agent` aujourd'hui — qui rejoint ce port ;
* **une réponse scriptée**, dans les tests : confirmer, refuser, ou ne jamais
  répondre.

Le `Backend` reçoit le port à sa construction, sans valeur par défaut : un
`Backend` qui n'en a pas ne se construit pas, donc aucune décision critique ne
passe faute de dialogue. Le `Backend` s'ouvre avant `tauri::Builder`, pour
qu'un échec de démarrage arrive sur stderr (`main.rs`) ; `NativeDialog` se
construit donc sans `AppHandle`, le reçoit au `setup`, et **répond `false`**
tant qu'il ne l'a pas — fermer, c'est refuser, et n'avoir pas pu ouvrir aussi. C'est le `Backend`, et non la commande Tauri, qui
décide qu'une décision est critique et qui appelle le port : la règle vit à
côté de la configuration qu'elle lit.

Le port ne s'attend que depuis un contexte asynchrone : le plugin dessine le
dialogue par `run_on_main_thread`, et une commande Tauri synchrone tourne sur
ce thread ([I-05](../../CLAUDE.md#i-05)). `ai_answer_sample` reste synchrone —
elle remet les colonnes à l'appel d'agent qui attend — et c'est **cet appel**,
sur sa tâche, qui ouvre le dialogue avant de lire.

Ce qui se teste, sans fenêtre ni `MockRuntime` :

* **le texte** — les fonctions qui composent une `Confirmation` sont pures :
  le nom et l'environnement y sont, une chaîne hostile y est échappée où
  qu'elle apparaisse, une instruction longue y est coupée **et le dit**, sa
  fin y figure, chaque champ modifié d'une édition y figure, aucun champ
  secret n'y figure ;
* **la conduite** — sur un `Backend` monté avec la réponse scriptée et une
  horloge contrôlée : un `DELETE` sans `WHERE` sur `production` ouvre le
  dialogue ; `decide(…, true)` sur `production` refusé, sans réponse
  après l'échéance, ou confirmé en moins d'une seconde n'exécute rien et ne
  laisse aucune commande en attente ; confirmé, exécute ; une commande retenue
  sur `development` dont la connexion est passée en `production` ouvre le
  dialogue ; `production → development` refusé laisse la configuration
  inchangée et n'envoie aucune commande ; un échantillon refusé ne lit aucune
  ligne ; une seconde décision critique pendant la première est refusée sans
  être consommée ; une décision non critique n'appelle pas le port ;
* **les libellés** — un test vérifie que le libellé de confirmation de chaque
  `Confirmation` diffère de « Cancel ».

Seul `NativeDialog` échappe aux tests automatiques : il se vérifie à la main
dans `make desktop-dev`, sur les trois plateformes, une fois par changement de
version du plugin.

### 5. Ce que l'écran de la webview devient

L'écran reste là où l'on **lit et choisit** : l'instruction complète avec sa
coloration, les colonnes à cocher, le formulaire de connexion. Pour une
décision critique, son bouton d'accord ne l'accorde plus : il demande au
backend, qui ouvre le dialogue. La webview apprend l'issue par la réponse de la
commande, comme aujourd'hui.

## Conséquences

* **+** Ce qu'une XSS peut faire sur ces trois décisions se borne à **ouvrir
  un dialogue** que l'utilisateur voit, qui dit la vérité parce que le backend
  l'écrit, et que fermer suffit à refuser.
* **+** Un seul modèle de confirmation forte, déjà en service pour la
  déclaration d'un agent externe, et désormais testable par le même port.
* **+** Le changement de marquage cesse d'être la porte dérobée d'I-02 : il
  n'est plus possible de déclasser une connexion puis d'y écrire sans que
  l'utilisateur ait confirmé le déclassement dans un dialogue qui nomme la
  connexion et son ancien environnement.
* **−** **Deux confirmations** pour une écriture en production : l'écran de la
  webview, puis le dialogue. C'est le cas le plus exposé à la lassitude, et
  cet ADR l'alourdit. Ce qui le borne : le périmètre critique est étroit, et
  le dialogue ne s'ouvre qu'une fois le choix fait dans l'écran.
* **−** Un dialogue natif est pauvre : ni coloration, ni défilement fiable,
  ni case à cocher. L'instruction y est coupée ; son milieu ne se relit que
  dans l'écran, qu'un script aurait pu falsifier. Le classement du backend, le
  début, la fin et le nombre de caractères manquants sont ce qui reste vrai.
* **−** Une confirmation donnée dans la première seconde est perdue, et
  l'utilisateur rapide doit recommencer depuis l'écran.
* **−** **Trois limites que cet ADR nomme sans les fermer**, parce qu'elles
  débordent sa décision :
  * créer une **seconde** connexion vers la même cible, marquée
    `development` ou `Sampled`, contourne les familles 2 et 3 par duplication
    plutôt que par changement, dès que la cible ne demande aucun secret
    (fichier SQLite ou DuckDB, serveur sans mot de passe) ;
  * sous `Sampled`, le message d'erreur complet du serveur rejoint l'agent
    (`crates/oxyn-ai/src/failure.rs`), et un tel message peut citer des
    valeurs de lignes : c'est une sortie de lignes **non approuvée**, que la
    famille 2 ne couvre pas ;
  * le dialogue de déclaration d'un agent externe, qui rejoint le port,
    montre les valeurs de ses variables d'environnement, où se trouve souvent
    un jeton — sa forme reste celle
    qu'[ADR-0026](0026-agents-externes-acp.md) a décidée.
* **−** Trois moteurs, trois rendus : le dialogue de macOS, de Windows et de
  GTK n'ont ni la même largeur ni le même ordre de boutons, et `NativeDialog`
  ne se teste qu'à la main.
* **−** Un script peut encore **ouvrir** un dialogue critique au moment de son
  choix ; il ne peut ni l'empiler ni le rouvrir sur la même décision, mais un
  utilisateur qui confirme sans lire reste possible.

**Coût de sortie :** faible. Retirer l'appel au port dans les trois chemins
rend la confirmation à la webview ; le trait et `NativeDialog` restent pour
`ai_save_external_agent`. Aucun format persisté ne change.

**Reconsidérer si** la webview gagne un moyen **vérifiable** de distinguer un
geste de l'utilisateur d'un appel de script — une permission Tauri liée à un
événement d'entrée réel, par exemple ; si les écritures en production
confirmées sans lecture deviennent fréquentes, auquel cas la réponse serait de
retirer l'écran de la webview pour ce cas, pas le dialogue ; si l'une des trois
limites nommées aux Conséquences est fermée par une décision — elle rejoint
alors, ou non, la liste du § 1 ; ou si une nouvelle
décision fait sortir des données ou déclasse une connexion — elle rejoint alors
la liste du § 1, qui se modifie par un ADR.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Garder la confirmation dans la webview, durcie (CSP, `capabilities`) | La CSP rend la XSS plus difficile ; elle ne rend pas un bouton de la webview inaccessible à un script qui s'y exécute. La défense ne tiendrait qu'à l'absence de faille, qu'on ne sait pas prouver |
| Un jeton à usage unique rendu à la webview avec la demande | Le script lit le jeton là où l'écran le lit : même origine, même DOM |
| Tout confirmer par dialogue natif | La lassitude viderait le dialogue de son sens : il doit rester rare pour être lu |
| Ne dialoguer que pour l'abaissement d'un marquage | Fixer quel sens est sûr est une seconde règle de niveau ; `development → production` puis `production → development` redevient un contournement en deux temps |
| Une fenêtre Tauri secondaire dessinée en HTML | C'est encore une webview, avec son script ; la garantie viendrait de l'isolation entre fenêtres, pas de l'hôte |
| Tester par `tauri::test` et `MockRuntime` | Le test dépendrait du comportement du plugin hors d'une vraie fenêtre ; le port sépare ce qu'on décide de ce que l'hôte dessine |
