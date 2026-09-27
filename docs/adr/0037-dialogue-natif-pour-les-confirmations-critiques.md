# ADR-0037 — A critical decision is confirmed in a native dialog of the host, never in the webview

**Status:** proposed · **Date:** 2026-09-25

**Clarifies:** [ADR-0029](0029-interface-tauri-shadcn.md), whose Consequences
name the new surface — "an XSS in the webview reaches the Tauri commands" —
without saying what stays out of its reach. This ADR says it for three
decisions.

**Complements:** [ADR-0034](0034-echantillon-pour-toute-destination.md), whose
§ 4 makes `ai_answer_sample` "a user gesture in the screen": that is true of
what the agent can send, not of what a script in the webview can call.

## Context

A confirmation drawn in the webview protects against an unfortunate click, not
against a script. Everything the webview can click, a script injected into the
webview can call directly, without drawing anything. On 2026-09-24, three
decisions nevertheless rested on a webview button:

| Decision | What grants it today |
|---|---|
| Write or DDL on a `production` connection ([I-02](../../CLAUDE.md#i-02)) | `decide(command, approved: true)` — `crates/oxyn-desktop/src/commands.rs`, then `Backend::decide`, which calls `executor.approve("human", …)` without any other check |
| Editing or deleting a `production` connection | `decide_connection_change(command, approved: true)` — `commands/settings.rs` |
| Rows going out to an agent | `ai_answer_sample(connection, request, columns)` for an agent request; the `sample` field of `AskRequest` for a pinned sample |
| Change of environment or privacy tier | `update_connection`, then `decide_connection_change` when the connection is or becomes `production` — the `PolicyGate` retains the more restrictive of the two markings (`crates/oxyn-core/src/policy.rs`); **no** confirmation otherwise: a tier change on a `development` connection goes through without approval |

A script that obtains the identifier of a pending command — it transits through
the webview, that is where it is displayed — approves it in one call. A script
that reads the catalog approves a sample that no agent asked the user for. And
the last case does not even need an identifier: switching a connection from
`production` to `development`, then writing, removes the confirmation of
[I-02](../../CLAUDE.md#i-02) in three calls, the first of which itself returns
the identifier to approve; switching `Metadata` to `Sampled` reopens the row
egress the user had closed.

The repository already has the answer, for one case: `ai_save_external_agent`
(`commands/ai.rs`) only declares an agent after a dialog of the
`tauri-plugin-dialog` plugin, drawn by the host. The comment gives the reason:
"a script injected in the webview can call this command, it cannot click a
window it does not draw". This ADR extends that model, and only that model, to
decisions whose effect is irreversible or makes data go out.

**External fact, checked in the source of `tauri-plugin-dialog` 2.7.3 (the
`Cargo.lock` version), `src/lib.rs`, `MessageDialogBuilder::show`, on
2026-09-25 ([I-12](../../CLAUDE.md#i-12)).** The callback receives `true` for
`Ok`, `Yes`, or a custom button **whose label equals** that of the confirmation
button; any other outcome — Cancel, closing — gives `false`. Two identical
labels would therefore make Cancel a confirmation.

## Decision

### 1. What is critical, and nothing else

Three families of decisions go through a native dialog:

1. **a write or a DDL on a `production` connection** — any approval, through
   `decide`, `decide_connection` or `decide_connection_change`, of a mutating
   command whose connection is `production` **at the time of approval**,
   whatever the reason the `PolicyGate` held it — a `DROP` or a `DELETE`
   without `WHERE` is held for "unbounded mutation" before being held for
   production, and it is the worst case. The environment is the one the gate
   retains, the more restrictive of the announced and the recorded one, read
   by **its** computation and not copied: a command held on `development`
   whose connection has since become `production` falls under the dialog. The
   creation of a `production` connection, which the gate holds as DDL and
   `decide_connection` approves, falls under it too. For an `Actor::Agent`,
   nothing changes: it is a refusal, and no dialog opens
   ([I-02](../../CLAUDE.md#i-02));
2. **any approved row egress to an agent** — the pinned sample, when the
   question that carries it goes out, and the sample requested by the
   `request_sample` tool, when the user has checked its columns
   ([ADR-0034](0034-echantillon-pour-toute-destination.md));
3. **any change of environment or privacy tier of a connection**, in both
   directions and whatever the starting environment. The criterion is the
   change, not its direction: deciding that one direction is safe is writing a
   second tier rule that nobody will review. The `PolicyGate` only holds such a
   change if one of the two markings is `production`: a tier change on
   `development` goes through without approval. The `Backend` therefore
   compares the edit to the recorded configuration and opens the dialog
   **before** sending the command, whether or not it is then held. If it
   is — the connection is or becomes `production` —, its approval also falls
   under family 1 and opens its own dialog: two dialogs for this rare case,
   rather than an approval granted without going through
   `decide_connection_change`.

The declaration of an external agent already has its native dialog, decided by
[ADR-0026](0026-agents-externes-acp.md): it stays outside these three families,
and only joins the port of § 4.

The rest keeps the confirmation in the webview: write outside `production`,
approval of an agent command outside `production`, renaming, deletion of a
connection outside `production`. **Refusing** is never critical: an
`approved: false` goes through no dialog.

### 2. The dialog names the connection from the backend

The dialog text is composed in Rust from what the **backend** holds — the
recorded configuration, the command held by the executor, the pending sample
request —, never from an argument coming from the webview. A script can choose
what to have approved; it cannot choose what the dialog says about it.

Each dialog carries:

* **the connection**: its name as recorded, its environment in full
  (`PRODUCTION`), and the non-secret address the connection screen already
  displays (host, port, database, or file path) — two connections with the
  same name are thus told apart. No field marked secret appears in it
  ([I-03](../../CLAUDE.md#i-03));
* **the object**:
  * for a write, first what the backend classified — the intent and the
    `PolicyGate`'s reason —, then the text of the statement as the executor
    holds it: in full up to **1,000 characters**, beyond that its beginning
    and its end (500 characters each) separated by "… N more characters …".
    The cut is stated, not guessed; and showing the end prevents a harmless
    1,000-character preamble from hiding on its own what follows;
  * for a sample, the relation, the checked columns one by one, the maximum
    number of rows, and the recipient as the recorded configuration describes
    it — endpoint host for a provider, command for an external agent, and
    whether the rows leave the machine. Not just its label: a script can
    declare a provider under another's label;
  * for a connection edit, **each** modified field, old and new value, and
    whether the secret is kept or re-entered: the edit is confirmed whole, it
    is shown whole;
  * for a deletion, the connection alone — that is the whole object;
* **no raw string**: everything that comes from an untrusted input — connection
  name (workspace file), statement, relation and column names (catalog),
  recipient label ([SECURITY](../SECURITY.md#input-surface), points 2 and
  3) — goes through the same escaping function: control characters and
  bidirectional direction marks made visible, length bounded;
* **two buttons with distinct labels**, the confirmation one saying the effect
  ("Write to production", "Send rows", "Change marking") and the other
  "Cancel". The labels are constants, not parameters: two equal labels would
  make Cancel a confirmation (Context).

### 3. Closing is refusing, and a refusal exhausts the decision

Everything that is not the confirmation button counts as a refusal: Cancel,
Escape, closing the dialog window, stopping the application, a dialog that
could not open, an abandoned response channel.

A refusal **consumes** what it refuses, like a refusal in the screen:

* the held command is rejected (`executor.reject`) and does not stay pending —
  otherwise a script would call `decide` again until the reflex click;
* the sample request is declined, and the exchange is exhausted as
  [ADR-0034 § 3](0034-echantillon-pour-toute-destination.md) requires; a dialog
  response arriving after the five-minute expiry reads nothing;
* the question that carried a pinned sample does not go out;
* the marking change is not recorded, and nothing else of the same edit is —
  an edit is confirmed whole or not at all.

**A confirmation that is too fast is a refusal.** On macOS, the confirmation
button is the default button of the alert — `CFUserNotificationDisplayAlert`
for a dialog without a parent window, Oxyn's case —, hence the one Enter
triggers, and the plugin does not allow designating another (source of `rfd`
0.16.0, `src/backend/macos/utils/user_alert.rs`, checked on 2026-09-25). A
script that opens the dialog while the user is typing — Cmd+Enter to execute —
would get the next keystroke. A confirmation received less than **one second**
after the dialog is **displayed** therefore counts as a refusal, and consumes
the decision like any refusal. The second starts when the host presents the
dialog, not at the request: a dialog waiting its turn behind another still on
screen (deadline below) is displayed late, and has not been read for all that.
Each dialog of such a queue has its own second.

**A deadline for every dialog: five minutes**, the bound
[ADR-0034](0034-echantillon-pour-toute-destination.md) already sets for the
sample request. The plugin cannot close a dialog programmatically; past this
delay, the decision is consumed as refused, the next dialog can be requested,
and the late response of the dialog left on screen is ignored — a write
approved the next day would be a write nobody saw go out. The host displays
only one critical dialog at a time: the next one waits for the one left on
screen to be closed, and is never displayed if its own deadline passes first.
The deadline runs from the request, not from display: it bounds the life of
the decision, which waiting for its turn does not extend.

**A single critical dialog at a time.** A critical decision that arrives while
a dialog is open is refused immediately, with a message that says so; it does
not wait its turn. A queue would let a script stack dialogs that the user would
close by reflex, the last one being the real one. That refusal **does not
consume** the decision: otherwise a script would make the user's legitimate
decision fail by opening a dialog just before it.

**The webview never opens a message dialog.** The guarantee rests on the fact
that only the backend composes a native-looking dialog:
`capabilities/main.json` grants neither `dialog:allow-message`, nor
`dialog:allow-ask`, nor `dialog:allow-confirm`, and will not grant them. A
script that obtained them would draw dialogs of the same appearance, with text
of its choosing.

### 4. The dialog goes through a port, and is tested without a window

`oxyn-desktop` carries a `HostConfirm` trait — a single asynchronous method
that takes a `Confirmation` (title, body, confirmation label, severity) and the
deadline, and returns the outcome: refused, or confirmed with the instant the
dialog was displayed, from which the second of § 3 is counted. The host is the
one that knows when it presents a dialog; `NativeDialog` timestamps it on the
main thread, where the plugin draws, once the screen is free of the previous
one — and not before that thread's queue, which a webview script can fill.
What remains is the latency between that instant and the alert on screen, out
of a script's reach, which the manual check measures. Reaching its deadline
before its turn, the dialog is not drawn. Two implementations: this is a
boundary with the host and not an indirection
([CLAUDE.md](../../CLAUDE.md#code-organization)):

* **`NativeDialog`**, on the `tauri-plugin-dialog` plugin, like
  `ai_save_external_agent` today — which joins this port;
* **a scripted response**, in tests: confirm, refuse, or never answer.

The `Backend` receives the port at construction, without a default value: a
`Backend` that has none does not build, so no critical decision goes through
for lack of a dialog. The `Backend` opens before `tauri::Builder`, so that a
startup failure lands on stderr (`main.rs`); `NativeDialog` is therefore built
without an `AppHandle`, receives it at `setup`, and **refuses** as long as it
does not have it — closing is refusing, and so is not having been able to open.
It is the `Backend`, and not the Tauri command, that decides a decision is
critical and calls the port: the rule lives next to the configuration it reads.

The port is only awaited from an asynchronous context: the plugin draws the
dialog through `run_on_main_thread`, and a synchronous Tauri command runs on
that thread ([I-05](../../CLAUDE.md#i-05)). `ai_answer_sample` stays
synchronous — it hands the columns to the waiting agent call — and it is **that
call**, on its task, that opens the dialog before reading.

What is tested, without a window or `MockRuntime`:

* **the text** — the functions that compose a `Confirmation` are pure: the name
  and the environment are in it, a hostile string is escaped wherever it
  appears, a long statement is cut **and says so**, its end appears, each
  modified field of an edit appears, no secret field appears;
* **the behavior** — on a `Backend` mounted with the scripted response and
  scripted delays: a `DELETE` without `WHERE` on `production` opens the
  dialog; `decide(…, true)` on `production` refused, without an answer after
  the deadline, or confirmed in less than a second executes nothing and leaves
  no pending command; confirmed, executes; of two queued dialogs, the second
  refuses a confirmation arriving less than a second after its own display,
  even more than a second after its request; a command held
  on `development` whose connection has become `production` opens the
  dialog; `production → development` refused leaves the configuration
  unchanged and sends no command; a refused sample reads no row; a second
  critical decision during the first is refused without being consumed; a
  non-critical decision does not call the port;
* **the labels** — a test checks that the confirmation label of each
  `Confirmation` differs from "Cancel".

Only `NativeDialog` escapes automated tests: it is checked by hand in
`make desktop-dev`, on the three platforms, once per plugin version change.

### 5. What the webview screen becomes

The screen stays where one **reads and chooses**: the full statement with its
highlighting, the columns to check, the connection form. For a critical
decision, its approval button no longer grants it: it asks the backend, which
opens the dialog. The webview learns the outcome from the command's response,
as today.

## Consequences

* **+** What an XSS can do on these three decisions is limited to **opening a
  dialog** that the user sees, that tells the truth because the backend writes
  it, and that closing is enough to refuse.
* **+** A single strong confirmation model, already in service for the
  declaration of an external agent, and now testable through the same port.
* **+** The marking change stops being I-02's back door: it is no longer
  possible to downgrade a connection then write to it without the user having
  confirmed the downgrade in a dialog that names the connection and its former
  environment.
* **−** **Two confirmations** for a write in production: the webview screen,
  then the dialog. It is the case most exposed to fatigue, and this ADR makes it
  heavier. What bounds it: the critical perimeter is narrow, and the dialog
  only opens once the choice is made in the screen.
* **−** A native dialog is poor: no highlighting, no reliable scrolling, no
  checkbox. The statement is cut in it; its middle can only be re-read in the
  screen, which a script could have forged. The backend's classification, the
  beginning, the end and the number of missing characters are what remains
  true.
* **−** A confirmation given within the first second is lost, and the fast
  user has to start again from the screen.
* **−** **Three limits this ADR names without closing them**, because they go
  beyond its decision:
  * creating a **second** connection to the same target, marked
    `development` or `Sampled`, bypasses families 2 and 3 by duplication
    rather than by change, as soon as the target requires no secret (SQLite
    or DuckDB file, server without a password);
  * under `Sampled`, the server's full error message reaches the agent
    (`crates/oxyn-ai/src/failure.rs`), and such a message can quote row
    values: it is an **unapproved** row egress, which family 2 does not
    cover;
  * the declaration dialog of an external agent, which joins the port, shows
    the values of its environment variables, where a token often sits — its
    form stays the one [ADR-0026](0026-agents-externes-acp.md) decided.
* **−** Three engines, three renderings: the macOS, Windows and GTK dialogs
  have neither the same width nor the same button order, and `NativeDialog`
  can only be tested by hand.
* **−** A script can still **open** a critical dialog at the time of its
  choosing; it can neither stack it nor reopen it on the same decision, but a
  user who confirms without reading remains possible.

**Exit cost:** low. Removing the port call in the three paths gives the
confirmation back to the webview; the trait and `NativeDialog` stay for
`ai_save_external_agent`. No persisted format changes.

**Reconsider if** the webview gains a **verifiable** way to tell a user gesture
from a script call — a Tauri permission tied to a real input event, for
example; if production writes confirmed without reading become frequent, in
which case the answer would be to remove the webview screen for that case, not
the dialog; if one of the three limits named in the Consequences is closed by a
decision — it then joins, or not, the list of § 1; or if a new decision makes
data go out or downgrades a connection — it then joins the list of § 1, which
is changed by an ADR.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Keep the confirmation in the webview, hardened (CSP, `capabilities`) | CSP makes XSS harder; it does not make a webview button unreachable for a script running in it. The defense would only hold by the absence of a flaw, which cannot be proven |
| A single-use token returned to the webview with the request | The script reads the token where the screen reads it: same origin, same DOM |
| Confirm everything through a native dialog | Fatigue would empty the dialog of its meaning: it must stay rare to be read |
| Only dialog for lowering a marking | Fixing which direction is safe is a second tier rule; `development → production` then `production → development` becomes a two-step bypass again |
| A secondary Tauri window drawn in HTML | It is still a webview, with its script; the guarantee would come from isolation between windows, not from the host |
| Test with `tauri::test` and `MockRuntime` | The test would depend on the plugin's behavior outside a real window; the port separates what we decide from what the host draws |
