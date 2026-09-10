//! Export belongs to the producing console, including when its tab is hidden.
use super::*;

impl QueryConsole {
    /// Asks the platform where to write, then submits the export.
    ///
    /// The dialog is modal to the system but **not** blocking here: the window
    /// keeps drawing while it is open ([I-05](../../../CLAUDE.md#i-05)), which
    /// is the same shape `connection_form` uses to pick a database file.
    pub(in crate::workspace) fn choose_export_destination(
        &mut self,
        format: ExportFormat,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(result) = self.last_result else {
            return;
        };
        if self.export_active.is_some() {
            return;
        }
        let depart = std::env::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let propose = format!("result.{}", format.extension());
        let attente = cx.prompt_for_new_path(&depart, Some(&propose));
        cx.spawn(async move |this, cx| {
            // Cancelled dialog, failed dialog, empty choice: in all three the
            // user closed the window they opened, and there is nothing to say.
            let Ok(Ok(Some(destination))) = attente.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.start_export(result, format, destination, cx);
            });
        })
        .detach();
    }

    /// Submits the export to the bus and follows its outcome.
    pub(in crate::workspace) fn start_export(
        &mut self,
        result: ResultId,
        format: ExportFormat,
        destination: PathBuf,
        cx: &mut Context<'_, Self>,
    ) {
        // The file dialog is not modal to Oxyn: a new execution may have run
        // while it was open, and the result on screen is no longer this one.
        // Writing it anyway would produce a file whose name says one query and
        // whose rows come from another.
        if self.closed
            || self.document_closing
            || self.export_active.is_some()
            || self.last_result != Some(result)
        {
            return;
        }
        let Some(connection) = self.connection else {
            return;
        };
        let shown = destination.to_string_lossy().into_owned();
        let command = super::super::export::export_command(connection, result, format, destination);
        let id = CommandId::new();
        let cancel = CancelToken::new();
        self.export_active = Some((id, cancel.clone()));
        self.export
            .clone()
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
                this.export.clone().update(cx, |export, cx| match result {
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
    pub(in crate::workspace) fn cancel_export(&mut self, cx: &mut Context<'_, Self>) {
        if let Some((_, cancel)) = &self.export_active {
            cancel.cancel();
            cx.notify();
        }
    }
}
