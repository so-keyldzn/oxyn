---
name: non-argumente-plutot-que-composant-de-plus
description: On a list of interface gaps, a quantified "that one is useless here" is preferred to one more component
metadata:
  type: feedback
---

When I am given a list of components to write, return a **reasoned refusal**
for those the repository already covers or that the data does not allow, rather
than delivering the whole list.

**Why:** explicitly requested — "I prefer a reasoned 'that one is useless here'
to one more component". The repository pays for each component in stories, axe
and review; a duplicated gesture also creates two paths for a single action, of
which only one will be reviewed.

**How to apply:** before writing, read the neighboring components and look for
whether the **gesture** already exists under another name. The refusal must name
the file that already covers the need, or the missing backend data — never "it
seems redundant to me". Corollary: if a component asks for data the backend does
not return, expose it as a prop and say which one, instead of computing it in
the webview (which [front.md] forbids anyway).

**The most expensive corollary: do not design a prop without a checked
source.** Ask for the real shape (file and lines) *before* writing the types.
Lived: I designed an agent plan with "failed" and "abandoned" states, an error
`detail`, a step identifier and a revision counter — none of them exist in the
protocol, and the counter would have **lied**, counting ordinary progress as
replacements. A surface without a source ends up empty or filled with a
front-end invention.

And when a surface will **never** have a source, delete it and write it in the
report as "dropped for lack of a source": it is useful information, not a
failure.

Two collaboration rules that go with it, in this multi-agent repository: do not
modify a file owned by another agent, and **report** a contradiction between an
instruction and an ADR instead of settling it (CLAUDE.md: "report it, do not
settle it alone").
