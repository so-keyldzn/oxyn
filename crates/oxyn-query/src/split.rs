//! Découpage d'un texte SQL en instructions, et lecture de ses mots nus.
//!
//! Un éditeur contient un *lot* : plusieurs instructions séparées par des
//! points-virgules. Pour classer ce lot il faut d'abord le découper, et un
//! découpage naïf sur `';'` se trompe dès la première apostrophe :
//!
//! ```text
//! SELECT ';' ;                       -- un point-virgule dans une chaîne
//! SELECT 1; -- puis ; en commentaire
//! CREATE FUNCTION f() ... $$ BEGIN ... ; ... END $$;
//! ```
//!
//! Chacune de ces lignes est **une** instruction. Le scanner de ce module
//! reconnaît donc, sans construire d'AST : chaînes, identifiants cités,
//! commentaires `--`, `#`, `/* */`, et corps `$$ … $$` de PostgreSQL.
//!
//! # La direction de l'erreur
//!
//! Un scanner peut se tromper. Quand il se trompe, il **fusionne** : une chaîne
//! non fermée avale la fin du texte, un commentaire non fermé aussi. Le
//! fragment obtenu ne se lit plus, donc [`classify`](crate::classify()) le classe
//! [`Unknown`](oxyn_core::StatementIntent::Unknown), donc le `PolicyGate`
//! demande une approbation. Une erreur de découpage coûte une confirmation de
//! trop, jamais une écriture de moins.

use std::ops::Range;

use oxyn_core::SqlDialect;

use crate::{QueryError, validate};

/// Une instruction isolée dans un lot.
///
/// Le texte est **emprunté** au lot d'origine et rogné de ses espaces : c'est
/// exactement ce qu'on donne à l'analyseur, et [`span`](Self::span) permet de
/// remonter à l'endroit du buffer de l'éditeur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fragment<'a> {
    /// Texte de l'instruction, sans le point-virgule final ni les espaces de bord.
    pub text: &'a str,
    /// Bornes en octets de [`text`](Self::text) dans le lot d'origine.
    pub span: Range<usize>,
    /// Le fragment contient au moins un commentaire.
    ///
    /// Utilisé par [`format`](crate::format()) : le `Display` de l'AST de
    /// `sqlparser` ne conserve pas les commentaires, donc un fragment qui en
    /// porte n'est pas reformaté.
    pub has_comment: bool,
    /// Le fragment se terminait par un `;` explicite dans le lot.
    pub terminated: bool,
    /// A line comment in this fragment holds a lone `\r` followed by text, in a
    /// dialect whose lexer was not checked ([`LineCommentEnd::Unverified`]).
    ///
    /// Where that comment ends depends on the server, so which statements the
    /// fragment holds cannot be known: [`classify`](crate::classify()) makes it
    /// `Unknown` and [`validate`](crate::validate()) refuses it.
    pub unreadable_comment: bool,
}

/// Which bytes end a `--` (or `#`) comment, as the server's own lexer reads it.
///
/// Neither choice is safe by default. Ending too late hides a statement the
/// server runs — PostgreSQL runs `-- x\rDROP TABLE audit`. Ending too early
/// exposes a `/*` or a quote the server reads as comment text, and the
/// statement behind it vanishes into what the splitter takes for a block
/// comment — SQLite runs the `DROP` in `SELECT 1; -- x\r/*\nDROP TABLE audit -- */`.
/// Sources: docs/RESEARCH-NOTES.md, « Fin d'un commentaire `--` ».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LineCommentEnd {
    /// `\n` only: SQLite (`tokenize.c`).
    LineFeed,
    /// `\n` or `\r`: PostgreSQL (`scan.l`, `newline [\n\r]`).
    LineFeedOrCarriageReturn,
    /// Not checked against the server's lexer. The comment runs to `\n`, and a
    /// lone `\r` followed by anything but whitespace marks the fragment
    /// [`unreadable_comment`](Fragment::unreadable_comment).
    Unverified,
}

/// Un mot nu du texte : ni dans une chaîne, ni dans un identifiant cité, ni
/// dans un commentaire.
///
/// C'est la matière du filet de sécurité par mots-clés de
/// [`classify`](crate::classify()) : reconnaître `DELETE` dans
/// `SELECT * FROM t WHERE note = 'DELETE'` serait un faux positif, et ce type
/// existe pour que ce faux positif soit impossible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word<'a> {
    /// Le mot, tel qu'écrit (la casse est conservée).
    pub text: &'a str,
    /// Bornes en octets dans le texte d'origine.
    pub span: Range<usize>,
    /// Le mot est immédiatement suivi d'une parenthèse ouvrante : c'est un
    /// appel de fonction, pas un mot-clé d'instruction.
    ///
    /// `TRUNCATE(x, 2)` et `INSERT('abc', 1, 1, 'z')` sont des fonctions de
    /// MySQL ; les confondre avec `TRUNCATE TABLE` et `INSERT INTO` ferait
    /// demander une approbation pour un `SELECT`.
    pub call: bool,
}

/// Les particularités lexicales d'un dialecte.
///
/// Ce n'est pas une grammaire : uniquement ce qu'il faut savoir pour traverser
/// un texte sans le comprendre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplitProfile {
    /// `\` échappe le caractère suivant à l'intérieur d'une chaîne (MySQL,
    /// ClickHouse, Snowflake, BigQuery). PostgreSQL **non** : avec
    /// `standard_conforming_strings` actif, `'a\'` est une chaîne complète.
    pub backslash_escapes: bool,
    /// Un préfixe `E` active les échappements par `\` pour cette chaîne
    /// (`E'\''` de PostgreSQL).
    pub escape_string_prefix: bool,
    /// Les accents graves citent un identifiant (MySQL, ClickHouse, BigQuery).
    pub backtick_quotes: bool,
    /// Les crochets citent un identifiant (SQL Server, SQLite).
    pub bracket_quotes: bool,
    /// `$tag$ … $tag$` délimite un corps littéral (PostgreSQL, Redshift, DuckDB).
    pub dollar_quotes: bool,
    /// `#` ouvre un commentaire de fin de ligne (MySQL).
    pub hash_line_comments: bool,
    /// `/* /* */ */` s'imbrique (PostgreSQL, DuckDB, ClickHouse).
    pub nested_block_comments: bool,
    /// Where a line comment ends.
    pub line_comment_end: LineCommentEnd,
}

impl SplitProfile {
    /// Le profil d'un dialecte.
    ///
    /// Un dialecte inconnu reçoit le profil le plus **inclusif** : toutes les
    /// formes de citation sont reconnues. Reconnaître une citation qui n'existe
    /// pas dans le dialecte réel fusionne au pire deux instructions, ce qui
    /// mène à `Unknown` ; ne pas la reconnaître découperait au milieu d'une
    /// chaîne et produirait deux moitiés dont l'une pourrait se lire comme une
    /// instruction valide qu'on n'a jamais écrite.
    #[must_use]
    pub fn for_dialect(dialect: SqlDialect) -> Self {
        match dialect {
            SqlDialect::Postgres | SqlDialect::Redshift => Self {
                backslash_escapes: false,
                escape_string_prefix: true,
                backtick_quotes: false,
                bracket_quotes: false,
                dollar_quotes: true,
                hash_line_comments: false,
                nested_block_comments: true,
                line_comment_end: if matches!(dialect, SqlDialect::Postgres) {
                    LineCommentEnd::LineFeedOrCarriageReturn
                } else {
                    LineCommentEnd::Unverified
                },
            },
            SqlDialect::MySql => Self {
                backslash_escapes: true,
                escape_string_prefix: false,
                backtick_quotes: true,
                bracket_quotes: false,
                dollar_quotes: false,
                hash_line_comments: true,
                nested_block_comments: false,
                line_comment_end: LineCommentEnd::Unverified,
            },
            SqlDialect::Sqlite => Self {
                backslash_escapes: false,
                escape_string_prefix: false,
                backtick_quotes: true,
                bracket_quotes: true,
                dollar_quotes: false,
                hash_line_comments: false,
                nested_block_comments: false,
                line_comment_end: LineCommentEnd::LineFeed,
            },
            SqlDialect::SqlServer => Self {
                backslash_escapes: false,
                escape_string_prefix: false,
                backtick_quotes: false,
                bracket_quotes: true,
                dollar_quotes: false,
                hash_line_comments: false,
                nested_block_comments: true,
                line_comment_end: LineCommentEnd::Unverified,
            },
            SqlDialect::ClickHouse => Self {
                backslash_escapes: true,
                escape_string_prefix: false,
                backtick_quotes: true,
                bracket_quotes: false,
                dollar_quotes: false,
                hash_line_comments: true,
                nested_block_comments: true,
                line_comment_end: LineCommentEnd::Unverified,
            },
            SqlDialect::DuckDb => Self {
                backslash_escapes: false,
                escape_string_prefix: true,
                backtick_quotes: true,
                bracket_quotes: false,
                dollar_quotes: true,
                hash_line_comments: false,
                nested_block_comments: true,
                line_comment_end: LineCommentEnd::Unverified,
            },
            SqlDialect::Snowflake | SqlDialect::BigQuery => Self {
                backslash_escapes: true,
                escape_string_prefix: false,
                backtick_quotes: true,
                bracket_quotes: false,
                dollar_quotes: matches!(dialect, SqlDialect::Snowflake),
                hash_line_comments: false,
                nested_block_comments: false,
                line_comment_end: LineCommentEnd::Unverified,
            },
            // `Ansi`, `Oracle`, et toute valeur ajoutée plus tard : le profil
            // inclusif.
            _ => Self::permissive(),
        }
    }

    /// Le profil qui reconnaît toutes les formes de citation connues.
    #[must_use]
    pub const fn permissive() -> Self {
        Self {
            backslash_escapes: false,
            escape_string_prefix: true,
            backtick_quotes: true,
            bracket_quotes: true,
            dollar_quotes: true,
            hash_line_comments: false,
            nested_block_comments: true,
            line_comment_end: LineCommentEnd::Unverified,
        }
    }
}

impl Default for SplitProfile {
    fn default() -> Self {
        Self::permissive()
    }
}

/// Découpe un lot en instructions.
///
/// Les fragments vides — suite de `;;`, texte fait de seuls commentaires — sont
/// écartés : ils n'ont rien à exécuter. Un commentaire qui **précède** une
/// instruction lui reste attaché ; c'est bien son texte.
///
/// ```
/// use oxyn_core::SqlDialect;
/// use oxyn_query::split;
///
/// let lot = "SELECT ';' ; DELETE FROM t; -- ; pas un séparateur";
/// let fragments = split(lot, SqlDialect::Postgres);
/// assert_eq!(fragments.len(), 2);
/// assert_eq!(fragments[0].text, "SELECT ';'");
/// assert_eq!(fragments[1].text, "DELETE FROM t");
/// ```
#[must_use]
pub fn split(sql: &str, dialect: SqlDialect) -> Vec<Fragment<'_>> {
    split_with(sql, SplitProfile::for_dialect(dialect))
}

/// Retourne l'instruction complète sous le curseur, ou rien dans un séparateur.
///
/// Le curseur est un indice d'octet UTF-8. Une position au milieu d'un caractère,
/// dans un séparateur ou dans un espace hors instruction ne sélectionne rien. Le
/// fragment est validé avant d'être rendu : un texte incomplet ne peut pas être
/// exécuté comme « instruction courante ».
pub fn current_statement(
    sql: &str,
    dialect: SqlDialect,
    cursor_byte: usize,
) -> Result<Option<Fragment<'_>>, QueryError> {
    if cursor_byte > sql.len() || !sql.is_char_boundary(cursor_byte) {
        return Err(QueryError::Selection {
            cursor: cursor_byte,
            message: "cursor is not on a UTF-8 boundary".into(),
        });
    }
    let fragments = if dialect == SqlDialect::Sqlite {
        sqlite_statements(sql, cursor_byte)?
    } else {
        split(sql, dialect)
    };
    let fragment = fragments
        .iter()
        .find(|fragment| {
            (fragment.span.start <= cursor_byte && cursor_byte < fragment.span.end)
                || (fragment.span.end == sql.len() && cursor_byte == sql.len())
        })
        .filter(|fragment| {
            cursor_byte != fragment.span.start || !starts_with_comment(fragment.text, dialect)
        })
        .or_else(|| {
            fragments.iter().rev().find(|fragment| {
                fragment.terminated
                    && terminator_after(sql, fragment.span.end).is_some_and(|separator| {
                        cursor_byte == separator || cursor_byte == separator + 1
                    })
            })
        });
    let Some(fragment) = fragment else {
        return Ok(None);
    };
    if dialect == SqlDialect::Sqlite && is_sqlite_trigger(fragment.text) {
        if sqlite_trigger_is_ambiguous(fragment.text) {
            return Err(QueryError::Selection {
                cursor: cursor_byte,
                message: "SQLite trigger contains an ambiguous unquoted keyword".into(),
            });
        }
        if !words(fragment.text, SqlDialect::Sqlite)
            .last()
            .is_some_and(|word| word.text.eq_ignore_ascii_case("end"))
        {
            return Err(QueryError::Selection {
                cursor: cursor_byte,
                message: "SQLite trigger body is incomplete".into(),
            });
        }
    } else {
        validate(fragment.text, dialect).map_err(|error| match error {
            QueryError::Syntax { message, span } => QueryError::Syntax {
                message,
                span: fragment.span.start.saturating_add(span.start)
                    ..fragment.span.start.saturating_add(span.end),
            },
            other => other,
        })?;
    }
    Ok(Some(fragment.clone()))
}

fn starts_with_comment(sql: &str, dialect: SqlDialect) -> bool {
    sql.starts_with("--")
        || sql.starts_with("/*")
        || (dialect == SqlDialect::MySql && sql.starts_with('#'))
}

fn terminator_after(sql: &str, from: usize) -> Option<usize> {
    let tail = sql.get(from..)?;
    let whitespace = tail.len() - tail.trim_start().len();
    let position = from.checked_add(whitespace)?;
    (sql.as_bytes().get(position) == Some(&b';')).then_some(position)
}

fn is_sqlite_trigger(sql: &str) -> bool {
    let words = words(sql, SqlDialect::Sqlite);
    let Some(first) = words.first() else {
        return false;
    };
    if !first.text.eq_ignore_ascii_case("create") {
        return false;
    }
    let second = words.get(1).map(|word| word.text);
    let trigger = match second {
        Some(word)
            if word.eq_ignore_ascii_case("temp") || word.eq_ignore_ascii_case("temporary") =>
        {
            words.get(2).map(|word| word.text)
        }
        word => word,
    };
    trigger.is_some_and(|word| word.eq_ignore_ascii_case("trigger"))
}

fn sqlite_trigger_is_ambiguous(sql: &str) -> bool {
    let words = words(sql, SqlDialect::Sqlite);
    let Some(trigger) = words
        .iter()
        .position(|word| word.text.eq_ignore_ascii_case("trigger"))
    else {
        return false;
    };
    if words
        .get(trigger + 1)
        .is_some_and(|word| word.text.eq_ignore_ascii_case("begin"))
    {
        return true;
    }
    words
        .iter()
        .filter(|word| word.text.eq_ignore_ascii_case("case"))
        .any(|word| {
            sql.get(word.span.end..)
                .is_some_and(|tail| tail.trim_start().starts_with('='))
        })
}

fn sqlite_statements(sql: &str, cursor: usize) -> Result<Vec<Fragment<'_>>, QueryError> {
    let mut fragments = Vec::new();
    let mut start = 0usize;
    let mut has_comment = false;
    let mut has_code = false;
    let mut prefix = 0u8;
    let mut trigger = false;
    let mut seen_on = false;
    let mut relation_seen = false;
    let mut body = false;
    let mut body_statement_start = false;
    let mut ended = false;
    scan(
        sql,
        SplitProfile::for_dialect(SqlDialect::Sqlite),
        &mut |token, span| match token {
            // The SQLite profile ends comments at `\n`, as `tokenize.c` does:
            // an unreadable comment cannot occur here.
            Tok::Comment | Tok::UnreadableComment => has_comment = true,
            Tok::Word => {
                let word = sql.get(span.clone()).unwrap_or_default();
                let mut began_body = false;
                if !has_code {
                    prefix = u8::from(word.eq_ignore_ascii_case("create"));
                } else if prefix == 1 {
                    if word.eq_ignore_ascii_case("temp") || word.eq_ignore_ascii_case("temporary") {
                        prefix = 2;
                    } else if word.eq_ignore_ascii_case("trigger") {
                        prefix = 3;
                        trigger = true;
                    } else {
                        prefix = 0;
                    }
                } else if prefix == 2 {
                    if word.eq_ignore_ascii_case("trigger") {
                        prefix = 3;
                        trigger = true;
                    } else {
                        prefix = 0;
                    }
                }
                if trigger && word.eq_ignore_ascii_case("on") {
                    seen_on = true;
                } else if seen_on && !relation_seen {
                    relation_seen = true;
                } else if trigger && relation_seen && word.eq_ignore_ascii_case("begin") {
                    body = true;
                    body_statement_start = true;
                    began_body = true;
                } else if body
                    && !ended
                    && word.eq_ignore_ascii_case("end")
                    && body_statement_start
                    && end_closes_trigger(sql, span.end)
                {
                    ended = true;
                }
                if body && !ended && !began_body {
                    body_statement_start = false;
                }
                has_code = true;
            }
            Tok::Semicolon if !(body && !ended) => {
                if has_code {
                    let flags = Flags::new(has_comment, true, false);
                    push_fragment(sql, start..span.start, flags, &mut fragments);
                }
                start = span.end;
                has_comment = false;
                has_code = false;
                prefix = 0;
                trigger = false;
                seen_on = false;
                relation_seen = false;
                body = false;
                body_statement_start = false;
                ended = false;
            }
            Tok::Quoted | Tok::Symbol => {
                has_code = true;
                if body && !ended {
                    body_statement_start = false;
                }
            }
            Tok::Semicolon => body_statement_start = true,
        },
    );
    if trigger && !body {
        return Err(QueryError::Selection {
            cursor,
            message: "SQLite trigger body cannot be established".into(),
        });
    }
    if has_code {
        let flags = Flags::new(has_comment, false, false);
        push_fragment(sql, start..sql.len(), flags, &mut fragments);
    }
    Ok(fragments)
}

fn end_closes_trigger(sql: &str, from: usize) -> bool {
    let mut tail = sql.get(from..).unwrap_or_default().trim_start();
    loop {
        if let Some(rest) = tail.strip_prefix("--") {
            tail = rest
                .split_once('\n')
                .map_or("", |(_, after)| after)
                .trim_start();
        } else if let Some(rest) = tail.strip_prefix("/*") {
            let Some((_, after)) = rest.split_once("*/") else {
                return false;
            };
            tail = after.trim_start();
        } else {
            break;
        }
    }
    tail.is_empty() || tail.starts_with(';')
}

/// Découpe un lot avec un profil lexical explicite.
#[must_use]
pub fn split_with(sql: &str, profile: SplitProfile) -> Vec<Fragment<'_>> {
    let mut fragments = Vec::new();
    let mut start = 0usize;
    let mut has_comment = false;
    let mut has_code = false;
    let mut unreadable = false;

    scan(sql, profile, &mut |token, span| match token {
        Tok::Comment => has_comment = true,
        // Never dropped as « comments only »: what it hides may run.
        Tok::UnreadableComment => {
            has_comment = true;
            has_code = true;
            unreadable = true;
        }
        Tok::Semicolon => {
            if has_code {
                let flags = Flags::new(has_comment, true, unreadable);
                push_fragment(sql, start..span.start, flags, &mut fragments);
            }
            start = span.end;
            has_comment = false;
            has_code = false;
            unreadable = false;
        }
        Tok::Word | Tok::Quoted | Tok::Symbol => has_code = true,
    });

    if has_code {
        let flags = Flags::new(has_comment, false, unreadable);
        push_fragment(sql, start..sql.len(), flags, &mut fragments);
    }
    fragments
}

/// Le texte contient-il au moins un commentaire ?
///
/// Un commentaire qui n'appartient à aucune instruction — un lot fait d'un seul
/// `-- note` — n'apparaît dans aucun [`Fragment`] : cette fonction est le seul
/// moyen de savoir qu'il est là.
#[must_use]
pub fn contains_comment(sql: &str, dialect: SqlDialect) -> bool {
    let mut seen = false;
    scan(
        sql,
        SplitProfile::for_dialect(dialect),
        &mut |token, _span| {
            if matches!(token, Tok::Comment | Tok::UnreadableComment) {
                seen = true;
            }
        },
    );
    seen
}

/// Les mots nus du texte, dans l'ordre.
///
/// Tout ce qui est cité ou commenté est absent du résultat.
///
/// ```
/// use oxyn_core::SqlDialect;
/// use oxyn_query::split::words;
///
/// let mots = words("SELECT x /* DROP */ FROM 'DELETE'", SqlDialect::Ansi);
/// let textes: Vec<&str> = mots.iter().map(|m| m.text).collect();
/// assert_eq!(textes, ["SELECT", "x", "FROM"]);
/// ```
#[must_use]
pub fn words(sql: &str, dialect: SqlDialect) -> Vec<Word<'_>> {
    words_with(sql, SplitProfile::for_dialect(dialect))
}

/// Les mots nus du texte, avec un profil lexical explicite.
#[must_use]
pub fn words_with(sql: &str, profile: SplitProfile) -> Vec<Word<'_>> {
    let mut out: Vec<Word<'_>> = Vec::new();
    scan(sql, profile, &mut |token, span| {
        if token != Tok::Word {
            return;
        }
        let Some(text) = sql.get(span.clone()) else {
            return;
        };
        out.push(Word {
            text,
            span,
            call: false,
        });
    });

    let bytes = sql.as_bytes();
    for word in &mut out {
        word.call = next_visible_byte(bytes, word.span.end) == Some(b'(');
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Le scanner
// ─────────────────────────────────────────────────────────────────────────────

/// Ce que le scanner sait distinguer. Il ne comprend pas le SQL : il sait
/// seulement où il a le droit de regarder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    /// Un mot nu (mot-clé, identifiant non cité).
    Word,
    /// Une chaîne, un identifiant cité, un corps `$$ … $$`.
    Quoted,
    /// Un commentaire, quelle que soit sa forme.
    Comment,
    /// A line comment whose end depends on a lexer nobody checked
    /// ([`LineCommentEnd::Unverified`]): it may hold a statement, so it counts
    /// as code and makes its fragment unreadable.
    UnreadableComment,
    /// Le séparateur d'instructions.
    Semicolon,
    /// Tout le reste : ponctuation, opérateurs, nombres.
    Symbol,
}

/// Parcourt le texte une fois et signale chaque élément significatif.
///
/// Un seul scanner sert au découpage, à la détection des commentaires et à la
/// lecture des mots : trois copies de cette boucle finiraient par diverger, et
/// c'est celle qui garde le filet de sécurité qui divergerait.
fn scan(sql: &str, profile: SplitProfile, on: &mut dyn FnMut(Tok, Range<usize>)) {
    let b = sql.as_bytes();
    let mut i = 0usize;

    while let Some(&c) = b.get(i) {
        match c {
            b'-' if b.get(i + 1) == Some(&b'-') => {
                let (end, token) = skip_line_comment(b, i + 2, profile.line_comment_end);
                on(token, i..end);
                i = end;
            }
            b'#' if profile.hash_line_comments => {
                let (end, token) = skip_line_comment(b, i + 1, profile.line_comment_end);
                on(token, i..end);
                i = end;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let end = skip_block_comment(b, i, profile.nested_block_comments);
                on(Tok::Comment, i..end);
                i = end;
            }
            b'\'' => {
                let escapes = profile.backslash_escapes
                    || (profile.escape_string_prefix && has_escape_prefix(b, i));
                let end = skip_quoted(b, i, b'\'', escapes);
                on(Tok::Quoted, i..end);
                i = end;
            }
            b'"' => {
                let end = skip_quoted(b, i, b'"', profile.backslash_escapes);
                on(Tok::Quoted, i..end);
                i = end;
            }
            b'`' if profile.backtick_quotes => {
                let end = skip_quoted(b, i, b'`', false);
                on(Tok::Quoted, i..end);
                i = end;
            }
            b'[' if profile.bracket_quotes => {
                let end = skip_bracket(b, i);
                on(Tok::Quoted, i..end);
                i = end;
            }
            b'$' if profile.dollar_quotes => match skip_dollar_quoted(b, i) {
                Some(end) => {
                    on(Tok::Quoted, i..end);
                    i = end;
                }
                None => {
                    // Un `$` seul : emplacement de paramètre `$1`, ou opérateur.
                    on(Tok::Symbol, i..i + 1);
                    i += 1;
                }
            },
            b';' => {
                on(Tok::Semicolon, i..i + 1);
                i += 1;
            }
            _ if is_word_start(c) => {
                let end = skip_word(b, i);
                on(Tok::Word, i..end);
                i = end;
            }
            _ if c.is_ascii_whitespace() => i += 1,
            _ => {
                on(Tok::Symbol, i..i + 1);
                i += 1;
            }
        }
    }
}

/// Premier octet d'un mot nu. Les octets ≥ `0x80` en font partie : un
/// identifiant accentué est un identifiant.
const fn is_word_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}

/// `$` est volontairement **exclu** : `AS$$corps$$` doit ouvrir un corps
/// dollar, pas produire un mot `AS$$` qui laisserait le corps se faire couper
/// sur ses points-virgules.
const fn is_word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}

fn skip_word(b: &[u8], mut i: usize) -> usize {
    i += 1;
    while let Some(&c) = b.get(i) {
        if !is_word_byte(c) {
            break;
        }
        i += 1;
    }
    i
}

/// End of a line comment starting at `i`, and how it reads.
///
/// The end must be the server's: a comment that runs past it hides a statement
/// the server executes, from the splitter and from the keyword sweep alike.
fn skip_line_comment(b: &[u8], mut i: usize, end: LineCommentEnd) -> (usize, Tok) {
    let mut lone_carriage_return = false;
    let mut unreadable = false;
    while let Some(&c) = b.get(i) {
        match (c, end) {
            (b'\n', _) | (b'\r', LineCommentEnd::LineFeedOrCarriageReturn) => break,
            (b'\r', LineCommentEnd::Unverified) => lone_carriage_return = true,
            // Only text after the `\r` makes the two readings differ; trailing
            // blanks read the same either way.
            (_, LineCommentEnd::Unverified) if lone_carriage_return && !c.is_ascii_whitespace() => {
                unreadable = true;
            }
            _ => {}
        }
        i += 1;
    }
    let token = if unreadable {
        Tok::UnreadableComment
    } else {
        Tok::Comment
    };
    (i, token)
}

/// Un commentaire de bloc non fermé avale la fin du texte : c'est ce que fait
/// un vrai serveur, et cela mène à un fragment illisible donc `Unknown`.
fn skip_block_comment(b: &[u8], start: usize, nested: bool) -> usize {
    let mut i = start + 2;
    let mut depth = 1usize;
    while let Some(&c) = b.get(i) {
        if nested && c == b'/' && b.get(i + 1) == Some(&b'*') {
            depth += 1;
            i += 2;
            continue;
        }
        if c == b'*' && b.get(i + 1) == Some(&b'/') {
            i += 2;
            depth -= 1;
            if depth == 0 {
                return i;
            }
            continue;
        }
        i += 1;
    }
    b.len()
}

/// Traverse une région citée. Le doublement du délimiteur (`''`, `""`) est
/// toujours reconnu ; l'échappement par `\` dépend du dialecte.
fn skip_quoted(b: &[u8], start: usize, quote: u8, backslash: bool) -> usize {
    let mut i = start + 1;
    while let Some(&c) = b.get(i) {
        if backslash && c == b'\\' {
            i += 2;
            continue;
        }
        if c == quote {
            if b.get(i + 1) == Some(&quote) {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    b.len()
}

/// `[identifiant]` de SQL Server : le crochet fermant se double pour être
/// littéral, le crochet ouvrant non.
fn skip_bracket(b: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while let Some(&c) = b.get(i) {
        if c == b']' {
            if b.get(i + 1) == Some(&b']') {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    b.len()
}

/// Fin du délimiteur ouvrant `$tag$`, ou `None` si ce `$` n'en ouvre pas un.
///
/// `$1` n'est pas un délimiteur : une étiquette ne commence pas par un chiffre.
/// C'est ce qui permet de ne pas confondre un corps de fonction avec un
/// emplacement de paramètre PostgreSQL.
fn dollar_tag_end(b: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 1;
    while let Some(&c) = b.get(i) {
        if c == b'$' {
            return Some(i + 1);
        }
        let acceptable = if i == start + 1 {
            c.is_ascii_alphabetic() || c == b'_'
        } else {
            c.is_ascii_alphanumeric() || c == b'_'
        };
        if !acceptable {
            return None;
        }
        i += 1;
    }
    None
}

fn skip_dollar_quoted(b: &[u8], start: usize) -> Option<usize> {
    let tag_end = dollar_tag_end(b, start)?;
    let tag = b.get(start..tag_end)?;
    let mut i = tag_end;
    while i + tag.len() <= b.len() {
        if b.get(i..i + tag.len()) == Some(tag) {
            return Some(i + tag.len());
        }
        i += 1;
    }
    // Corps non fermé : on avale la fin.
    Some(b.len())
}

/// Le `'` en `start` est-il précédé d'un préfixe `E` isolé ?
fn has_escape_prefix(b: &[u8], start: usize) -> bool {
    let Some(&previous) = start.checked_sub(1).and_then(|k| b.get(k)) else {
        return false;
    };
    if !matches!(previous, b'e' | b'E') {
        return false;
    }
    match start.checked_sub(2).and_then(|k| b.get(k)) {
        Some(&before) => !is_word_byte(before),
        None => true,
    }
}

fn next_visible_byte(b: &[u8], mut i: usize) -> Option<u8> {
    while let Some(&c) = b.get(i) {
        if !c.is_ascii_whitespace() {
            return Some(c);
        }
        i += 1;
    }
    None
}

/// What the scanner learned about a fragment, beside its bounds.
#[derive(Clone, Copy)]
struct Flags {
    has_comment: bool,
    terminated: bool,
    unreadable_comment: bool,
}

impl Flags {
    const fn new(has_comment: bool, terminated: bool, unreadable_comment: bool) -> Self {
        Self {
            has_comment,
            terminated,
            unreadable_comment,
        }
    }
}

fn push_fragment<'a>(sql: &'a str, range: Range<usize>, flags: Flags, out: &mut Vec<Fragment<'a>>) {
    let start = range.start;
    let Some(raw) = sql.get(range) else {
        return;
    };
    let text = raw.trim();
    if text.is_empty() {
        return;
    }
    let lead = raw.len() - raw.trim_start().len();
    let from = start + lead;
    out.push(Fragment {
        text,
        span: from..from + text.len(),
        has_comment: flags.has_comment,
        terminated: flags.terminated,
        unreadable_comment: flags.unreadable_comment,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn textes(sql: &str, dialect: SqlDialect) -> Vec<String> {
        split(sql, dialect)
            .into_iter()
            .map(|f| f.text.to_owned())
            .collect()
    }

    #[test]
    fn current_statement_syntax_errors_keep_document_offsets() {
        let sql = "SELECT 1; SELECT";
        let error = current_statement(sql, SqlDialect::Postgres, sql.len())
            .expect_err("incomplete second statement");
        let QueryError::Syntax { span, .. } = error else {
            panic!("syntax error");
        };
        assert_eq!(span.start, sql.rfind("SELECT").expect("second SELECT"));
        assert_eq!(span.end, sql.len());
    }

    #[test]
    fn instruction_courante_garde_le_corps_du_trigger_sqlite() {
        let sql = "CREATE TRIGGER t AFTER INSERT ON x BEGIN INSERT INTO log VALUES(CASE WHEN NEW.id > 0 THEN 1 ELSE 0 END); UPDATE x SET seen = 1; END; SELECT 2";
        let cursor = sql.find("UPDATE x").expect("trigger body");
        let fragment = current_statement(sql, SqlDialect::Sqlite, cursor)
            .expect("valid trigger")
            .expect("trigger selection");
        assert!(fragment.text.starts_with("CREATE TRIGGER"));
        assert!(fragment.text.contains("UPDATE x SET seen"));
        assert!(!fragment.text.contains("SELECT 2"));
    }

    #[test]
    fn instruction_courante_refuse_les_positions_et_textes_ambigus() {
        let sql = "SELECT 'é';  SELECT 2";
        assert!(current_statement(sql, SqlDialect::Postgres, 9).is_err());
        assert_eq!(
            current_statement(sql, SqlDialect::Postgres, 13).expect("second space"),
            None
        );
        assert!(current_statement("SELECT 'unfinished", SqlDialect::Postgres, 4).is_err());
    }

    #[test]
    fn instruction_courante_associe_le_separateur_a_l_instruction_precedente() {
        let first = "SELECT 1";
        let trailing = "SELECT 1;";
        assert_eq!(
            current_statement(trailing, SqlDialect::Postgres, trailing.len())
                .expect("trailing separator")
                .expect("statement")
                .text,
            first
        );
        let adjacent = "SELECT 1;SELECT 2";
        assert_eq!(
            current_statement(adjacent, SqlDialect::Postgres, first.len())
                .expect("separator")
                .expect("previous")
                .text,
            first
        );
        assert_eq!(
            current_statement(adjacent, SqlDialect::Postgres, first.len() + 1)
                .expect("next statement")
                .expect("next")
                .text,
            "SELECT 2"
        );
        assert_eq!(
            current_statement("SELECT 1;  SELECT 2", SqlDialect::Postgres, 9)
                .expect("first space")
                .expect("previous")
                .text,
            first
        );
        assert!(
            current_statement("SELECT 1;  SELECT 2", SqlDialect::Postgres, 10)
                .expect("second space")
                .is_none()
        );
        let dollar = "SELECT $$;$$;";
        assert_eq!(
            current_statement(dollar, SqlDialect::Postgres, dollar.len())
                .expect("dollar quote")
                .expect("statement")
                .text,
            "SELECT $$;$$"
        );
    }

    #[test]
    fn instruction_courante_ne_saute_pas_au_dela_d_un_commentaire_introductif() {
        for sql in [
            "SELECT 1; -- commentaire\nDELETE FROM t;",
            "SELECT 1;-- commentaire\nDELETE FROM t;",
        ] {
            let cursor = sql.find(';').expect("separator") + 1;
            assert_eq!(
                current_statement(sql, SqlDialect::Postgres, cursor)
                    .expect("selection")
                    .expect("previous statement")
                    .text,
                "SELECT 1"
            );
        }
    }

    #[test]
    fn instruction_courante_refuse_les_mots_ambigus_dans_un_trigger_sqlite() {
        for sql in [
            "CREATE TRIGGER BEGIN AFTER INSERT ON x BEGIN UPDATE x SET id = 1; END;",
            "CREATE TRIGGER t AFTER INSERT ON x BEGIN UPDATE x SET CASE = 1; END;",
        ] {
            assert!(
                current_statement(sql, SqlDialect::Sqlite, 8).is_err(),
                "{sql}"
            );
        }
    }

    #[test]
    fn instruction_courante_ne_isole_jamais_un_corps_de_trigger_sqlite() {
        let trigger = "CREATE TEMP TRIGGER t AFTER INSERT ON source BEGIN SELECT end FROM source; UPDATE other SET id = CASE WHEN NEW.id > 0 THEN 999 ELSE 1 END; END;";
        for needle in ["SELECT end", "UPDATE other", "CASE WHEN", "END;"] {
            let cursor = trigger.find(needle).expect("body token");
            let result = current_statement(trigger, SqlDialect::Sqlite, cursor);
            assert!(
                result.is_err()
                    || result
                        .expect("selection")
                        .is_some_and(|fragment| fragment.text.starts_with("CREATE TEMP TRIGGER")),
                "{needle}"
            );
        }
    }

    #[test]
    fn create_table_avec_des_identifiants_reserves_n_est_pas_un_trigger() {
        let sql = "CREATE TABLE things (trigger text, begin integer, end integer); UPDATE things SET end = 1;";
        let cursor = sql.find("UPDATE").expect("update");
        assert_eq!(
            current_statement(sql, SqlDialect::Sqlite, cursor)
                .expect("table statements")
                .expect("update")
                .text,
            "UPDATE things SET end = 1"
        );
    }

    #[test]
    fn un_lot_simple_se_decoupe() {
        assert_eq!(
            textes("SELECT 1; SELECT 2", SqlDialect::Ansi),
            ["SELECT 1", "SELECT 2"]
        );
    }

    #[test]
    fn le_dernier_point_virgule_ne_cree_pas_de_fragment_vide() {
        assert_eq!(textes("SELECT 1;", SqlDialect::Ansi), ["SELECT 1"]);
        assert_eq!(textes("SELECT 1;;;", SqlDialect::Ansi), ["SELECT 1"]);
        assert_eq!(textes("  ;  ", SqlDialect::Ansi), Vec::<String>::new());
    }

    #[test]
    fn un_point_virgule_dans_une_chaine_ne_separe_pas() {
        assert_eq!(textes("SELECT ';'", SqlDialect::Ansi), ["SELECT ';'"]);
        assert_eq!(
            textes("SELECT 'a;b', 'c;d' FROM t", SqlDialect::Ansi),
            ["SELECT 'a;b', 'c;d' FROM t"]
        );
    }

    #[test]
    fn une_apostrophe_doublee_ne_ferme_pas_la_chaine() {
        assert_eq!(
            textes("SELECT 'l''été ; suite'", SqlDialect::Postgres),
            ["SELECT 'l''été ; suite'"]
        );
    }

    /// Le piège cité par la spécification : un commentaire qui contient un
    /// point-virgule.
    #[test]
    fn un_point_virgule_dans_un_commentaire_ne_separe_pas() {
        assert_eq!(
            textes("SELECT 1 -- garder ; ici\n, 2", SqlDialect::Ansi),
            ["SELECT 1 -- garder ; ici\n, 2"]
        );
        assert_eq!(
            textes("SELECT /* ; */ 1", SqlDialect::Ansi),
            ["SELECT /* ; */ 1"]
        );
    }

    #[test]
    fn un_commentaire_de_bloc_s_imbrique_en_postgres() {
        assert_eq!(
            textes("SELECT /* a /* ; */ ; */ 1", SqlDialect::Postgres),
            ["SELECT /* a /* ; */ ; */ 1"]
        );
    }

    #[test]
    fn un_identifiant_cite_ne_separe_pas() {
        assert_eq!(
            textes(r#"SELECT "a;b" FROM t"#, SqlDialect::Postgres),
            [r#"SELECT "a;b" FROM t"#]
        );
        assert_eq!(
            textes("SELECT `a;b` FROM t", SqlDialect::MySql),
            ["SELECT `a;b` FROM t"]
        );
        assert_eq!(
            textes("SELECT [a;b] FROM t", SqlDialect::SqlServer),
            ["SELECT [a;b] FROM t"]
        );
    }

    /// Un corps `$$ … $$` contient presque toujours des points-virgules :
    /// c'est le cas où un découpage naïf casse le plus visiblement.
    #[test]
    fn un_corps_dollar_ne_separe_pas() {
        let sql = "CREATE FUNCTION f() RETURNS int AS $$ BEGIN DELETE FROM t; RETURN 1; END $$ LANGUAGE plpgsql; SELECT 2";
        let fragments = textes(sql, SqlDialect::Postgres);
        assert_eq!(fragments.len(), 2, "{fragments:?}");
        assert!(fragments[0].starts_with("CREATE FUNCTION"));
        assert_eq!(fragments[1], "SELECT 2");
    }

    #[test]
    fn une_etiquette_dollar_nommee_est_reconnue() {
        let sql = "DO $corps$ BEGIN ; END $corps$; SELECT 1";
        assert_eq!(
            textes(sql, SqlDialect::Postgres),
            ["DO $corps$ BEGIN ; END $corps$", "SELECT 1"]
        );
    }

    /// `$1` est un emplacement de paramètre, pas une ouverture de corps : le
    /// confondre ferait avaler tout le reste du lot.
    #[test]
    fn un_emplacement_de_parametre_n_ouvre_pas_un_corps() {
        assert_eq!(
            textes("SELECT $1; DELETE FROM t", SqlDialect::Postgres),
            ["SELECT $1", "DELETE FROM t"]
        );
    }

    #[test]
    fn mysql_echappe_par_antislash() {
        // `'a\';'` est une seule chaîne en MySQL : l'antislash protège l'apostrophe.
        assert_eq!(
            textes(r"SELECT 'a\';' , 1", SqlDialect::MySql),
            [r"SELECT 'a\';' , 1"]
        );
    }

    #[test]
    fn postgres_n_echappe_pas_par_antislash_sans_prefixe() {
        // Sans préfixe `E`, `'a\'` est une chaîne complète : le `;` sépare.
        assert_eq!(
            textes(r"SELECT 'a\'; SELECT 2", SqlDialect::Postgres),
            [r"SELECT 'a\'", "SELECT 2"]
        );
    }

    #[test]
    fn postgres_echappe_avec_le_prefixe_e() {
        assert_eq!(
            textes(r"SELECT E'a\';' , 1", SqlDialect::Postgres),
            [r"SELECT E'a\';' , 1"]
        );
        // `table_e` ne doit pas être pris pour un préfixe : le `e` est collé à
        // un mot.
        assert_eq!(
            textes(r"SELECT ligne'a\'; SELECT 2", SqlDialect::Postgres),
            [r"SELECT ligne'a\'", "SELECT 2"]
        );
    }

    #[test]
    fn mysql_commente_avec_le_diese() {
        assert_eq!(
            textes("SELECT 1 # ; rien\n, 2", SqlDialect::MySql),
            ["SELECT 1 # ; rien\n, 2"]
        );
    }

    #[test]
    fn une_chaine_non_fermee_fusionne_plutot_que_de_couper() {
        // Direction de l'erreur : un seul fragment illisible, donc `Unknown`.
        let fragments = split("SELECT 'oups ; DELETE FROM t", SqlDialect::Ansi);
        assert_eq!(fragments.len(), 1);
    }

    #[test]
    fn les_bornes_designent_le_texte_exact() {
        let sql = "  SELECT 1 ;\n  DELETE FROM t  ";
        for fragment in split(sql, SqlDialect::Ansi) {
            assert_eq!(
                sql.get(fragment.span.clone()),
                Some(fragment.text),
                "bornes fausses pour {fragment:?}"
            );
        }
    }

    #[test]
    fn le_drapeau_de_commentaire_suit_le_fragment() {
        let fragments = split("SELECT 1; -- note\nSELECT 2", SqlDialect::Ansi);
        assert_eq!(fragments.len(), 2);
        assert!(!fragments[0].has_comment);
        assert!(fragments[1].has_comment);
    }

    #[test]
    fn la_terminaison_est_rapportee() {
        let fragments = split("SELECT 1; SELECT 2", SqlDialect::Ansi);
        assert!(fragments[0].terminated);
        assert!(!fragments[1].terminated);
    }

    #[test]
    fn un_lot_de_commentaires_seuls_ne_donne_aucune_instruction() {
        assert!(split("-- rien\n/* rien non plus */", SqlDialect::Ansi).is_empty());
        assert!(contains_comment(
            "-- rien\n/* rien non plus */",
            SqlDialect::Ansi
        ));
        assert!(!contains_comment(
            "SELECT '-- pas un commentaire'",
            SqlDialect::Ansi
        ));
    }

    #[test]
    fn les_mots_nus_ignorent_chaines_et_commentaires() {
        let mots = words(
            "SELECT x /* DROP */ FROM t WHERE note = 'DELETE FROM u' -- TRUNCATE",
            SqlDialect::Ansi,
        );
        let textes: Vec<&str> = mots.iter().map(|m| m.text).collect();
        assert_eq!(textes, ["SELECT", "x", "FROM", "t", "WHERE", "note"]);
    }

    #[test]
    fn un_appel_de_fonction_est_signale() {
        let mots = words(
            "SELECT TRUNCATE(1.234, 2), truncate  (x)",
            SqlDialect::MySql,
        );
        let vus: Vec<(&str, bool)> = mots.iter().map(|m| (m.text, m.call)).collect();
        assert_eq!(
            vus,
            [
                ("SELECT", false),
                ("TRUNCATE", true),
                ("truncate", true),
                ("x", false),
            ],
            "{mots:?}"
        );
    }

    #[test]
    fn les_bornes_des_mots_designent_le_texte() {
        let sql = "SELECT énergie FROM t";
        for mot in words(sql, SqlDialect::Ansi) {
            assert_eq!(sql.get(mot.span.clone()), Some(mot.text));
        }
    }
}
