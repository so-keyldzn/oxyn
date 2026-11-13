//! Byte-bounded NDJSON frames; framing errors never quote received values.

use std::marker::PhantomData;
use std::sync::Arc;

use oxyn_core::{OxynError, Result, StatementIntent};
use serde::de::{DeserializeOwned, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

use crate::http::Http;
use crate::wire::{Cell, Col, Fault};

pub(super) const LINE_BYTES: usize = 32 * 1024 * 1024;
pub(super) const HEADER_BYTES: usize = 1024 * 1024;
const COLUMNS: usize = 4096;

pub(super) struct Frames {
    response: reqwest::Response,
    buffer: Vec<u8>,
    position: usize,
    ended: bool,
    http: Arc<Http>,
}

impl Frames {
    pub fn new(response: reqwest::Response, http: Arc<Http>) -> Self {
        Self {
            response,
            http,
            buffer: Vec::new(),
            position: 0,
            ended: false,
        }
    }

    pub async fn next<T: DeserializeOwned>(&mut self, limit: usize) -> Result<Option<T>> {
        loop {
            let remaining = self.buffer.get(self.position..).ok_or_else(malformed)?;
            let line = remaining.iter().position(|byte| *byte == b'\n');
            if let Some(length) = line.or_else(|| self.ended.then_some(remaining.len())) {
                if length > limit {
                    return Err(too_large());
                }
                let line = remaining.get(..length).ok_or_else(malformed)?;
                self.position = self
                    .position
                    .saturating_add(length)
                    .saturating_add(usize::from(!self.ended || length < remaining.len()));
                if line.iter().all(u8::is_ascii_whitespace) {
                    if self.ended && length == remaining.len() {
                        return Ok(None);
                    }
                    continue;
                }
                return serde_json::from_slice(line)
                    .map(Some)
                    .map_err(|_| malformed());
            }
            if remaining.len() > limit {
                return Err(too_large());
            }
            // position was validated by get above. Compact once per transport
            // read, not once per row, so a chunk of narrow rows is not quadratic.
            self.buffer.drain(..self.position);
            self.position = 0;
            match self.response.chunk().await {
                Ok(Some(chunk)) => {
                    if self.buffer.len().saturating_add(chunk.len()) > LINE_BYTES {
                        return Err(too_large());
                    }
                    self.buffer.extend_from_slice(&chunk);
                }
                Ok(None) => self.ended = true,
                Err(error) => {
                    return Err(crate::error::transport(
                        StatementIntent::Read,
                        error,
                        self.http.spec().token(),
                    ));
                }
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum Entry {
    StepBegin {
        step: u32,
        #[serde(deserialize_with = "bounded")]
        cols: Vec<Col>,
    },
    Row {
        #[serde(deserialize_with = "bounded")]
        row: Vec<Cell>,
    },
    StepEnd,
    ReplicationIndex {
        replication_index: Option<u64>,
    },
    StepError {
        step: u32,
        error: Fault,
    },
    Error {
        error: Fault,
    },
}

fn bounded<'de, D, T>(deserializer: D) -> std::result::Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Items<T>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>> Visitor<'de> for Items<T> {
        type Value = Vec<T>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a bounded list of columns or values")
        }
        fn visit_seq<A: SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut items = Vec::new();
            while let Some(item) = seq.next_element()? {
                if items.len() >= COLUMNS {
                    return Err(serde::de::Error::custom("too many columns"));
                }
                items.push(item);
            }
            Ok(items)
        }
    }
    deserializer.deserialize_seq(Items(PhantomData))
}

pub(super) fn malformed() -> OxynError {
    OxynError::Query("the server returned an incomplete or invalid Hrana cursor stream".into())
}

fn too_large() -> OxynError {
    OxynError::Query(
        "a Hrana cursor frame exceeds the preview byte budget; select fewer columns in a console"
            .into(),
    )
}
