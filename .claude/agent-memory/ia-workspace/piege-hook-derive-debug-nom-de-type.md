---
name: piege-hook-derive-debug-nom-de-type
description: The I-03 hook refuses `#[derive(Debug)]` on any type whose NAME contains Token/Secret/ApiKey/Credential/Password/Dsn/ConnectionString, even with no secret inside
metadata:
  type: feedback
---

`.claude/hooks/code_interdit.py` refuses a `#[derive(Debug)]` as soon as the
**type name** matches `(?:Credential|Secret|Password|Passwd|Token|ApiKey|Dsn|ConnectionString)`.
It does not look at the fields.

**Why:** the rule implements the checkable corollary of I-03 — no type carrying
a secret derives `Debug`, because it is the `tracing::debug!("{x:?}")` added six
months later that leaks. The hook is a wall, not a reminder: it has no
exemption, and a test on the name is the only criterion a regex can apply.

**How to apply:** the false positive that costs time is a **token counting**
type — `PromptTokensDetails`, `TokenUsage`, `TokenCount` all contain "Token"
while they only carry integers. Two ways out, both acceptable:

* name the type without the trigger word (`CountResponse` rather than
  `TokenCount`, `UsageDetails` rather than `PromptTokensDetails`);
* keep the name and write `impl fmt::Debug` by hand.

Do not work around the hook through the shell: `code_interdit.py` only sees
writes going through Write and Edit, and a `perl -0pi -e` on a source file
escapes *all* invariant checks — the `PreToolUse` hook says so explicitly
anyway.
