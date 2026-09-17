//! Explicit preparation of editable SQL. Inspection never invokes this path by itself.

use super::*;
use oxyn_core::QueryLanguage;

#[derive(Clone)]
pub(in crate::workspace) enum OpenQuery {
    Copy {
        text: String,
        title: String,
        origin: String,
        /// D'où vient ce texte, quand un agent l'a écrit.
        ///
        /// `None` pour une copie d'historique ou de document : elle recopie ce
        /// que l'utilisateur avait déjà, et la provenance de l'original — s'il
        /// en avait une — reste sur l'original. Une copie n'hérite pas d'une
        /// marque, elle en reçoit une ou pas
        /// ([ADR-0023](../../../../docs/adr/0023-fournisseurs-declares-et-provenance.md)).
        provenance: Option<oxyn_core::Provenance>,
    },
    Working(Box<Document>),
}
impl gpui::EventEmitter<OpenQuery> for QueryLibrary {}

impl QueryLibrary {
    pub(super) fn can_open_copy(&self) -> bool {
        self.current.is_some()
            && match &self.detail {
                Some(Detail::History(entry)) => {
                    matches!(entry.record.language, QueryLanguage::Sql(_))
                        && !entry.record.requires_reconciliation()
                }
                Some(Detail::Document(doc)) => matches!(doc.language, QueryLanguage::Sql(_)),
                None => false,
            }
    }
    pub(super) fn can_edit_original(&self) -> bool {
        matches!((&self.detail, &self.current), (Some(Detail::Document(doc)), Some((connection, _)))
            if doc.connection == Some(*connection) && matches!(doc.language, QueryLanguage::Sql(_)))
    }
    pub(super) fn open_copy(&mut self, cx: &mut Context<'_, Self>) {
        if !self.can_open_copy() {
            return;
        }
        let (title, origin) = match &self.detail {
            Some(Detail::History(entry)) => (
                "History copy.sql".to_owned(),
                format!(
                    "{} · {} · {}",
                    if entry.record.actor_kind.is_agent() {
                        "Agent history"
                    } else {
                        "History"
                    },
                    entry
                        .record
                        .connection_name
                        .as_deref()
                        .unwrap_or("Unavailable connection"),
                    entry.record.language
                ),
            ),
            Some(Detail::Document(doc)) => (
                if self.working_copy {
                    doc.title.clone()
                } else {
                    doc.saved_title.clone().unwrap_or_else(|| doc.title.clone())
                },
                format!(
                    "Saved query · {} · {}",
                    self.selected
                        .and_then(|index| self.rows.get(index))
                        .map_or("Unavailable connection", |row| row.connection.as_str()),
                    doc.language
                ),
            ),
            None => return,
        };
        let title = if title.len() <= 256 && !title.trim().is_empty() {
            title
        } else {
            "Query copy.sql".into()
        };
        cx.emit(OpenQuery::Copy {
            text: self.reader.read(cx).text(),
            title,
            origin,
            provenance: None,
        });
    }
    pub(super) fn edit_original(&mut self, cx: &mut Context<'_, Self>) {
        if self.can_edit_original()
            && let Some(Detail::Document(doc)) = &self.detail
        {
            cx.emit(OpenQuery::Working(doc.clone()));
        }
    }
}
