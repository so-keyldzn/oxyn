# ADR-0027 — La porte d'I-04 vaut pour les **deux** destinations, ou elle ne vaut pour aucune

**Statut :** proposé · **Date :** 2026-09-15

**Précise :** [ADR-0026](0026-agents-externes-acp.md), qui a ouvert une seconde
destination sans lui donner la porte typée de la première.

## Contexte

[I-04](../../CLAUDE.md#i-04) dit que rien ne rejoint une invite IA hors du point
de passage unique qui applique le niveau de la connexion. Ce que le dépôt en a
fait est plus fort qu'une consigne : `ContextBuilder::build` est la **seule**
fabrique d'`AgentContext`, `AgentSession::new` exige ce type, et l'invariant est
donc tenu par le compilateur. `crates/oxyn-ai/src/lib.rs` l'annonçait ainsi :
« vérifié par le compilateur, pas par la relecture ».

ADR-0026 a ajouté une seconde destination — un agent externe, processus enfant
parlant ACP. Son point d'entrée est :

```rust
pub async fn run_turn(
    agent: &ExternalAgentConfig,
    tier: PrivacyTier,
    prompt: &str,          // ← n'importe quelle String
    cancel: &CancelToken,
    observer: Arc<dyn AgentObserver>,
) -> Result<TurnEnd>
```

Le niveau y gouverne le **lancement** : `allows_external_agent` refuse avant que
le processus ne démarre, et c'est bien testé. Mais il ne gouverne pas
**l'assemblage de ce qui est envoyé**, parce qu'il n'y a rien à assembler : le
paramètre est une chaîne libre.

### Ce qui n'est pas un problème aujourd'hui

Il n'y a **aucune fuite**. L'unique appelant, `Workspace::ask_external_agent`, ne
transmet que la question tapée par l'utilisateur. Aucun contenu de catalogue,
aucune valeur de ligne, aucun nom de table n'entre dans cette invite. Vérifié en
relisant le seul chemin qui mène à `start_agent_turn`.

### Ce qui l'est

La **garantie** a changé de nature sans que personne ne le décide. Pour un
fournisseur, la question « qu'est-ce qui est sorti ? » se répond en relisant une
fonction. Pour un agent, elle demande de relire tous les appelants présents et à
venir — c'est-à-dire la propriété qu'I-04 existe précisément pour supprimer.

`.claude/rules/ia.md` nomme le raccourci qui la détruit : « un raccourci “juste
pour le schéma, c'est du `Metadata` de toute façon” détruit cette propriété —
plus personne ne peut répondre à “qu'est-ce qui est sorti ?” ». Ici, ce
raccourci s'écrirait en un `format!`, sans qu'aucun type ni aucun test ne
rougisse. C'est une invitation, pas un défaut : le prochain qui voudra donner le
schéma à un agent externe n'aura rien à contourner.

## Décision

**L'option B est retenue**, et mise en œuvre le 2026-09-15 :
`crates/oxyn-ai/src/external/prompt.rs` porte un `AgentPrompt` dont le seul
constructeur, `from_user`, exige le niveau de la connexion.
[`run_turn`](../../crates/oxyn-ai/src/external/turn.rs) ne prend plus de `&str`.

Ce qui a emporté le choix, contre **A** : `AgentContext` porte un schéma rendu,
des relations retenues et un budget de jetons dont un agent externe n'a l'usage
d'aucun. Contre **C** : le changement était encore mécanique — un seul appelant
—, et c'est le moment où il coûte le moins.

`run_turn` **garde sa propre vérification de niveau**. Ce n'est pas une
redondance à nettoyer : la porte protège l'assemblage, la vérification protège le
lancement, et aucune des deux ne doit s'en remettre à l'autre. Le test
`le_niveau_local_refuse_avant_meme_de_lancer_le_processus` compose désormais son
invite sous un niveau permissif pour éprouver cette seconde ligne seule.

Ce qui suit reste la trace des trois options telles qu'elles ont été pesées.

### A — `run_turn` prend un `&AgentContext`

Le chemin agent emprunte la porte du chemin fournisseur. I-04 redevient vérifié
par le compilateur pour les deux destinations, et la phrase de `lib.rs` redevient
vraie sans réserve.

*Coût* : `AgentContext` a été conçu pour une conversation à outils — il porte le
schéma, les capacités, le dialecte. Un agent externe n'en a l'usage d'aucun : il
ne reçoit qu'un texte. Faire passer un contexte riche là où une phrase suffit est
le genre d'abstraction que [CLAUDE.md](../../CLAUDE.md#organisation-du-code)
déconseille — « pas d'abstraction pour un seul appelant ».

### B — Un type d'invite dédié, mince, produit par la même porte

Un `AgentPrompt` (nom à décider) que **seul** un constructeur appliquant le
niveau sait fabriquer, et que `run_turn` exige. L'invariant redevient typé sans
imposer un contexte dont l'agent n'a que faire.

*Coût* : un type de plus, et la discipline de ne jamais lui donner de
constructeur public naïf. C'est peu, mais c'est une frontière à maintenir.

### C — Statu quo, assumé et écrit

On accepte que le chemin agent tienne I-04 par la relecture, à la condition
expresse que **rien d'autre que la saisie de l'utilisateur** n'y entre — et on
l'écrit là où quelqu'un le lira avant d'ajouter un `format!`.

*Coût* : la garantie reste inégale entre les deux destinations, et la
documentation doit cesser d'affirmer le contraire — ce qui a été fait le
2026-09-15 dans `oxyn-ai/src/lib.rs` et
[AI-PROVIDERS](../AI-PROVIDERS.md).

## Ce que cet ADR ne tranche pas

Le placement exact du constructeur de l'option B, et la question de savoir si la
même porte doit filtrer ce que l'agent **renvoie**. Le retour est déjà traité
séparément : seuls les fragments de texte remontent, `plan` et `tool_call` sont
écartés, et aucune sortie ne devient une `Command`
([I-07](../../CLAUDE.md#i-07)).

## Coût de sortie

**Faible tant qu'il n'y a qu'un appelant, et il croît vite.** Aujourd'hui,
passer de **C** à **A** ou **B** touche une signature, un site d'appel et deux
tests — une demi-journée. Le coût suit ensuite le nombre d'appelants de
`run_turn` : chacun devra fabriquer son invite par la porte, et chacun aura
entre-temps eu de bonnes raisons de composer sa chaîne lui-même. C'est la raison
d'écrire cet ADR **maintenant**, alors que le changement est encore mécanique.

Le retour en arrière, lui, est trivial dans tous les sens : aucune donnée
persistée, aucun format, aucune API publique hors du workspace.

## Condition de reconsidération

Trois faits rouvriraient la question, même si **C** était retenue :

* **un second appelant de `run_turn`** apparaît, quel qu'il soit ;
* quoi que ce soit d'autre que la saisie de l'utilisateur rejoint l'invite d'un
  agent — un nom de table, un extrait de schéma, un message d'erreur serveur ;
* le protocole ACP gagne un moyen de transmettre du contexte structuré, auquel
  cas la question ne portera plus sur une chaîne mais sur ce qu'on y met.

Le premier est vérifiable mécaniquement : `grep` sur `run_turn` doit rendre un
seul site d'appel hors tests. Si ce compte change et que cet ADR est resté en
**C**, il n'a plus de fondement.

**Le deuxième fait s'est produit le 2026-09-23**, et l'option **B** l'a absorbé
sans changer de forme : la structure de la base rejoint désormais l'invite qui
ouvre une session d'agent, par un second constructeur, `with_schema`, qui la fait
rendre par `ContextBuilder::build` sous le même niveau. `AgentPrompt` porte
l'`AgentContext` produit, pour que l'appelant dise ce qui est parti. Le reste de
la structure passe par l'outil `describe_schema`, commun aux deux destinations et
rendu par la même fonction. La décision et ses limites sont dans
[ADR-0030 § 4 bis](0030-outils-oxyn-exposes-a-un-agent-externe.md#4-bis-la-structure-de-la-base--un-outil-pour-toutes-les-destinations).

## Conséquences

Quelle que soit l'option retenue, une chose ne doit pas rester en l'état : un
document d'autorité affirmant une garantie du compilateur qui ne vaut que pour
une destination sur deux. C'est corrigé ; cet ADR existe pour que la correction
soit un choix consigné et non un oubli.

Si **A** ou **B** est retenue, le changement est mécanique et local : une
signature, un appelant, et les tests de `crates/oxyn-ai/src/external/tests.rs`.
Si **C** est retenue, il n'y a rien à écrire dans le code — seulement à ne pas
oublier pourquoi.
