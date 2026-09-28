#!/usr/bin/env python3
"""Compare the versions cited in docs/RESEARCH-NOTES.md with the registry.

This is not a hook: it is the tool that makes invariant I-12 practical. It
modifies nothing — deciding on an upgrade belongs to a human.

    python3 .claude/hooks/verifier_versions.py
"""

from __future__ import annotations

import json
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
NOTES = REPO_ROOT / "docs" / "RESEARCH-NOTES.md"
AGENT = "oxyn-verification-versions"
STABLE_CHANNEL = "https://static.rust-lang.org/dist/channel-rust-stable.toml"


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


if __name__ == "__main__":
    sys.exit(main())
