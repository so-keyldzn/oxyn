//! Bounded local page reads are retained with their console.
use super::*;

impl QueryConsole {
    pub(in crate::workspace) fn on_page_event(
        &mut self,
        event: &GridEvent,
        cx: &mut Context<'_, Self>,
    ) {
        match event {
            GridEvent::PageRequested { generation, batch } => {
                self.load_result_page(*generation, *batch, cx)
            }
            GridEvent::CancelPageRequested { generation } => {
                if self
                    .page_active
                    .as_ref()
                    .is_some_and(|request| request.2 == *generation)
                    && let Some((_, token, _)) = self.page_active.take()
                {
                    token.cancel();
                }
            }
            _ => {}
        }
    }

    fn load_result_page(&mut self, generation: u64, batch: usize, cx: &mut Context<'_, Self>) {
        let Some(result) = self.displayed_result else {
            self.grid.clone().update(cx, |grid, cx| {
                grid.complete_page(
                    generation,
                    batch,
                    Some("This result is no longer available.".into()),
                    cx,
                )
            });
            return;
        };
        if let Some((_, cancel, _)) = self.page_active.take() {
            cancel.cancel();
        }
        let Some(connection) = self.connection else {
            return;
        };
        let id = CommandId::new();
        let cancel = CancelToken::new();
        self.page_active = Some((id, cancel.clone(), generation));
        let response = self.backend.dispatch(
            id,
            Command::ReadResultPage {
                connection,
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
                if this.page_active.as_ref().map(|run| run.0) != Some(id) {
                    if let Ok(Outcome::NeedsApproval { command, .. }) = outcome {
                        drop(this.backend.decide(command, false, CancelToken::new()));
                    }
                    return;
                }
                this.page_active = None;
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
                if this.displayed_result == Some(result) {
                    this.grid.clone().update(cx, |grid, cx| {
                        grid.complete_page(generation, batch, error, cx)
                    });
                }
            });
        })
        .detach();
    }
}
