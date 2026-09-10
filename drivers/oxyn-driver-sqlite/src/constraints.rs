//! Reads declared SQLite constraints without rewriting their stored SQL.

use oxyn_catalog::{Constraint, ConstraintKind};
use oxyn_core::{CancelToken, OxynError, Result};
use rusqlite::Connection;

use crate::schema_sql::{Token, TokenKind, invalid_sql, list_parts, stored_sql, tokens};

const MAX_CONSTRAINTS: usize = 1024;
const MAX_DEFINITION_BYTES: usize = 16_384;

pub(crate) fn read(
    connection: &Connection,
    database: &str,
    name: &str,
    cancel: &CancelToken,
) -> Result<Vec<Constraint>> {
    if cancel.is_cancelled() {
        return Err(OxynError::Cancelled);
    }
    let (kind, sql) = stored_sql(connection, database, name)?;
    if cancel.is_cancelled() {
        return Err(OxynError::Cancelled);
    }
    if kind == "view" {
        return Ok(Vec::new());
    }
    declarations(&sql, cancel)
}

fn declarations(sql: &str, cancel: &CancelToken) -> Result<Vec<Constraint>> {
    let tokens = tokens(sql, cancel)?;
    if !tokens.first().is_some_and(|t| t.keyword("CREATE")) {
        return Err(invalid_sql());
    }
    let table = tokens
        .iter()
        .position(|t| t.keyword("TABLE"))
        .ok_or_else(invalid_sql)?;
    if tokens.iter().take(table).any(|t| t.keyword("VIRTUAL")) {
        return Err(OxynError::NotSupported {
            capability: "constraints of SQLite virtual tables".into(),
        });
    }
    let parts = list_parts(tokens.get(table + 1..).ok_or_else(invalid_sql)?)?;
    let mut constraints = Vec::new();
    for part in parts {
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        let first = part.first().ok_or_else(invalid_sql)?;
        if first.keyword("CONSTRAINT") || kind_at(part, 0).is_some() {
            table_constraints(sql, part, &mut constraints)?;
        } else {
            column_constraints(sql, part, &mut constraints)?;
        }
    }
    Ok(constraints)
}

fn table_constraints(sql: &str, tokens: &[Token<'_>], output: &mut Vec<Constraint>) -> Result<()> {
    // SQLite accepts adjacent table constraints without a separating comma.
    let mut start = 0;
    let mut index = 0;
    let mut depth = 0usize;
    while let Some(token) = tokens.get(index) {
        let boundary = token.keyword("CONSTRAINT")
            || kind_at(tokens, index).is_some_and(|(kind, _)| {
                matches!(
                    kind,
                    ConstraintKind::PrimaryKey | ConstraintKind::Unique | ConstraintKind::Check
                ) || (kind == ConstraintKind::ForeignKey && token.keyword("FOREIGN"))
            });
        if depth == 0 && boundary {
            if index > start {
                append_table(
                    output,
                    sql,
                    tokens.get(start..index).ok_or_else(invalid_sql)?,
                )?;
            }
            start = index;
            let (_, kind_index) = named_start(tokens, index)?;
            let (_, consumed) = kind_at(tokens, kind_index).ok_or_else(invalid_sql)?;
            index = kind_index + consumed;
            continue;
        }
        match token.kind {
            TokenKind::Open => depth += 1,
            TokenKind::Close => depth = depth.checked_sub(1).ok_or_else(invalid_sql)?,
            _ => {}
        }
        index += 1;
    }
    if depth != 0 {
        return Err(invalid_sql());
    }
    append_table(output, sql, tokens.get(start..).ok_or_else(invalid_sql)?)
}

fn append_table(output: &mut Vec<Constraint>, sql: &str, tokens: &[Token<'_>]) -> Result<()> {
    let (name, kind_index) = named_start(tokens, 0)?;
    let (kind, _) = kind_at(tokens, kind_index).ok_or_else(invalid_sql)?;
    let fields = match kind {
        ConstraintKind::PrimaryKey | ConstraintKind::Unique | ConstraintKind::ForeignKey => {
            list_parts(tokens.get(kind_index..).ok_or_else(invalid_sql)?)?
                .into_iter()
                .map(|column| column.first().ok_or_else(invalid_sql)?.identifier())
                .collect::<Result<Vec<_>>>()?
        }
        ConstraintKind::Check => Vec::new(),
        _ => return Err(invalid_sql()),
    };
    append(output, sql, tokens, name, kind, fields)
}

fn named_start(tokens: &[Token<'_>], index: usize) -> Result<(String, usize)> {
    if keyword(tokens, index, "CONSTRAINT") {
        let name = tokens
            .get(index + 1)
            .ok_or_else(invalid_sql)?
            .identifier()?;
        Ok((name, index + 2))
    } else {
        Ok((String::new(), index))
    }
}

fn keyword(tokens: &[Token<'_>], index: usize, word: &str) -> bool {
    tokens.get(index).is_some_and(|token| token.keyword(word))
}

fn kind_at(tokens: &[Token<'_>], index: usize) -> Option<(ConstraintKind, usize)> {
    if keyword(tokens, index, "PRIMARY") && keyword(tokens, index + 1, "KEY") {
        Some((ConstraintKind::PrimaryKey, 2))
    } else if keyword(tokens, index, "NOT") && keyword(tokens, index + 1, "NULL") {
        Some((ConstraintKind::NotNull, 2))
    } else if keyword(tokens, index, "FOREIGN") && keyword(tokens, index + 1, "KEY") {
        Some((ConstraintKind::ForeignKey, 2))
    } else if keyword(tokens, index, "REFERENCES") {
        Some((ConstraintKind::ForeignKey, 1))
    } else if keyword(tokens, index, "UNIQUE") {
        Some((ConstraintKind::Unique, 1))
    } else if keyword(tokens, index, "CHECK") {
        Some((ConstraintKind::Check, 1))
    } else {
        None
    }
}

fn column_constraints(sql: &str, tokens: &[Token<'_>], output: &mut Vec<Constraint>) -> Result<()> {
    let column = tokens.first().ok_or_else(invalid_sql)?.identifier()?;
    let mut active: Option<(usize, String, ConstraintKind)> = None;
    let mut depth = 0usize;
    let mut index = 1;
    while let Some(token) = tokens.get(index) {
        match token.kind {
            TokenKind::Open => {
                depth += 1;
            }
            TokenKind::Close => {
                depth = depth.checked_sub(1).ok_or_else(invalid_sql)?;
            }
            _ => {}
        }
        if depth != 0 {
            index += 1;
            continue;
        }
        let named = token.keyword("CONSTRAINT");
        let (name, kind_index) = named_start(tokens, index)?;
        let kind = kind_at(tokens, kind_index);
        // SET NULL / SET DEFAULT belongs to a reference action, not a new option.
        let set_action = index > 0
            && keyword(tokens, index - 1, "SET")
            && active
                .as_ref()
                .is_some_and(|(_, _, kind)| *kind == ConstraintKind::ForeignKey);
        let other_option = ["DEFAULT", "COLLATE", "GENERATED", "AS", "NULL"]
            .iter()
            .any(|word| keyword(tokens, kind_index, word))
            && !set_action;
        if named || kind.is_some() || other_option {
            if let Some((start, previous_name, previous_kind)) = active.take() {
                append_column(
                    output,
                    sql,
                    tokens.get(start..index).ok_or_else(invalid_sql)?,
                    previous_name,
                    previous_kind,
                    &column,
                )?;
            }
            if let Some((kind, consumed)) = kind {
                active = Some((index, name, kind));
                index = kind_index + consumed;
                continue;
            }
            if named && !other_option {
                return Err(invalid_sql());
            }
            index = kind_index + 1;
            continue;
        }
        index += 1;
    }
    if depth != 0 {
        return Err(invalid_sql());
    }
    if let Some((start, name, kind)) = active {
        append_column(
            output,
            sql,
            tokens.get(start..).ok_or_else(invalid_sql)?,
            name,
            kind,
            &column,
        )?;
    }
    Ok(())
}

fn append_column(
    output: &mut Vec<Constraint>,
    sql: &str,
    tokens: &[Token<'_>],
    name: String,
    kind: ConstraintKind,
    column: &str,
) -> Result<()> {
    // A column CHECK may refer to other columns. Do not guess expression dependencies.
    let fields = if kind == ConstraintKind::Check {
        Vec::new()
    } else {
        vec![column.to_owned()]
    };
    append(output, sql, tokens, name, kind, fields)
}

fn append(
    output: &mut Vec<Constraint>,
    sql: &str,
    tokens: &[Token<'_>],
    name: String,
    kind: ConstraintKind,
    fields: Vec<String>,
) -> Result<()> {
    if output.len() >= MAX_CONSTRAINTS {
        return Err(OxynError::CatalogUnavailable(
            "constraint metadata exceeds 1024 entries".into(),
        ));
    }
    let start = tokens.first().ok_or_else(invalid_sql)?.start;
    let end = tokens.last().ok_or_else(invalid_sql)?.end;
    let definition = sql.get(start..end).ok_or_else(invalid_sql)?;
    if definition.len() > MAX_DEFINITION_BYTES {
        return Err(OxynError::CatalogUnavailable(
            "constraint definition exceeds 16 KiB".into(),
        ));
    }
    output.push(Constraint::new(name, kind, fields).with_expression(definition));
    Ok(())
}

#[cfg(test)]
mod tests;
