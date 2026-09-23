---
description: Passe de conformité shadcn sur apps/desktop — relevé en parallèle, tri, correction par lots, vérification
argument-hint: "[chemin sous apps/desktop/src, défaut : tout src hors components/ui] [--releve]"
allowed-tools: Bash, Read, Grep, Glob, Agent, AskUserQuestion
---

Objet : mettre **$ARGUMENTS** (à défaut, tout `apps/desktop/src` hors
`components/ui`) en conformité avec les conventions shadcn, sans changer de
comportement. Avec `--releve`, s'arrêter après l'étape 3.

Le travail est fait par l'agent [`shadcniste`](../agents/shadcniste.md) ; cette
commande l'orchestre. C'est ici que vit le parallélisme, parce qu'un sous-agent
ne peut pas en lancer d'autres.

## 1. Le contexte

```!
cd apps/desktop && pnpm exec shadcn info --json 2>/dev/null | head -40
```

```!
git status --short apps/desktop
```

Un fichier déjà modifié dans l'arbre de travail appartient à quelqu'un :
**l'exclure des lots** et le dire, plutôt que de mêler deux travaux dans un
même diff.

## 2. Le relevé mécanique

Ce que `rg` sait voir. C'est le point de départ, pas le verdict : chaque motif a
ses faux positifs, que `shadcniste` connaît.

```!
cd apps/desktop/src && for p in \
  'space-[xy]-' \
  '\bw-(\d+)\b[^"]*\bh-\1\b' \
  '\b(bg|text|border|ring|fill|stroke)-(red|green|blue|yellow|orange|amber|emerald|gray|slate|zinc|neutral|stone|sky|indigo|violet|purple|pink|rose|lime|teal|cyan|fuchsia)-\d+' \
  '\bdark:' '\bz-(\[|\d)' 'text-ellipsis' 'asChild' 'className=\{`' '<hr' 'animate-pulse' \
  'lucide' '\b(isLoading|isPending)=' '<button\b' 'variant=\{[^}]*\?' ; do
  n=$(rg -c --pcre2 -g '*.tsx' -g '!components/ui/**' -e "$p" . 2>/dev/null | awk -F: '{s+=$2} END{print s+0}')
  [ "$n" != 0 ] && printf '%4s  %s\n' "$n" "$p"
done; true
```

Le reste — `FieldGroup`, `Empty`, `Alert`, `Card` complète, `ToggleGroup`,
items hors de leur groupe, `Dialog` sans titre, icônes sans `data-icon` — ne se
voit qu'en lisant. C'est le travail de l'étape 3.

## 3. Le relevé par lots, en parallèle

Découper le périmètre en **lots disjoints** d'une quinzaine de composants, story
comprise avec son composant : `components/oxyn/assistant-*`, le reste de
`components/oxyn` en deux ou trois lots, `features/`, `routes/`.

Lancer un `shadcniste` **en mode relevé** par lot, tous dans le même message.
Chaque prompt donne la liste exacte des fichiers, le mode, et les écarts de
l'étape 2 qui tombent dans le lot.

Rassembler les tableaux, retirer les doublons, puis présenter à l'utilisateur :

- le compte par règle et par risque (`mécanique`, `visuel`, `décision`) ;
- **chaque `décision`**, une par une, avec `AskUserQuestion` quand elle a des
  options nettes : registre à utiliser, jeton à ajouter, composant à installer,
  composant de `ui/` à mettre à jour depuis l'amont.

Avec `--releve`, la commande s'arrête ici.

## 4. La correction par lots

Mêmes lots, `shadcniste` **en mode correction**, en parallèle — **trois à la
fois au plus** : chacun lance ses stories dans Chromium, et au-delà les échecs
dus à la charge noient les vrais. Chaque prompt reprend les écarts retenus du
lot et les décisions prises par l'utilisateur.

Les lots sont disjoints, donc un seul arbre de travail suffit. Si un lot doit
toucher un fichier partagé (`styles.css`, un fichier de `components/ui` mis à
jour par la CLI), le sortir du parallélisme et le traiter **après**, seul.

## 5. La vérification

Elle ne se délègue pas à ceux qui ont écrit.

```bash
make front
make qualite
```

Puis relancer le relevé de l'étape 2 : les compteurs doivent avoir baissé, et
tout ce qui reste doit figurer dans un rapport d'agent avec sa raison. Un écart
qui a disparu sans être rapporté comme corrigé est à regarder.

Enfin `relecteur-invariants` sur le diff : une correction de balisage peut
toucher un `invoke`, un identifiant de connexion dans une story, un état vide.

## Le rapport final

1. Avant/après du relevé mécanique.
2. Les corrections, groupées par règle, avec le nombre de fichiers.
3. Ce qui n'a pas été corrigé, et pourquoi.
4. Le résultat de `make qualite`, tel quel.

Pas de commit : il appartient à l'utilisateur.

## Avec l'outil Workflow

Si l'utilisateur demande explicitement un workflow, les étapes 3 à 5 s'y
transposent directement : `parallel()` des relevés, tri en script, un
`pipeline()` des corrections plafonné à trois, puis la vérification. Les
décisions de l'étape 3 se prennent **avant** de lancer le script, parce qu'un
workflow ne pose pas de question en cours de route.
