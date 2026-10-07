# ADR-0055 — The SQL agent is the default of a new conversation, whatever the display order

**Status:** accepted (2026-10-07) · **Date:** 2026-10-07 ·
**Deciders:** Nicolas Boromée

**Refines:** [ADR-0049](0049-agents-declared-as-markdown-files.md), § 2 and
§ 4, on one point: which agent a new conversation runs when the user picks
none. Targeting, the picker's order and everything else in ADR-0049 remain
in force.

## Context

ADR-0049 § 2 says: "the default is the SQL agent when it is offered,
otherwise the first offered one in the order of § 4", and § 4 orders the
picker "shipped, then user, then plugin, each by `name`". With the two
shipped agents, `Schema` sorts before `SQL`.

The code shipped in 0.0.6 (#200, #203) does not follow the display order:

- the backend runs the SQL agent for a conversation that names no agent
  (`requested_agent(None)` in `crates/oxyn-desktop/src/backend/ai/agents.rs`);
- the picker (`assistant-agent-picker.tsx`) shows `SQL_AGENT_ID` selected
  when the thread has no `agentId`, while listing `Schema` first.

A Codex review on #200 and #202 read § 2 as "the first agent by name" and
flagged the gap. The user decided on 2026-10-07 that SQL stays the default.

## Decision

1. A new conversation, and a blank thread sent without `agentId`, runs the
   **SQL agent** (`sql_agent()`, id `0199a3c0-0000-7000-8000-000000000001`),
   whatever the order the picker displays.
2. The picker shows the SQL agent **selected** in a blank thread. The list
   keeps the order of ADR-0049 § 4: display order and default are separate.
3. The "otherwise the first offered one in the order of § 4" fallback of
   ADR-0049 § 2 no longer decides the default: the default never follows the
   display order. A conversation that recorded an agent keeps it
   (ADR-0049 § 6); one whose agent is gone falls back to SQL, as before.

## Consequences

* **+** The default is the general-purpose role the assistant panel was
  built around: writing, fixing and explaining queries on the open
  connection.
* **+** Adding an agent — shipped, user or plugin — never changes what a new
  conversation runs. With an alphabetical default, a user file named
  `Analytics` would silently take over every new thread.
* **+** Backend and front agree on one constant, already shipped in 0.0.6:
  no migration, no change of stored data.
* **−** The selected agent is not the first line of the list, which can
  surprise a user who expects the first option to be the default.
* **−** The SQL agent is special-cased by id. Today it is offered for every
  target (`agents/sql.md` has empty `applies_to` and `recipients`, and a user
  file cannot take its id). Should a later edit narrow it, a new
  conversation on a target it is not offered for would be refused
  (`not_offered`) instead of falling back to another agent.

**Exit cost:** low. One function on the backend (`requested_agent`) and one
selection in the picker; no persisted format depends on it.

**Reconsider if** the user can choose their own default agent, or if a
shipped agent other than SQL becomes the panel's primary role.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| The first offered agent by name, as ADR-0049 § 2 reads | A display concern would decide behavior; every new agent could change the default without anyone choosing it |
| Sort SQL first in the picker so that "first offered" and "default" coincide | Breaks the order of ADR-0049 § 4 for one entry, and still ties the default to the display |
| Amend ADR-0049 in place | ADR-0049 was accepted on 2026-10-06; an accepted ADR is not rewritten (`.claude/rules/documentation.md`) |
