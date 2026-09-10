//! Writing the displayed result to a file, correlated with its own command.
//!
//! Split from the execution path on purpose: an export runs *beside* a
//! statement, with its own identifier and its own cancellation token, and
//! sharing either would make cancelling one stop the other.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum ResultSource {
    Query,
    Preview,
}

/// Whether a finished execution may be exported, and why not when it may not.
///
/// A free function, and tested as one: this is the decision that turns a
/// truncated result into a file that looks like a whole table. Left inside the
/// async closure that receives the outcome, it is reachable by no test at all —
/// producing a truncated buffer through a `Workspace` would mean overflowing
/// the memory budget for real. The day someone "simplifies" it back to
/// `!cancelled && complete`, everything stays green
/// ([ui-gpui](../../../../.claude/rules/ui-gpui.md): le calcul sort, la vue
/// dessine).
///
/// `complete` means **the stream is closed**, not that every row is there:
/// `BatchSink::seal` marks a row-limited or budget-stopped result truncated and
/// *then* complete.
pub(super) const fn exportability(
    cancelled: bool,
    complete: bool,
    truncated: bool,
) -> Option<NotExportable> {
    if cancelled || !complete {
        Some(NotExportable::NoResult)
    } else if truncated {
        Some(NotExportable::Truncated)
    } else {
        None
    }
}

/// Builds the export of a result the executor still holds.
///
/// Carries a [`ResultId`], never rows: the executor streams the shared
/// `ResultBuffer` to the file, so nothing is materialised to be written
/// ([I-06](../../../CLAUDE.md#i-06)).
pub(crate) fn export_command(
    connection: oxyn_core::ConnectionId,
    result: ResultId,
    format: ExportFormat,
    destination: PathBuf,
) -> Command {
    Command::Export {
        connection,
        result,
        format,
        destination,
    }
}

impl Workspace {
    fn export_result(&self, source: ResultSource, cx: &gpui::App) -> Option<ResultId> {
        match source {
            ResultSource::Query => self.console.read(cx).last_result,
            ResultSource::Preview => self.preview_result,
        }
    }

    fn export_view(&self, source: ResultSource) -> Entity<ResultExport> {
        match source {
            ResultSource::Query => self.export.clone(),
            ResultSource::Preview => self.preview_export.clone(),
        }
    }

    /// Asks the platform where to write, then submits the export.
    ///
    /// The dialog is modal to the system but **not** blocking here: the window
    /// keeps drawing while it is open ([I-05](../../../CLAUDE.md#i-05)), which
    /// is the same shape `connection_form` uses to pick a database file.
    pub(super) fn choose_export_destination(
        &mut self,
        source: ResultSource,
        format: ExportFormat,
        cx: &mut Context<'_, Self>,
    ) {
        if source == ResultSource::Query {
            self.console.update(cx, |console, cx| {
                console.choose_export_destination(format, cx)
            });
            return;
        }
        let Some(result) = self.export_result(source, cx) else {
            return;
        };
        if self.export_active.is_some() {
            return;
        }
        let depart = std::env::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let stem = if source == ResultSource::Preview {
            "preview"
        } else {
            "result"
        };
        let propose = format!("{stem}.{}", format.extension());
        let attente = cx.prompt_for_new_path(&depart, Some(&propose));
        cx.spawn(async move |this, cx| {
            // Cancelled dialog, failed dialog, empty choice: in all three the
            // user closed the window they opened, and there is nothing to say.
            let Ok(Ok(Some(destination))) = attente.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.start_export(source, result, format, destination, cx);
            });
        })
        .detach();
    }

    /// Submits the export to the bus and follows its outcome.
    pub(super) fn start_export(
        &mut self,
        source: ResultSource,
        result: ResultId,
        format: ExportFormat,
        destination: PathBuf,
        cx: &mut Context<'_, Self>,
    ) {
        if source == ResultSource::Query {
            self.console.update(cx, |console, cx| {
                console.start_export(result, format, destination, cx)
            });
            return;
        }
        // The file dialog is not modal to Oxyn: a new execution may have run
        // while it was open, and the result on screen is no longer this one.
        // Writing it anyway would produce a file whose name says one query and
        // whose rows come from another.
        if self.export_active.is_some() || self.export_result(source, cx) != Some(result) {
            return;
        }
        let shown = destination.to_string_lossy().into_owned();
        let command = export_command(self.connection, result, format, destination);
        let id = CommandId::new();
        let cancel = CancelToken::new();
        self.export_active = Some((id, cancel.clone(), source));
        self.export_view(source)
            .update(cx, |export, cx| export.running(cx));
        let response = self.backend.dispatch(id, command, cancel);
        cx.spawn(async move |this, cx| {
            let result = response.await.unwrap_or_else(|_| {
                Err(OxynError::Internal("The executor stopped answering".into()))
            });
            let _ = this.update(cx, |this, cx| {
                if this.export_active.as_ref().map(|run| run.0) != Some(id) {
                    return;
                }
                this.export_active = None;
                this.export_view(source)
                    .update(cx, |export, cx| match result {
                        Ok(Outcome::Exported { rows, bytes, .. }) => {
                            export.written(rows, bytes, shown, cx);
                        }
                        Ok(Outcome::Denied { reason, .. }) => export.failed(reason, false, cx),
                        // The bytes already written stay on disk. Returning to the
                        // neutral state would leave a truncated file with nothing
                        // saying where it stops.
                        Err(OxynError::Cancelled) => export.cancelled(shown, cx),
                        Err(error) => {
                            let retryable = error.is_retryable();
                            export.failed(error.to_string(), retryable, cx);
                        }
                        Ok(_) => export.failed("Unexpected export response", false, cx),
                    });
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Stops the export under way. The file already begun is left as it is:
    /// truncating it here would delete something the user may want to inspect.
    pub(super) fn cancel_export(&mut self, cx: &mut Context<'_, Self>) {
        if let Some((_, cancel, _)) = &self.export_active {
            cancel.cancel();
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_resultat_tronque_nest_pas_exportable_bien_quil_soit_clos() {
        // Le cas qui compte : `SELECT * FROM commandes` sur cinquante millions
        // de lignes, le tampon s'arrête au budget mémoire, se clôt — et rien à
        // l'écran ne le distingue d'un résultat complet. Exporter écrirait un
        // CSV que l'utilisateur croirait être la table.
        assert_eq!(
            exportability(false, true, true),
            Some(NotExportable::Truncated)
        );
    }

    #[test]
    fn un_resultat_entier_est_exportable() {
        assert_eq!(exportability(false, true, false), None);
    }

    #[test]
    fn un_resultat_annule_ou_encore_ouvert_ne_lest_pas() {
        // Les deux valent « pas de résultat » et non « tronqué » : l'un et
        // l'autre se corrigent en relançant, pas en réduisant la requête.
        assert_eq!(
            exportability(true, true, false),
            Some(NotExportable::NoResult)
        );
        assert_eq!(
            exportability(false, false, false),
            Some(NotExportable::NoResult)
        );
        assert_eq!(
            exportability(true, false, true),
            Some(NotExportable::NoResult)
        );
    }
}
