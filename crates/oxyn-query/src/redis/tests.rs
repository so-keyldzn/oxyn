use super::*;

#[test]
fn the_line_under_the_cursor_is_the_command() {
    let text = "GET a\nDEL b\nSET c 1";
    assert_eq!(current_command(text, 0), Ok(Some("GET a")));
    // Right after the last character of the first line.
    assert_eq!(current_command(text, 5), Ok(Some("GET a")));
    assert_eq!(current_command(text, 8), Ok(Some("DEL b")));
    assert_eq!(current_command(text, text.len()), Ok(Some("SET c 1")));
}

#[test]
fn a_blank_line_runs_nothing() {
    assert_eq!(current_command("GET a\n\nDEL b", 6), Ok(None));
    assert_eq!(current_command("GET a\n     \nDEL b", 9), Ok(None));
    assert_eq!(current_command("", 0), Ok(None));
}

#[test]
fn a_line_break_inside_quotes_belongs_to_the_value() {
    let text = "SET k \"two\nlines\"\nDEL b";
    assert_eq!(current_command(text, 12), Ok(Some("SET k \"two\nlines\"")));
    assert_eq!(current_command(text, text.len()), Ok(Some("DEL b")));
    let single = "SET k 'it\\'s\nfine'\nGET k";
    assert_eq!(current_command(single, 2), Ok(Some("SET k 'it\\'s\nfine'")));
}

#[test]
fn an_escaped_quote_does_not_close_the_argument() {
    let text = "SET k \"a\\\"\nb\"\nGET k";
    assert_eq!(current_command(text, 0), Ok(Some("SET k \"a\\\"\nb\"")));
}

#[test]
fn a_quote_never_closed_asks_for_a_selection() {
    let text = "GET a\nSET k \"open\nDEL b";
    assert!(matches!(
        current_command(text, 8),
        Err(QueryError::Selection { .. })
    ));
    // A command before the open quote is still found.
    assert_eq!(current_command(text, 2), Ok(Some("GET a")));
}

#[test]
fn a_cursor_inside_a_character_is_refused() {
    assert!(matches!(
        current_command("GET é", 5),
        Err(QueryError::Selection { .. })
    ));
}

#[test]
fn carriage_returns_stay_out_of_the_command() {
    assert_eq!(current_command("GET a\r\nDEL b", 1), Ok(Some("GET a")));
}

#[test]
fn every_credential_the_grammar_carries_is_redacted() {
    for (line, expected) in [
        ("AUTH hunter2", "AUTH <redacted>"),
        ("auth app hunter2", "auth app <redacted>"),
        (
            "HELLO 3 AUTH app hunter2 SETNAME x",
            "HELLO 3 AUTH app <redacted> SETNAME x",
        ),
        (
            "ACL SETUSER alan on >hunter2 +@read <old \"#c3ab8f\" !d1e2",
            "ACL SETUSER alan on ><redacted> +@read <<redacted> #<redacted> !<redacted>",
        ),
        (
            "CONFIG SET maxmemory 1gb requirepass hunter2 masterauth 'two words'",
            "CONFIG SET maxmemory 1gb requirepass <redacted> masterauth <redacted>",
        ),
        (
            "config set TLS-KEY-FILE-PASS hunter2",
            "config set TLS-KEY-FILE-PASS <redacted>",
        ),
        // Pairs shifted by a missing value or an unquoted value.
        (
            "CONFIG SET maxmemory requirepass hunter2",
            "CONFIG SET maxmemory requirepass <redacted>",
        ),
        (
            "CONFIG SET save 3600 1 requirepass hunter2",
            "CONFIG SET save 3600 1 requirepass <redacted>",
        ),
        (
            "MIGRATE h 6379 k 0 5000 AUTH hunter2",
            "MIGRATE h 6379 k 0 5000 AUTH <redacted>",
        ),
        (
            "MIGRATE h 6379 \"\" 0 5000 AUTH2 app hunter2 KEYS a b",
            "MIGRATE h 6379 \"\" 0 5000 AUTH2 app <redacted> KEYS a b",
        ),
    ] {
        assert_eq!(redact_secrets(line), expected, "{line}");
    }
}

/// What the server receives decides, not the text as written: every line
/// below runs as a credential command once decoded.
#[test]
fn a_credential_is_recognized_on_the_decoded_argv() {
    for (line, expected) in [
        // Escaped command names and keywords.
        ("\"A\\x55TH\" hunter2", "\"A\\x55TH\" <redacted>"),
        ("\"\\x61uth\" app 'hunter2'", "\"\\x61uth\" app <redacted>"),
        ("aUtH \"hunter 2\"", "aUtH <redacted>"),
        (
            "\"\\x41CL\" \"set\\x75ser\" u on \"\\x3ehunter2\"",
            "\"\\x41CL\" \"set\\x75ser\" u on ><redacted>",
        ),
        (
            "ACL SETUSER u \"\\x3ehunter2\" '<old' \"\\x23c3ab8f\"",
            "ACL SETUSER u ><redacted> <<redacted> #<redacted>",
        ),
        (
            "hello 3 \"\\x41UTH\" app \"hun\\x74er2\"",
            "hello 3 \"\\x41UTH\" app <redacted>",
        ),
        (
            "CONFIG \"S\\x45T\" \"requirep\\x61ss\" hunter2 MasterAuth \"x\" masteruser bob",
            "CONFIG \"S\\x45T\" \"requirep\\x61ss\" <redacted> MasterAuth <redacted> \
                 masteruser <redacted>",
        ),
        (
            "MIGRATE h 6379 k 0 5000 \"\\x61uth2\" app hunter2",
            "MIGRATE h 6379 k 0 5000 \"\\x61uth2\" app <redacted>",
        ),
        (
            "migrate h 6379 k 0 5000 Auth 'hunter2'",
            "migrate h 6379 k 0 5000 Auth <redacted>",
        ),
        // A quote glued to the next character is refused by the driver,
        // but kept in the history: the argument runs on.
        ("AUTH \"hun\"ter2", "AUTH <redacted>"),
        ("AUTH \"hunter2", "AUTH <redacted>"),
    ] {
        assert_eq!(redact_secrets(line), expected, "{line}");
    }
}

#[test]
fn an_escape_the_server_does_not_decode_stays_what_it_is() {
    // `sdssplitargs` has no octal escape: `"\076"` is `076`, not `>`,
    // and outside quotes a backslash is a byte — neither is a rule.
    for line in [
        "ACL SETUSER u on \"\\076hunter2\"",
        "ACL SETUSER u on \\x3ehunter2",
        "A\\x55TH hunter2",
    ] {
        assert_eq!(redact_secrets(line), line, "{line}");
    }
}

#[test]
fn a_command_without_credentials_is_kept_byte_for_byte() {
    for line in [
        "SET password hunter2",
        "GET \"a > b\"",
        "ACL WHOAMI",
        "CONFIG SET maxmemory 1gb",
        "EVAL \"return 1\" 0",
        "",
    ] {
        assert_eq!(redact_secrets(line), line);
    }
}

#[test]
fn each_line_of_a_console_is_redacted_as_its_own_command() {
    assert_eq!(
        redact_secrets("GET a\nAUTH hunter2\nCONFIG SET requirepass \"x\ny\""),
        "GET a\nAUTH <redacted>\nCONFIG SET requirepass <redacted>"
    );
}
