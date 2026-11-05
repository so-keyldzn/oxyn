# Contributing to Oxyn

Thank you for considering a contribution. This page covers the two things
every contribution needs: the license agreement, and the quality gate.

## The Contributor License Agreement

Every external contribution needs your acceptance of the
[Contributor License Agreement](CLA.md) before it can be merged. You give it
once, and it covers all your future contributions.

**Why a CLA.** Oxyn is licensed under the GPL-3.0-or-later. The driver contract
(`oxyn-core`, `oxyn-catalog`, `oxyn-data` and `oxyn-driver`) is under
Apache-2.0: see [NOTICE](NOTICE). The CLA lets the copyright holder, Nicolas
Boromée, keep offering the whole project under one set of terms. It also lets
them relicense the project, and transfer these rights to the company that will
maintain Oxyn. Without it, every contributor would have to agree to any such
change, and a single one could block it. The reasons are recorded in
[ADR-0044](docs/adr/0044-licence-gpl-et-contrat-apache.md).

**What the CLA does not do.** You keep the copyright on your work. You remain
free to use your contribution for anything else, under any license.

**How to accept it.** Read [CLA.md](CLA.md), then post this sentence as a
comment on your first pull request:

> I have read the Oxyn Individual Contributor License Agreement, version 1.0,
> and I accept it for this pull request and all my future contributions.

If you contribute on behalf of an employer, section 4 of the CLA applies: you
must have their permission to do so.

**How it is checked.** There is no bot and no automated check: the `qualite`
check does not look at the CLA. The maintainer reads your comment and
verifies it, by hand, before merging a pull request from someone who is not
the maintainer. Without it, the pull request waits, whatever its review
state. Your comment stays in the pull request as the record of your
acceptance, and a later pull request of yours does not need a new one unless
the CLA changes version.

Code copied from another project needs the same care: say where it comes from
and under which license (section 7 of the CLA). GPL code from another project
cannot be accepted, even though the licenses are compatible, because its
copyright holder has not accepted the CLA.

## Before you open a pull request

```bash
make qualite
```

This is the single quality gate: formatting, lints, types, tests, stories and
their accessibility checks, documentation, and the dependency licenses. CI
runs the same targets and adds none. A pull request that does not pass it is
not ready for review.

A few conventions that the gate cannot check:

- everything is in **English**: code, identifiers, comments, error messages,
  documentation, ADRs, commit messages and pull requests
  ([ADR-0047](docs/adr/0047-english-as-the-repository-language.md)).
  [`i18n/fr/`](i18n/README.md) holds French mirrors of the English
  documents; English is authoritative;
- commit messages follow Conventional Commits: `type(scope): subject`, in
  lowercase, imperative, no final period, 72 characters at most;
- a new dependency is justified in the pull request: what it brings, and what
  doing without it would cost. Its license must be in the list accepted by
  `deny.toml`.

[CLAUDE.md](CLAUDE.md) lists the thirteen invariants of the codebase, and
[docs/](docs/README.md) holds the authoritative documents. A contradiction
between the code and one of them is a bug: report it rather than decide alone.
