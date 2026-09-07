#!/usr/bin/env python3
"""Validate the local Codex files without starting agents or MCP servers."""

from pathlib import Path
import re
import sys
import tomllib


ROOT = Path(__file__).resolve().parents[1]
DIRECTORY = ROOT / ".codex"


def main() -> int:
    errors = []
    names = set()
    files = sorted(DIRECTORY.rglob("*.toml"))
    for path in files:
        try:
            config = tomllib.loads(path.read_text(encoding="utf-8"))
        except tomllib.TOMLDecodeError as error:
            errors.append(f"{path.relative_to(ROOT)}: {error}")
            continue
        if path.parent != DIRECTORY / "agents":
            continue
        for key in ("name", "description", "developer_instructions"):
            if not isinstance(config.get(key), str) or not config[key].strip():
                errors.append(f"{path.name}: missing non-empty string {key}")
        name = config.get("name")
        if not isinstance(name, str):
            continue
        if name in names or name != path.stem:
            errors.append(f"{path.name}: duplicate name or filename mismatch")
        names.add(name)
        guide = ROOT / ".claude" / "agents" / f"{name}.md"
        instructions = config.get("developer_instructions", "")
        if not guide.is_file() or str(guide.relative_to(ROOT)) not in instructions:
            errors.append(f"{path.name}: missing shared specialty guide")
        if ".Codex/" in instructions:
            errors.append(f"{path.name}: invalid .Codex/ reference")
        if name.startswith("relecteur-") or name == "detecteur-divergence":
            if config.get("sandbox_mode") != "read-only":
                errors.append(f"{path.name}: reviewer must default to read-only")

    for path in DIRECTORY.rglob("*.md"):
        for target in re.findall(r"\[[^\]]+\]\(([^)]+)\)", path.read_text()):
            if target.startswith(("https://", "http://", "mailto:", "#")):
                continue
            if not (path.parent / target.split("#", 1)[0]).exists():
                errors.append(f"{path.name}: broken link {target}")

    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"Codex files OK: {len(files)} TOML files, {len(names)} agent profiles.")
    print("Local structure only; runtime schema and service access are not tested.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
