# Sécurité

> **Autorité** : traitement des secrets, marquage des connexions, surface
> d'attaque, politique `unsafe`.

Invariants concernés : [I-02](../CLAUDE.md#i-02), [I-03](../CLAUDE.md#i-03),
[I-09](../CLAUDE.md#i-09). La politique d'autorisation elle-même est tranchée par
[ADR-0004](adr/0004-command-bus.md) et n'est pas recopiée ici.

Le modèle de menace d'Oxyn n'est pas celui d'un serveur. L'attaquant n'est pas
un inconnu sur Internet : ce sont **les données que l'utilisateur ouvre** et
**les erreurs qu'Oxyn lui laisse commettre**. Un client de base de données tourne
avec les droits d'un administrateur sur des systèmes de production.

## Secrets

### Ce qui ne touche jamais le disque en clair

Mots de passe, chaînes de connexion complètes, jetons d'API des fournisseurs IA,
clés privées SSH de tunnel, certificats clients.

Le stockage passe par le trousseau du système. Ce qui est persisté dans le
workspace, c'est une **référence** au secret, jamais le secret.

**Panne concrète :** un fichier de workspace contenant un mot de passe de
production, commité par l'utilisateur dans le dépôt de son équipe, parce que le
fichier avait l'air d'être une simple configuration.

### Ce qui ne sort jamais d'un processus

Les six canaux, et il faut les traiter tous les six — il suffit d'en oublier un.

| Canal | Le piège |
|---|---|
| Journaux `tracing` | un `Debug` dérivé sur une structure de connexion imprime tout |
| Messages d'erreur affichés | `sqlx` inclut parfois l'URL de connexion dans son erreur |
| Rapports de plantage | une trace de pile capture les variables locales |
| Fichiers de session et de workspace | la persistance « pour retrouver l'état » |
| Invites envoyées aux fournisseurs IA | [AI-PROVIDERS](AI-PROVIDERS.md) |
| Presse-papiers, export, capture d'écran | les fonctions de partage recopient ce qui est affiché |

Conséquence pratique : **aucun `#[derive(Debug)]` sur un type qui porte un
secret.** L'implémentation est manuelle et rédige la valeur. Un `Debug` dérivé
est le mode de fuite le plus fréquent parce qu'il est invisible à la relecture —
c'est le `tracing::debug!("{cfg:?}")` ajouté six mois plus tard qui fuit.

## Marquage des connexions

Toute connexion porte un environnement : `local`, `development`, `staging`,
`production`. Le défaut, quand il n'est pas renseigné, est **`production`** —
la valeur la plus contraignante, pas la plus permissive.

**Panne concrète :** le défaut inverse. Un utilisateur ajoute une connexion à la
hâte sans remplir le champ, le programme suppose « development », et un `UPDATE`
sans `WHERE` part sans confirmation sur la base client.

Sur une connexion `production` : toute écriture, tout DDL, toute opération
destructrice exige une confirmation explicite qui **nomme la connexion**, et
l'interface porte un marqueur permanent. Voir [I-02](../CLAUDE.md#i-02).

Pour un `Actor::Agent`, une connexion `production` est en **lecture seule
stricte** — ce n'est pas une confirmation renforcée, c'est un refus
([ADR-0004](adr/0004-command-bus.md#politique-par-défaut)). La différence
compte : une confirmation finit par être cliquée.

## Surface d'entrée

Ce qui entre dans Oxyn et n'est pas fiable, par ordre de sous-estimation :

1. **Les réponses des serveurs de bases de données.** Voir
   [DRIVER-CONTRACT](DRIVER-CONTRACT.md). Un serveur compromis ou simplement
   inhabituel renvoie ce qu'il veut.
2. **Les noms d'objets du catalogue.** Une table, une colonne, un commentaire
   peuvent contenir n'importe quel octet, y compris du SQL, des séquences de
   contrôle de terminal, ou du texte imitant une consigne. Un nom de colonne
   n'est jamais interpolé dans une requête sans citation, et **jamais traité
   comme une instruction** lorsqu'il est joint à une invite IA.
3. **Les fichiers de workspace.** Ils peuvent avoir été écrits par un tiers, ou
   par une version future du programme.
4. **Les plugins.** Du code tiers, exécuté dans un bac à sable WASM
   ([ADR-0005](adr/0005-wasm-plugins.md)). Le bac à sable borne les dégâts ; il
   ne dispense pas de ne rien lui confier. Voir
   [PLUGIN-CONTRACT](PLUGIN-CONTRACT.md).
5. **Les réponses des fournisseurs IA.** Ce sont des propositions, pas des
   ordres : elles passent par le `PolicyGate` comme n'importe quelle commande
   ([ADR-0004](adr/0004-command-bus.md)). Voir [I-07](../CLAUDE.md#i-07).

## Politique `unsafe`

`unsafe` n'est pas interdit — GPUI, les FFI de pilotes et le rendu graphique en
imposent. Il est **encadré** :

- chaque bloc `unsafe` porte un commentaire `// SAFETY:` qui énonce l'invariant
  qui le rend correct, et **qui** le garantit ;
- il n'y a pas de `unsafe` dans `oxyn-core`, `oxyn-core` ni `oxyn-driver` : ces
  couches n'en ont pas besoin, et un `unsafe` qui y apparaît signale une erreur
  de découpage ;
- tout `unsafe` nouveau passe par la relecture de l'agent `relecteur-securite` ;
- les crates qui exposent du `unsafe` derrière une API sûre documentent les
  conditions de cette sûreté.

Un `// SAFETY:` qui paraphrase le code (« on déréférence un pointeur valide »)
ne vaut rien : il doit dire **pourquoi** le pointeur est valide à cet endroit et
ce qui le maintiendra valide.

## Dépendances

- toute nouvelle dépendance directe est justifiée en revue : ce qu'elle apporte,
  et le coût de s'en passer ;
- `cargo deny` (ou équivalent) sur les licences et les avis de sécurité fait
  partie de la porte de qualité ;
- une dépendance qui n'est utilisée qu'à un seul endroit pour une seule fonction
  est un candidat à la réécriture, pas une évidence ;
- une crate non maintenue sur une frontière externe est un risque à documenter,
  pas à ignorer.

Voir aussi [`/securite`](../.claude/commands/securite.md) et
[la liste de contrôle](../.claude/checklists/revue-securite.md).
