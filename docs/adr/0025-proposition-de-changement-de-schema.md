# ADR-0025 — A schema change proposal is SQL to review, never a write

**Status:** proposed · **Date:** 2026-09-13

**Clarifies:** [ADR-0018](0018-apercu-ddl.md), on what one is allowed to do
with DDL once it is read.

## Context

Board `229:7637` carries a `Propose change…` control (`229:7663`, 164 px, in
the structure bar, to the right of `Refresh structure` and `Copy DDL`). The
plan had so far classified it as a pending "product decision".

**That classification was wrong, and the board says so itself.** Its
definition panel (`229:7749`) carries, under the DDL and above the
`Open DDL in console` button, this sentence:

> Changes require a SQL review naming commerce-prod before execution.

It is word for word the guarantee of [I-02](../../CLAUDE.md#i-02): a review that
**names the connection**, **before** execution. The mockup therefore does not
ask to choose between "apply" and "propose". It has already decided: the
control proposes, and execution remains a separate, reviewed action.

Three earlier decisions frame the rest, and none is to be reopened:

* [ADR-0018](0018-apercu-ddl.md) already separates **reading** DDL
  from executing it, and `Open DDL in console` is its implemented precedent:
  text goes to the console, nothing executes;
* [I-01](../../CLAUDE.md#i-01) forbids a second execution path. A control
  that applied a DDL directly would create one, and that is the one the AI
  would take;
* [I-10](../../CLAUDE.md#i-10) forbids concatenating a **received** identifier.
  A modification DDL is made almost entirely of them: schema, table, column,
  constraint names — all come from the catalog, hence from the server.

The numeric constraint that makes the last point concrete: `public.customers`
in the mockup has 8 columns, and a table named
`"users"; DROP TABLE audit; --` is legal in PostgreSQL.

## Decision

`Propose change…` **composes text and opens it in a console. It executes
nothing, and takes no new path.**

The action takes the path `Open DDL in console` already takes —
`open_library_query` with `library::OpenQuery::Copy { text, title, origin,
provenance }` (`workspace/definition.rs`, `open_definition_console`) — and nothing else. From there,
the proposal is ordinary user SQL: it goes through `oxyn-query` for its
classification, the `PolicyGate` for its authorization, and the existing
production approval if the connection is one. **No new command variant**, no
bypass of the bus.

Only `origin` changes: it says the text is a change template to complete, not
a definition that was read.

`provenance` stays `None`, and this version of the ADR corrects a first draft
that claimed the opposite. The repository had already settled the question for
the twin case — the bound query template of `metadata.rs` — with the reason
that holds here word for word: **a template composed by Oxyn is written neither
by the user nor by an agent.** Provenance marks who wrote a text
([ADR-0023](0023-fournisseurs-declares-et-provenance.md)); composing a skeleton
to complete is not writing. Deciding otherwise would have introduced two
different behaviors for two neighboring templates, which is precisely the kind
of inconsistency an ADR must avoid rather than create.

Each identifier the proposal inserts is quoted by
`oxyn_catalog::quote_identifier` with `QuoteStyle::for_dialect`, never
concatenated. **Values** — a default, a constraint expression — cannot be bound
in a DDL: they are therefore taken **verbatim from the catalog**, without
reformatting. Rewriting an expression the server returned changes its meaning
without saying so.

A first draft added "or the proposal is refused". That refusal did not exist in
the code, and it no longer has a reason to: the whole template being commented
out (see just below), an exotic expression is copied without being able to
trigger anything.

For an `Actor::Agent`, `Propose change…` is **unavailable**, not
"confirmable". It is [I-02](../../CLAUDE.md#i-02) to the letter: for an agent, it
is a refusal, not a stronger confirmation. An agent that wants a schema change
writes SQL in a console like anyone, and that SQL is reviewed.

This unavailability holds by **absence of a path** — no tool of the registry
reaches the action —, which is stronger than a refusal. But nothing kept it
true: `schema_change_is_not_a_tool`
(`oxyn-ai/src/tools.rs`) now takes care of it, and fails the day a structure
tool appears.

**The whole template is commented out, line by line** — not only its header.
It is the property that makes the action safe, and it deserves to be stated
here: `--` only comments until the next line break, and a column name or a
default expression can contain one. The body is therefore composed bare, then
each physical line receives its prefix. The user **uncomments** the statement
they want; nothing goes out on a distracted `Run`.

The composed text **also** carries a SQL comment at the top, naming the
connection and the source relation. `origin` and `provenance` are metadata of
the tab: they do not survive a copy-paste into a ticket or a message, and that
is precisely where the proposal will be reviewed three days later, by someone
else.

The scope of the first version is what the board shows and no more: rename a
column, change its nullability, change its default, drop a named constraint.
**No** `DROP TABLE`, no type change, no data migration — a type change rewrites
the table and can fail midway on real data, which is a migration topic, not a
structure panel one.

## Consequences

* **+** No new execution path: the proposal is SQL, and everything that
  protects SQL already protects it. I-01 held by construction rather than by
  vigilance.
* **+** The guarantee the mockup writes — "a SQL review naming commerce-prod
  before execution" — is held literally, since the review is the one that
  exists.
* **+** The user sees the exact SQL before it goes out. A panel that applied a
  change while showing a summary would require trusting the translation; here
  there is no translation to believe.
* **−** Two actions instead of one: propose, then execute. On an obvious column
  rename, it will feel heavy, and it will be.
* **−** The composed SQL can be **edited** before execution, including into
  something the panel would never have proposed. It is the accepted consequence
  of treating it as user SQL; the alternative — locked text — would recreate
  the second path we refuse.
* **−** The restricted scope will leave out the type change, which is precisely
  what is most often asked for after a rename.

**Exit cost:** low as long as the decision holds. The composer is a pure
function — `proposed_change(cache, path, tab, index, dialect, connection)`,
which **enumerates** the legal operations for the dialect rather than receiving
one: the user chooses in the text. Testable without a database and without an
interface; leaving it would require adding an execution command, hence
reopening I-01 — which is the real cost, and it is deliberately placed there.

**Reconsider if** a professional user reports that the double step pushes them
to write their DDL by hand without reviewing what the panel proposed: the
protection would then be bypassed by its own weight, which is worse than not
having it.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Apply the change from the panel, behind a confirmation | Creates the second execution path [I-01](../../CLAUDE.md#i-01) forbids, and a confirmation ends up being clicked — it is the reasoning of [I-02](../../CLAUDE.md#i-02) on agents, which also holds for humans in a hurry |
| Open a rich modification form (type, constraints, column order) | The real scope of such a form is a migration. It fails midway on real data, and a structure panel has nowhere to say so |
| Compose the DDL on the driver side rather than in the core | The driver already quotes identifiers; also entrusting it with the **shape** of the change would duplicate the logic in each driver, and ADR-0003 wants one driver per protocol, not one DDL generator per product |
| Lock the proposed SQL to prevent editing it | Would make the console inconsistent with itself — a tab whose text cannot be edited — and would prevent nothing: the user retypes the SQL next to it |
| Let the agent propose, with a stronger human confirmation | [I-02](../../CLAUDE.md#i-02) is explicit: for an `Actor::Agent`, it is a refusal. A stronger confirmation is exactly what the invariant names as insufficient |
