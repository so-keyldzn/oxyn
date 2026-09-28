---
name: piege-hook-nom-secret-token
description: The code_interdit hook refuses any derive(Debug) on a type whose name contains Token, Secret, Dsn — including when it carries no secret
metadata:
  type: feedback
---

`SECRET_NAME` in `.claude/hooks/code_interdit.py` is a pattern on the **type
name**, not on its fields:
`Credential|Secret|Password|Passwd|Token|ApiKey|Dsn|ConnectionString`. Any
`#[derive(..., Debug, ...)]` on a `struct` or `enum` whose name contains one of
these words is refused, whatever its content.

**Why:** a `TokenUsage` carrying only token counters was refused as a secret
carrier. The hook is right nine times out of ten and it is a wall, not a
reminder: it is not up for discussion.

**How to apply:** rename rather than work around. `TokenUsage` → `TurnUsage`.
Writing a manual `Debug` to keep the name means adding code to neutralize a
protection — and the next reader will read "this type carries a secret". The
type name is anyway the first clue given to the reviewer.

The refusal comes when the file is written, so **before** any compilation:
choosing the name knowingly avoids rewriting an 800-line file. See also
[[piege-hook-code-interdit-diff]], about the hook reading the edit's text and not
the file.
