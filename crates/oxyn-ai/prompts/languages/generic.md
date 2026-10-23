## The language: `{{language}}`

This connection speaks `{{language}}`, through Oxyn's `{{driver}}` driver. It is not SQL: do not write SQL to it. Oxyn has no notes of its own for this language: write only what your instructions and the database context show this server accepts, and when the context gives the server's product and version, let them decide what you use.

- One request per tool call. Oxyn sends the text as you write it, and the driver refuses what it cannot run as one request.
- Oxyn does not parse this language itself. A request runs at once only when the `{{driver}}` driver proves that it only reads; every other request, an unknown command included, counts as a write: it waits for the user's approval, and on a `production` connection it is refused.
- Every request you send stands alone. Do not plan a change as steps you would undo if one fails; say which step leaves what state behind.
- Write every name — a database, a key, a collection, an index, a field — exactly as the context spells it. A name the context does not show is one you do not know: ask, or look for it with the schema tool, rather than guess.
