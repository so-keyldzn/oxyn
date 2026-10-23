//! Bounded conversion from REST JSON responses to Arrow batches.

use std::sync::Arc;
use std::time::Instant;

use arrow::array::{ArrayRef, StringArray};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use oxyn_core::{ExecStats, OxynError, Result, StatementHandle};
use oxyn_driver::Cursor;
use serde_json::Value;

/// A one-page cursor. The HTTP reader caps every page before JSON decoding;
/// this cursor therefore never retains a whole unbounded server result.
pub(crate) struct JsonCursor {
    handle: StatementHandle,
    schema: SchemaRef,
    batch: Option<RecordBatch>,
    stats: ExecStats,
    started: Instant,
}

impl JsonCursor {
    pub(crate) fn from_response(handle: StatementHandle, response: Value) -> Result<Self> {
        let (schema, batch) = search_batch(&response)?;
        Ok(Self {
            handle,
            schema,
            batch: Some(batch),
            stats: ExecStats::default(),
            started: Instant::now(),
        })
    }
}

#[async_trait]
impl Cursor for JsonCursor {
    fn handle(&self) -> StatementHandle {
        self.handle
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    async fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        let batch = self.batch.take();
        if let Some(batch) = &batch {
            self.stats.record_batch(
                u64::try_from(batch.num_rows()).unwrap_or(u64::MAX),
                u64::try_from(batch.get_array_memory_size()).unwrap_or(u64::MAX),
            );
        }
        self.stats.total_time = self.started.elapsed();
        Ok(batch)
    }

    fn stats(&self) -> ExecStats {
        self.stats
    }
}

fn search_batch(response: &Value) -> Result<(SchemaRef, RecordBatch)> {
    let hits = response.pointer("/hits/hits").and_then(Value::as_array);
    let Some(hits) = hits else {
        return raw_json_batch(response);
    };
    let mut names = vec!["_index".to_owned(), "_id".to_owned(), "_score".to_owned()];
    for hit in hits {
        if let Some(source) = hit.get("_source").and_then(Value::as_object) {
            for name in source.keys() {
                if !names.contains(name) {
                    names.push(name.clone());
                }
            }
        }
    }
    let fields: Vec<_> = names
        .iter()
        .map(|name| Arc::new(Field::new(name, DataType::Utf8, true)))
        .collect();
    let schema = Arc::new(Schema::new(fields));
    let mut columns: Vec<Vec<Option<String>>> = (0..names.len())
        .map(|_| Vec::with_capacity(hits.len()))
        .collect();
    for hit in hits {
        let source = hit.get("_source").and_then(Value::as_object);
        for (index, name) in names.iter().enumerate() {
            let value = hit
                .get(name)
                .or_else(|| source.and_then(|source| source.get(name)));
            columns[index].push(value.map(value_text));
        }
    }
    let arrays: Vec<ArrayRef> = columns
        .into_iter()
        .map(|column| Arc::new(StringArray::from(column)) as ArrayRef)
        .collect();
    let batch = RecordBatch::try_new(Arc::clone(&schema), arrays).map_err(|_| {
        OxynError::Internal("the Elasticsearch response could not become an Arrow batch".to_owned())
    })?;
    Ok((schema, batch))
}

fn raw_json_batch(response: &Value) -> Result<(SchemaRef, RecordBatch)> {
    let schema = Arc::new(Schema::new(vec![Arc::new(Field::new(
        "json",
        DataType::Utf8,
        false,
    ))]));
    let array: ArrayRef = Arc::new(StringArray::from(vec![Some(value_text(response))]));
    let batch = RecordBatch::try_new(Arc::clone(&schema), vec![array]).map_err(|_| {
        OxynError::Internal("the Elasticsearch response could not become an Arrow batch".to_owned())
    })?;
    Ok((schema, batch))
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        // JSON text preserves `unsigned_long`, arbitrary JSON numbers, arrays,
        // objects and base64 binary without a lossy `f64` conversion.
        value => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use arrow::array::StringArray;

    use super::*;

    #[test]
    fn a_search_keeps_unsigned_integers_as_json_text() {
        let response = serde_json::json!({"hits":{"hits":[{
            "_index":"logs", "_id":"1", "_score":1.0,
            "_source":{"counter":18446744073709551615_u64}
        }]}});
        let (_, batch) = search_batch(&response).expect("batch");
        let column = batch.column_by_name("counter").expect("counter");
        let column = column.as_any().downcast_ref::<StringArray>().expect("utf8");
        assert_eq!(column.value(0), "18446744073709551615");
    }
}
