import { SQLDialect } from "@codemirror/lang-sql"

/**
 * ClickHouse's lexical rules, as its 26.3 server reads them
 * (docs/RESEARCH-NOTES.md, ClickHouse lexer): `\` escapes inside strings and
 * backtick names, `#` opens a line comment like `--`, `$tag$…$tag$` is a
 * string, and both backticks and double quotes quote a name.
 *
 * The keywords stay the standard SQL ones: ClickHouse's own (`ENGINE`,
 * `PREWHERE`, `FINAL`, `SETTINGS`…) are listed by the server in
 * `system.keywords`, and the editor will read them from there rather than
 * from a list copied here.
 */
export const ClickHouseSQL = SQLDialect.define({
  backslashEscapes: true,
  hashComments: true,
  doubleDollarQuotedStrings: true,
  identifierQuotes: '`"',
})
