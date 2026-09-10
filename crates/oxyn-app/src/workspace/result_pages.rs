//! Loads existing pages through the bus and correlates their completion with a grid.

use super::*;

impl Workspace {
    pub(super) fn result_grid(&self, source: ResultSource) -> Entity<DataGrid> {
        match source {
            ResultSource::Query => self.grid.clone(),
            ResultSource::Preview => self.preview_grid.clone(),
        }
    }

    pub(super) fn on_page_event(
        &mut self,
        source: ResultSource,
        event: &GridEvent,
        cx: &mut Context<'_, Self>,
    ) {
        if source == ResultSource::Query {
            self.console
                .update(cx, |console, cx| console.on_page_event(event, cx));
            return;
        }
        match event {
            GridEvent::PageRequested { generation, batch } => {
                self.load_result_page(source, *generation, *batch, cx)
            }
            GridEvent::CancelPageRequested { generation } => {
                if self
                    .page_reads
                    .get(&source)
                    .is_some_and(|run| run.2 == *generation)
                    && let Some((_, cancel, _)) = self.page_reads.remove(&source)
                {
                    cancel.cancel();
                }
            }
            _ => {}
        }
    }

    fn load_result_page(
        &mut self,
        source: ResultSource,
        generation: u64,
        batch: usize,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(result) = self.displayed_results.get(&source).copied() else {
            self.result_grid(source).update(cx, |grid, cx| {
                grid.complete_page(
                    generation,
                    batch,
                    Some("This result is no longer available.".into()),
                    cx,
                )
            });
            return;
        };
        if let Some((_, cancel, _)) = self.page_reads.remove(&source) {
            cancel.cancel();
        }
        let id = CommandId::new();
        let cancel = CancelToken::new();
        self.page_reads
            .insert(source, (id, cancel.clone(), generation));
        let response = self.backend.dispatch(
            id,
            Command::ReadResultPage {
                connection: self.connection,
                result,
                batch,
            },
            cancel,
        );
        cx.spawn(async move |this, cx| {
            let outcome = response.await.unwrap_or_else(|_| {
                Err(OxynError::Internal(
                    "The result page worker stopped answering".into(),
                ))
            });
            let _ = this.update(cx, |this, cx| {
                if this.page_reads.get(&source).map(|run| run.0) != Some(id) {
                    if let Ok(Outcome::NeedsApproval { command, .. }) = outcome {
                        drop(this.backend.decide(command, false, CancelToken::new()));
                    }
                    return;
                }
                this.page_reads.remove(&source);
                let error = match outcome {
                    Ok(Outcome::ResultPageRead {
                        result: loaded,
                        batch: loaded_batch,
                    }) if loaded == result && loaded_batch == batch => None,
                    Ok(Outcome::NeedsApproval { command, .. }) => {
                        drop(this.backend.decide(command, false, CancelToken::new()));
                        Some(
                            "The connection policy does not allow automatic local page loading."
                                .into(),
                        )
                    }
                    Ok(Outcome::Denied { reason, .. }) => Some(reason),
                    Err(error) => Some(error.to_string()),
                    _ => Some("Unexpected result page response".into()),
                };
                if this.displayed_results.get(&source) == Some(&result) {
                    this.result_grid(source).update(cx, |grid, cx| {
                        grid.complete_page(generation, batch, error, cx)
                    });
                }
            });
        })
        .detach();
    }
}
