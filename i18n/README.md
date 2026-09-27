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

Every Markdown document of the repository has a mirror, except the
third-party skills of `.claude/skills/` and the agent memories of
`.claude/agent-memory/`, which are tooling notes. A new document gets its
mirror in the same pull request.
