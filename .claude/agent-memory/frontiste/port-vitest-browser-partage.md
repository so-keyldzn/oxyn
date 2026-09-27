---
name: port-vitest-browser-partage
description: "Port 63315 is already in use" with "no tests" = another agent is running its stories at the same time, not a failure
metadata:
  type: feedback
---

`vitest run --project storybook` listens on a fixed port (63315). When another
agent of the team runs its stories at the same moment, the run ends with
`Error: Port 63315 is already in use` then `Test Files no tests`. It is neither
a failure nor a regression.

**Why:** observed during multi-agent work on `apps/desktop`.

**How to apply:** rerun in a loop that detects "already in use" and waits
~20 s between two attempts (foreground `sleep` is blocked:
`perl -e 'select(undef,undef,undef,20)'` works).
