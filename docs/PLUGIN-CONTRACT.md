# Plugin contract

> **Authority**: what a plugin can do, what it cannot, and what the host
> guarantees it.

The mechanism is settled by [ADR-0005](adr/0005-wasm-plugins.md): **wasmtime**,
Component Model, **WIT** interfaces, permissions declared in the manifest and
approved at install time. Three surfaces: drivers (`oxyn:driver`), declarative
agents, export formats and visualizations.

Delivered in **phase 4**, once the traits have been stabilized by six or more
native implementations. This document exists before the code because it carries
constraints that must hold from the design of the traits — a trait drawn without
them will not cross the WASM boundary.

## What the sandbox already guarantees

[ADR-0005](adr/0005-wasm-plugins.md) settles by construction what, with native
libraries, would have required discipline: a faulty plugin can neither bring
down the workspace, nor read the keychain, nor open a network connection it was
not granted. This document does not repeat it.

## What the sandbox does not guarantee

Four constraints that no sandbox enforces for you.

### 1. A plugin goes through the command bus like everyone else

A plugin emits `Command`s ([ADR-0004](adr/0004-command-bus.md)). It gets no
handle to a driver, nor the list of open connections.

**Concrete failure:** a syntax-highlighting plugin, installed for its theme,
enumerates the connections and exfiltrates the production hosts. The sandbox
prevents it from opening a socket it was not granted, but nothing prevents it
from displaying the hosts in its own panel. The protection is not to give them
to it.

### 2. A plugin does not block the UI thread

A consequence of [I-05](../CLAUDE.md#i-05). A WASM component runs off the UI
thread, with a time and fuel limit. Without a limit, a plugin in an infinite
loop does not bring down the host — it freezes it, which amounts to the same
thing for the user.

### 3. The interface version is checked at load time

A plugin built against an earlier version of a WIT interface is **refused** with
a clear message, never loaded "to see". The Component Model makes the
incompatibility detectable: it still has to be treated as a refusal.

### 4. The boundary cost is paid in Arrow

Results cross in Arrow IPC ([ADR-0002](adr/0002-arrow-result-model.md)), not in
structures serialized field by field. A WASM driver that converts its batches
back loses most of what the columnar model brought.

## What this contract imposes on today's traits

This is the reason this document exists during phases 0 to 3. An `oxyn-driver`
trait that cannot cross the WASM boundary will close the door on
[ADR-0005](adr/0005-wasm-plugins.md) without anyone noticing before
phase 4:

- no generic type that cannot be resolved at the boundary;
- no synchronous callback from the plugin to the host outside the WIT interfaces;
- no implicit shared state between the host and the implementation;
- every error expressible as a value, never as a panic crossing the boundary.

## What remains to be settled

- the compatibility policy of WIT interfaces between Oxyn versions;
- the distribution and origin verification of plugins;
- the format and granularity of the permissions manifest.

To be written with [`/adr`](../.claude/commands/adr.md).
