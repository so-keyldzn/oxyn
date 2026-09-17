---
name: memoire-a-la-racine-du-depot
description: La mémoire d'agent s'écrit dans /.claude/agent-memory/frontiste/ du dépôt, jamais sous apps/desktop — sinon prettier la voit et make qualite échoue
metadata:
  type: feedback
---

Écrire la mémoire d'agent dans `<racine du dépôt>/.claude/agent-memory/frontiste/`,
**en chemin absolu**, jamais relativement au répertoire de travail.

**Why:** écrite sous `apps/desktop/.claude/…`, elle tombe dans le périmètre de
`prettier --check .` que `make front` lance **depuis `apps/desktop`**. Prettier
reformate le Markdown et juge ces fichiers mal formatés : `make qualite`
échouait là-dessus et nulle part ailleurs, donc sur un symptôme sans rapport
avec le code. Le team-lead a dû déplacer les fichiers et fusionner l'index à la
main.

**La cause racine, constatée le 2026-09-16** : la consigne de session de
l'agent `frontiste` désigne elle-même
`apps/desktop/.claude/agent-memory/frontiste/` comme répertoire de mémoire (« This
directory already exists — write to it directly »), parce qu'elle se résout
depuis le répertoire de travail. L'outillage **recrée ce répertoire, vide**, même
après suppression. Suivre la consigne à la lettre reproduit la faute. Le
correctif appartient à la définition de l'agent, pas à cette mémoire.

**How to apply:** à chaque `Write` dans `agent-memory`, **ignorer le chemin
annoncé par la consigne de session** et écrire sous la racine. Le répertoire de travail
de cette session est `apps/desktop`, pas la racine — c'est précisément ce qui
rend le piège invisible. Deux corollaires : l'index `MEMORY.md` de la racine est
**partagé** (y ajouter une ligne, ne jamais l'écraser), et tout ce qui atterrit
sous `apps/desktop/` passe par la porte front, y compris ce qui n'est pas du
code.
