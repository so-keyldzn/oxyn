# MCP servers

> **Authority**: which MCP servers the repository declares, why, and why the
> obvious candidates are rejected.

Configuration: [`.mcp.json`](../.mcp.json) at the root declares four servers.
Approval goes through `enabledMcpjsonServers`, and the committed
[`.claude/settings.json`](../.claude/settings.json) pre-approves **only
`fetch`**: it needs nothing but `uvx`. The other three are not pre-approved on
purpose — `figma` reaches a remote account service, and `figma-desktop` and
`semantiq` need a local application or binary a clone may not have — so a clone
approves them when it first uses them, or lists them in its own, untracked
`.claude/settings.local.json`. Codex declares the two Figma servers on its side
([.codex/README.md](../.codex/README.md#figma)).

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

### `figma`

| | |
|---|---|
| Origin | the Figma remote MCP service, `https://mcp.figma.com/mcp` (HTTP) |
| Launch | none: a remote endpoint, authorized in the Figma account; no credential is stored in the repository |

**Why it is not redundant.** The mockups the interface follows
([FIGMA-HANDOFF](FIGMA-HANDOFF.md)) live in Figma; nothing native to Claude Code
reads a frame, its measurements or its variables. It is the one declared server
that sends requests to an external service, which is why it is not pre-approved.

### `figma-desktop`

| | |
|---|---|
| Origin | the Dev Mode MCP server of Figma Desktop |
| Launch | none: `http://127.0.0.1:3845/mcp`, available only while Figma Desktop runs with its MCP server enabled |

**Why it is not redundant.** The same reading as `figma`, on the file open in the
local application, with no round trip to a remote service. The mockup tokens of
[RESEARCH-NOTES](RESEARCH-NOTES.md#figma-mockup-tokens) were read this way.
Both Figma servers are optional: nothing in the build or in the gates depends on
them.

### `semantiq`

| | |
|---|---|
| Origin | the `semantiq` code-search tool, installed on the machine |
| Launch | `semantiq serve --project .`; its index is the local `.semantiq.db*`, ignored by git and rebuilt on demand |

**Why it is not redundant.** `Grep` and `Glob` find a string; they do not find
"where is this behavior implemented" described in words, nor a symbol's callers,
hierarchy or impact. `semantiq` answers those from a local index, with no network
access. It is not conclusive: `find_refs` stops at its `limit` and counts a `use`
as a definition, and `deps` does not follow `pub use` re-exports, so "nothing
else uses X" is checked with grep. [CLAUDE.md](../CLAUDE.md#searching-the-code)
says when to start with it and when grep stays the tool.

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
2. **The need is covered without a boundary.** Drivers now exist
   ([IMPLEMENTATION-PLAN](IMPLEMENTATION-PLAN.md#phase-2--the-protocols-that-matter))
   and are tested against real servers in CI (`make test-serveurs`); a developer
   querying a local database does it with the client tools below.

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
