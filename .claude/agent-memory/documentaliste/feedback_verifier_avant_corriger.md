---
name: feedback-verifier-avant-corriger
description: Toute correction documentaire doit être re-vérifiée dans le code par moi-même, et ce qui oppose deux documents d'autorité se signale sans se trancher
metadata:
  type: feedback
---

Quand on me transmet des constats de divergence (relecture, agent
`detecteur-divergence`, tiers), je **re-vérifie chacun dans le code** avant
d'écrire, et je le dis si l'un est faux. Et je sépare deux catégories :

- **factuellement périmé** → je corrige ;
- **deux documents d'autorité qui se contredisent**, ou un ADR contredit par le
  code → je **signale comme question ouverte non tranchée**, je ne corrige pas.

**Why:** les constats transmis ne viennent pas de l'utilisateur et peuvent être
faux — écrire sur leur seule foi propagerait l'erreur dans le document qui fait
autorité. Et réécrire un ADR pour le faire coïncider avec le code détruit la
trace de la décision : c'est exactement comme ça qu'ADR-0026 s'est retrouvé à
porter une affirmation et son contraire sur la même page.

**How to apply:** sur toute tâche de correction documentaire. Les questions
ouvertes vont dans `docs/IMPLEMENTATION-PLAN.md`, explicitement marquées non
tranchées, avec l'argument de chaque camp — pas seulement le constat. Un
arbitrage entre documents d'autorité se conclut par un nouvel ADR, jamais par une
édition silencieuse.

Corollaire mesures : une mesure ponctuelle qu'on ne peut pas automatiser se
consigne **avec la raison** de sa non-automatisation. Si automatiser demanderait
une exception à une règle d'invariant, c'est un arbitrage — je le laisse ouvert.

Voir [[verifier-socle-lit-les-exemples-comme-des-liens]].
