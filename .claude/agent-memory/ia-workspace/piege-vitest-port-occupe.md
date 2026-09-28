---
name: piege-vitest-port-occupe
description: `make qualite` fails at the front stage on "Port 63315 is already in use" when another vitest runs in parallel — all tests pass nonetheless
metadata:
  type: feedback
---

The `front` stage of `make qualite` can fail with `Error: Port 63315 is already in use` (browser vitest), while "Test Files 27 passed" is displayed right below. It is not a regression: another vitest (a teammate, or a previous `make qualite` not yet exited) was holding the port.

**Why:** observed on 2026-09-23, in a team, with several agents running the gate; `lsof -nP -iTCP:63315 -sTCP:LISTEN` was empty a few seconds later, and the rerun passed.

**How to apply:** before looking for a cause in the front end, read the unhandled error; if it is the port, check with `lsof` and rerun.
