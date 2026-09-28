---
name: query-desactivee-est-pending
description: TanStack Query v5 — an `enabled: false` query is `isPending` forever; "loading" is read with `fetchStatus !== "idle"`
metadata:
  type: feedback
---

A `useQuery` never enabled (`enabled: false`) stays `isPending === true`: `isPending` means "no data yet", not "in progress". Showing "Listing models…" on `isPending` alone shows it for an agent or a provider refused by the tier, which request nothing.

**Why:** encountered while wiring the model list state into the assistant's header (`use-provider-models.ts`, query disabled outside a usable provider).

**How to apply:** an "in progress" state is written `isPending && fetchStatus !== "idle"`; the error is read on `isError`, never inferred from the message.
