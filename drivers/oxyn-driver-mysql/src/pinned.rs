//! The session settings the driver pins, and the typed `SET` it refuses.
//!
//! Every connection opens with `SET time_zone = '+00:00'` and `SET NAMES
//! utf8mb4` (ADR-0050 §6), and the decoders rest on them: a `TIMESTAMP` is
//! tagged UTC without looking at the value, and a text is read as `utf8mb4`.
//! A user's `SET time_zone = 'Europe/Paris'` would leave the tag lying — values
//! two hours off, with nothing on screen to say so — and a reconnect would
//! silently flip them back, since the setup runs again. `SET NAMES latin1` does
//! the same to text. The statement is therefore refused before anything is
//! sent, like a typed `USE` ([`crate::session`]), and the message names the way
//! out.
//!
//! # What it covers
//!
//! The statements of the text, split as the server would, that **lead** with
//! `SET`: `SET NAMES`, `SET CHARACTER SET`, `SET CHARSET`, and an assignment
//! list one of whose **targets** is in [`PINNED`]. The list is parsed far
//! enough to tell a target from its value — an assignment ends at a top-level
//! comma, its target at its first `=` or `:=` — so reading a pinned variable on
//! the right-hand side (`SET @utc = @@session.time_zone = '+00:00'`) is not
//! refused. A target is named whatever its scope (`SET GLOBAL time_zone` is
//! refused too: over-reading only costs a refusal) and whatever its quoting
//! (`` @@`session`.`time_zone` ``); a user variable (`@time_zone`) is not one.
//!
//! The text is read twice, as [`oxyn_query::changes_context`] reads it: as
//! typed, and as the server runs it, where the body of an executable comment
//! (`/*!40103 SET TIME_ZONE='+00:00' */`) is code. A `mysqldump` file opens
//! and closes with those, so its prologue and epilogue are refused: they only
//! save and restore the session settings, and the dump runs without them.
//!
//! # What it cannot cover
//!
//! A text the driver cannot read is not read: the body of a `CALL`ed
//! procedure, a dynamic `PREPARE … FROM @sql` or MariaDB's `EXECUTE
//! IMMEDIATE`, whose statement is built at run time, change the setting behind
//! the refusal. So does a text lexed under `NO_BACKSLASH_ESCAPES`, where a
//! string the check reads as running on (`'a\'`) ends earlier for the server.

use oxyn_core::SqlDialect;
use oxyn_query::{Token, TokenKind};

/// The system variables a decoder rests on.
///
/// `time_zone` is the `TIMESTAMP` tag; the three character-set variables decide
/// how text is read back and how the driver's own bound text is read by the
/// server.
const PINNED: [&str; 5] = [
    "time_zone",
    "character_set_results",
    "character_set_client",
    "character_set_connection",
    "collation_connection",
];

/// Does a statement of the text re-point a setting the driver pinned?
pub(crate) fn changes_pinned_setting(text: &str) -> bool {
    let any_statement = |text: &str| {
        oxyn_query::split(text, SqlDialect::MySql)
            .iter()
            .any(|fragment| statement_changes_pinned_setting(fragment.text))
    };
    any_statement(&oxyn_query::runnable_text(text, SqlDialect::MySql)) || any_statement(text)
}

fn statement_changes_pinned_setting(statement: &str) -> bool {
    let tokens = oxyn_query::tokens(statement, SqlDialect::MySql);
    let Some((verb, rest)) = tokens.split_first() else {
        return false;
    };
    if !is_word(verb, "set") {
        return false;
    }
    let is = |index: usize, expected: &str| rest.get(index).is_some_and(|t| is_word(t, expected));
    if is(0, "names") || is(0, "charset") || (is(0, "character") && is(1, "set")) {
        return true;
    }
    // MariaDB's `SET STATEMENT a = 1, b = 2 FOR SELECT …`: the list is the
    // same, and the statement after `FOR` is not part of it.
    let list = if is(0, "statement") && !rest.get(1).is_some_and(starts_assignment) {
        let list = rest.get(1..).unwrap_or_default();
        list.iter()
            .position(|t| is_word(t, "for"))
            .and_then(|end| list.get(..end))
            .unwrap_or(list)
    } else {
        rest
    };
    assignments(list).any(|target| names_pinned(target))
}

/// The targets of a `SET` assignment list: what precedes the first `=` or
/// `:=` of each assignment, assignments being separated by top-level commas.
///
/// A comma inside parentheses — `IF(a, b, c)`, a subquery — belongs to a value.
/// A piece with no `=` is its own target, read the same way: over-reading it
/// only costs a refusal of what the server would refuse anyway.
fn assignments<'t, 'a>(list: &'t [Token<'a>]) -> impl Iterator<Item = &'t [Token<'a>]> {
    let mut depth = 0_usize;
    list.split(move |token| {
        match token.text {
            "(" => depth = depth.saturating_add(1),
            ")" => depth = depth.saturating_sub(1),
            _ => {}
        }
        depth == 0 && token.text == ","
    })
    .map(|assignment| {
        let end = assignment
            .iter()
            .position(starts_assignment)
            .unwrap_or(assignment.len());
        assignment.get(..end).unwrap_or(assignment)
    })
}

/// `=`, or the `:` of `:=`. The scanner gives one symbol per byte, and the
/// target of an assignment never holds a `:`.
fn starts_assignment(token: &Token<'_>) -> bool {
    token.kind == TokenKind::Symbol && matches!(token.text, "=" | ":")
}

/// Does an assignment target name a pinned system variable?
///
/// `@name` is a user variable, whatever follows; anything else — `name`,
/// `SESSION name`, `@@name`, `@@session.name`, each part quoted or not — is a
/// system variable, and any of its names that is pinned counts.
fn names_pinned(target: &[Token<'_>]) -> bool {
    let at = |index: usize| target.get(index).is_some_and(|t| t.text == "@");
    if at(0) && !at(1) {
        return false;
    }
    target.iter().any(|token| {
        let name = match token.kind {
            TokenKind::Word => Some(token.text),
            TokenKind::Quoted => unquote(token.text),
            _ => None,
        };
        name.is_some_and(|name| {
            PINNED
                .iter()
                .any(|pinned| name.eq_ignore_ascii_case(pinned))
        })
    })
}

/// The name inside `` `…` `` or `"…"` (an identifier under `ANSI_QUOTES`).
///
/// An escaped quote inside is left as written: no pinned name holds one.
fn unquote(quoted: &str) -> Option<&str> {
    ["`", "\""].iter().find_map(|quote| {
        quoted
            .strip_prefix(quote)
            .and_then(|inner| inner.strip_suffix(quote))
    })
}

fn is_word(token: &Token<'_>, expected: &str) -> bool {
    token.kind == TokenKind::Word && token.text.eq_ignore_ascii_case(expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typed_set_of_a_pinned_setting_is_refused() {
        for text in [
            "SET time_zone = 'Europe/Paris'",
            "set TIME_ZONE='+02:00'",
            "SET SESSION time_zone = '+02:00'",
            "SET LOCAL time_zone = '+02:00'",
            "SET @@time_zone = 'UTC'",
            "SET @@session.time_zone = '+02:00'",
            "SET GLOBAL time_zone = '+02:00'",
            "SET PERSIST time_zone = '+02:00'",
            "SET @@persist_only.time_zone = '+02:00'",
            "SET time_zone := '+02:00'",
            "SET time_zone /* now */ = '+02:00'",
            "SET time_zone = DEFAULT",
            "SET @a = 1, time_zone = '+02:00'",
            "SET @a = IF(1, 2, 3), time_zone = '+02:00'",
            "SET @a = (SELECT 1, 2), @@session.time_zone = '+02:00'",
            // Quoted targets name the same variable.
            "SET @@session.`time_zone` = '+02:00'",
            "SET @@`session`.`time_zone` = '+02:00'",
            "SET @@`time_zone` = '+02:00'",
            "SET `time_zone` = '+02:00'",
            "SET SESSION `TIME_ZONE` = '+02:00'",
            "SET @@session . time_zone = '+02:00'",
            "SET \"time_zone\" = '+02:00'",
            "SET NAMES latin1",
            "set names 'latin1' collate latin1_bin",
            "SET CHARACTER SET latin1",
            "SET CHARSET latin1",
            "SET character_set_results = latin1",
            "SET character_set_client = latin1",
            "SET @@character_set_connection = latin1",
            "SET collation_connection = latin1_bin",
            "SET STATEMENT time_zone = '+02:00' FOR SELECT ts FROM t",
            // A second statement does not hide it.
            "SELECT 1; SET time_zone = '+02:00'",
            "/* x */ SET time_zone = '+02:00'",
            "-- zone\nSET NAMES latin1",
            // The server runs the body of an executable comment.
            "/*!50000 SET time_zone = '+02:00' */",
            "/*! SET time_zone = '+02:00' */",
            "/*M!100000 SET NAMES latin1 */",
            "SELECT 1 /*!50000 ; SET time_zone = '+02:00' */",
            "SET @a = 1 /*!50000 , time_zone = '+02:00' */",
            "/*!50000 SET @@session.`time_zone` = '+02:00' */",
            // A dump's prologue and epilogue re-point them too.
            "/*!40101 SET NAMES utf8mb4 */",
            "/*!40103 SET TIME_ZONE='+00:00' */",
            "/*!40103 SET TIME_ZONE=@OLD_TIME_ZONE */",
            "/*!40101 SET CHARACTER_SET_CLIENT=@OLD_CHARACTER_SET_CLIENT */",
        ] {
            assert!(changes_pinned_setting(text), "`{text}` must be refused");
        }
    }

    #[test]
    fn what_does_not_re_point_a_pinned_setting_is_not_refused() {
        for text in [
            "",
            "SET",
            "SELECT @@session.time_zone",
            "SELECT 'SET time_zone = 1'",
            "SET @tz = @@session.time_zone",
            "SET @tz := @@time_zone",
            "SET @time_zone = '+02:00'",
            "SET @`time_zone` = '+02:00'",
            "SET @'time_zone' = '+02:00'",
            "SET @names = 1",
            "SET sql_mode = ''",
            "SET SESSION TRANSACTION READ ONLY",
            "SET autocommit = 0",
            // A pinned variable read, or compared, on the right-hand side.
            "SET @is_utc = @@session.time_zone = '+00:00'",
            "SET @is_utc = @@session.`time_zone` = '+00:00'",
            "SET @a = 1, @b = @@time_zone = 'UTC', @c = time_zone = 1",
            "SET @a = IF(@@time_zone = '+00:00', 1, 0)",
            "SET @a = 'x, time_zone = 1'",
            "SET @a = (1, time_zone = 2)",
            "SET STATEMENT max_statement_time = 1 FOR UPDATE t SET time_zone = 'UTC'",
            "UPDATE t SET time_zone = 'UTC'",
            "UPDATE t SET names = 'a', time_zone = 'b'",
            "SELECT CONVERT_TZ(ts, '+00:00', '+02:00') FROM t",
            "CALL p()",
            "CREATE PROCEDURE p() BEGIN SET time_zone = '+02:00'; END",
            "/*!40101 SET @OLD_CHARACTER_SET_CLIENT=@@CHARACTER_SET_CLIENT */",
            "/*!40103 SET @OLD_TIME_ZONE=@@TIME_ZONE */",
            "/* SET time_zone = 'UTC' */ SELECT 1",
            "-- SET time_zone = 'UTC'\nSELECT 1",
            "SELECT '/*!50000 SET time_zone = 1 */'",
        ] {
            assert!(!changes_pinned_setting(text), "`{text}` must be accepted");
        }
    }

    #[test]
    fn an_unterminated_comment_or_string_is_not_an_assignment() {
        assert!(!changes_pinned_setting("SET @a = time_zone /* = 1"));
        assert!(!changes_pinned_setting("SET @a = 'x, time_zone = 1"));
        assert!(!changes_pinned_setting("SET @a = (1, time_zone = 2"));
    }
}
