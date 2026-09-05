---
description: Implémenter un changement en respectant les invariants
argument-hint: "<ce qu'il faut implémenter>"
allowed-tools: Bash, Read, Write, Edit, Grep, Glob, WebFetch
---

Objet : implémenter **$ARGUMENTS**.

## Avant d'écrire

Si le geste a sa propre commande, l'utiliser plutôt que celle-ci — elle charge
le contrat précis :

| Geste | Commande |
|---|---|
| Driver de base de données | [`/driver`](driver.md) |
| Commande du command bus | [`/commande`](commande.md) |
| Vue GPUI | [`/vue`](vue.md) |
| Décision structurante | [`/adr`](adr.md) |

Sinon : relire les documents d'autorité de la frontière touchée
(`docs/README.md`), et la règle `.claude/rules/` du répertoire visé — **en
particulier si le fichier n'existe pas encore**, car une règle `paths:` ne se
charge que sur lecture d'un fichier existant.

## La règle qui gouverne les autres

**Rien ne contourne le command bus** ([I-01](../../CLAUDE.md#i-01)). Aucun
raccourci temporaire, aucun « en attendant que la commande existe ». Un second
chemin d'exécution ne disparaît jamais, et c'est celui que l'IA empruntera.

## Constructions à utiliser / jamais

| Utiliser | Jamais | Pourquoi |
|---|---|---|
| `?`, `let … else` | `unwrap()`, `expect()` sur un chemin réseau | [I-09](../../CLAUDE.md#i-09) : la panique tue l'application et le travail non sauvegardé |
| `u32::try_from(n)?` | `n as u32` | tronque en silence : un identifiant de 5 milliards devient 705 032 704 |
| `impl fmt::Debug` manuel sur un porteur de secret | `#[derive(Debug)]` | [I-03](../../CLAUDE.md#i-03) : fuite invisible à la relecture |
| Un module nommé par son sujet | `utils`, `common`, `helpers` | point de couplage universel |
| `TODO(2026-09-05) : ce qui le débloque` | `TODO` nu | un TODO non daté n'est jamais relu |

## Les pièges transverses

**L'abstraction pour un seul appelant.** Un trait à une implémentation qui n'est
pas une frontière est une indirection, pas un découplage : il rend le code plus
difficile à lire sans rien rendre remplaçable.

**L'optimisation sans mesure.** On ne remplace pas du code clair par du code
rapide sans le chiffre avant et après
(`docs/PERFORMANCE.md`). La complexité est payée d'avance, le gain est supposé.

**Le code mort « au cas où ».** Git s'en souvient. Le code commenté, lui, sera
lu par quelqu'un qui croira qu'il compte.

**Le commentaire qui redit le code.** Un commentaire dit *pourquoi*. Ce que fait
le code, le code le dit — et lui reste vrai après la prochaine modification.

## Vérifier

```bash
make qualite
```

**Rien n'est terminé sans cette commande.** Ni « ça compile », ni « le test
passe » : la porte inclut le format, clippy en `-D warnings`, les tests et la
documentation.

Puis `.claude/checklists/fin-de-tache.md`.

## Rappels

- une version externe ne s'écrit jamais de mémoire : [`/versions`](versions.md) ;
- le code, les identifiants et les commentaires sont en **anglais** ; les
  commits et la documentation en **français** ;
- si le code contredit un document de `docs/`, c'est un bug : le signaler, ne
  pas trancher seul.
