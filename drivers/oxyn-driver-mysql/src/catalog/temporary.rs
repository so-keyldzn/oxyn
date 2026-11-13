//! Temporary tables shadow persistent names but are absent from information_schema.

use std::sync::atomic::Ordering;

use super::*;

const TEMPORARY_DDL: &str = "CREATE TEMPORARY TABLE ";
const MAX_ROWS: usize = 4096;
const MAX_BYTES: usize = 8 * 1024 * 1024;

impl MysqlCatalog {
    pub(crate) fn note_execution(&self) {
        self.shared.local_state.store(true, Ordering::Release);
    }

    pub(crate) fn has_local_state(&self) -> bool {
        self.shared.local_state.load(Ordering::Acquire)
    }

    pub(super) async fn is_temporary(
        &self,
        path: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<bool> {
        if !self.has_local_state() {
            return Ok(false);
        }
        let (database, name) = relation_of(path)?;
        let quoted =
            CatalogPath::for_relation(None, Some(database), name)?.qualify(QuoteStyle::Backtick);
        let row = match self
            .show_create(format!("SHOW CREATE TABLE {quoted}"), cancel)
            .await
        {
            Ok(row) => row,
            Err(error) => {
                // SELECT on a view does not imply SHOW VIEW. Only that permission
                // failure may use a confirmed persistent view's public metadata.
                let denied = matches!(&error, OxynError::Driver { source, .. }
                    if source.downcast_ref::<crate::MysqlError>()
                        .is_some_and(|error| error.code() == Some(1142)));
                if denied {
                    let rows = self
                        .rows(
                            SQL_RELATION,
                            vec![Value::from(database), Value::from(name)],
                            cancel,
                        )
                        .await?;
                    if rows.first().and_then(|row| text(row, 0)).as_deref() == Some("VIEW") {
                        return Ok(false);
                    }
                }
                return Err(error);
            }
        };
        let definition = required(&row, 1)?;
        Ok(definition
            .trim_start()
            .get(..TEMPORARY_DDL.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(TEMPORARY_DDL)))
    }

    async fn show_rows(
        &self,
        sql: String,
        cancel: &CancelToken,
    ) -> Result<Vec<Vec<Option<Value>>>> {
        let mut lease = self.shared.lease(cancel).await?;
        let found = self
            .shared
            .run(
                &mut lease,
                cancel,
                StatementIntent::Read,
                move |connection| {
                    Box::pin(async move {
                        let mut response = connection.query_iter(sql).await?;
                        let mut rows = Vec::new();
                        let mut bytes = 0_usize;
                        while let Some(row) = response.next().await? {
                            bytes =
                                bytes.saturating_add(row.columns_ref().len().saturating_mul(16));
                            for index in 0..row.len() {
                                if let Some(Value::Bytes(value)) = row.as_ref(index) {
                                    bytes = bytes.saturating_add(value.len());
                                }
                            }
                            if rows.len() >= MAX_ROWS || bytes > MAX_BYTES {
                                return Ok(Introspected::Refused(
                                    "temporary metadata exceeding its byte or row budget".into(),
                                ));
                            }
                            rows.push(row);
                        }
                        if !response.is_empty() {
                            return Ok(Introspected::Refused(
                                "an unexpected metadata result set".into(),
                            ));
                        }
                        Ok(Introspected::Rows(rows))
                    })
                },
            )
            .await?;
        match found {
            Introspected::Rows(rows) => Ok(rows.into_iter().map(Row::unwrap_raw).collect()),
            Introspected::Refused(reason) => {
                lease.abandon();
                Err(OxynError::Query(format!(
                    "the server returned {reason}; reconnect before continuing"
                )))
            }
        }
    }

    pub(super) async fn temporary_relation(
        &self,
        path: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Relation> {
        let (database, name) = relation_of(path)?;
        let quoted =
            CatalogPath::for_relation(None, Some(database), name)?.qualify(QuoteStyle::Backtick);
        let rows = self
            .show_rows(format!("SHOW FULL COLUMNS FROM {quoted}"), cancel)
            .await?;
        let mut fields = Vec::with_capacity(rows.len());
        for (index, row) in rows.iter().enumerate() {
            let name = required(row, 0)?;
            let raw = required(row, 1)?;
            let base = raw.split(['(', ' ']).next().unwrap_or_default();
            let parameters = raw
                .split_once('(')
                .and_then(|(_, tail)| tail.split_once(')'))
                .map(|(values, _)| values);
            let mut sizes = parameters
                .unwrap_or_default()
                .split(',')
                .map(|value| value.trim().parse::<u64>().ok());
            let logical = logical_type(base, sizes.next().flatten(), sizes.next().flatten());
            let position = u32::try_from(index + 1)
                .map_err(|_| OxynError::Query("too many temporary columns".into()))?;
            let mut field = Field::new(name, position, logical, raw);
            if text(row, 3).as_deref() == Some("NO") {
                field = field.not_null();
            }
            if text(row, 4).as_deref() == Some("PRI") {
                field = field.primary_key();
            }
            if let Some(default) = text(row, 5) {
                field = field.with_default(default);
            }
            if let Some(comment) = text(row, 8).filter(|value| !value.is_empty()) {
                field = field.with_comment(comment);
            }
            fields.push(field);
        }
        // SHOW's key label alone proves neither full key order nor uniqueness.
        Ok(Relation::new(name, RelationKind::Table)
            .with_fields(fields)
            .with_retained_preview(true))
    }

    pub(super) async fn temporary_indexes(
        &self,
        path: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<Index>> {
        let (database, name) = relation_of(path)?;
        let quoted =
            CatalogPath::for_relation(None, Some(database), name)?.qualify(QuoteStyle::Backtick);
        let rows = self
            .show_rows(format!("SHOW INDEX FROM {quoted}"), cancel)
            .await?;
        let mut indexes: Vec<Index> = Vec::new();
        for row in rows {
            let name = required(&row, 2)?;
            let column = text(&row, 4).unwrap_or_else(|| "(expression)".into());
            if let Some(index) = indexes.iter_mut().find(|index| index.name == name) {
                index.fields.push(column);
            } else {
                let mut index = Index::new(name, vec![column]);
                if number(&row, 1) == Some(0) {
                    index = index.unique();
                }
                index.method = text(&row, 10);
                indexes.push(index);
            }
        }
        Ok(indexes)
    }
}
