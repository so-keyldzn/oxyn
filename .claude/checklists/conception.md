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
- [ ] Before writing, locate equivalent behavior and identify what the change
      makes obsolete, using the code organization rules above. Choose the
      simplest design that meets the actual requirements, and explain
      consequential trade-offs against a plausible alternative; familiarity
      alone is not a justification.
- [ ] Examine the failure cases relevant to this domain: malformed or partial
      server responses for a driver; concurrency, cancellation and stale results
      for asynchronous work; privacy and trust boundaries for AI; loading, empty
      and error states, keyboard access and focus for UI; resource limits for
      streaming. Select the applicable cases, not a ritual list for every task.
- [ ] Check the affected contracts and callers, including compatibility and
      recovery when data or interfaces persist beyond the current operation.

## Assess consequences

- [ ] Trace concrete failure scenarios for the change: data disclosure, loss or
      corruption, unauthorized mutation, or partial results mistaken for a
      complete success. Identify the affected people, data and downstream work;
      consider the applicable scenarios, not every hypothetical hazard.
- [ ] Assess severity, reach, detectability, reversibility and recovery effort.
      Include lost user work, interruption, remediation and loss of trust.
      Distinguish demonstrated consequences from assumptions; use numbers only
      when evidence supports them, never invented probabilities or cost estimates.
- [ ] Check failure states against the authoritative contract: a swallowed error
      or fallback value must not disguise failed, partial or unknown work as
      completed success. Verify how the caller or user detects the problem and
      what recovery is possible, respecting [ambiguous error handling](../../CLAUDE.md#i-13).
- [ ] Check that useful diagnostics and recovery paths preserve
      [secret handling](../../CLAUDE.md#i-03) and the
      [AI privacy boundary](../../CLAUDE.md#i-04) where applicable. Follow the
      [security contract](../../docs/SECURITY.md), including when reporting a
      failure; exposing sensitive data is not a debugging shortcut.

## Decide how to demonstrate it

- [ ] Choose checks that demonstrate the intended behavior and relevant failure
      cases. Prefer a reproducible regression scenario for a bug and observable
      outcomes over tests that merely repeat the implementation.
- [ ] Prioritize verification of severe consequences and failures that are hard
      to detect. Show how those scenarios are checked or state the validation
      gap; this prioritization never makes an invariant optional.
- [ ] Support performance claims with measurements and the applicable budgets.
- [ ] Carry significant assumptions, trade-offs and validation limits into the
      review and final report. Finish with the [end-of-task checklist](fin-de-tache.md).
