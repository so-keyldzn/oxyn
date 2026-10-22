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
//! `SET` and assign one of [`PINNED`], whatever the scope — `SET GLOBAL
//! time_zone` is refused too, since over-reading only costs a refusal —, and
//! `SET NAMES` and `SET CHARACTER SET`. It reads bare words: a quoted name, a
//! comment and a user variable (`@time_zone`) are not code. Reading a pinned
//! variable (`SET @tz = @@session.time_zone`, `SELECT @@time_zone`) is not
//! refused.
//!
//! # What it cannot cover
//!
//! A text the driver cannot read is not read. The body of a `CALL`ed procedure,
//! a dynamic `PREPARE … FROM 'SET time_zone = …'` or MariaDB's `EXECUTE
//! IMMEDIATE`, and a quoted system variable (`` @@session.`time_zone` ``) change
//! the setting behind the refusal. Neither does it read the body of an
//! executable comment (`/*!40103 SET TIME_ZONE='+00:00' */`): `mysqldump` opens
//! and closes every dump with those, pinning the value the session already has
//! or restoring the one it captured, and refusing them would refuse every dump
//! replayed in the editor.

use oxyn_core::SqlDialect;

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
    oxyn_query::split(text, SqlDialect::MySql)
        .iter()
        .any(|fragment| statement_changes_pinned_setting(fragment.text))
}

fn statement_changes_pinned_setting(statement: &str) -> bool {
    let words = oxyn_query::words(statement, SqlDialect::MySql);
    let is = |index: usize, expected: &str| {
        words
            .get(index)
            .is_some_and(|word| word.text.eq_ignore_ascii_case(expected))
    };
    if !is(0, "set") {
        return false;
    }
    let verb_is_not_a_variable = words
        .get(1)
        .is_some_and(|word| !is_user_variable(statement, word.span.start));
    if verb_is_not_a_variable && (is(1, "names") || (is(1, "character") && is(2, "set"))) {
        return true;
    }
    words.iter().skip(1).any(|word| {
        PINNED
            .iter()
            .any(|name| word.text.eq_ignore_ascii_case(name))
            && !is_user_variable(statement, word.span.start)
            && is_assigned(statement, word.span.end)
    })
}

/// `@name` is a user variable, `@@name` a system one.
fn is_user_variable(statement: &str, start: usize) -> bool {
    let before = statement.as_bytes().get(..start).unwrap_or_default();
    before
        .iter()
        .rev()
        .take_while(|byte| **byte == b'@')
        .count()
        == 1
}

/// Is the word at `end` the target of an assignment, `=` or `:=`?
///
/// Skips blanks and `/* … */`, because `time_zone /* x */ = 1` is the same
/// statement; what follows is a read (`SET @tz = @@time_zone`) otherwise.
fn is_assigned(statement: &str, end: usize) -> bool {
    let mut rest = statement.as_bytes().get(end..).unwrap_or_default();
    loop {
        match rest {
            [byte, tail @ ..] if byte.is_ascii_whitespace() => rest = tail,
            [b'/', b'*', tail @ ..] => {
                let Some(close) = tail.windows(2).position(|pair| pair == b"*/") else {
                    return false;
                };
                rest = tail.get(close + 2..).unwrap_or_default();
            }
            [b'=', ..] | [b':', b'=', ..] => return true,
            _ => return false,
        }
    }
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
            "SET time_zone := '+02:00'",
            "SET time_zone /* now */ = '+02:00'",
            "SET @a = 1, time_zone = '+02:00'",
            "SET NAMES latin1",
            "set names 'latin1' collate latin1_bin",
            "SET CHARACTER SET latin1",
            "SET character_set_results = latin1",
            "SET character_set_client = latin1",
            "SET @@character_set_connection = latin1",
            "SET collation_connection = latin1_bin",
            "SET STATEMENT time_zone = '+02:00' FOR SELECT ts FROM t",
            // A second statement does not hide it.
            "SELECT 1; SET time_zone = '+02:00'",
            "/* x */ SET time_zone = '+02:00'",
            "-- zone\nSET NAMES latin1",
        ] {
            assert!(changes_pinned_setting(text), "`{text}` must be refused");
        }
    }

    #[test]
    fn what_does_not_re_point_a_pinned_setting_is_not_refused() {
        for text in [
            "",
            "SELECT @@session.time_zone",
            "SELECT 'SET time_zone = 1'",
            "SET @tz = @@session.time_zone",
            "SET @time_zone = '+02:00'",
            "SET @names = 1",
            "SET sql_mode = ''",
            "SET SESSION TRANSACTION READ ONLY",
            "SET autocommit = 0",
            "SET @@session.`time_zone` = 'UTC'",
            "UPDATE t SET time_zone = 'UTC'",
            "UPDATE t SET names = 'a', time_zone = 'b'",
            "SELECT CONVERT_TZ(ts, '+00:00', '+02:00') FROM t",
            "CALL p()",
            "CREATE PROCEDURE p() BEGIN SET time_zone = '+02:00'; END",
            // The dump prologue and epilogue: refusing them would refuse
            // every dump replayed in the editor.
            "/*!40101 SET NAMES utf8mb4 */",
            "/*!40103 SET TIME_ZONE='+00:00' */",
            "/*!40103 SET TIME_ZONE=@OLD_TIME_ZONE */",
            "/*!40101 SET CHARACTER_SET_CLIENT=@OLD_CHARACTER_SET_CLIENT */",
            "-- SET time_zone = 'UTC'\nSELECT 1",
        ] {
            assert!(!changes_pinned_setting(text), "`{text}` must be accepted");
        }
    }

    #[test]
    fn an_unterminated_comment_is_not_an_assignment() {
        assert!(!changes_pinned_setting("SET @a = time_zone /* = 1"));
    }
}
