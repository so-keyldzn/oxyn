# ADR-0011 — A dense workbench as the common structure of the workspace

**Status:** accepted · **Date:** 2026-09-07

## Context

The Oxyn mockup presents two layouts: 37 business screens of Figma pages 04 to 06
use 88 px page headers, while the three flows of page 22 place objects and
consoles in a dense workbench. The state boards of page 07 reuse the first
layout. The product request calls for a database area close to an IDE and
requires consistency between screens.

`crates/oxyn-app/src/workspace/` already has navigation between SQL and objects.
It still keeps a title and a subtitle in `content.rs`, however: the existence of
the code does not mean the whole Figma target is implemented.

## Decision

Structure B, the dense workbench of Figma page 22, is the common target.
[UX-SPEC](../UX-SPEC.md#common-workspace-structure) defines its composition
and the scope of the other pages. Pages 04 to 07 remain content and behavior
specifications; their old headers are not to be reproduced. The security and
data-handling invariants remain unchanged.

## Consequences

- **+** Tables, consoles and object inspection share the same navigation context
  and keep more room for the data.
- **+** The existing scenarios remain usable without rewriting their privacy,
  execution and error contracts.
- **−** The old boards are no longer complete captures of the window to
  implement; their scope must remain explicitly stated.
- **−** Aligning the code and the width variants requires an adaptation of the
  workspace, separate from this mockup decision.

**Exit cost:** replace the structure in the views of `oxyn-app` and `oxyn-ui`,
then rework the Figma flows and their navigation. The domain and the drivers
are outside this structure thanks to their existing separation. The cost
depends on the number of integrated views; it exceeds a local layout change.

**Reconsider if** usability tests show that users lose the connection or object
context when moving between consoles, data and assistants, or if the compact
variant prevents an essential flow.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| A page-based application with a large header everywhere | Consumes useful space and does not match the chosen IDE direction |
| Two structures depending on the feature family | Makes the active context and the navigation rules between screens ambiguous |
| Redo all the business boards immediately | Needless cost to settle the structure; their content remains valid once their scope is explicit |
