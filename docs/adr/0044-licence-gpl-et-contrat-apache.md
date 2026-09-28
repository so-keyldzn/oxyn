# ADR-0044 — The application is under GPL-3.0-or-later, the driver contract under Apache-2.0, and what is paid for is an account service

**Status:** accepted · **Date:** 2026-09-25

## Context

Until 2026-09-25, the whole repository is under Apache-2.0 (`Cargo.toml`,
`license = "Apache-2.0"` at the workspace level). Three facts make this choice
untenable in the long run:

- **The repository will become public** so that the application can be
  downloaded. A version published under Apache-2.0 stays so forever: anyone can
  take it, close it and sell it, without giving anything back.
- **The copyright holder wants to monetize.** This is Nicolas Boromée, sole
  author of the repository at this date. A company will come later, and the
  rights will be transferred to it.
- **Relicensing requires holding the rights to all the code.** Today, that is
  the case: a single author, a private repository. At the first external
  contributor without a written agreement, it no longer is, and the licensing
  model freezes.

The user settled it on 2026-09-25: the application moves to a copyleft
license, and what a third party must link to extend it stays under a
permissive license.

Two principles of [VISION](../VISION.md) bound the answer:

- **Open by default**: "open formats, no lock-in". A license that stops being
  free in the OSI sense, or a local feature put behind a subscription, would
  contradict it;
- **Privacy first**: "offline by default". A paid service can therefore only
  be an option, never a condition for using the product.

## Decision

### Two licenses, split by what a third party must link

**The application is under `GPL-3.0-or-later`.** It is the `license` value of
`[workspace.package]`. It covers the core crates, `oxyn-desktop`, the shipped
drivers, and the `apps/desktop` front end (`package.json`, `license` field).

**What a third party must link to write a driver stays under `Apache-2.0`.** An
external driver author implements the traits of `oxyn-driver`. The graph of
this crate, surveyed with `cargo tree -p oxyn-driver -e normal`, contains three
other workspace crates, and no others:

| Crate | Why it is in the set |
|---|---|
| `oxyn-driver` | the `Driver`, `Session` and `Cursor` traits, and the capabilities ([ADR-0003](0003-driver-capabilities.md)) |
| `oxyn-data` | `BatchSource`, `ResultBuffer` and the `RecordBatch` stream a driver produces ([ADR-0002](0002-arrow-result-model.md)) |
| `oxyn-catalog` | the metadata model a driver's introspection fills (`CatalogProvider`) |
| `oxyn-core` | the identifiers, errors and common types the other three depend on |

These four manifests write `license = "Apache-2.0"`. They do not inherit the
workspace value. The set is **minimal**: a crate only enters it if a
third-party driver cannot compile without it.

**Plugins.** A WASM plugin does not link `oxyn-plugin`: it is compiled against
WIT interfaces, and the host loads it ([ADR-0005](0005-wasm-plugins.md)).
`oxyn-plugin` is therefore the host, under GPL. The WIT interfaces do not exist
yet (phase 4). They will be published under `Apache-2.0`, in a directory or a
crate that carries that license alone. An Apache file inside a GPL crate would
show neither in its manifest nor in `cargo deny`.

The split is written in `NOTICE`. The texts are at the root: `LICENSE-GPL`,
downloaded from gnu.org, and `LICENSE-APACHE`.

### GPL crates are not published

Each GPL crate declares `publish.workspace = true`, and the workspace declares
`publish = false`. `deny.toml` enables `[licenses.private] ignore = true`:
`cargo deny` does not judge the license of a workspace crate that is not
published. `GPL-3.0-or-later` does **not** enter the `allow` list. There it
would also accept a third-party GPL dependency, and a copyleft dependency must
remain an explicit trade-off. The four Apache crates stay publishable: it is
through crates.io that a driver author will get them.

### A CLA for every external contribution

Every external contribution is covered by the CLA (`CLA.md`), based on the
Apache Software Foundation's *Individual Contributor License Agreement* v2.2.
The contributor keeps their rights. They grant the copyright holder a
perpetual, irrevocable and sublicensable license, which includes the right to
relicense. This license passes to the assignee of that copyright, notably to
the company the holder will found.

The CLA is in place **before** the first external contributor and **before**
the repository goes public. `CONTRIBUTING.md` explains it. Automatic signing
(CLA Assistant) is enabled when going public: it is a setting of the GitHub
organization, not a file of the repository.

Direct consequence: **no third-party GPL code is copied into the repository**,
even though the licenses now allow it. Its copyright does not belong to the
holder: it is covered by no CLA and would forbid relicensing.

### What is paid for: a service tied to an account

**All the code of this repository is usable without a subscription.** No local
feature is restricted, none waits for a license key. What is paid for are
**services** provided by a server and attached to an account: account login,
hosted AI, synchronization between machines.

The server of these services is not in this repository. It is not distributed:
the GPL therefore does not oblige it to publish its source, and nothing obliges
it to share the application's license.

These services are **optional**. The application works entirely without an
account and offline: opening a database, exploring it, querying it, and using
a local AI provider or one's own API key. That is "Privacy first": the cloud
adds, it replaces nothing. It is also "Open by default": what Oxyn writes stays
readable without Oxyn ([I-11](../../CLAUDE.md#i-11)), account or not.

### Third-party notices ship with the application

Distributing a binary means redistributing its dependencies, and most of their
licenses require reproducing their text. `script/licences-tierces generer`
therefore gathers at build time:

- the Rust dependencies of `oxyn-desktop`, with `cargo-about`, without build or
  development dependencies;
- the production npm dependencies of `apps/desktop`, with `pnpm licenses list`.

The result is `apps/desktop/src/generated/third-party-licenses.json`, outside
git, which the front end embeds and the settings display in the *About*
section. A release build (`make desktop PROFIL=release`) fails without this
file. `make licences-npm` refuses an npm license outside the list, like
`make deny` for Rust. Both checks rely on the same list, that of `deny.toml`.

## Consequences

* **+** A closed fork of the application is no longer possible: whoever
  distributes a modified version must publish its source under the GPL.
* **+** The copyright holder keeps the freedom to relicense, to sell a
  commercial license or to transfer the rights to the company. The CLA extends
  that freedom to external contributions.
* **+** A driver or plugin author is not contaminated: they link Apache-2.0
  code and choose their own license, closed included.
* **+** The business model requires restricting no feature, and contradicts
  neither "Open by default" nor "Privacy first".
* **−** The four Apache crates contain a real part of the product: the driver
  traits, the catalog model, the result buffer. A third party can take them
  into a closed product. It is the price of opening up to drivers.
* **−** The boundary moves with the code. A core crate that becomes a
  dependency of `oxyn-driver` must move to Apache, which requires holding all
  its rights. Otherwise, the dependency must be removed.
* **−** A CLA discourages some contributors, especially when it allows
  relicensing. It is the condition of the business model, and it is accepted.
* **−** Distributing a GPL binary requires offering its source. As long as the
  repository is private, no binary can be distributed outside the holder.
* **−** Every build now depends on `cargo-about` to produce the third-party
  notices. Without the tool, a development build warns and displays without
  notices. A release build fails.

**Exit cost:** as long as the holder holds the rights to all the code,
changing the license only requires their files: manifests, `LICENSE-*`,
`NOTICE`, `deny.toml`. The CLA guarantees this remains true after the first
contributions. But a version already published under the GPL stays so for
those who received it. It is therefore possible to restrict the following
versions, never the previous ones.

**Reconsider if** one of the following conditions is met:

- the company is created and the rights are transferred to it: `NOTICE`, the
  CLA and the bundle's `copyright` field change holder;
- a cloud provider resells the application as a service without giving
  anything back, which a network license such as the AGPL would prevent;
- a driver author must link a crate that is not in the Apache set.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Apache-2.0 alone, the previous license | a closed fork becomes possible, and it is final: every published version allows it forever |
| FSL or BSL, source-available converted to a free license after a delay | it is no longer open source in the OSI sense during the delay. That contradicts "Open by default", and an audience of data professionals reads the difference |
| Dual AGPL and commercial license | the AGPL targets whoever serves the software over a network, but Oxyn is a desktop application that serves no one. The only server is that of the paid services, which stays outside the repository |
| GPL for everything, driver contract included | every third-party driver or plugin would become GPL. The driver ecosystem sought by [ADR-0003](0003-driver-capabilities.md) and [ADR-0005](0005-wasm-plugins.md) would be closed to proprietary database vendors |
| Declare `GPL-3.0-or-later` in the `allow` of `deny.toml` | it would also accept a third-party GPL dependency, without a trade-off. `publish = false` exempts our crates and them alone |
| No CLA, only a DCO (*Developer Certificate of Origin*) | a DCO certifies a contribution's origin, not a license to the holder. It allows neither relicensing nor transferring the rights |
| Restrict local features (open core) | that contradicts "Open by default". The restricted code would also become a second repository to maintain, under another license |
