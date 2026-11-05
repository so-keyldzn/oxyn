# Translations

`i18n/fr/` holds French mirrors of English documents, at the same path —
`.claude/` becomes `claude/`, so that Claude Code does not load a mirror as a
second rule, command or agent. **English is authoritative**; a mirror decides
nothing ([ADR-0047](../docs/adr/0047-english-as-the-repository-language.md)).

Each mirror starts with:

```html
<!-- oxyn-translation source="CLAUDE.md" sha256="0123456789ab" -->
```

`sha256` is the first 12 hex digits of the SHA-256 of the English source when
the mirror was last translated. `make socle` refuses a mirror whose source has
changed since, and prints the expected value.

**When you edit an English document that has a mirror**: update the French
text, then the `sha256` in its header, in the same pull request. If you do not
write French, say so in the pull request: the maintainer updates the mirror
before merging.

Every Markdown document of the repository has a mirror, except:

- the third-party skills (`.claude/skills/`, `.agents/skills/` — the `oxyn-*`
  skills of `.agents/skills/` are ours and do have a mirror);
- the agent memories of `.claude/agent-memory/`, which are tooling notes;
- the audit reports of `.claude/audits/`, dated and written once;
- `.github/`, whose templates the forge renders as they are;
- `crates/`, whose agent definitions and prompts are runtime data;
- `CLA.md`, a legal text kept in English only;
- `i18n/` itself: the mirrors, and this note.

A new document gets its mirror in the same pull request. `make socle` refuses a
tracked Markdown document with neither a mirror nor a place in this list; the
list it reads is `MIRROR_EXEMPT` in
[`.claude/verifier_socle.py`](../.claude/verifier_socle.py).
