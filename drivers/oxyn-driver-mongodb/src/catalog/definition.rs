//! A collection's definition: the commands that would create it again.
//!
//! MongoDB stores no creation statement. `listCollections` returns, under
//! `options`, the options the collection was created with — validator,
//! capped size, view pipeline, time-series settings — and `listIndexes` each
//! index's key and options. The definition is **reconstructed** from those two
//! replies as console commands: one `create`, one `createIndexes` when the
//! collection has indexes besides `_id_`, each a top-level JSON object the
//! console runs on its own ([ADR-0063](../../../../docs/adr/0063-mongodb-driver-database-commands-in-extended-json.md)).
//!
//! Every name and value is written by the Extended JSON writer, escaped,
//! never pasted ([I-10](../../../../CLAUDE.md#i-10)). Relaxed mode keeps the
//! text readable: an `Int64` option reads as a plain number, which the server
//! accepts for every option it applies to.

use bson::RawDocumentBuf;
use bson::raw::{RawBsonRef, RawDocument};
use oxyn_catalog::{DefinitionSource, RelationDefinition};
use oxyn_core::{OxynError, Result};

use crate::ejson::{self, Mode};

/// Index fields the server reports that are not options of `createIndexes`:
/// the index version it chose and, on older servers, the namespace.
const REPORTED_ONLY: &[&str] = &["v", "ns"];

/// The `_id` index every collection has, created with it.
const ID_INDEX: &str = "_id_";

/// The definition of `name` in `database`, from its `listCollections` entry
/// and its `listIndexes` documents.
///
/// # Errors
/// [`OxynError::CatalogUnavailable`] for a reply whose BSON cannot be written
/// back, or a definition beyond the preview bounds.
pub(super) fn reconstruct(
    database: &str,
    name: &str,
    entry: &RawDocument,
    indexes: &[RawDocumentBuf],
) -> Result<RelationDefinition> {
    let mut text = String::new();
    let mut create = Command::open(&mut text, "create", name, database);
    if let Ok(options) = entry.get_document("options") {
        for element in options {
            let (key, value) = element.map_err(unreadable)?;
            create.field(key.as_str(), value)?;
        }
    }
    create.close();

    let secondary: Vec<&RawDocumentBuf> = indexes
        .iter()
        .filter(|index| index.get_str("name").ok() != Some(ID_INDEX))
        .collect();
    if !secondary.is_empty() {
        text.push_str("\n\n");
        let mut create_indexes = Command::open(&mut text, "createIndexes", name, database);
        create_indexes.key("indexes");
        // One index per line, each written compact like the values above.
        create_indexes.out.push_str("[\n    ");
        for (position, index) in secondary.into_iter().enumerate() {
            if position > 0 {
                create_indexes.out.push_str(",\n    ");
            }
            create_indexes.out.push('{');
            let mut first = true;
            for element in index.iter() {
                let (key, value) = element.map_err(unreadable)?;
                if REPORTED_ONLY.contains(&key.as_str()) {
                    continue;
                }
                if !first {
                    create_indexes.out.push(',');
                }
                first = false;
                ejson::write_string(create_indexes.out, key.as_str());
                create_indexes.out.push(':');
                create_indexes.out.push_str(&write(value)?);
            }
            create_indexes.out.push('}');
        }
        create_indexes.out.push_str("\n  ]");
        create_indexes.close();
    }

    let definition = RelationDefinition {
        sql: text,
        source: DefinitionSource::Reconstructed,
        notes: vec![
            "Reconstructed from listCollections and listIndexes: MongoDB stores no creation \
             command. Each object is one console command; numbers are written relaxed."
                .to_owned(),
        ],
    };
    definition.validate()?;
    Ok(definition)
}

/// A command written one top-level field per line.
struct Command<'a> {
    out: &'a mut String,
}

impl<'a> Command<'a> {
    /// `{"<verb>": <collection>, "$db": <database>` — the target is always
    /// written, so the command runs where the collection is, whatever the
    /// console's context.
    fn open(out: &'a mut String, verb: &str, collection: &str, database: &str) -> Self {
        out.push_str("{\n  ");
        ejson::write_string(out, verb);
        out.push_str(": ");
        ejson::write_string(out, collection);
        let mut command = Self { out };
        command.key("$db");
        ejson::write_string(command.out, database);
        command
    }

    fn key(&mut self, key: &str) {
        self.out.push_str(",\n  ");
        ejson::write_string(self.out, key);
        self.out.push_str(": ");
    }

    fn field(&mut self, key: &str, value: RawBsonRef<'_>) -> Result<()> {
        self.key(key);
        self.out.push_str(&write(value)?);
        Ok(())
    }

    fn close(self) {
        self.out.push_str("\n}");
    }
}

fn write(value: RawBsonRef<'_>) -> Result<String> {
    ejson::value(value, Mode::Relaxed).map_err(|error| {
        OxynError::CatalogUnavailable(format!(
            "the collection's options cannot be written: {error}"
        ))
    })
}

fn unreadable(error: bson::error::Error) -> OxynError {
    OxynError::CatalogUnavailable(format!(
        "the server's catalog reply cannot be read: {error}"
    ))
}

#[cfg(test)]
mod tests {
    use bson::{RawDocumentBuf, doc};

    use super::*;

    fn raw(document: &bson::Document) -> RawDocumentBuf {
        RawDocumentBuf::try_from(document).expect("a document the test builds is valid BSON")
    }

    #[test]
    fn options_and_secondary_indexes_become_two_commands() {
        let entry = raw(&doc! {
            "name": "orders",
            "type": "collection",
            "options": {
                "validator": { "$jsonSchema": { "required": ["total"] } },
                "validationLevel": "strict",
            },
        });
        let indexes = [
            raw(&doc! { "v": 2, "key": { "_id": 1 }, "name": "_id_" }),
            raw(
                &doc! { "v": 2, "key": { "customer": 1, "at": -1 }, "name": "by_customer", "unique": true },
            ),
        ];
        let definition = reconstruct("shop", "orders", &entry, &indexes).expect("written");
        assert_eq!(definition.source, DefinitionSource::Reconstructed);
        assert_eq!(
            definition.sql,
            "{\n  \"create\": \"orders\",\n  \"$db\": \"shop\",\n  \"validator\": \
             {\"$jsonSchema\":{\"required\":[\"total\"]}},\n  \"validationLevel\": \"strict\"\n}\
             \n\n{\n  \"createIndexes\": \"orders\",\n  \"$db\": \"shop\",\n  \"indexes\": [\n    \
             {\"key\":{\"customer\":1,\"at\":-1},\"name\":\"by_customer\",\"unique\":true}\n  ]\n}"
        );
        // The whole text is two console commands, each read on its own.
        let second = definition
            .sql
            .find("\n\n")
            .expect("a blank line between the commands");
        for (start, verb) in [(0, "create"), (second.saturating_add(2), "createIndexes")] {
            let text = oxyn_query::mongo::command_at(&definition.sql, start.saturating_add(1))
                .expect("a command under the cursor");
            assert_eq!(crate::command::parse(text).expect("parses").name, verb);
        }
    }

    #[test]
    fn each_command_reads_back_as_one_console_command() {
        let entry = raw(&doc! { "name": "v", "type": "view",
        "options": { "viewOn": "orders", "pipeline": [ { "$match": { "x": 1 } } ] } });
        let definition = reconstruct("shop", "v", &entry, &[]).expect("written");
        let command = crate::command::parse(&definition.sql).expect("a console command");
        assert_eq!(command.name, "create");
        assert_eq!(command.database.as_deref(), Some("shop"));
        assert_eq!(command.document.get_str("viewOn").ok(), Some("orders"));
    }

    #[test]
    fn a_hostile_name_is_a_json_string_never_text() {
        let name = "o\"rders\"}, {\"dropDatabase\": 1";
        let entry = raw(&doc! { "name": name, "options": {} });
        let definition = reconstruct("shop", name, &entry, &[]).expect("written");
        let command = crate::command::parse(&definition.sql).expect("one command");
        assert_eq!(command.name, "create");
        assert_eq!(command.document.get_str("create").ok(), Some(name));
    }
}
