#!/usr/bin/env python3
"""Checks the coherence of the steering foundation.

This check exists because the foundation has a failure mode of its own: it
degrades silently. A dead link, a rule whose `paths:` no longer matches
anything, an invariant cited nowhere — nothing fails, and the foundation
becomes decorative without anyone noticing.

Called by `make socle`, hence by `make qualite`.
"""

from __future__ import annotations

import hashlib
import re
import sys
import tomllib
import unicodedata
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
CLAUDE_MD = REPO_ROOT / "CLAUDE.md"

# I-08: the only crate allowed to know Tauri (ADR-0029). The prefix covers
# `tauri-build` and the `tauri-plugin-*`s, which pull `tauri` with them.
TAURI_DIRECTORY = "oxyn-desktop"

# The invariants are anchored in CLAUDE.md by <a id="i-NN"></a>.
ANCHOR_PATTERN = re.compile(r'<a id="(i-\d+)"></a>')
# The path is empty for a link to a section of the same file: `(#title)`.
LINK_PATTERN = re.compile(r"\[[^\]]+\]\(([^)#]*)(#[^)]+)?\)")

# A template contains placeholders, written as links to show the expected
# shape: `[ADR-XXXX](XXXX-title.md)`. They are not dead links, they are holes.
# The templates' convention — `XXXX` for a number, `NNNN` for a title — serves
# as a marker, which avoids exempting `.claude/templates/` entirely and no
# longer checking its real links.
PLACEHOLDER_PATTERN = re.compile(r"XXXX|NNNN|AAAA-MM-JJ")


def _markdown_files() -> list[Path]:
    file_list = [CLAUDE_MD, REPO_ROOT / "AGENTS.md", REPO_ROOT / "README.md"]
    for directory in ("docs", ".claude", ".agents", "i18n"):
        file_list += sorted((REPO_ROOT / directory).rglob("*.md"))
    return [f for f in file_list if f.is_file()]


HEADING_PATTERN = re.compile(r"^#{1,6}\s+(.+?)(?:\s+#+)?\s*$")
HTML_ID_PATTERN = re.compile(r'<a\s+(?:id|name)="([^"]+)"')
FENCE_PATTERN = re.compile(r"^\s*(```|~~~)")
INLINE_LINK_PATTERN = re.compile(r"!?\[([^\]]*)\]\([^)]*\)")


def _slug(heading: str) -> str:
    """The slug GitHub gives a heading, computed on its rendered text.

    Punctuation disappears without its neighboring spaces merging: "The
    “budgets”" keeps its two spaces, hence two hyphens. That is the detail a
    hand-written fragment misses most often.
    """
    # Odd segments are inline code: rendered as is, `<u8>` included.
    segments = heading.split("`")
    for i in range(0, len(segments), 2):
        prose = INLINE_LINK_PATTERN.sub(r"\1", segments[i])
        segments[i] = re.sub(r"<[^>]+>", "", prose).replace("*", "")
    body = "".join(segments)
    kept = (
        c for c in body.lower()
        if c in " -" or unicodedata.category(c)[0] in "LMN" or unicodedata.category(c) == "Pc"
    )
    return "".join(kept).replace(" ", "-")


def _anchors(file_path: Path) -> set[str]:
    """Headings (with the -1, -2 suffix of duplicates) and <a id>s of a file."""
    anchors: set[str] = set()
    seen: dict[str, int] = {}
    in_code = False
    for line in file_path.read_text(encoding="utf-8").splitlines():
        if FENCE_PATTERN.match(line):
            in_code = not in_code
            continue
        if in_code:
            continue
        anchors.update(HTML_ID_PATTERN.findall(line))
        heading = HEADING_PATTERN.match(line)
        if heading:
            slug = _slug(heading.group(1))
            n = seen.get(slug, 0)
            seen[slug] = n + 1
            anchors.add(slug if n == 0 else f"{slug}-{n}")
    return anchors


def check_links() -> list[str]:
    """A dead link in an authoritative document points to a rule that no
    longer exists: the reader concludes the rule is gone. A dead fragment is
    sneakier: the page opens, at the top, and the reader concludes the cited
    section does not exist or looks elsewhere."""
    problems = []
    anchors: dict[Path, set[str]] = {}
    for file_path in _markdown_files():
        body = file_path.read_text(encoding="utf-8")
        rel = file_path.relative_to(REPO_ROOT)
        for link in LINK_PATTERN.finditer(body):
            target = link.group(1).strip()
            if target.startswith(("http://", "https://", "mailto:")):
                continue
            if PLACEHOLDER_PATTERN.search(target):
                continue
            if not target and not link.group(2):
                continue
            resolved = (file_path.parent / target).resolve() if target else file_path.resolve()
            if not resolved.exists():
                problems.append(f"{rel}: dead link to \"{target}\"")
                continue
            fragment = link.group(2)
            if not fragment or resolved.suffix != ".md":
                continue
            if resolved not in anchors:
                anchors[resolved] = _anchors(resolved)
            if fragment[1:] not in anchors[resolved]:
                problems.append(
                    f"{rel}: dead anchor \"{target}{fragment}\" — no heading "
                    "or <a id> matches it"
                )
    return problems


def check_invariants() -> list[str]:
    """An invariant without an anchor is an invariant no document can cite."""
    problems = []
    body = CLAUDE_MD.read_text(encoding="utf-8")
    anchors = set(ANCHOR_PATTERN.findall(body))
    if not anchors:
        return ["CLAUDE.md: no anchored invariant (<a id=\"i-NN\"></a>)"]

    cited_names: set[str] = set()
    for file_path in _markdown_files():
        for link in LINK_PATTERN.finditer(file_path.read_text(encoding="utf-8")):
            fragment = link.group(2)
            if fragment and fragment[1:].startswith("i-"):
                cited_names.add(fragment[1:])

    for missing in sorted(cited_names - anchors):
        problems.append(f"Invariant \"{missing}\" cited but not anchored in CLAUDE.md")
    for orphan in sorted(anchors - cited_names):
        problems.append(
            f"Invariant \"{orphan}\" defined but cited by no document: "
            "an invariant nothing ties to the code is a wish"
        )
    return problems


def check_rules() -> list[str]:
    """A rule without `paths:` loads at EVERY session, like CLAUDE.md, and ruins
    the context budget. It is the costliest and most invisible defect of the
    foundation."""
    problems = []
    directory = REPO_ROOT / ".claude" / "rules"
    if not directory.is_dir():
        return ["`.claude/rules/` is missing"]
    for rule in sorted(directory.glob("*.md")):
        body = rule.read_text(encoding="utf-8")
        if not body.startswith("---"):
            problems.append(f"{rule.name}: no frontmatter — the rule loads at every session")
            continue
        header = body.split("---", 2)[1]
        if "paths:" not in header:
            problems.append(f"{rule.name}: no `paths:` — the rule loads at every session")
    return problems


# ADR-0047: a French mirror records the fingerprint of the English text it
# translates. Without it, a rule fixed in English stays wrong in French, and a
# French reader applies the old rule without any error anywhere.
MIRRORS = "i18n/fr"
TRANSLATION_PATTERN = re.compile(
    r'<!-- oxyn-translation source="([^"]+)" sha256="([0-9a-f]{12})" -->'
)


def fingerprint(file_path: Path) -> str:
    return hashlib.sha256(file_path.read_bytes()).hexdigest()[:12]


def translation_errors(root: Path) -> list[str]:
    problems = []
    directory = root / MIRRORS
    if not directory.is_dir():
        return []
    for mirror in sorted(directory.rglob("*.md")):
        rel = mirror.relative_to(root)
        header = TRANSLATION_PATTERN.search(mirror.read_text(encoding="utf-8"))
        if not header:
            problems.append(
                f"{rel}: no `<!-- oxyn-translation source=\"…\" sha256=\"…\" -->` "
                "header — nothing tells whether this mirror is up to date"
            )
            continue
        source = root / header.group(1)
        if not source.is_file():
            problems.append(f"{rel}: source \"{header.group(1)}\" does not exist")
            continue
        expected_value = fingerprint(source)
        if header.group(2) != expected_value:
            problems.append(
                f"{rel}: stale French mirror — \"{header.group(1)}\" changed since "
                f"it was translated. Update the translation, then set "
                f'sha256="{expected_value}" (ADR-0047)'
            )
    return problems


def check_translations() -> list[str]:
    return translation_errors(REPO_ROOT)


def check_hooks() -> list[str]:
    """A non-executable hook is a hook that does not run, with no visible error."""
    problems = []
    directory = REPO_ROOT / ".claude" / "hooks"
    executables = {
        "code_interdit.py",
        "bash_interdit.py",
        "message_commit.py",
        "formater.py",
        "contexte_session.py",
        "rappel_qualite.py",
    }
    for item_name in sorted(executables):
        file_path = directory / item_name
        if not file_path.exists():
            problems.append(f"missing hook: {item_name}")
        elif not file_path.stat().st_mode & 0o111:
            problems.append(f"non-executable hook (chmod +x): {item_name}")
    return problems


def check_inert_paths() -> list[str]:
    """A warning, not an error: a `paths:` that matches no file will never
    trigger. That is the expected state as long as `crates/` is empty."""
    warnings = []
    directory = REPO_ROOT / ".claude" / "rules"
    for rule in sorted(directory.glob("*.md")):
        body = rule.read_text(encoding="utf-8")
        if not body.startswith("---"):
            continue
        patterns = re.findall(r'^\s*-\s*"([^"]+)"', body.split("---", 2)[1], re.MULTILINE)
        if patterns and not any(any(REPO_ROOT.glob(m)) for m in patterns):
            warnings.append(
                f"{rule.name}: no file matches its `paths:` — "
                "the rule is inert"
            )
    return warnings


def _manifests() -> list[Path]:
    """The manifests of the workspace members, in repository order."""
    file_list = []
    for directory in ("crates", "drivers"):
        base = REPO_ROOT / directory
        if base.is_dir():
            file_list += sorted(base.glob("*/Cargo.toml"))
    return file_list


def _dependencies(manifest: dict) -> list[tuple[str, str, object]]:
    """(section, name, declaration) for every declared dependency."""
    found_items = []
    for section in ("dependencies", "dev-dependencies", "build-dependencies"):
        for item_name, declaration in manifest.get(section, {}).items():
            found_items.append((section, item_name, declaration))
    return found_items


def check_dependency_graph() -> list[str]:
    """I-08 and manifest compliance, read by a machine rather than reviewed.

    These three defects share the same failure mode: the project compiles, the
    tests pass, clippy stays silent, and the cost only shows when it is too late
    to undo.

    - a `tauri*` outside `oxyn-desktop` permanently closes off the CLI, headless
      tests and the next interface change; `gpui`, removed with the old
      interface (ADR-0029), comes back nowhere;
    - a manifest without `[lints] workspace = true` removes ALL the
      repository's lints from its crate, `unsafe_code = "deny"` included;
    - a dependency declared with its own version brings two copies of the same
      crate into the graph, with mutually incompatible types.

    The check needs no Rust code: without a `Cargo.toml` at the root, it has
    nothing to say and says so by saying nothing.
    """
    if not (REPO_ROOT / "Cargo.toml").is_file():
        return []

    problems = []
    for path_str in _manifests():
        directory = path_str.parent.name
        rel = path_str.relative_to(REPO_ROOT)
        try:
            manifest = tomllib.loads(path_str.read_text(encoding="utf-8"))
        except tomllib.TOMLDecodeError as err:
            # A Python traceback here would suggest a defect of the check. The
            # manifest is unreadable: that is what must be shown.
            problems.append(f"{rel}: unreadable TOML — {err}")
            continue

        if manifest.get("lints", {}).get("workspace") is not True:
            problems.append(
                f"{rel}: no `[lints] workspace = true` — the crate escapes every "
                "lint of the repository, without anything reporting it"
            )

        if not manifest.get("package", {}).get("description"):
            problems.append(
                f"{rel}: no `description` — it is the only sentence that says "
                "why this crate exists on its own"
            )

        for section, item_name, declaration in _dependencies(manifest):
            if item_name == "gpui":
                problems.append(
                    f"I-08: {rel} depends on `gpui` in [{section}]. The GPUI "
                    "interface was removed in favor of Tauri (ADR-0029)"
                )
            if item_name.startswith("tauri") and directory != TAURI_DIRECTORY:
                problems.append(
                    f"I-08: {rel} depends on `{item_name}` in [{section}]. Only "
                    f"{TAURI_DIRECTORY} may (ADR-0029)"
                )
            inherited = isinstance(declaration, dict) and declaration.get("workspace")
            if not inherited:
                problems.append(
                    f"{rel}: `{item_name}` in [{section}] does not inherit from the workspace "
                    f"— write `{item_name}.workspace = true` and resolve the version "
                    "in the root Cargo.toml"
                )
    return problems


# `make qualite` and the CI workflow must say the same thing. CI calls the
# gate's targets in pieces, in parallel jobs: a target added to `qualite`
# without being added to the workflow would never run on a machine, and nothing
# would say so.
MAKEFILE = REPO_ROOT / "Makefile"
QUALITY_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "qualite.yml"
RULE_PATTERN = re.compile(r"^([A-Za-z][\w-]*)\s*:(?!=)\s*(.*)$")
SUB_MAKE_PATTERN = re.compile(r"\$\(MAKE\)((?:\s+[^\s;&|]+)+)")
CI_MAKE_PATTERN = re.compile(r"^\s*(?:-\s+)?run:\s*make\s+(.+)$")


def _makefile_rules() -> dict[str, tuple[list[str], bool]]:
    """Target → (targets it reaches, whether it carries a recipe that checks)."""
    rules: dict[str, tuple[list[str], bool]] = {}
    current_value: str | None = None
    for line in MAKEFILE.read_text(encoding="utf-8").splitlines():
        if line.startswith("\t") and current_value:
            reached, acceptance = rules[current_value]
            sub_commands = SUB_MAKE_PATTERN.search(line)
            if sub_commands:
                reached += [m for m in sub_commands.group(1).split() if not m.startswith("-")]
            else:
                acceptance = True
            rules[current_value] = (reached, acceptance)
            continue
        m = RULE_PATTERN.match(line)
        if m and not line.startswith("."):
            current_value = m.group(1)
            deps = [d for d in m.group(2).split() if "$" not in d]
            rules[current_value] = (deps, False)
        elif line and not line.startswith(("\t", "#", "ifeq", "else", "endif")):
            current_value = None
    return rules


def _reached(rules: dict[str, tuple[list[str], bool]], roots: list[str]) -> set[str]:
    views: set[str] = set()
    stack = list(roots)
    while stack:
        target = stack.pop()
        if target in views:
            continue
        views.add(target)
        stack += rules.get(target, ([], False))[0]
    return views


def check_ci_coverage() -> list[str]:
    if not QUALITY_WORKFLOW.is_file() or not MAKEFILE.is_file():
        return []
    rules = _makefile_rules()
    called: list[str] = []
    for line in QUALITY_WORKFLOW.read_text(encoding="utf-8").splitlines():
        m = CI_MAKE_PATTERN.match(line)
        if m:
            # `SHARD=${{ matrix.tranche }}/2`: the expression contains spaces.
            words = re.sub(r"\$\{\{.*?\}\}", "", m.group(1)).split()
            called += [c for c in words if "=" not in c and not c.startswith("-")]
    if not called:
        return [f"{QUALITY_WORKFLOW.relative_to(REPO_ROOT)}: no `run: make …` — CI does not pass the gate"]
    unknown = [c for c in called if c not in rules]
    covered = _reached(rules, called)
    # Only the targets that check something count: an aggregate like `front`
    # runs nothing itself, and `qualite` is the whole we compare against.
    forgotten = sorted(
        c for c in _reached(rules, ["qualite"]) - {"qualite"}
        if rules[c][1] and c not in covered
    )
    problems = [f"qualite.yml calls `make {c}`, a target missing from the Makefile" for c in unknown]
    problems += [
        f"qualite.yml does not cover `make {c}`, reached by `make qualite` — "
        "add it to a job, otherwise it never runs in CI"
        for c in forgotten
    ]
    return problems


# On a pull request, jobs are skipped depending on the areas touched, and
# branch protection only requires the `qualite` job, which aggregates the
# others (ADR-0045). A job that passes the gate without appearing in its
# `needs` could fail without blocking the merge.
AGGREGATE_JOB = "qualite"
JOB_PATTERN = re.compile(r"^  ([A-Za-z][\w-]*):\s*$")
NEEDS_PATTERN = re.compile(r"^    needs:\s*(.+?)\s*$")


def _workflow_jobs(body: str) -> dict[str, tuple[set[str], bool]]:
    """Job → (its `needs`, whether it calls `make`)."""
    jobs: dict[str, tuple[set[str], bool]] = {}
    in_jobs = False
    current: str | None = None
    for line in body.splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        if not line.startswith(" "):
            in_jobs = line.rstrip() == "jobs:"
            current = None
            continue
        if not in_jobs:
            continue
        m = JOB_PATTERN.match(line)
        if m:
            current = m.group(1)
            jobs[current] = (set(), False)
            continue
        if current is None:
            continue
        needs, make = jobs[current]
        n = NEEDS_PATTERN.match(line)
        if n:
            needs = set(re.findall(r"[\w-]+", n.group(1)))
        if CI_MAKE_PATTERN.match(line):
            make = True
        jobs[current] = (needs, make)
    return jobs


def aggregate_errors(body: str) -> list[str]:
    jobs = _workflow_jobs(body)
    if AGGREGATE_JOB not in jobs:
        return [
            f"qualite.yml has no `{AGGREGATE_JOB}` job: it is the one branch "
            "protection requires (ADR-0045)"
        ]
    expected_items = jobs[AGGREGATE_JOB][0]
    return [
        f"qualite.yml: job `{item_name}` calls `make` without appearing in the "
        f"`needs` of job `{AGGREGATE_JOB}` — its failure would not block the merge"
        for item_name, (_, make) in sorted(jobs.items())
        if make and item_name not in expected_items
    ]


def check_ci_aggregate() -> list[str]:
    if not QUALITY_WORKFLOW.is_file():
        return []
    return aggregate_errors(QUALITY_WORKFLOW.read_text(encoding="utf-8"))


def main() -> int:
    problems = (
        check_links()
        + check_invariants()
        + check_rules()
        + check_hooks()
        + check_dependency_graph()
        + check_ci_coverage()
        + check_ci_aggregate()
        + check_translations()
    )
    warnings = check_inert_paths()

    for message in warnings:
        print(f"warning        {message}")
    for message in problems:
        print(f"ERROR          {message}")

    if problems:
        print(f"\nFoundation: {len(problems)} error(s).")
        return 1
    print(f"\nFoundation coherent ({len(warnings)} warning(s)).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
