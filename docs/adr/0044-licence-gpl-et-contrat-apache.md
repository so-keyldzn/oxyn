# ADR-0044 — L'application est sous GPL-3.0-or-later, le contrat des drivers sous Apache-2.0, et ce qui se paie est un service de compte

**Statut :** accepté · **Date :** 2026-09-25

## Contexte

Jusqu'au 2026-09-25, tout le dépôt est sous Apache-2.0 (`Cargo.toml`,
`license = "Apache-2.0"` au niveau du workspace). Trois faits rendent ce choix
intenable à terme :

- **Le dépôt deviendra public** pour que l'application soit téléchargeable. Une
  version publiée sous Apache-2.0 le reste pour toujours : n'importe qui peut la
  reprendre, la fermer et la vendre, sans rien rendre.
- **Le titulaire du copyright veut monétiser.** Il s'agit de Nicolas Boromée,
  seul auteur du dépôt à cette date. Une société viendra plus tard, et les droits
  lui seront cédés.
- **Relicencier suppose de détenir les droits de tout le code.** Aujourd'hui,
  c'est le cas : un seul auteur, un dépôt privé. Au premier contributeur externe
  sans accord écrit, ce ne l'est plus, et le modèle de licence se fige.

L'utilisateur a tranché le 2026-09-25 : l'application passe sous une licence
copyleft, et ce qu'un tiers doit lier pour l'étendre reste sous une licence
permissive.

Deux principes de [VISION](../VISION.md) bornent la réponse :

- **Open by default** : « formats ouverts, pas de verrouillage ». Une licence
  qui cesse d'être libre au sens de l'OSI, ou une fonctionnalité locale mise
  derrière un abonnement, le contredirait ;
- **Privacy first** : « hors-ligne par défaut ». Un service payant ne peut donc
  être qu'une option, jamais une condition pour se servir du produit.

## Décision

### Deux licences, partagées par ce qu'un tiers doit lier

**L'application est sous `GPL-3.0-or-later`.** C'est la valeur `license` de
`[workspace.package]`. Elle couvre les crates du cœur, `oxyn-desktop`, les
drivers livrés, et le front `apps/desktop` (`package.json`, champ `license`).

**Ce qu'un tiers doit lier pour écrire un driver reste sous `Apache-2.0`.** Un
auteur de driver externe implémente les traits d'`oxyn-driver`. Le graphe de
cette crate, relevé par `cargo tree -p oxyn-driver -e normal`, contient trois
autres crates du workspace, et aucune autre :

| Crate | Pourquoi elle est dans l'ensemble |
|---|---|
| `oxyn-driver` | les traits `Driver`, `Session` et `Cursor`, et les capacités ([ADR-0003](0003-driver-capabilities.md)) |
| `oxyn-data` | `BatchSource`, `ResultBuffer` et le flux de `RecordBatch` qu'un driver produit ([ADR-0002](0002-arrow-result-model.md)) |
| `oxyn-catalog` | le modèle de métadonnées que l'introspection d'un driver remplit (`CatalogProvider`) |
| `oxyn-core` | les identifiants, les erreurs et les types communs, dont dépendent les trois autres |

Ces quatre manifestes écrivent `license = "Apache-2.0"`. Ils n'héritent pas de
la valeur du workspace. L'ensemble est **minimal** : une crate n'y entre que si
un driver tiers ne peut pas compiler sans elle.

**Les plugins.** Un plugin WASM ne lie pas `oxyn-plugin` : il est compilé contre
des interfaces WIT, et l'hôte le charge ([ADR-0005](0005-wasm-plugins.md)).
`oxyn-plugin` est donc l'hôte, sous GPL. Les interfaces WIT n'existent pas
encore (phase 4). Elles seront publiées sous `Apache-2.0`, dans un répertoire ou
une crate qui porte cette licence seule. Un fichier sous Apache à l'intérieur
d'une crate sous GPL ne se verrait ni dans son manifeste ni dans `cargo deny`.

La répartition est écrite dans `NOTICE`. Les textes sont à la
racine : `LICENSE-GPL`, téléchargé depuis gnu.org, et `LICENSE-APACHE`.

### Les crates sous GPL ne sont pas publiées

Chaque crate sous GPL déclare `publish.workspace = true`, et le workspace
déclare `publish = false`. `deny.toml` active `[licenses.private] ignore = true` :
`cargo deny` ne juge pas la licence d'une crate du workspace qui ne se publie
pas. `GPL-3.0-or-later` n'entre **pas** dans la liste `allow`. Elle y
accepterait aussi une dépendance tierce sous GPL, et une dépendance copyleft
doit rester un arbitrage explicite. Les quatre crates sous Apache restent
publiables : c'est par crates.io qu'un auteur de driver les obtiendra.

### Un CLA pour toute contribution externe

Toute contribution externe est couverte par le CLA (`CLA.md`), fondé sur
l'*Individual Contributor License Agreement* v2.2 de l'Apache Software
Foundation. Le contributeur garde ses droits. Il accorde au titulaire du
copyright une licence perpétuelle, irrévocable et sous-licenciable, ce qui
inclut le droit de relicencier. Cette licence passe au cessionnaire de ce
copyright, notamment à la société que le titulaire fondera.

Le CLA est en place **avant** le premier contributeur externe et **avant** le
passage du dépôt en public. `CONTRIBUTING.md` l'explique. La
signature automatique (CLA Assistant) s'active au passage en public : c'est un
réglage de l'organisation GitHub, pas un fichier du dépôt.

Conséquence directe : **aucun code tiers sous GPL ne se recopie dans le dépôt**,
même si les licences le permettent désormais. Son copyright n'appartient pas au
titulaire : il n'est couvert par aucun CLA et interdirait de relicencier.

### Ce qui se paie : un service lié à un compte

**Tout le code de ce dépôt est utilisable sans abonnement.** Aucune
fonctionnalité locale n'est bridée, aucune n'attend une clé de licence. Ce qui
se paie, ce sont des **services** rendus par un serveur et rattachés à un compte :
la connexion au compte, l'IA hébergée, la synchronisation entre machines.

Le serveur de ces services n'est pas dans ce dépôt. Il n'est pas distribué : la
GPL ne l'oblige donc pas à publier son source, et rien ne l'oblige à partager
la licence de l'application.

Ces services sont **optionnels**. L'application fonctionne entièrement sans
compte et hors-ligne : ouvrir une base, l'explorer, l'interroger, et se servir
d'un fournisseur IA local ou de sa propre clé d'API. C'est
« Privacy first » : le cloud s'ajoute, il ne se substitue à rien. C'est aussi
« Open by default » : ce qu'Oxyn écrit reste lisible sans Oxyn ([I-11](../../CLAUDE.md#i-11)),
compte ou pas.

### Les mentions tierces sont livrées avec l'application

Distribuer un binaire, c'est redistribuer ses dépendances, et la plupart de
leurs licences demandent de reproduire leur texte. `script/licences-tierces
generer` réunit donc au build :

- les dépendances Rust d'`oxyn-desktop`, avec `cargo-about`, sans les
  dépendances de build ni de développement ;
- les dépendances npm de production d'`apps/desktop`, avec `pnpm licenses list`.

Le résultat est `apps/desktop/src/generated/third-party-licenses.json`, hors de
git, que le front embarque et que les réglages affichent dans la section
*About*. Un build de publication
(`make desktop PROFIL=release`) échoue sans ce fichier. `make licences-npm`
refuse une licence npm hors liste, comme `make deny` pour Rust. Les deux
contrôles s'appuient sur la même liste, celle de `deny.toml`.

## Conséquences

* **+** Un fork fermé de l'application n'est plus possible : quiconque
  distribue une version modifiée doit en publier le source sous GPL.
* **+** Le titulaire du copyright garde la liberté de relicencier, de vendre une
  licence commerciale ou de céder les droits à la société. Le CLA étend cette
  liberté aux contributions externes.
* **+** Un auteur de driver ou de plugin n'est pas contaminé : il lie du code
  sous Apache-2.0 et choisit sa propre licence, fermée comprise.
* **+** Le modèle économique ne demande de brider aucune fonctionnalité, et ne
  contredit ni « Open by default » ni « Privacy first ».
* **−** Les quatre crates sous Apache contiennent une part réelle du produit :
  les traits du driver, le modèle de catalogue, le tampon de résultats. Un tiers
  peut les reprendre dans un produit fermé. C'est le prix de l'ouverture aux
  drivers.
* **−** La frontière se déplace avec le code. Une crate du cœur qui devient une
  dépendance d'`oxyn-driver` doit passer sous Apache, ce qui suppose d'en
  détenir tous les droits. Sinon, la dépendance doit être retirée.
* **−** Un CLA décourage une partie des contributeurs, surtout quand il permet de
  relicencier. C'est la condition du modèle économique, et elle est assumée.
* **−** Distribuer un binaire sous GPL oblige à en offrir le source. Tant que le
  dépôt est privé, aucun binaire ne peut être distribué hors du titulaire.
* **−** Tout build dépend désormais de `cargo-about` pour produire les mentions
  tierces. Sans l'outil, un build de développement avertit et s'affiche sans
  mentions. Un build de publication, lui, échoue.

**Coût de sortie :** tant que le titulaire détient les droits de tout le code,
changer de licence ne demande que ses fichiers : manifestes, `LICENSE-*`,
`NOTICE`, `deny.toml`. Le CLA garantit que cela reste vrai après les premières
contributions. Mais une version déjà publiée sous GPL le reste pour ceux qui
l'ont reçue. Il est donc possible de restreindre les versions suivantes, jamais
les précédentes.

**Reconsidérer si** une des conditions suivantes est remplie :

- la société est créée et les droits lui sont cédés : `NOTICE`, le CLA et le
  champ `copyright` du bundle changent de titulaire ;
- un fournisseur de cloud revend l'application comme un service sans rien
  reverser, ce qu'une licence réseau comme l'AGPL empêcherait ;
- un auteur de driver doit lier une crate qui n'est pas dans l'ensemble Apache.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Apache-2.0 seule, la licence d'avant | un fork fermé devient possible, et il est définitif : chaque version publiée l'autorise pour toujours |
| FSL ou BSL, un source disponible converti en licence libre après un délai | ce n'est plus de l'open source au sens de l'OSI pendant le délai. Cela contredit « Open by default », et un public de professionnels des données lit la différence |
| Double licence AGPL et licence commerciale | l'AGPL vise celui qui sert le logiciel à travers un réseau, or Oxyn est une application de bureau qui ne se sert à personne. Le seul serveur est celui des services payants, qui reste hors du dépôt |
| GPL pour tout, contrat des drivers compris | tout driver ou plugin tiers deviendrait GPL. L'écosystème de drivers voulu par [ADR-0003](0003-driver-capabilities.md) et [ADR-0005](0005-wasm-plugins.md) serait fermé aux éditeurs de bases propriétaires |
| Déclarer `GPL-3.0-or-later` dans `allow` de `deny.toml` | cela accepterait aussi une dépendance tierce sous GPL, sans arbitrage. `publish = false` exempte nos crates et elles seules |
| Pas de CLA, seulement un DCO (*Developer Certificate of Origin*) | un DCO certifie l'origine d'une contribution, pas une licence au titulaire. Il ne permet ni de relicencier ni de céder les droits |
| Brider des fonctionnalités locales (open core) | cela contredit « Open by default ». Le code bridé deviendrait aussi un second dépôt à maintenir, sous une autre licence |
