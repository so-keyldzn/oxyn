# ADR-0033 — Oxyn coupe Codex dans toutes les couches de configuration qu'il peut lire, et laisse à l'organisation celles qu'il ne peut pas lire

**Statut :** proposé · **Date :** 2026-09-23

**Précise :** [ADR-0032](0032-agent-externe-confine-au-lancement.md) sur un
point. L'ADR-0032 coupe par leur nom les serveurs MCP et les plugins déclarés
dans `~/.codex/config.toml`. Or Codex en charge aussi depuis d'autres couches.
Le reste de l'ADR-0032 reste en vigueur.

## Contexte

Codex 0.154.0, que lance `codex-acp` 1.12.0, fusionne sa configuration à partir
de neuf couches. La fusion se fait table par table, si bien que `mcp_servers` et
`plugins` réunissent les entrées de toutes les couches
([RESEARCH-NOTES](../RESEARCH-NOTES.md#les-couches-de-configuration-que-codex-charge--relu-le-2026-09-23),
relu au tag `rust-v0.154.0`). Ce qu'Oxyn écrit dans `CODEX_CONFIG` forme la
couche `SessionFlags`, de rang 7 :

| Couche | Rang | Déclare des serveurs | Oxyn la lit | Au-dessus d'Oxyn |
|---|---|---|---|---|
| `/etc/codex/config.toml` | 2 | oui | **oui**, depuis cet ADR | non |
| Fragments cloud de l'espace de travail | 3 | oui | **non** : le serveur les livre à Codex | non |
| `$CODEX_HOME/config.toml` | 4 | oui | oui (ADR-0032) | non |
| `.codex/` de projet | 6 | oui | sans objet : le répertoire de l'agent est neuf, vide et privé | non |
| `/etc/codex/managed_config.toml` | 8 | oui | **oui**, depuis cet ADR | **oui** |
| MDM macOS `com.openai.codex` | 9 | oui | **non** | **oui** |

Chaque couche qu'Oxyn ne lit pas peut déclarer un serveur MCP que l'agent
utilise sans rien demander, en contournant le `PolicyGate`. C'est l'écart que
l'ADR-0032 visait à fermer.

## Décision

1. **Oxyn lit les trois couches locales** : système, utilisateur et géré
   (`oxyn_ai::external::confine::codex_config_layers`). Il ne garde que les
   noms de `mcp_servers` et de `plugins`, et les coupe tous par
   `enabled = false`. Les règles de l'ADR-0032 valent pour chaque couche : un
   fichier illisible, spécial ou de plus de 1 Mio arrête le lancement. Au-delà
   de 256 noms par table, toutes couches confondues, aussi. Enfin, un serveur
   nommé `oxyn` est refusé.
2. **Une couche gérée qui écrit `enabled = true` l'emporte sur Oxyn.** Codex
   démarre quand même, puisque c'est la politique de l'organisation. En
   revanche, l'agent n'est plus présenté comme confiné : il perd le badge
   « Restricted by Oxyn » et l'écran l'avertit comme un agent qu'Oxyn ne
   confine pas (`managed_turns_on`).
3. **Oxyn ne lit ni la couche MDM ni les fragments cloud, et l'assume.** Lire
   le MDM demanderait de lancer un processus (`defaults`), ce que le dépôt
   refuse hors du seul point de lancement, ou de lier `CoreFoundation`. Les
   fragments cloud, eux, ne sont lisibles nulle part en local. Ces couches
   appartiennent à l'administrateur de l'organisation, pas à l'utilisateur, et
   leurs serveurs sont ceux que l'organisation a choisis. Pour Codex, l'écran
   ajoute : « except what your organization's managed configuration turns on ».

## Conséquences

* **+** Un serveur MCP déclaré dans `/etc/codex/config.toml` ou dans le
  fichier géré est coupé comme ceux de `~/.codex`. L'écart de l'ADR-0032 se
  réduit aux couches qui relèvent de l'organisation.
* **+** Oxyn ne présente plus comme confiné un Codex qu'une politique gérée
  ouvre. Le badge dit ce qui est vrai.
* **−** Un serveur déclaré par MDM ou par le cloud de l'organisation reste
  disponible à l'agent, **sans aucune indication**, puisque Oxyn ne le voit
  pas. L'écran le dit en général, pas pour ce serveur-là.
* **−** Lister les agents lit désormais deux fichiers de `/etc` pour chaque
  Codex déclaré, sur le pool bloquant (I-05).
* **−** Un `/etc/codex/config.toml` illisible pour l'utilisateur, avec des
  droits `0600 root` par exemple, empêche Codex de démarrer depuis Oxyn, alors
  qu'il démarrerait dans un terminal. Ce refus est voulu : un fichier illisible
  ne permet pas de couper ce qu'il déclare.

**Coût de sortie :** faible. `codex_layers.rs` est un module d'environ deux
cents lignes, et le champ `confined` existe déjà côté IPC.

**Reconsidérer si** une version épinglée de Codex ou de `codex-acp` offre un
interrupteur qui coupe tous les serveurs MCP sauf ceux de la session. Il
remplacerait toute cette lecture. À reconsidérer aussi si l'on constate que
des organisations poussent par MDM des serveurs que leurs utilisateurs
refusent.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Refuser de lancer Codex dès qu'une couche système ou gérée déclare un serveur | Cela punit l'utilisateur d'une politique qu'il ne contrôle pas, sans rien protéger de plus que la coupure par nom. |
| Seulement documenter l'écart | Deux couches sont lisibles en quelques lignes. Laisser leurs serveurs actifs sous un badge « Restricted by Oxyn » ferait mentir le badge. |
| Lire le MDM avec `defaults read com.openai.codex` | Cela lance un processus, que le dépôt interdit hors du seul point de lancement, parce qu'un enfant hérite de l'environnement (I-03). Lier `CoreFoundation` pour un seul appel serait une dépendance plateforme de plus pour une couche qui appartient de toute façon à l'organisation. |
| Retirer le badge à tout Codex | Oxyn confine effectivement tout ce qu'il peut lire. Le retirer à tout Codex rendrait l'avertissement si fréquent qu'il ne signalerait plus rien. |
