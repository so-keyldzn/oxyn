# Hooks

Ce que `CLAUDE.md` ne peut que **demander**, un hook l'**impose**. Un hook n'est
pas un rappel : c'est un mur.

Un hook n'est écrit que pour un invariant dont la violation est **silencieuse**
et **détectable par une expression régulière**. Ce qui se voit à la compilation
ou aux tests appartient à une règle, pas ici.

## Les quatre faits de protocole

Ils décident du code. Ils sont écrits ici pour ne pas être re-découverts à
chaque hook ajouté.

**1. `deny` parle au modèle, `ask` parle à l'utilisateur.**
Sur un `deny`, `permissionDecisionReason` est transmise **à Claude** : elle doit
donc dire quoi faire à la place. Sur un `ask`, elle va **à l'utilisateur seul** ;
sans `additionalContext`, Claude voit son action suspendue sans savoir par quoi
et retente à l'identique. `protocole_hook.demander()` reprend donc la raison dans
les deux champs.

**2. `systemMessage` est un champ de premier niveau, et `Stop` ne le jette pas.**
C'est le canal d'un rappel de fin de tour. `additionalContext` au même endroit
relancerait le travail au lieu d'informer.

**3. Le champ `if:` de `settings.json` est best-effort.**
On ne s'en sert que là où ne pas s'exécuter est sans conséquence — le formateur.
**Jamais** sur un hook de refus : il y ouvrirait une faille silencieuse.

**4. Un hook défaillant n'interrompt jamais le travail.**
Toute erreur d'exécution se termine en sortie 0 silencieuse. Seul un refus
délibéré parle. C'est le rôle du `except Exception: sys.exit(0)` qui clôt chaque
hook.

## Les fichiers

| Fichier | Événement | Rôle |
|---|---|---|
| `protocole_hook.py` | — | lire l'événement, écrire la décision. **Rien d'autre** : ce n'est pas un fourre-tout |
| `code_interdit.py` | `PreToolUse` sur `Write\|Edit` | refuse les motifs interdits, interroge sur les douteux |
| `bash_interdit.py` | `PreToolUse` sur `Bash` | refuse les commandes interdites, **et interroge dès qu'une commande écrit un fichier du dépôt** |
| `message_commit.py` | `PreToolUse` sur `Bash` | format du message de commit |
| `formater.py` | `PostToolUse` | `rustfmt` sur le fichier écrit |
| `contexte_session.py` | `SessionStart` | injecte l'état réel — c'est ce qui permet à `CLAUDE.md` de ne rien contenir de périssable |
| `rappel_qualite.py` | `Stop` | rappelle `make qualite`, **une fois par session** |
| `verifier_versions.py` | — | outil de [I-12](../../CLAUDE.md#i-12), appelé par [`/versions`](../commands/versions.md) |
| `test_hooks.py` | — | fige les décisions attendues |

## Pourquoi `bash_interdit.py` intercepte les écritures

Sans cela, **tous** les garde-fous de `code_interdit.py` se contournent par
`cat > fichier.rs`, `sed -i` ou `python3 -c`. Un hook d'écriture qui ne couvre
pas le shell ne couvre rien.

C'est pour la même raison que le hook **déplie les lanceurs** (`uv run`, `npx`,
`xargs`, `timeout`…) : sans cela, `timeout 30 git push --force` passe. Ce cas
précis est un défaut qui a été trouvé par les tests, pas à la relecture — d'où
la règle suivante.

## Ajouter un motif

Un motif ajouté sans son faux positif dans `test_hooks.py` sera refusé en revue.

Le filtrage se fait sur les **mots** de la commande, jamais sur la ligne brute :
sinon `echo "ne jamais faire git push --force"` se fait refuser. Pour le code, on
ignore les lignes de commentaire, sinon une mention de `gpui` dans un `//` est
prise pour un import.

**Un faux positif bloque le travail à chaque tour, et le hook finit désactivé —
emportant les vrais positifs avec lui.** C'est le mode de panne d'un hook zélé,
et il est pire que l'absence de hook.

## Vérifier

```bash
python3 .claude/hooks/test_hooks.py
```

Les tests lancent les hooks en sous-processus avec de vrais événements : ils
vérifient le comportement réel, pas les intentions.
