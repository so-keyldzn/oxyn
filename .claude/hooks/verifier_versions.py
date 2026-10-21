#!/usr/bin/env python3
"""Check the versions of docs/RESEARCH-NOTES.md, against the registry and the manifests.

This is not a hook: it is the tool that makes invariant I-12 practical. It
modifies nothing — deciding on an upgrade belongs to a human.

    python3 .claude/hooks/verifier_versions.py

Notes against the crates.io registry (network). It reports and never fails: a
cited version may be frozen on purpose. This is the move of `/versions`.

    python3 .claude/hooks/verifier_versions.py --manifests

Manifests against the notes (no network). It fails when a version pinned in
the root `Cargo.toml` (`[workspace.dependencies]`) or in
`apps/desktop/package.json` (`dependencies`, `devDependencies` and the pnpm
version of `packageManager`) is not recorded in the notes. `make versions` runs
it, hence `make qualite` and the `controles` job of CI: a version bump travels
with its dated row (I-12), or the gate refuses it.

The two comparisons look in opposite directions and must not share a reader.
`cited_versions()` reads the `| crate | version |` rows of the notes to compare
them with the registry; for a crate held back on purpose, that cell is the
latest published version, not the pinned one. `--manifests` starts from what the
manifests pin.

How a pin is found in the notes, line by line. The pin `name` at `x.y.z` is
recorded when ONE line of the notes holds `name` between backticks and `x.y.z`
as a whole token:

- not glued to a longer version (`1.0` is not `1.0.5`) nor to a pre-release
  (`1.0` is not `1.0-rc.1`), with a trailing sentence period accepted;
- a `v` or `=` prefix is accepted (an exact Cargo pin `=0.1.9` is recorded as
  `0.1.9`);
- `path` dependencies (the workspace crates) carry no version and are skipped.

A pin that is not recorded is either DIFFERS (the name appears between
backticks, but never on a line with the pinned version: the status lists the
versions those lines cite) or MISSING (the name never appears between
backticks). A missing `packageManager` field is also a gap: CI reads the pnpm
version there and nowhere else.

The unit of matching is the line. A row that lists several packages cannot say
which version belongs to which name; the `Pinned manifest versions` table of the
notes therefore holds one package per row, and the rows elsewhere count too.
"""

from __future__ import annotations

import json
import re
import sys
import tomllib
import urllib.error
import urllib.request
from pathlib import Path
from typing import NamedTuple

REPO_ROOT = Path(__file__).resolve().parents[2]
NOTES = REPO_ROOT / "docs" / "RESEARCH-NOTES.md"
CARGO_TOML = REPO_ROOT / "Cargo.toml"
PACKAGE_JSON = REPO_ROOT / "apps" / "desktop" / "package.json"
AGENT = "oxyn-verification-versions"
STABLE_CHANNEL = "https://static.rust-lang.org/dist/channel-rust-stable.toml"

# What the status column shows of the versions a line cites: a name such as
# `tauri` appears on a dozen rows, and listing them all would hide the answer.
CITED_SHOWN = 6
CITED_VERSION = re.compile(r"(?<![\w.-])\d+(?:\.\d+)+")


class Pin(NamedTuple):
    manifest: str
    name: str
    version: str


def _http(url: str) -> str | None:
    http_request = urllib.request.Request(url, headers={"User-Agent": AGENT})
    try:
        with urllib.request.urlopen(http_request, timeout=15) as response:
            return response.read().decode("utf-8", errors="replace")
    except (urllib.error.URLError, OSError, TimeoutError):
        return None


def crate_version(item_name: str) -> str | None:
    raw = _http(f"https://crates.io/api/v1/crates/{item_name}")
    if not raw:
        return None
    try:
        return json.loads(raw).get("crate", {}).get("max_stable_version")
    except json.JSONDecodeError:
        return None


def stable_rust_version() -> str | None:
    raw = _http(STABLE_CHANNEL)
    if not raw:
        return None
    block = re.search(r"\[pkg\.rust\]\s*\nversion\s*=\s*\"([^\"]+)\"", raw)
    return block.group(1).split()[0] if block else None


def cited_versions() -> dict[str, str]:
    """The `| \\`crate\\` | \\`x.y.z\\` |` table rows of RESEARCH-NOTES."""
    if not NOTES.exists():
        return {}
    cited_items: dict[str, str] = {}
    for line in NOTES.read_text(encoding="utf-8").splitlines():
        m = re.match(r"\|\s*`([a-z0-9_-]+)`\s*\|\s*`([0-9][^`]*)`\s*\|", line)
        if m:
            cited_items[m.group(1)] = m.group(2)
    return cited_items


def cargo_pins() -> list[Pin]:
    with CARGO_TOML.open("rb") as handle:
        dependencies = tomllib.load(handle).get("workspace", {}).get("dependencies", {})
    pins = []
    for dependency, spec in dependencies.items():
        if isinstance(spec, dict):
            if "path" in spec:
                continue
            spec = spec.get("version")
        if isinstance(spec, str):
            pins.append(Pin("Cargo.toml", dependency, spec.removeprefix("=").strip()))
    return pins


def npm_pins() -> list[Pin]:
    package = json.loads(PACKAGE_JSON.read_text(encoding="utf-8"))
    pins = [
        Pin("package.json", dependency, version)
        for section in ("dependencies", "devDependencies")
        for dependency, version in package.get(section, {}).items()
    ]
    # `pnpm@11.1.2`, possibly followed by `+sha512.<hash>`, which is not part of
    # the version.
    manager, _, version = package.get("packageManager", "").rpartition("@")
    pins.append(Pin("package.json", manager or "packageManager", version.partition("+")[0]))
    return pins


def gap_status(pin: Pin, notes: list[str]) -> str | None:
    """`None` when the notes record the pin, otherwise why they do not."""
    if not pin.version:
        return "NO VERSION PINNED in the manifest"
    token = re.compile(rf"(?<![\w.-])[v=]?{re.escape(pin.version)}(?![\w-]|\.\d)")
    named = [line for line in notes if f"`{pin.name}`" in line]
    if not named:
        return "MISSING: the notes never name it"
    if any(token.search(line) for line in named):
        return None
    cited = list(dict.fromkeys(CITED_VERSION.findall(" ".join(named))))
    if not cited:
        return "DIFFERS: the notes name it without any version"
    more = ", ..." if len(cited) > CITED_SHOWN else ""
    return f"DIFFERS: the notes cite {', '.join(cited[:CITED_SHOWN])}{more}"


def check_manifests() -> int:
    try:
        notes = NOTES.read_text(encoding="utf-8").splitlines()
        pins = cargo_pins() + npm_pins()
    except (OSError, ValueError) as error:
        print(f"Cannot read the manifests or the notes: {error}", file=sys.stderr)
        return 1

    gaps = [(pin, status) for pin in pins if (status := gap_status(pin, notes))]
    if not gaps:
        print(f"{len(pins)} pinned versions are recorded in docs/RESEARCH-NOTES.md.")
        return 0

    name_width = max(len(pin.name) for pin, _ in gaps)
    version_width = max(len(pin.version) for pin, _ in gaps)
    print(f"{'manifest':<13} {'name':<{name_width}} {'pinned':<{version_width}} status")
    print("-" * (13 + name_width + version_width + 40))
    for pin, status in gaps:
        print(f"{pin.manifest:<13} {pin.name:<{name_width}} {pin.version:<{version_width}} {status}")
    print(
        f"\n{len(gaps)} of {len(pins)} pinned versions are not recorded in "
        "docs/RESEARCH-NOTES.md. Read each one in its registry (crates.io or npm) "
        "and add a dated row to the '### Pinned manifest versions' table, in the "
        "same commit as the manifest change (I-12, `/versions`). A pin held back "
        "on purpose says why in its row.",
    )
    return 1


def main() -> int:
    cited_items = cited_versions()
    if not cited_items:
        print("No version cited in docs/RESEARCH-NOTES.md.", file=sys.stderr)
        return 1

    gaps = 0
    print(f"{'crate':<14} {'cited':<14} {'registry':<14} status")
    print("-" * 56)
    for item_name, cited in sorted(cited_items.items()):
        upstream = crate_version(item_name)
        if upstream is None:
            print(f"{item_name:<14} {cited:<14} {'?':<14} registry unreachable")
            continue
        state = "up to date" if upstream == cited else "MISMATCH"
        if state == "MISMATCH":
            gaps += 1
        print(f"{item_name:<14} {cited:<14} {upstream:<14} {state}")

    stable = stable_rust_version()
    if stable:
        print(f"\nRust stable in the registry: {stable}")

    if gaps:
        print(
            f"\n{gaps} mismatch(es). This is not an error: a cited version "
            "may be deliberately frozen. Update docs/RESEARCH-NOTES.md with "
            "today's date, or justify the mismatch in place.",
        )
    return 0


def run(arguments: list[str]) -> int:
    if not arguments:
        return main()
    if arguments == ["--manifests"]:
        return check_manifests()
    print("usage: verifier_versions.py [--manifests]", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(run(sys.argv[1:]))
