## Problem and outcome

Describe the concrete problem and the behavior obtained after this change.

## Scope

- Linked issue(s):
- Areas touched:
- Authoritative documents concerned:

## Validation

- [ ] `make qualite`
- [ ] Targeted tests added or run
- [ ] Error, cancellation and concurrency scenarios checked if relevant
- [ ] Native acceptance check done if rendering or windows are concerned

List here the commands actually run and their results. Distinguish targeted
tests, headless tests, tests against PostgreSQL and the native acceptance check.

## Security and data

- [ ] No secret, sensitive identifier or user data is added to logs, errors, fixtures or screenshots.
- [ ] Commands go through the command bus and the policy checks.
- [ ] Ambiguous writes are not replayed automatically.
- [ ] The external boundaries concerned have been reviewed.

## Risks and limits

Describe what remains unproven, the skipped tests, the external dependencies
and the deployment or migration steps required.

## Final checklist

- [ ] The change respects invariants I-01 to I-13.
- [ ] The authoritative documentation stays consistent with the code.
- [ ] Added TODOs are dated and say what unblocks them.
- [ ] External licenses and versions are justified and documented.
- [ ] Commit messages and this description are in English.
