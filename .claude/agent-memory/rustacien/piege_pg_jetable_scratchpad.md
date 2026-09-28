---
name: piege-pg-jetable-scratchpad
description: Launching a disposable PostgreSQL to check a server behavior — Unix socket path too long in the scratchpad, psql unusable to test the extended protocol on several commands
metadata:
  type: feedback
---

To check a real server behavior, Postgres.app provides `initdb` and `pg_ctl`,
in `/Applications/Postgres.app/Contents/Versions/latest/bin`.

**First trap: the Unix socket.** The session scratchpad path exceeds 103 bytes,
and `pg_ctl start` dies with "Unix-domain socket path is too long". The server
must be started with TCP only:
`-o "-p 5499 -k '' -c listen_addresses=127.0.0.1"`.

**Second trap: `psql` cannot test the extended protocol on a multi-command
text.** Its lexer, `psqlscan.l`, splits on `;` before sending. `\bind \g`
therefore never sees `SELECT 1; DROP …` as a single Parse message. A minimal
wire client in Python (`socket` and `struct`; Startup, then
Parse/Bind/Execute/Sync with `trust` authentication) does the job in about fifty
lines.

**Why:** these two traps cost me two attempts while checking the end of a `--`
comment at `\r`.

**How to apply:** as soon as a question is about what the server really runs,
notably the difference between simple and extended protocol. Remember to stop
the server with `pg_ctl stop -m fast`.
