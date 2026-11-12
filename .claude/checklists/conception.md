# Engineering judgment

Before changing the repository, reason as a specialist in the domain touched.
Apply this checklist even when no formal plan is needed. Keep the depth and
written explanation proportionate to the task; a small change needs no new
planning document.

## Understand the problem

- [ ] State the user-visible outcome, constraints and observable completion
      criterion before choosing an implementation.
- [ ] Read the relevant [specialty profiles](../README.md#agents), rules and
      authoritative contracts. Follow the current implementation and its callers;
      an old example or a familiar pattern is not evidence that it fits here.
- [ ] Separate verified facts from assumptions. Check assumptions that determine
      correctness, and state what remains uncertain instead of inventing a fact.
- [ ] For a bug, identify the cause and the conditions that expose it before
      choosing a fix; do not merely hide the symptom or suppress its error.

## Choose a design that fits

- [ ] Identify the responsibility that owns the change, its inputs, outputs and
      dependencies. Apply the [code organization rules](../../CLAUDE.md#code-organization)
      before extending an existing file or creating another one.
- [ ] Inspect what can already be reused. Choose the simplest design that meets
      the actual requirements, and explain consequential trade-offs against a
      plausible alternative; familiarity alone is not a justification.
- [ ] Examine the failure cases relevant to this domain: malformed or partial
      server responses for a driver; concurrency, cancellation and stale results
      for asynchronous work; privacy and trust boundaries for AI; loading, empty
      and error states, keyboard access and focus for UI; resource limits for
      streaming. Select the applicable cases, not a ritual list for every task.
- [ ] Check the affected contracts and callers, including compatibility and
      recovery when data or interfaces persist beyond the current operation.

## Decide how to demonstrate it

- [ ] Choose checks that demonstrate the intended behavior and relevant failure
      cases. Prefer a reproducible regression scenario for a bug and observable
      outcomes over tests that merely repeat the implementation.
- [ ] Support performance claims with measurements and the applicable budgets.
- [ ] Carry significant assumptions, trade-offs and validation limits into the
      review and final report. Finish with the [end-of-task checklist](fin-de-tache.md).
