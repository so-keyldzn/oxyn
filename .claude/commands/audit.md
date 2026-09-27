---
description: Audit the repository with several agents and prepare or publish the GitHub issues
argument-hint: "[scope, default: whole repository; publication only on request]"
---

# Multi-agent audit

Follow the [audit workflow](../workflows/audit-multi-agents.md) for
**$ARGUMENTS**. It complements the [review](relire.md) with a map of the whole
repository, an independent contradiction pass and the tracking of findings on
GitHub.

The Claude workflow engine can run
[`audit-multi-agents.js`](../workflows/audit-multi-agents.js). In Codex, follow
the same steps with the session's collaboration tools. Claude metadata creates
neither permissions nor technical isolation.

A request for an audit alone produces the report. An explicit request to create
issues authorizes publishing them with `gh`, without further confirmation.
The review does not fix the product and creates no commit.
