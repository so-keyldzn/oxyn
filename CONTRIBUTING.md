# Contributing to Oxyn

Thank you for considering a contribution. This page covers the two things
every contribution needs: the license agreement, and the quality gate.

## The Contributor License Agreement

Every external contribution needs a signed
[Contributor License Agreement](CLA.md) before it can be merged. You sign it
once, and it covers all your future contributions.

**Why a CLA.** Oxyn is licensed under the GPL-3.0-or-later. The driver contract
(`oxyn-core`, `oxyn-catalog`, `oxyn-data` and `oxyn-driver`) is under
Apache-2.0: see [NOTICE](NOTICE). The CLA lets the copyright holder, Nicolas
Boromée, keep offering the whole project under one set of terms. It also lets
them relicense the project, and transfer these rights to the company that will
maintain Oxyn. Without it, every contributor would have to agree to any such
change, and a single one could block it. The reasons are recorded in
[ADR-0044](docs/adr/0044-licence-gpl-et-contrat-apache.md), in French.

**What the CLA does not do.** You keep the copyright on your work. You remain
free to use your contribution for anything else, under any license.

**How to sign.** When you open a pull request, a bot asks you to sign the CLA
in a comment. The pull request cannot be merged until you have signed. If the
bot does not appear, say so in the pull request and the maintainer will send
you the agreement.

Code copied from another project needs the same care: say where it comes from
and under which license (section 7 of the CLA). GPL code from another project
cannot be accepted, even though the licenses are compatible, because its
copyright holder has not signed the CLA.

## Before you open a pull request

```bash
make qualite
```

This is the single quality gate: formatting, lints, types, tests, stories and
their accessibility checks, documentation, and the dependency licenses. CI
runs the same targets and adds none. A pull request that does not pass it is
not ready for review.

A few conventions that the gate cannot check:

- code, identifiers, comments and error messages are in **English**;
  documentation, ADRs and commit messages are in **French**;
- commit messages follow Conventional Commits;
- a new dependency is justified in the pull request: what it brings, and what
  doing without it would cost. Its license must be in the list accepted by
  `deny.toml`.

[CLAUDE.md](CLAUDE.md) lists the thirteen invariants of the codebase, and
[docs/](docs/README.md) holds the authoritative documents. A contradiction
between the code and one of them is a bug: report it rather than decide alone.
