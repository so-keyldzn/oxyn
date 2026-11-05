<!-- oxyn-translation source="CONTRIBUTING.md" sha256="f4c1c8b4505f" -->

> Traduction française de [CONTRIBUTING.md](../../CONTRIBUTING.md). **La version anglaise fait foi.**

# Contribuer à Oxyn

Merci d'envisager une contribution. Cette page couvre les deux choses dont toute
contribution a besoin : l'accord de licence, et la porte de qualité.

## L'accord de licence du contributeur

Toute contribution externe demande que vous acceptiez
l'[accord de licence du contributeur](../../CLA.md) (CLA) avant de pouvoir
être fusionnée. Vous le donnez une fois, et il couvre toutes vos contributions
futures.

**Pourquoi un CLA.** Oxyn est sous licence GPL-3.0-or-later. Le contrat des
drivers (`oxyn-core`, `oxyn-catalog`, `oxyn-data` et `oxyn-driver`) est sous
Apache-2.0 : voir [NOTICE](../../NOTICE). Le CLA permet au titulaire du droit
d'auteur, Nicolas Boromée, de continuer à proposer tout le projet sous un seul
ensemble de conditions. Il lui permet aussi de changer la licence du projet, et
de transférer ces droits à la société qui maintiendra Oxyn. Sans lui, chaque
contributeur devrait accepter un tel changement, et un seul pourrait le bloquer.
Les raisons sont consignées dans
[ADR-0044](docs/adr/0044-licence-gpl-et-contrat-apache.md).

**Ce que le CLA ne fait pas.** Vous gardez le droit d'auteur sur votre travail.
Vous restez libre d'utiliser votre contribution pour tout autre usage, sous
n'importe quelle licence.

**Comment l'accepter.** Lisez [CLA.md](../../CLA.md), puis publiez cette
phrase (en anglais, telle quelle) en commentaire de votre première pull
request :

> I have read the Oxyn Individual Contributor License Agreement, version 1.0,
> and I accept it for this pull request and all my future contributions.

Si vous contribuez pour le compte d'un employeur, la section 4 du CLA
s'applique : vous devez avoir son autorisation.

**Comment c'est vérifié.** Il n'y a ni bot ni contrôle automatique : le
contrôle `qualite` ne regarde pas le CLA. Le mainteneur lit votre commentaire
et le vérifie, à la main, avant de fusionner une pull request de quelqu'un qui
n'est pas le mainteneur. Sans lui, la pull request attend, quel que soit l'état
de sa relecture. Votre commentaire reste dans la pull request comme trace de
votre acceptation, et une pull request ultérieure n'en demande pas de nouveau,
sauf si le CLA change de version.

Le code copié d'un autre projet demande le même soin : dire d'où il vient et
sous quelle licence (section 7 du CLA). Du code GPL venu d'un autre projet ne
peut pas être accepté, même si les licences sont compatibles, parce que son
titulaire du droit d'auteur n'a pas accepté le CLA.

## Avant d'ouvrir une pull request

```bash
make qualite
```

C'est la porte de qualité unique : format, lints, types, tests, stories et leurs
contrôles d'accessibilité, documentation, et licences des dépendances. La CI
lance les mêmes cibles et n'en ajoute aucune. Une pull request qui ne la passe
pas n'est pas prête à être relue.

Quelques conventions que la porte ne peut pas vérifier :

- tout est en **anglais** : code, identifiants, commentaires, messages d'erreur,
  documentation, ADR, messages de commit et pull requests
  ([ADR-0047](docs/adr/0047-english-as-the-repository-language.md)).
  [`i18n/fr/`](../README.md) porte des miroirs français des documents anglais ;
  l'anglais fait foi ;
- les messages de commit suivent Conventional Commits : `type(portee): sujet`,
  en minuscules, à l'impératif, sans point final, 72 caractères au plus ;
- une nouvelle dépendance se justifie dans la pull request : ce qu'elle apporte,
  et ce que coûterait de s'en passer. Sa licence doit figurer dans la liste
  acceptée par `deny.toml`.

[CLAUDE.md](CLAUDE.md) liste les treize invariants du code, et
[docs/](docs/README.md) porte les documents d'autorité. Une contradiction entre
le code et l'un d'eux est un bug : la signaler plutôt que trancher seul.
