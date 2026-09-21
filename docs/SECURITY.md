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

### Ce qui sort vers un destinataire IA laisse une trace

**Ce qui part vers un destinataire IA est journalisé**, dans `ai_egress`, table
locale en ajout seul protégée comme `audit_journal` et jamais élaguée : la
connexion, la source, les **noms** des colonnes envoyées, le nombre de lignes,
le destinataire (fournisseur ou agent, modèle, portée `local`, `remote` ou
`unresolved` mesurée pour cet envoi), la commande qui a lu les données et la
conversation. **Jamais une valeur ni un jeton** : il n'y a pas de colonne pour
les ranger, et le fichier refuse une liste de colonnes qui contiendrait autre
chose que des noms. L'entrée s'écrit **avant** l'envoi ; si elle échoue, rien
ne part.

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
5. **La webview.** Une valeur de cellule, un nom d'objet ou une réponse de modèle
   rendus dans le DOM sont le premier vecteur d'une XSS, et une XSS dans la
   webview atteint les commandes Tauri ([ADR-0029](adr/0029-interface-tauri-shadcn.md)).
   D'où : rendu en texte seulement — jamais `dangerouslySetInnerHTML` sur une
   donnée reçue —, CSP stricte dans `crates/oxyn-desktop/tauri.conf.json`,
   *capabilities* minimales dans `capabilities/main.json`, et une surface IPC
   dont chaque commande est relue comme un changement de sécurité.
6. **Les réponses des fournisseurs IA.** Ce sont des propositions, pas des
   ordres : elles passent par le `PolicyGate` comme n'importe quelle commande
   ([ADR-0004](adr/0004-command-bus.md)). Voir [I-07](../CLAUDE.md#i-07).

## Politique `unsafe`

**`unsafe` est refusé à la compilation.** `[workspace.lints.rust]` porte
`unsafe_code = "deny"`, et aucune des quinze crates n'en contient ni ne le
réautorise. C'est le manifeste qui fait foi ici, parce que c'est lui qui est
exécuté : ce document décrivait auparavant une politique d'encadrement que la
compilation n'accorde pas, et [ADR-0021](adr/0021-marqueur-d-arret.md) a fondé
une décision d'architecture — ne pas vérifier un pid — sur le refus, pas sur
l'encadrement.

Lever ce refus est une décision d'architecture, pas un `#[allow]` local :

- elle passe par un **ADR** qui dit ce que `unsafe` achète et ce qu'il coûte ;
- le `#[allow(unsafe_code)]` qui en découle est porté au plus près, jamais au
  workspace ;
- chaque bloc porte un `// SAFETY:` qui énonce l'invariant qui le rend correct,
  et **qui** le garantit ;
- il passe par la relecture de l'agent `relecteur-securite` ;
- la crate qui l'expose derrière une API sûre documente les conditions de cette
  sûreté.

Les dépendances externes, elles, en contiennent — Tauri, ses
webviews et les FFI de pilotes en imposent. Le refus porte sur **ce que ce dépôt écrit**.

Un `// SAFETY:` qui paraphrase le code (« on déréférence un pointeur valide »)
ne vaut rien : il doit dire **pourquoi** le pointeur est valide à cet endroit et
ce qui le maintiendra valide.

## Dépendances

- toute nouvelle dépendance directe est justifiée en revue : ce qu'elle apporte,
  et le coût de s'en passer ;
- `cargo deny` sur les licences et les avis de sécurité fait partie de la porte
  de qualité, par la cible `make deny` que `make qualite` appelle. Elle **avertit
  sans bloquer** quand `cargo-deny` n'est pas installé : une porte qui échoue sur
  un outil absent finit par être contournée, et c'est alors tout le contrôle qui
  disparaît. La configuration vit dans `deny.toml` ;
- une dépendance qui n'est utilisée qu'à un seul endroit pour une seule fonction
  est un candidat à la réécriture, pas une évidence ;
- une crate non maintenue sur une frontière externe est un risque à documenter,
  pas à ignorer ;
- un avis de sécurité écarté l'est **dans `deny.toml`, avec sa raison écrite**.
  Le seul aujourd'hui est RUSTSEC-2024-0429 — une *unsoundness* de
  `glib::VariantStrIter`, atteinte par `tauri` → `gtk 0.18` → `atk` → `glib 0.18`.
  La correction est dans `glib 0.20`, que `gtk 0.18` refuse : rien ne se monte
  ici tant que Tauri n'a pas changé de GTK. Ce code est celui de Linux ; il
  n'est pas compilé sur macOS, la seule cible livrée aujourd'hui, et Oxyn
  n'appelle pas cet itérateur. À rouvrir à la prochaine montée de Tauri.

Voir aussi [`/securite`](../.claude/commands/securite.md) et
[la liste de contrôle](../.claude/checklists/revue-securite.md).
