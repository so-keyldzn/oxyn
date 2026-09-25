---
name: socle-ne-verifie-pas-index-adr
description: make socle passe sans avertissement quand un ADR manque dans docs/README.md, contrairement à ce qu'affirme .claude/commands/adr.md
metadata:
  type: feedback
---

`make socle` ne contrôle pas que chaque fichier de `docs/adr/` figure dans l'index
`docs/README.md`. `.claude/verifier_socle.py` ne lit que CLAUDE.md, AGENTS.md et
README.md racine pour ce sujet (constaté le 2026-09-25 : un ADR-0042 absent de
l'index a donné « Socle cohérent (0 avertissement) »).

**Why:** `.claude/commands/adr.md` dit que ce contrôle « attrape les ADR absents de
l'index » ; s'y fier laisse un ADR introuvable.

**How to apply:** après avoir écrit un ADR, vérifier l'index à la main
(`grep NNNN docs/README.md`) ou le signaler à l'appelant s'il s'est réservé le
raccordement. Re-vérifier le script avant de citer ce souvenir : il a pu être corrigé.
