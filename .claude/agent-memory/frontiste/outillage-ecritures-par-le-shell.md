---
name: outillage-ecritures-par-le-shell
description: Writing a file through a shell redirection (cat > … <<EOF) triggers an arbitration request from the hook; use Write/Edit in this repository
metadata:
  type: feedback
---

In this repository, writing a front-end file through a shell redirection
(`cat > file <<'EOF'`) triggers an arbitration request from `code_interdit.py`,
even when the environment instruction recommends preferring Bash over the
dedicated tools. Use `Write` / `Edit`.

**Why:** a write that goes through the shell escapes the invariant checks wired
on `Write` and `Edit` (a `use gpui` outside its crate, a hard-coded secret). The
hook is not a reminder, it is a wall: it asks for the user's agreement, which
interrupts the work for nothing.

**How to apply:** create or replace a file → `Write`; modify → `Edit`. The shell
stays the right tool to *read* (`cat`, `sed -n`) and to search (`grep`, `find`),
which are not intercepted.
