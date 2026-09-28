---
name: alert-porte-role-alert
description: The generated Alert component hard-codes role="alert"; migrating a static callout to Alert turns it into an announced live region, unless role is overridden
metadata:
  type: feedback
---

`ui/alert.tsx` renders `<div role="alert" … {...props}>`: every `Alert` is an
assertive live region. The `role` is set **before** the props spread, so
`<Alert role="note">` replaces it.

**Why:** the "Callouts use Alert" rule pushes to convert a homemade warning
callout (dashed border, icon) into `Alert`. If the callout is static and mounted
when a dialog opens, the conversion makes the screen reader read it before the
title, and adds a `role="alert"` that the `play`s counting alerts
(`getByRole("alert")` for an error) see twice. Neither the typecheck nor axe
flag the change.

**How to apply:** before migrating a callout to `Alert`, decide whether it must
be announced. Static → pass `role="note"` (or report it as `décision`); an
error that appears → keep the default role. Check the `play`s that look for
`alert` in the same screen.
