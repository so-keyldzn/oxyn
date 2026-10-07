# Security

> **Authority**: handling of secrets, marking of connections, attack
> surface, `unsafe` policy.

Invariants involved: [I-02](../CLAUDE.md#i-02), [I-03](../CLAUDE.md#i-03),
[I-09](../CLAUDE.md#i-09). The authorization policy itself is settled by
[ADR-0004](adr/0004-command-bus.md) and is not copied here.

## Reporting a vulnerability

Never publish a secret, a proof of exploitation or user data
in an issue or a pull request. Use the repository's private
[GitHub Security Advisories](https://github.com/so-keyldzn/oxyn/security/advisories/new)
form. Describe the affected version or commit, the minimal scenario, the
estimated severity and the reproduction steps, after removing the secrets.

The maintainer acknowledges receipt through the private channel and coordinates the fix,
the validation and the possible publication of an advisory. If the private form
is not available, contact the maintainer through their GitHub profile and state
only that the report concerns security; do not send the
sensitive details in a public message.

Oxyn's threat model is not a server's. The attacker is not
a stranger on the Internet: it is **the data the user opens** and
**the mistakes Oxyn lets them make**. A database client runs
with an administrator's privileges on production systems.

## Secrets

### What never touches the disk in clear

Passwords, full connection strings, AI provider API tokens,
SSH tunnel private keys, client certificates.

Storage goes through the system keychain. What is persisted in the
workspace is a **reference** to the secret, never the secret.

SQL administration statements are the exceptional input surface where a
password is part of the statement text itself. Before a new statement reaches
query history, the append-only audit journal, or a conversation tool-call
record, Oxyn lexically replaces recognized password literals with
`'<redacted>'`; saved queries included in AI context receive the same redaction
before clipping or rendering. The driver still receives the original text.
This covers explicit password clauses, not arbitrary secrets in comments,
dynamic SQL, editor drafts, or free-form conversation text. Rows already present in
`audit_journal` are not rewritten because the journal remains strictly
append-only. A user who ran such SQL before this protection must rotate the
credential and protect or replace affected workspace files
([ADR-0053](adr/0053-redact-sql-passwords-before-persistence.md)).

**Concrete failure:** a workspace file containing a production password,
committed by the user to their team's repository, because the
file looked like mere configuration.

### External-agent environment

Secret environment values go to the OS keychain, under a fresh
`oxyn:agent-env:<random identifier>` reference for each value and each save.
Rust forces `*_API_KEY`, `*_TOKEN`, `*_SECRET`, names containing `PASSWORD`,
and the bare names `API_KEY`, `TOKEN`, `SECRET` into the keychain, ignoring
case. The user can mark any other variable secret. Other values stay in clear.
`external_agents.env` is readable JSON: `plain` holds name/value pairs and
`secret_refs` holds name/reference pairs. References never return to the webview.

Values are resolved on the blocking pool only while preparing a launch, after
the privacy check, and injected into the child's cleared environment. Missing
entries or keychain errors abort launch; no plaintext fallback exists. The
agent receives these credentials and may use them to authenticate; they never
join an AI prompt. Debug and child error reports redact them, including short
explicitly secret values.

Replacing a declaration writes fresh entries before saving through the bus;
only a successful save or deletion permits forgetting the old entries. A
failure can leave unreachable keychain entries, never overwrite an old value.
Legacy JSON arrays remain readable: the first list moves token-like names to the
keychain on the blocking pool before offering any agent. A failure aborts listing. This
upgrade changes live rows; historical SQLite pages, WAL files, backups and
copies from older releases may still contain the old value. Rotate previously
stored tokens to invalidate those copies. Unknown sensitive names in legacy
rows must be marked secret by replacing their declaration.

### The updater signing key

The minisign private key that signs updates is a secret of the **release**,
not of a user: it lives only in the `TAURI_SIGNING_PRIVATE_KEY` and
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` secrets of the `release` GitHub
environment, which only `v*` tags may deploy to — a workflow pushed on a
branch cannot read them —, and in two offline backups. Within a release run
it reaches two steps: the check that it is present, and the signing step,
which runs the pinned `tauri signer sign` alone. **The build never holds
it**: `tauri build` runs every `build.rs`, proc-macro and Vite plugin of the
dependency graph, and one compromised dependency version could read its
environment. It is never in the repository, a log, a chat or the clipboard;
scripts name the variables, never their values, and do not relay the signing
CLI's output. Only its public half is committed, in `tauri.conf.json`; until
it is, the placeholder there fails releases closed — the workflow refuses it
before building and before writing a manifest. Procedure, rotation and loss:
[RELEASE](RELEASE.md#updater-signing-key).

**Concrete failure:** the key pasted into a chat to "set up the secret", and
anyone who reads that log can sign an update every installed copy accepts.

### A secret does not follow its connection elsewhere

A secret entered for one destination is never presented to another one that
the user did not choose when entering it. When an edit changes
**a non-secret parameter declared by the driver** — host, port, database, SQLite
file, but also user, TLS mode or application name —, the saved
connection no longer references the keychain entry of the previous destination.
What was re-entered in the same edit is written to a **new**
entry, under a reference that no configuration carried before; without
re-entry, the connection no longer has a secret. The old entry is then
**forgotten** from the keychain. The name, the environment, the AI tier and the read-only
flag do not change the destination, and do not touch the secret. Values
are compared after trimming surrounding whitespace, as they are saved.

The new entry is what makes the rule exceptionless. The configuration is
saved **before** the keychain write — an edit held by the
policy then refused must change nothing —: a reference derived from the
identifier alone would, in the meantime, name the entry that still carries the old
password, and forever if the process stops between the two. With
a new reference, a failed write leaves the connection without a secret, and a
failed forget leaves an entry that nothing references any more; neither
presents the old secret to the new destination.

The form says so before saving
([UX-SPEC](UX-SPEC.md#editing-a-saved-connection)). The Test action
is only offered to a new draft, under a new identifier: it never reaches
the secret of a saved connection. AI providers follow the same rule
for their API key, when the family, scheme, host, port,
path or query of the base URL changes.

Every parameter rather than an "address": a password sent under another
role, or under `sslmode=disable` where it used to go under `verify-full`, leaks just
as much; when in doubt, forgetting only costs a re-entry.

**Concrete failure:** an address pasted from a message, or a typo
in the host, and the production password goes off to authenticate against a
third-party server on the first click, without the user having re-entered it.

### What never leaves a process

The six channels, and all six must be handled — forgetting one is enough.

| Channel | The trap |
|---|---|
| `tracing` logs | a `Debug` derived on a connection structure prints everything |
| Displayed error messages | `sqlx` sometimes includes the connection URL in its error |
| Crash reports | a stack trace captures local variables |
| Session and workspace files | persistence "to restore the state" |
| Prompts sent to AI providers | [AI-PROVIDERS](AI-PROVIDERS.md) |
| Clipboard, export, screenshot | sharing features copy what is displayed |

Practical consequence: **no `#[derive(Debug)]` on a type that carries a
secret.** The implementation is manual and redacts the value. A derived `Debug`
is the most frequent leak because it is invisible in review —
it is the `tracing::debug!("{cfg:?}")` added six months later that leaks.

### What goes out to an AI recipient leaves a trace

**What goes out to an AI recipient is logged**, in `ai_egress`, an append-only
local table protected like `audit_journal` and never pruned: the
connection, the source, the **names** of the columns sent, the number of rows,
the recipient (provider or agent, model, scope `local`, `remote` or
`unresolved` measured for that send), the command that read the data and the
conversation. **Never a value nor a token**: there is no column to
store them, and the file refuses a column list that would contain anything
other than names. The entry is written **before** the send; if it fails, nothing
goes out.

## Connection transport

The shared PostgreSQL and MySQL TLS policy is defined in
[ADR-0052](adr/0052-verified-tls-outside-local.md). It governs driver configuration,
connection-form defaults and the explicit Local exception.

## Connection marking

Every connection carries an environment: `local`, `development`, `staging`,
`production`. The default, when it is not set, is **`production`** —
the most restrictive value, not the most permissive.

**Concrete failure:** the opposite default. A user adds a connection in a
hurry without filling the field, the program assumes "development", and an `UPDATE`
without `WHERE` goes out without confirmation on the customer database.

On a `production` connection: every write, every DDL, every destructive
operation requires an explicit confirmation that **names the connection**, and
the interface carries a permanent marker. See [I-02](../CLAUDE.md#i-02). This
confirmation is a native dialog, not a webview button
([Input surface](#input-surface), item 5).

A **read** goes through without confirmation, and that is what makes it dangerous: a
`SELECT` that calls a `VOLATILE` function, or that reads a view that does,
writes, and no analysis of the text can see it. On a
`production` connection, `oxyn-exec` therefore bounds every execution classified as a read to
read-only, whatever bounds the caller requested, and the
driver has the server refuse the write (PostgreSQL: `READ ONLY`
transaction; SQLite: `sqlite3_stmt_readonly`). Such a function can only be called
in production in a form the classification does not read as a
read — `CALL`, a `DO` block —, hence after the confirmation that names the
connection. The bound does not confine everything: a PostgreSQL `READ ONLY` transaction
still writes to a temporary table, and holds back nothing that
leaves the transaction — `dblink_exec` to another connection,
`pg_terminate_backend`, `set_config`, advisory locks.

A request made **only** of bare transaction verbs — `START TRANSACTION`,
`BEGIN`, `COMMIT`, `ROLLBACK`, `SAVEPOINT`, `RELEASE`, each parsed as such,
never a `BEGIN … END` block — is not a read, and is not bounded: it calls no
function, and MySQL cannot apply the bound inside an open transaction, so a
bounded `COMMIT` or `ROLLBACK` was refused and the transaction could never be
settled. Settling a transaction the user opened is not a new write: the writes
it holds were each confirmed under the connection's name. A verb followed or
preceded by any other statement keeps the bound.

**Concrete failure:** `SELECT public.audit_touch()` in a production
console, where the function inserts a row: the write went out without
confirmation, classified as a read.

For an `Actor::Agent`, a `production` connection is **strictly
read-only** — it is not a stronger confirmation, it is a refusal
([ADR-0004](adr/0004-command-bus.md#default-policy)). The difference
matters: a confirmation ends up being clicked.

An `Actor::Agent` does not drive a transaction either, whatever the
environment: `BEGIN`, `COMMIT`, `ROLLBACK`, `SAVEPOINT`, `RELEASE` and
their synonyms (`END`, `ABORT`, `PREPARE TRANSACTION`…) are refused to it. These
statements neither read nor write by themselves, but they commit or
roll back what the session holds — on a shared session, the user's
writes. The refusal does not depend on the transaction state: the `PolicyGate`
does not consult it ([ADR-0039](adr/0039-etat-de-transaction-d-une-session.md)).

This refusal has two known limits. It only reads SQL: a language that
`oxyn-query` does not parse is `Unknown`, subject to approval outside production,
and the first non-SQL driver will have to learn its own transaction verbs.
And it only covers **explicit** ends: under SQLite, an interrupted write
(Stop, timeout) or one that hit `SQLITE_FULL`, `SQLITE_IOERR`
or `SQLITE_BUSY` can roll back the whole open transaction — an approved agent
write on a shared session can therefore too. The assistant
panel currently opens its own session, separate from the consoles.

## Input surface

What enters Oxyn and is untrusted, in order of underestimation:

1. **Database server responses.** See
   [DRIVER-CONTRACT](DRIVER-CONTRACT.md). A compromised or merely
   unusual server returns whatever it wants.
2. **Catalog object names.** A table, a column, a comment
   can contain any byte, including SQL, terminal control
   sequences, or text imitating an instruction. A column name
   is never interpolated into a query without quoting, and **never treated
   as an instruction** when it is attached to an AI prompt.
3. **Workspace files.** They may have been written by a third party, or
   by a future version of the program.

   **User agent files** belong here
   ([ADR-0049](adr/0049-agents-declared-as-markdown-files.md), accepted):
   `<name>.md` in `agents/` of the data directory, read at launch and on
   reload. A file someone else wrote can carry a role prompt that tells the
   model to insist, to hide what it does or to ignore the user. What bounds
   it: the file is refused above 64 KiB, outside UTF-8, with a YAML that uses
   anchors, aliases, merge keys, duplicate keys, unsupported tags or an
   unknown field, with a tool the registry does not have, or with a
   placeholder outside the four closed variables; its `id` cannot take a
   shipped agent's audit identity; it supplies a **role** only and never
   replaces the dialect or recipient fragments Oxyn ships; an error names the
   file and the line, never its content ([I-03](../CLAUDE.md#i-03)). Reading
   the directory is bounded too: only the `*.md` files directly in it, at
   most 64 of them in name order (the rest listed as one error), regular
   files only — a symbolic link is refused, not followed —, and the 64 KiB
   cap is checked on the size and again while reading. `agents/` itself must
   be a directory, not a link to one. On macOS and Linux it is opened once
   without following a link and every file is opened relative to it, without
   following a link nor waiting on a FIFO, so an entry swapped after the
   listing gains nothing; no Windows build ships, and a port has to keep
   that property. The webview names a connection when it asks for a reload,
   never a path. A file name is shown with its control and bidirectional
   characters replaced. What it
   can never obtain: a tool, a tier, a connection, an endpoint, a key or a
   write the `PolicyGate` would refuse — targeting narrows, it grants
   nothing. The panel marks it as a user agent, and the step that reads
   these files is reviewed for security before it merges.
4. **Plugins.** Third-party code, run in a WASM sandbox
   ([ADR-0005](adr/0005-wasm-plugins.md)). The sandbox bounds the damage; it
   does not exempt from entrusting it with nothing. See
   [PLUGIN-CONTRACT](PLUGIN-CONTRACT.md).
5. **The webview.** A cell value, an object name or a model response
   rendered in the DOM are the first vector of an XSS, and an XSS in the
   webview reaches the Tauri commands ([ADR-0029](adr/0029-interface-tauri-shadcn.md)).
   Hence: text-only rendering — never `dangerouslySetInnerHTML` on
   received data —, a strict CSP in `crates/oxyn-desktop/tauri.conf.json`,
   minimal *capabilities* in `capabilities/main.json`, and an IPC surface
   in which every command is reviewed as a security change.

   **Several windows do not make several surfaces**
   ([ADR-0043](adr/0043-multi-fenetre.md)). The capability covers windows
   through the `workspace-*` pattern, with the same three permissions, none of which
   creates a window or a webview. A window only opens through
   `open_window`, in Rust, bounded to 16. The identity of a window comes from the
   `Webview` Tauri provides to the command, never from a label sent by
   the JavaScript. Every command that targets a console, a session, a
   command, a result, a document or a connection's assistant refuses what
   another window owns (`backend/windows.rs`). Nothing is broadcast:
   each window has its `Channel`s, filtered in Rust, and `clippy.toml` forbids
   the emit methods of `tauri::Emitter`. What an XSS in a window
   gets out of it: opening windows up to the bound, closing its own, holding
   or cancelling its own closing, rewriting the list of its own consoles
   for the next launch (`report_window_consoles`, 256 at most, never
   a document another window writes), moving one of its own
   consoles to a new window (`open_in_new_window`, which takes neither a
   label nor a target window). It reaches nothing of another window.
   One caveat, which comes down to a secret: Tauri's `start_dragging` and
   `internal_toggle_maximize` accept the label of another
   window (`tauri` 2.11.5, `src/window/plugin.rs`, `get_window`). No
   command therefore returns to the front end the label of a window, not even its
   own; a command that did would give an XSS the means to move or
   maximize the other windows.

   **A confirmation drawn in the webview does not withstand a script running
   in it**: the script calls the command the button would have called. Critical
   decisions are therefore confirmed in a **native host dialog**,
   composed by the backend, whose dismissal counts as a refusal. What is critical,
   what the dialog says and how it is tested live in
   [ADR-0037](adr/0037-dialogue-natif-pour-les-confirmations-critiques.md), and
   nowhere else; everything else keeps its confirmation in the webview.

   `style-src` keeps `'self' 'unsafe-inline'` there: components insert a
   `<style>` at runtime, into the webview's DOM, whereas Tauri only puts
   a `nonce` on the `<style>` elements already present, as text, in the static
   HTML of the build — and that file contains none (checked in the
   source of `tauri-codegen` 2.6.3 and `tauri-utils` 2.9.3, the versions in
   `Cargo.lock`, on 2026-09-24 — [I-12](../CLAUDE.md#i-12)). Depending on it:

   - `ChartStyle` (`apps/desktop/src/components/ui/chart.tsx`);
   - the CodeMirror SQL editor (`sql-editor.tsx`), through `style-mod` 4.1.3;
   - Base UI 1.8.0's `ScrollArea` and `Select` (`scroll-area.tsx`,
     `select.tsx`), which hide the scrollbar with an injected
     `<style>`;
   - mermaid rendering (`mermaid-render.ts`, mermaid 11.17.2): `render()`
     without a container puts its diagram (and a theme `<style>`) into
     `document.body` long enough to measure the labels, before serializing the
     SVG and removing that element — checked in the installed source,
     `mermaid.core.mjs` (functions `render`, `appendDivSvgG`), on 2026-09-24.

   What the CSP bounds around it: `script-src` stays `'self'`, `img-src`,
   `font-src` and `connect-src` are closed — an injected CSS rule cannot
   exfiltrate anything. The content of `ChartStyle` is never received data:
   the keys (`s0`, `s1`…) come from Oxyn, the colors from a fixed palette
   (comment in `assistant-result-chart.tsx`) — that is what makes it
   compatible with the rule of item 5 above.

   **What would cancel it without a visible error:** a `<style>` appearing one
   day in the build's HTML — Tauri would then put a `nonce` on it, which
   makes `'unsafe-inline'` inoperative for the browser (same mechanism as
   for `script-src` in development, see
   [RESEARCH-NOTES](RESEARCH-NOTES.md)), and these components would lose their
   styles without any error reporting it — for mermaid, a diagram
   measured with the wrong font but drawn with the right one, hence
   labels overflowing their boxes, without an error either.

   **Removal condition:** only when each of these `<style>` elements receives a
   `nonce` passed to the front end (`EditorView.cspNonce` for CodeMirror,
   `CSPProvider` for Base UI) or disappears (`disableStyleElements` for Base
   UI, CSS variables set through `style={{}}` — CSSOM, not affected by
   `style-src` — for `ChartStyle`) — and, for mermaid, only when a
   future version accepts a `nonce` on its theme `<style>`, or renders outside
   the webview's main document.
6. **AI provider responses.** They are proposals, not
   orders: they go through the `PolicyGate` like any other command
   ([ADR-0004](adr/0004-command-bus.md)). See [I-07](../CLAUDE.md#i-07).
7. **Files dropped on the window.** A path received from the system is what
   the user dragged — or what a page open elsewhere put into
   the drag. It is classified in Rust (`crates/oxyn-desktop/src/file_drop.rs`):
   a symbolic link is refused, not followed, because the extension of its name
   says nothing of the targeted file; a `.sql` is read bounded to 4 MiB, as strict
   UTF-8, and opened in a console without being executed; a database file
   is only **offered** as a connection, in `production`, created only if
   the user validates it. The `subscribe_file_drops` command takes no
   path: the webview cannot request reading a file of its
   choice, and the front end only receives a text or a form value.
8. **The update manifest and archive.** What
   `https://github.com/so-keyldzn/oxyn/releases/latest/download/latest.json`
   returns, and the archive it points to, become code run with the user's
   rights ([ADR-0051](adr/0051-automatic-updates-from-github-releases.md)).
   The endpoint is a constant compiled into `oxyn-desktop`
   (`updates/channel.rs`), fetched over HTTPS only — the plugin refuses
   anything else in a release build. That check covers the endpoint, not the
   archive `url` the manifest names: Oxyn refuses an archive that is not an
   asset of the project's releases
   (`https://github.com/so-keyldzn/oxyn/releases/download/…`) before asking
   for a byte (`channel::is_release_asset`), follows redirects over HTTPS
   only, and drops an archive over 256 MiB, announced or received, before the
   plugin has buffered more — what makes the archive trustworthy is still its
   signature, not its transport. The archive is installed only if its
   minisign signature matches the public key compiled in, and the version
   signed with it matches the version the manifest announces
   (`requireSignedVersion`); a version not strictly greater than the running
   one is never offered. A build still carrying the placeholder public key
   refuses every update as a signature failure. Release notes come from the
   unsigned manifest — accepted: they are capped at 4 KiB, cut on a character
   boundary, and rendered as plain text, never as HTML or links. The webview never supplies
   a URL, a path or a version: the updater plugin is driven from Rust only,
   `tauri-plugin-opener` is not even registered — Rust calls its free
   function `open_url` —, `capabilities/main.json` grants no `updater:` nor
   `opener:` permission and a test keeps it so, and the release page opened
   in the browser is a fixed prefix followed by a version Rust validated as
   semver. The manifest itself is not signed: whoever controls
   the endpoint can withhold updates and choose the notes shown beside a
   genuine archive, not ship code. `restart_to_update({confirmed: true})`
   lets a script skip the "work would be stopped" dialog — at worst it
   restarts into an update already verified, as `cancel_exit` and the
   ordered exit already allow; open transactions still ask.

## `unsafe` policy

**`unsafe` is refused at compile time.** `[workspace.lints.rust]` carries
`unsafe_code = "deny"`, and of the fifteen crates — twelve under `crates/`,
three drivers under `drivers/` — a single function re-allows it:
`register` in `drivers/oxyn-driver-sqlite/src/vector_extension.rs`, which
registers the bundled sqlite-vec extension on a connection
([ADR-0054](adr/0054-bundle-sqlite-vec-in-the-sqlite-driver.md)). The manifest is authoritative here, because it is what is
executed: this document previously described a policy of supervised use that
compilation does not grant, and [ADR-0021](adr/0021-marqueur-d-arret.md) grounded
an architecture decision — not checking a pid — on the refusal, not on the
supervision.

Lifting this refusal is an architecture decision, not a local `#[allow]`:

- it goes through an **ADR** that says what `unsafe` buys and what it costs;
- the resulting `#[allow(unsafe_code)]` is placed as close as possible, never on the
  workspace;
- each block carries a `// SAFETY:` that states the invariant that makes it correct,
  and **who** guarantees it;
- it goes through the review of the `relecteur-securite` agent;
- the crate that exposes it behind a safe API documents the conditions of that
  safety.

External dependencies, for their part, contain it — Tauri, its
webviews and driver FFIs require it. The refusal covers **what this repository writes**.

A `// SAFETY:` that paraphrases the code ("we dereference a valid pointer")
is worthless: it must say **why** the pointer is valid at that place and
what will keep it valid.

## Dependencies

- every new direct dependency is justified in review: what it brings,
  and the cost of doing without it;
- `cargo deny` on licenses and security advisories is part of the quality
  gate, through the `make deny` target that `make qualite` calls. A local run
  **warns without blocking** when `cargo-deny` is not installed; under CI, a
  missing tool fails the gate, and the workflow installs a versioned binary
  whose SHA-256 checksum is pinned. The configuration lives in `deny.toml`.
  Every license it accepts is compatible with GPLv3, the application's license
  ([ADR-0044](adr/0044-licence-gpl-et-contrat-apache.md));
- the production npm dependencies of `apps/desktop` go through the same check,
  through `make licences-npm`, which `make front-controles` calls. The list is
  the one in `deny.toml`, completed for npm by `apps/desktop/licences-npm.toml`.
  `make audit-npm`, also reached by `make front-controles`, runs `pnpm audit
  --prod` and fails on a published advisory in that shipped graph;
- a dependency used in a single place for a single function
  is a candidate for rewriting, not a given;
- an unmaintained crate on an external boundary is a risk to document,
  not to ignore;
- a dismissed security advisory is dismissed **in `deny.toml`, with its written reason**.
  The only one today is RUSTSEC-2024-0429 — an *unsoundness* in
  `glib::VariantStrIter`, reached through `tauri` → `gtk 0.18` → `atk` → `glib 0.18`.
  The fix is in `glib 0.20`, which `gtk 0.18` refuses: nothing can be upgraded
  here until Tauri changes GTK. This code is Linux's; it
  is not compiled on macOS, the only target shipped today, and Oxyn
  does not call this iterator. To reopen at the next Tauri upgrade.

See also [`/securite`](../.claude/commands/securite.md) and
[the checklist](../.claude/checklists/revue-securite.md).
