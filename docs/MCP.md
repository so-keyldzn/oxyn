# MCP servers

> **Authority**: which MCP servers the repository declares, why, and why the
> obvious candidates are rejected.

Configuration: [`.mcp.json`](../.mcp.json) at the root, approval through
`enabledMcpjsonServers` in [`.claude/settings.json`](../.claude/settings.json)
— in the **committed** file, so that a clone has nothing to re-approve.

## The criterion

An MCP server gets in here when it brings something Claude Code cannot already
do natively. A redundant server costs twice: it consumes context with its tool
definitions, and it creates a second path to do the same thing — hence
uncertainty about which one is used.

## What is declared

### `fetch`

| | |
|---|---|
| Origin | official reference server, maintained ([RESEARCH-NOTES](RESEARCH-NOTES.md#mcp-ecosystem)) |
| Launch | `uvx mcp-server-fetch` |

**Why it is not redundant with `WebFetch`.** `WebFetch` converts the page then
has a small model summarize it: you get an answer to a question, not the text.
To read the exact signature of a `tauri` function on docs.rs or the exact value
of a parameter in the PostgreSQL documentation, that summary is a loss: it is
precisely the kind of approximation [I-12](../CLAUDE.md#i-12) forbids. `fetch`
returns the content.

`WebFetch` remains preferable to skim a long page from which you only want an
answer.

## What is rejected, and why

Documented so that the question is not asked again every six months.

| Candidate | Rejected because |
|---|---|
| `filesystem` | redundant with `Read`, `Write`, `Glob`, `Grep`, which in addition honor the repository's `permissions.deny` rules — which the server would not |
| `git` | redundant with `Bash(git …)`, already allowed for reading in the permissions |
| `memory` | would create a **competing source of truth** to `docs/`. That is exactly what the foundation tries to avoid: a rule lives in a single place |
| `sequential-thinking` | redundant with the model's native reasoning |
| `time` | no need in this repository |
| `everything` | demo server |
| **`postgres`, `sqlite`, `github`** | **archived reference servers** ([RESEARCH-NOTES](RESEARCH-NOTES.md#mcp-ecosystem), checked 2026-09-05). There is no official database MCP server |

## The case of database servers

It is the most visible gap for a project like Oxyn: being able to query a real
database while developing a driver.

It is not filled, for two reasons:

1. **No official server exists** — those of the reference repository are
   archived. Every candidate is a third-party server, to be audited before being
   plugged into a database.
2. **There is no driver to test yet.** The need is real from
   phase 2 ([IMPLEMENTATION-PLAN](IMPLEMENTATION-PLAN.md#phase-2--the-protocols-that-matter)),
   not before.

When the need arises, the decision will go through an ADR, and the criterion
will be the access surface: an MCP server that receives a production connection
string is one more external boundary, in the sense of
[ARCHITECTURE](ARCHITECTURE.md#les-frontières-externes). In the meantime, `psql`,
`mysql` and `sqlite3` through `Bash` on local databases cover the need without
adding a boundary.

## Servers requiring authorization

Some platform servers (GitHub, Linear, Slack, Notion…) are available but **not
authorized** in this session. They are not declared in `.mcp.json`: they are
account connectors, not a repository configuration. They are authorized in the
claude.ai account settings, or through `claude mcp` in an interactive session.
