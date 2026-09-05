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
            },
            SqlDialect::MySql => Self {
                backslash_escapes: true,
                escape_string_prefix: false,
                backtick_quotes: true,
                bracket_quotes: false,
                dollar_quotes: false,
                hash_line_comments: true,
                nested_block_comments: false,
            },
            SqlDialect::Sqlite => Self {
                backslash_escapes: false,
                escape_string_prefix: false,
                backtick_quotes: true,
                bracket_quotes: true,
                dollar_quotes: false,
                hash_line_comments: false,
                nested_block_comments: false,
            },
            SqlDialect::SqlServer => Self {
                backslash_escapes: false,
                escape_string_prefix: false,
                backtick_quotes: false,
                bracket_quotes: true,
                dollar_quotes: false,
                hash_line_comments: false,
                nested_block_comments: true,
            },
            SqlDialect::ClickHouse => Self {
                backslash_escapes: true,
                escape_string_prefix: false,
                backtick_quotes: true,
                bracket_quotes: false,
                dollar_quotes: false,
                hash_line_comments: true,
                nested_block_comments: true,
            },
            SqlDialect::DuckDb => Self {
                backslash_escapes: false,
                escape_string_prefix: true,
                backtick_quotes: true,
                bracket_quotes: false,
                dollar_quotes: true,
                hash_line_comments: false,
                nested_block_comments: true,
            },
            SqlDialect::Snowflake | SqlDialect::BigQuery => Self {
                backslash_escapes: true,
                escape_string_prefix: false,
                backtick_quotes: true,
                bracket_quotes: false,
                dollar_quotes: matches!(dialect, SqlDialect::Snowflake),
                hash_line_comments: false,
                nested_block_comments: false,
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

/// Découpe un lot avec un profil lexical explicite.
#[must_use]
pub fn split_with(sql: &str, profile: SplitProfile) -> Vec<Fragment<'_>> {
    let mut fragments = Vec::new();
    let mut start = 0usize;
    let mut has_comment = false;
    let mut has_code = false;

    scan(sql, profile, &mut |token, span| match token {
        Tok::Comment => has_comment = true,
        Tok::Semicolon => {
            if has_code {
                push_fragment(sql, start, span.start, has_comment, true, &mut fragments);
            }
            start = span.end;
            has_comment = false;
            has_code = false;
        }
        Tok::Word | Tok::Quoted | Tok::Symbol => has_code = true,
    });

    if has_code {
        push_fragment(sql, start, sql.len(), has_comment, false, &mut fragments);
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
            if token == Tok::Comment {
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
                let end = skip_to_eol(b, i + 2);
                on(Tok::Comment, i..end);
                i = end;
            }
            b'#' if profile.hash_line_comments => {
                let end = skip_to_eol(b, i + 1);
                on(Tok::Comment, i..end);
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

fn skip_to_eol(b: &[u8], mut i: usize) -> usize {
    while let Some(&c) = b.get(i) {
        if c == b'\n' {
            return i;
        }
        i += 1;
    }
    b.len()
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

fn push_fragment<'a>(
    sql: &'a str,
    start: usize,
    end: usize,
    has_comment: bool,
    terminated: bool,
    out: &mut Vec<Fragment<'a>>,
) {
    let Some(raw) = sql.get(start..end) else {
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
        has_comment,
        terminated,
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
