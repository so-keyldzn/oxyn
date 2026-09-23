---
name: query-desactivee-est-pending
description: TanStack Query v5 — une requête `enabled: false` est `isPending` pour toujours ; « chargement » se lit avec `fetchStatus !== "idle"`
metadata:
  type: feedback
---

Une `useQuery` jamais activée (`enabled: false`) reste `isPending === true` : `isPending` dit « pas encore de donnée », pas « en cours ». Afficher « Listing models… » sur `isPending` seul le montre pour un agent ou un fournisseur refusé par le niveau, qui ne demandent rien.

**Why:** rencontré en branchant l'état de la liste des modèles dans l'en-tête de l'assistant (`use-provider-models.ts`, requête désactivée hors fournisseur utilisable).

**How to apply:** un état « en cours » s'écrit `isPending && fetchStatus !== "idle"` ; l'erreur se lit sur `isError`, jamais en déduisant du message.
