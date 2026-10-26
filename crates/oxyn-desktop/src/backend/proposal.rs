//! `Propose change…`: SQL to review, and nothing executed.
//!
//! The decision and its reasons live in
//! [ADR-0025](../../../../docs/adr/0025-proposition-de-changement-de-schema.md).
//! What matters here: the text is composed from the **local catalog**, handed to
//! the front, and the front drops it in a console like `Open DDL in console`.
//! No command variant, no execution path, and no agent reaches it — an agent
//! that wants a schema change writes SQL in a console like anyone, and that SQL
//! is reviewed ([I-01](../../../../CLAUDE.md#i-01), [I-02](../../../../CLAUDE.md#i-02)).
//!
//! A template composed by Oxyn is written neither by the user nor by an agent:
//! it carries no provenance (ADR-0025).

use std::borrow::Cow;
use std::fmt::Write as _;

use oxyn_catalog::{CatalogCache, CatalogPath, QuoteStyle, quote_identifier};
use oxyn_core::{ConnectionId, SqlDialect};

use super::Backend;
use crate::ipc::ai::{ProposalTarget, SchemaProposal};
use crate::ipc::{CatalogAddress, IpcError};

impl Backend {
    /// The schema-change template for a column or a named constraint.
    ///
    /// `Ok(None)` when there is nothing this dialect can change on that target —
    /// the front then offers no button, rather than one that produces a
    /// statement the engine refuses ([ADR-0003](../../../../docs/adr/0003-driver-capabilities.md)).
    pub fn propose_schema_change(
        &self,
        connection: ConnectionId,
        address: &CatalogAddress,
        target: &ProposalTarget,
    ) -> Result<Option<SchemaProposal>, IpcError> {
        let config = self.config(connection)?;
        let dialect = oxyn_query::dialect_for(&config.driver);
        let path = address.to_path()?;
        let cache = self
            .inner
            .executor
            .catalog(connection)
            .ok_or_else(|| IpcError::invalid("This connection has no catalog cache"))?;
        let cache = cache.read();
        Ok(
            proposed_change(&cache, &path, target, dialect, &config.name).map(|sql| {
                SchemaProposal {
                    sql,
                    title: "Proposed change.sql".to_owned(),
                    origin:
                        "a schema-change template · uncomment and complete one statement before \
                         running"
                            .to_owned(),
                }
            }),
        )
    }
}

/// The header, written in the text because it must survive a copy to a ticket.
fn header(relation: &str, connection: &str) -> String {
    format!(
        "Proposed change · {relation} · {}\n\
         Nothing has been executed. Uncomment one statement, complete it, and review it\n\
         before running: this connection is named above for that reason.\n",
        visible(connection)
    )
}

/// Said once when a name or a default held a control character: the escaped
/// text no longer names the object exactly, and the reader must know why.
const ESCAPED_NOTE: &str = "A name or a default below holds control characters, shown escaped \
     (\\r, \\n, \\u{…}). It does not name the object exactly: check it in the catalog.\n";

/// A catalog value with every control or line-separating character made
/// visible.
///
/// PostgreSQL ends a `--` comment at `\r` as well as `\n`, and an editor may
/// break a line on U+2028. A column named `a\r;DROP TABLE audit;--` would
/// otherwise print as an innocent line and run once pasted into `psql`. A
/// template that shows `\r` is one nobody pastes with their eyes closed.
fn visible(text: &str) -> Cow<'_, str> {
    let hidden = |c: char| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}');
    if !text.chars().any(hidden) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        match c {
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if hidden(c) => {
                // Writing to a `String` cannot fail.
                let _ = write!(out, "\\u{{{:x}}}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    Cow::Owned(out)
}

/// Comments **every physical line**, whatever breaks it.
///
/// `--` comments only up to the next line break, and PostgreSQL's lexer counts
/// a lone `\r` as one (`scan.l`: `newline [\n\r]`) — which `str::lines` does
/// not. Splitting on `\r\n`, `\r` and `\n` alike makes the guarantee
/// structural: it no longer depends on what the identifiers hold, even if one
/// escaped [`visible`] ([I-10](../../../../CLAUDE.md#i-10)).
fn commented(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 8);
    let mut rest = text;
    while !rest.is_empty() {
        let (line, after) = match rest.find(['\r', '\n']) {
            Some(at) => {
                let (line, tail) = rest.split_at(at);
                let skip = if tail.starts_with("\r\n") { 2 } else { 1 };
                (line, tail.get(skip..).unwrap_or_default())
            }
            None => (rest, ""),
        };
        out.push_str("--");
        if !line.is_empty() {
            out.push(' ');
            out.push_str(line);
        }
        out.push('\n');
        rest = after;
    }
    out
}

/// The relation, quoted — and whether a segment had to be made [`visible`],
/// which the header must then say like any other name.
fn quoted_relation(path: &CatalogPath, style: QuoteStyle) -> Option<(String, bool)> {
    let mut escaped = false;
    let mut quote = |segment: &str| {
        let shown = visible(segment);
        escaped |= matches!(shown, Cow::Owned(_));
        quote_identifier(&shown, style)
    };
    let mut parts = Vec::new();
    if let Some(namespace) = path.namespace() {
        parts.push(quote(namespace));
    }
    parts.push(quote(path.relation()?));
    Some((parts.join("."), escaped))
}

/// SQLite knows no `ALTER COLUMN` and no `DROP CONSTRAINT`: an engine
/// constraint, not a precaution.
const fn alters_columns(dialect: SqlDialect) -> bool {
    matches!(dialect, SqlDialect::Postgres | SqlDialect::Redshift)
}

/// The template, or `None` when there is nothing to propose.
///
/// Identifiers come from the server and are **quoted**; values — a default —
/// are copied verbatim from the catalog, inside the comment.
fn proposed_change(
    cache: &CatalogCache,
    path: &CatalogPath,
    target: &ProposalTarget,
    dialect: SqlDialect,
    connection: &str,
) -> Option<String> {
    let style = QuoteStyle::for_dialect(dialect);
    let (table, escaped_relation) = quoted_relation(path, style)?;
    let mut header = header(&table, connection);

    match target {
        ProposalTarget::Column { name } => {
            let field = cache
                .relation(path)?
                .fields
                .iter()
                .find(|field| field.name == *name)?
                .clone();
            let shown_name = visible(&field.name);
            let default = field.default.as_deref().map(visible);
            if escaped_relation
                || matches!(shown_name, Cow::Owned(_))
                || matches!(default, Some(Cow::Owned(_)))
            {
                header.push_str(ESCAPED_NOTE);
            }
            let column = quote_identifier(&shown_name, style);
            let mut lines = format!(
                "{header}\n\
                 Rename the column. Replace new_name before running.\n\
                 ALTER TABLE {table} RENAME COLUMN {column} TO new_name;\n"
            );
            if alters_columns(dialect) {
                // Only the direction that changes the current state: a template
                // that does nothing reads as one that failed.
                let (nullability, current) = if field.nullable {
                    (
                        format!("ALTER TABLE {table} ALTER COLUMN {column} SET NOT NULL;"),
                        "nullable",
                    )
                } else {
                    (
                        format!("ALTER TABLE {table} ALTER COLUMN {column} DROP NOT NULL;"),
                        "NOT NULL",
                    )
                };
                lines.push_str(&format!(
                    "\nChange nullability. The column is currently {current}.\n{nullability}\n"
                ));
                match default.as_deref() {
                    Some(existing) => lines.push_str(&format!(
                        "\nChange or remove the default. It is currently: {existing}\n\
                         ALTER TABLE {table} ALTER COLUMN {column} SET DEFAULT new_expression;\n\
                         ALTER TABLE {table} ALTER COLUMN {column} DROP DEFAULT;\n"
                    )),
                    None => lines.push_str(&format!(
                        "\nAdd a default. The column has none.\n\
                         ALTER TABLE {table} ALTER COLUMN {column} SET DEFAULT new_expression;\n"
                    )),
                }
            }
            Some(commented(&lines))
        }
        ProposalTarget::Constraint { name } => {
            if !alters_columns(dialect) || name.is_empty() {
                return None;
            }
            let constraint = cache
                .constraints(path)?
                .iter()
                .find(|constraint| constraint.name == *name)?;
            let shown_name = visible(&constraint.name);
            if escaped_relation || matches!(shown_name, Cow::Owned(_)) {
                header.push_str(ESCAPED_NOTE);
            }
            let quoted = quote_identifier(&shown_name, style);
            Some(commented(&format!(
                "{header}\n\
                 Drop the constraint. This does not remove the data it protected.\n\
                 ALTER TABLE {table} DROP CONSTRAINT {quoted};\n"
            )))
        }
    }
}

#[cfg(test)]
mod tests;
