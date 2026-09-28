# ADR-0006 — AI privacy tiers, per connection

**Status:** accepted · **Date:** 2026-09-05

## Context
"Privacy first" and "AI when it adds value" pull against each other as soon as a
schema or rows are sent to a cloud API. A global setting is too coarse: the same
person may want a cloud model on their dev database and nothing at all on production.

## Decision
Three tiers, chosen **per connection**, with `Metadata` by default:

| Tier | What leaves the machine |
|---|---|
| `Local` | Nothing — local model only |
| `Metadata` *(default)* | DDL, names, types, indexes, cardinalities, execution plans |
| `Sampled` | + a sample of rows explicitly approved, column by column |

No provider is required: without configuration, the AI workspace is absent from the UI
and Oxyn remains a complete client.

## Consequences
* **+** The default is safe; sending data values is a deliberate act.
* **+** Compatible with regulated environments without special configuration.
* **−** Some features (duplicate detection, value inconsistencies) are degraded in
  `Metadata`: the UI must say so, not hide it.
* **−** Context compaction (pruning irrelevant tables on a 5,000-table database)
  becomes a component in its own right, not a detail.
