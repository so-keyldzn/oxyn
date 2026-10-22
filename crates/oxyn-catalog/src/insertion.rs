//! The text an editor inserts for a name the catalog gave it.
//!
//! A name comes from the server, so it is hostile input: `a"b` is a legal
//! table name, and so is `"users"; DROP TABLE audit; --`. A completion library
//! that wraps it in quotes without doubling the one inside produces text that
//! closes early ([I-10](../../../CLAUDE.md#i-10)). The quoting of Oxyn is
//! [`quote_identifier`], and this module is its only caller for what an editor
//! inserts, so that the interface never composes an identifier of its own
//! ([ADR-0057](../../../docs/adr/0057-editor-completion-reads-the-catalog-cache.md)).
//!
//! # Bare, or quoted
//!
//! A name goes in bare only when three things hold, and quoted otherwise:
//!
//! * it is made of `a-z`, `0-9` and `_`, and does not start with a digit —
//!   anything else (an upper-case letter, a space, a dot, an accent, a quote)
//!   would be read differently, or not at all, once unquoted;
//! * it is not a word the dialect reserves (the lists of the private `reserved`
//!   module): a table called `order` inserted bare is a statement the server
//!   rejects;
//! * the dialect is one whose reserved words are recorded. **A name not known to
//!   be safe is quoted**, which is also why an unknown dialect always quotes.

use oxyn_core::query::SqlDialect;

use crate::path::{QuoteStyle, quote_identifier};
use crate::reserved::needs_quoting;

/// The form of one name to insert into SQL for `dialect`.
///
/// Pure: it reads no catalog and no connection, and allocates the result.
/// Validates nothing — an empty name comes out as an empty pair of quotes, as
/// with [`quote_identifier`].
///
/// ```
/// use oxyn_catalog::insertion_form;
/// use oxyn_core::query::SqlDialect;
///
/// assert_eq!(insertion_form("users", SqlDialect::Postgres), "users");
/// // Reserved, so quoted.
/// assert_eq!(insertion_form("order", SqlDialect::Postgres), r#""order""#);
/// // PostgreSQL folds an unquoted name to lower case: this one must keep its case.
/// assert_eq!(insertion_form("Users", SqlDialect::Postgres), r#""Users""#);
/// assert_eq!(insertion_form("Users", SqlDialect::MySql), "`Users`");
/// // The quote inside is doubled, so the name cannot close the quoting early.
/// assert_eq!(insertion_form(r#"a"b"#, SqlDialect::Postgres), r#""a""b""#);
/// ```
#[must_use]
pub fn insertion_form(name: &str, dialect: SqlDialect) -> String {
    if is_simple(name) && needs_quoting(dialect, name) == Some(false) {
        return name.to_owned();
    }
    quote_identifier(name, QuoteStyle::for_dialect(dialect))
}

/// Whether showing `name` in an editor would show other text than the one that
/// runs.
///
/// Quoted, a name is still one identifier and the SQL stays intact; what changes
/// is what the eye reads. A line separator splits the suggestion over two lines,
/// a direction control reorders what follows it, a zero-width character makes
/// two names look the same. Such a name is not suggested — it stays reachable by
/// typing it, which is not a choice an editor should make for the user.
///
/// The set is the Unicode general categories `Cc` (control), `Cf` (format: soft
/// hyphen, zero-width and direction controls, the tag characters), `Zl` and
/// `Zp` (line and paragraph separators), plus the four fillers that render as
/// blank letters (`U+115F`, `U+1160`, `U+3164`, `U+FFA0`). The `Cf`, `Zl` and
/// `Zp` ranges are those of Unicode 16.0.0, listed here because the standard
/// library exposes `Cc` only: a character added to those categories by a later
/// Unicode version is not caught until the list is refreshed. Variation
/// selectors are not in it — they modify a glyph and hide none.
///
/// ```
/// use oxyn_catalog::hides_text;
///
/// assert!(!hides_text("café"));
/// assert!(hides_text("a\nb"));
/// // Right-to-left override: the rest of the line reads backwards.
/// assert!(hides_text("a\u{202E}b"));
/// // Zero-width space: two names that look the same.
/// assert!(hides_text("a\u{200B}b"));
/// ```
#[must_use]
pub fn hides_text(name: &str) -> bool {
    name.chars().any(|character| {
        character.is_control()
            || matches!(
                character,
                '\u{00AD}'
                    | '\u{0600}'..='\u{0605}'
                    | '\u{061C}'
                    | '\u{06DD}'
                    | '\u{070F}'
                    | '\u{0890}'..='\u{0891}'
                    | '\u{08E2}'
                    | '\u{115F}'
                    | '\u{1160}'
                    | '\u{180E}'
                    | '\u{200B}'..='\u{200F}'
                    | '\u{2028}'..='\u{202E}'
                    | '\u{2060}'..='\u{2064}'
                    | '\u{2066}'..='\u{206F}'
                    | '\u{3164}'
                    | '\u{FEFF}'
                    | '\u{FFA0}'
                    | '\u{FFF9}'..='\u{FFFB}'
                    | '\u{110BD}'
                    | '\u{110CD}'
                    | '\u{13430}'..='\u{1343F}'
                    | '\u{1BCA0}'..='\u{1BCA3}'
                    | '\u{1D173}'..='\u{1D17A}'
                    | '\u{E0001}'
                    | '\u{E0020}'..='\u{E007F}'
            )
    })
}

/// Lower-case ASCII letters, digits and `_`, not starting with a digit.
///
/// Narrower than what the engines accept bare, on purpose: `$` in PostgreSQL,
/// letters outside ASCII and upper case in a case-sensitive configuration all
/// quote. The cost is a pair of quotes; the alternative is a guess per engine.
fn is_simple(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z' | b'_'))
        && bytes.all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_name_goes_in_bare_where_the_dialect_is_known() {
        for dialect in [SqlDialect::Postgres, SqlDialect::MySql, SqlDialect::Sqlite] {
            assert_eq!(insertion_form("users", dialect), "users", "{dialect}");
            assert_eq!(insertion_form("_audit_2", dialect), "_audit_2", "{dialect}");
        }
    }

    #[test]
    fn a_reserved_word_is_quoted_in_the_style_of_its_dialect() {
        assert_eq!(insertion_form("order", SqlDialect::Postgres), r#""order""#);
        assert_eq!(insertion_form("user", SqlDialect::Postgres), r#""user""#);
        assert_eq!(insertion_form("order", SqlDialect::MySql), "`order`");
        assert_eq!(insertion_form("key", SqlDialect::Sqlite), r#""key""#);
        // Reserved by MariaDB only: the one MySQL driver serves both servers.
        assert_eq!(
            insertion_form("returning", SqlDialect::MySql),
            "`returning`"
        );
    }

    #[test]
    fn a_word_one_dialect_reserves_is_not_quoted_for_another() {
        // `rank` is a window function of MySQL; PostgreSQL takes it as a name.
        assert_eq!(insertion_form("rank", SqlDialect::Postgres), "rank");
        assert_eq!(insertion_form("rank", SqlDialect::MySql), "`rank`");
    }

    #[test]
    fn what_the_engine_would_read_differently_is_quoted() {
        let cases = [
            ("Users", "upper case"),
            ("my table", "a space"),
            ("a.b", "a dot"),
            ("1abc", "a leading digit"),
            ("café", "a letter outside ASCII"),
            ("price$", "a dollar sign"),
            ("", "nothing"),
        ];
        for (name, why) in cases {
            let form = insertion_form(name, SqlDialect::Postgres);
            assert!(
                form.starts_with('"') && form.ends_with('"'),
                "{why}: {form}"
            );
        }
    }

    #[test]
    fn a_hostile_name_stays_one_quoted_identifier() {
        let hostile = r#""users"; DROP TABLE audit; --"#;
        // The opening quote, then each of the name's two quotes doubled.
        assert_eq!(
            insertion_form(hostile, SqlDialect::Postgres),
            "\"\"\"users\"\"; DROP TABLE audit; --\""
        );
        // In backticks the double quotes are ordinary characters, and the
        // backtick is what gets doubled.
        assert_eq!(insertion_form("a`b", SqlDialect::MySql), "`a``b`");
    }

    #[test]
    fn a_name_that_shows_what_it_is_hides_nothing() {
        // Variation selectors and combining marks modify a glyph: they hide none.
        for name in [
            "users",
            "Users",
            "my table",
            "café",
            "日本語",
            r#"a"b"#,
            "a.b",
            "e\u{0301}",
            "star\u{FE0F}",
        ] {
            assert!(!hides_text(name), "{name}");
        }
    }

    #[test]
    fn a_name_that_shows_other_text_than_the_one_that_runs_is_caught() {
        let cases = [
            ("a\nb", "a line feed"),
            ("a\tb", "a tab"),
            ("a\u{0}b", "a NUL"),
            ("a\u{85}b", "a C1 control"),
            ("a\u{2028}b", "a line separator"),
            ("a\u{2029}b", "a paragraph separator"),
            ("a\u{200B}b", "a zero-width space"),
            ("a\u{200F}b", "a right-to-left mark"),
            ("a\u{202E}b", "a right-to-left override"),
            ("a\u{2066}b", "a directional isolate"),
            ("a\u{2069}b", "the end of an isolate"),
            ("a\u{2060}b", "a word joiner"),
            ("a\u{061C}b", "an Arabic letter mark"),
            ("\u{FEFF}a", "a byte-order mark"),
            ("a\u{00AD}b", "a soft hyphen"),
            ("a\u{2061}b", "an invisible function application"),
            ("a\u{206A}b", "a deprecated format control"),
            ("a\u{3164}b", "a Hangul filler"),
            ("a\u{115F}b", "a Hangul choseong filler"),
            ("a\u{E0041}b", "a tag letter"),
            ("a\u{FFF9}b", "an interlinear annotation anchor"),
        ];
        for (name, why) in cases {
            assert!(hides_text(name), "{why}");
        }
    }

    #[test]
    fn an_unknown_dialect_always_quotes() {
        // A name not known to be safe is quoted — including a plain one.
        for dialect in [
            SqlDialect::Ansi,
            SqlDialect::Redshift,
            SqlDialect::Snowflake,
        ] {
            assert_eq!(insertion_form("users", dialect), r#""users""#, "{dialect}");
        }
        assert_eq!(insertion_form("users", SqlDialect::SqlServer), "[users]");
    }
}
