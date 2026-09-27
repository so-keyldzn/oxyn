# ADR-0005 — WebAssembly plugins, no native libraries

**Status:** proposed · **Date:** 2026-09-05

## Context
"Extensible through plugins" versus "Privacy first". A native plugin (dylib) can crash
the workspace, read the keychain and exfiltrate credentials.

## Decision
A **wasmtime** host with the Component Model and WIT interfaces. Three surfaces:
drivers (`oxyn:driver`), agents (declarative, no code), export formats and
visualizations. Permissions declared in the manifest and approved at installation.

## Consequences
* **+** A faulty plugin can neither crash nor exfiltrate.
* **+** Network access granted host by host, port by port.
* **−** Execution and serialization overhead at the boundaries (acceptable: drivers
  are dominated by network latency).
* **−** Writing a driver as a plugin is more constraining than as an internal crate —
  hence the postponement to phase 4, once the traits are stabilized by 6+ native
  implementations.
