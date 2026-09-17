---
description: Implémenter ou modifier un driver de base de données
argument-hint: "<protocole ou crate, ex. postgres>"
allowed-tools: Bash, Read, Write, Edit, Grep, Glob, Skill, WebFetch
---

Objet : implémenter ou modifier le driver **$ARGUMENTS**.

Cette commande existe parce que `crates/` peut être vide : la règle
`.claude/rules/drivers.md` ne se charge que quand Claude *lit* un fichier
existant, donc jamais pour le premier fichier d'un driver neuf.

## Avant d'écrire

1. Lire `docs/DRIVER-CONTRACT.md` **en entier**. Ce n'est pas une formalité :
   les sept garanties sont chacune un incident déjà anticipé.
2. Lire `docs/adr/0003-driver-capabilities.md` et
   `docs/adr/0002-arrow-result-model.md`.
3. Lire `.claude/rules/drivers.md`.

## La question qui vient avant toutes les autres

**Est-ce un nouveau protocole, ou un produit qui parle un protocole déjà
implémenté ?**

Redshift parle PostgreSQL. MariaDB parle MySQL. OpenSearch parle Elasticsearch.
Memgraph parle Bolt. pgvector et TimescaleDB sont des extensions PostgreSQL.

Si le protocole existe déjà, **il n'y a pas de crate à créer** : la différence
se déclare en capacités. Créer la crate quand même, c'est se condamner à
maintenir deux décodeurs de protocole pour un dialecte — et à corriger chaque
bug deux fois, en oubliant une fois sur deux.

## Outils plutôt que mémoire

| Besoin | Outil |
|---|---|
| Version d'un crate de pilote | `/versions`, jamais la mémoire ([I-12](../../CLAUDE.md#i-12)) |
| Sémantique exacte d'un type serveur | la documentation officielle du SGBD, autorisée dans `permissions.allow` |
| Signature d'une API de pilote | docs.rs, autorisé |

## Constructions à utiliser / jamais

| Utiliser | Jamais | Pourquoi |
|---|---|---|
| Flux de `RecordBatch` | `Vec<Row>` complet | [I-06](../../CLAUDE.md#i-06) : OOM sur un clic dans la barre latérale |
| Lot borné **en octets** | lot borné en nombre de lignes | mille lignes × 1 Mo de BLOB = 1 Go |
| Annulation atteignant le serveur | abandon du futur | la requête tourne encore et tient une connexion |
| `Capabilities` évaluées par session | par driver | la version du serveur change ce qui existe |
| Erreur classée transitoire / permanente / **ambiguë** | booléen `is_retryable` | l'ambiguë ne se retente pas ([I-13](../../CLAUDE.md#i-13)) |
| Citation d'identifiant par le driver | `format!("SELECT * FROM {}", nom)` | [I-10](../../CLAUDE.md#i-10) |

## Les pièges, avec leur scénario de panne

**Le lot en nombre de lignes.** Marche sur les tables de démonstration, déclenche
l'OOM sur les vraies. Le symptôme est un processus tué sans trace sur macOS.

**L'annulation qui n'annule que le futur.** L'utilisateur ferme dix onglets ; dix
requêtes tournent toujours côté serveur, tenant dix connexions. La base refuse
les nouvelles connexions, et l'utilisateur conclut qu'Oxyn a cassé sa production.

**L'erreur ambiguë rejouée.** Un `INSERT` expire côté client alors que le serveur
l'a appliqué. Classé transitoire et rejoué : doublon dans les données, aucune
erreur nulle part. C'est le cas le plus tentant à traiter par une boucle de
retry, et le plus cher.

**Le `NUMERIC` converti en `f64`.** PostgreSQL accepte une précision arbitraire ;
`f64` non. Des montants sont corrompus, silencieusement, et la corruption est
définitive une fois recopiée dans un `UPDATE`.

**Le fuseau inventé à la lecture.** Un `timestamp` sans fuseau auquel on attribue
celui du poste : l'utilisateur recopie la valeur affichée dans un `UPDATE` et
décale la donnée de deux heures en base.

**Le `SET search_path` silencieux.** Change le sens de toutes les requêtes
suivantes de l'utilisateur, sans qu'il l'ait demandé ni qu'il puisse le voir.

## Vérifier

```bash
make qualite
```

Puis `.claude/checklists/revue-driver.md`, intégralement. Les deux tests qui ne
se contournent pas :

1. **l'annulation prouvée côté serveur** — vérifiée dans la vue des processus du
   SGBD, pas au retour de la fonction ;
2. **le flux sur un volume qui ne tiendrait pas en mémoire** — avec une borne sur
   la mémoire du processus, sinon le test passe par accident.

Enfin, lancer l'agent `relecteur-invariants` sur le résultat.

## Rappels

- un driver ne dépend que d'`oxyn-core`, `oxyn-driver`, `oxyn-data` et
  `oxyn-catalog` ;
- un driver ne retente jamais tout seul : la politique de reprise appartient à
  l'appelant, seul à savoir si l'opération est rejouable ;
- ne pas savoir faire est une réponse acceptable, la déclarer en capacité ;
  laisser croire ne l'est pas.
