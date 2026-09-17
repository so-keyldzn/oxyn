//! Named saves retain the exact acknowledged snapshot while editing can continue.

use super::*;
use oxyn_core::QueryDocumentUpdate;
use std::time::Duration;

/// Le repos de frappe au-delà duquel le brouillon part.
///
/// 250 ms : sous le budget de 300 ms de « retour visible » — donc imperceptible
/// à qui s'arrête de taper — et au-dessus de l'intervalle d'une frappe rapide,
/// pour qu'une phrase tapée d'un trait ne produise qu'une écriture. Un choix de
/// produit, pas une mesure ; il s'amende dans
/// [ADR-0024](../../../../docs/adr/0024-autosauvegarde-au-repos-de-frappe.md).
const DRAFT_IDLE: Duration = Duration::from_millis(250);

impl QueryConsole {
    pub(in crate::workspace) fn open_library_query(
        &mut self,
        query: super::super::library::OpenQuery,
        cx: &mut Context<'_, Self>,
    ) {
        match query {
            super::super::library::OpenQuery::Copy {
                text,
                title,
                origin,
                provenance,
            } => {
                // La marque suit le texte jusqu'à la première écriture. Elle
                // est posée ici et non plus loin : à partir de l'autosauvegarde,
                // plus rien ne sait d'où ce texte venait (ADR-0023).
                self.provenance = provenance;
                self.title = title.clone();
                self.name.update(cx, |name, cx| name.set_text(title, cx));
                self.editor
                    .update(cx, |editor, cx| editor.set_text(&text, cx));
                self.status.update(cx, |status, cx| status.set_notice(Some(format!("Copy from {origin}. Review for this connection before running; nothing was executed.")), cx));
            }
            super::super::library::OpenQuery::Working(document) => {
                let document = *document;
                self.id = document.id;
                if self.session.is_none() {
                    self.connection = document.connection;
                }
                self.document_revision = document.revision.max(document.saved_revision);
                self.document_language = document.language;
                self.writer = self.backend.document_writer(self.id, document.revision);
                self.saved_text = document.saved_content.unwrap_or_default();
                self.saved_title = document.saved_title.unwrap_or_default();
                self.has_saved_copy = document.is_saved;
                self.title = document.title.clone();
                let title = document.title;
                let text = document.content;
                self.name.update(cx, |name, cx| name.set_text(title, cx));
                self.editor
                    .update(cx, |editor, cx| editor.set_text(&text, cx));
                self.status.update(cx, |status, cx| {
                    status.set_notice(Some("Working copy resumed. Nothing was executed."), cx)
                });
            }
        }
        self.refresh_document_state(cx);
    }

    pub(crate) fn has_unsaved_changes(&self, cx: &gpui::App) -> bool {
        self.save_conflict
            || self.editor.read(cx).text() != self.saved_text
            || self.name.read(cx).text() != self.saved_title
    }

    fn refresh_document_state(&mut self, cx: &mut Context<'_, Self>) {
        self.dirty = self.has_unsaved_changes(cx);
        if self.save_active.is_none()
            && !self.document_closing
            && !self.save_problem
            && !self.save_conflict
        {
            self.save_notice = if !self.has_saved_copy {
                "No saved copy yet."
            } else if self.dirty {
                "Unsaved changes since the last save."
            } else {
                "Matches the saved version."
            }
            .into();
        }
        cx.notify();
    }

    /// Une frappe : on marque, on ne copie pas.
    ///
    /// `editor.text()` est un `join` sur toutes les lignes — une copie complète
    /// du document, jusqu'à un mégaoctet, sur le fil d'interface. La faire à
    /// chaque touche est la cause mesurée du seul dépassement de budget de
    /// trame du produit : 17 à-coups en 14 s de frappe, dont un de 50 ms pour un
    /// budget de 8 ms ([ADR-0024](../../../../docs/adr/0024-autosauvegarde-au-repos-de-frappe.md)).
    ///
    /// Le minuteur est relancé à chaque touche : une phrase tapée d'un trait
    /// n'écrit rien tant qu'elle dure, et écrit une fois quand elle s'arrête.
    pub(crate) fn document_changed(&mut self, cx: &mut Context<'_, Self>) {
        self.refresh_document_state(cx);
        if self.result_only || self.closed || self.document_closing || self.save_conflict {
            return;
        }
        // Compter la frappe suffit : **une seule** tâche est en vol, et elle se
        // rendort tant que le compteur bouge.
        //
        // Spawner une tâche et un minuteur à chaque touche coûte plus cher que
        // la copie qu'on cherchait à éviter : la première version de ce code
        // l'a fait, et la mesure est passée de 17 à-coups à **48**, avec un pic
        // de 50 ms à 210 ms. Une frappe rapide crée mille tâches en quelques
        // secondes ; c'est la création qui coûte, pas l'attente.
        self.draft_debounce = self.draft_debounce.wrapping_add(1);
        if self.draft_timer {
            return;
        }
        self.draft_timer = true;
        cx.spawn(async move |console, cx| {
            loop {
                let vu = match console.read_with(cx, |console, _| console.draft_debounce) {
                    Ok(vu) => vu,
                    Err(_) => return,
                };
                cx.background_executor().timer(DRAFT_IDLE).await;
                let encore = console.update(cx, |console, cx| {
                    if console.draft_debounce == vu {
                        // Rien n'a bougé pendant l'attente : c'est le repos.
                        console.draft_timer = false;
                        console.write_draft_now(cx);
                        false
                    } else {
                        true
                    }
                });
                match encore {
                    Ok(true) => continue,
                    _ => return,
                }
            }
        })
        .detach();
    }

    /// Copie le brouillon et l'envoie **maintenant**.
    ///
    /// Appelée par le minuteur au repos de frappe, et par les trois échappées
    /// immédiates d'ADR-0024 : perte de focus, fermeture, exécution. Ces
    /// trois-là bornent ce qu'un arrêt brutal peut coûter.
    pub(crate) fn write_draft_now(&mut self, cx: &mut Context<'_, Self>) {
        // Un envoi immédiat annule le minuteur en cours : sans cela, la même
        // révision partirait deux fois. Le drapeau retombe aussi, sans quoi la
        // frappe suivante croirait une tâche en vol et n'en armerait aucune —
        // le brouillon ne repartirait plus jamais.
        self.draft_debounce = self.draft_debounce.wrapping_add(1);
        self.draft_timer = false;
        if self.result_only || self.closed || self.document_closing || self.save_conflict {
            return;
        }
        let Some(revision) = self.document_revision.checked_add(1) else {
            return;
        };
        self.document_revision = revision;
        let update = QueryDocumentUpdate {
            document: self.id,
            revision,
            expected_revision: None,
            title: self.name.read(cx).text().to_owned(),
            language: self.document_language,
            text: self.editor.read(cx).text(),
            connection: self.connection,
            save_named: false,
            is_open: true,
            provenance: self.provenance.clone(),
        };
        if let Err(error) = update.validate() {
            self.draft_pending = None;
            self.draft_notice = format!("Draft not saved: {error}");
            cx.notify();
            return;
        }
        self.draft_pending = Some(revision);
        self.draft_notice = "Saving recovery draft…".into();
        let document = self.id;
        let response = self.writer.save(update, CancelToken::new());
        cx.spawn(async move |this, cx| {
            let result = response.await.unwrap_or_else(|_| {
                Err(OxynError::Internal("Draft writer stopped answering".into()))
            });
            let _ = this.update(cx, |this, cx| {
                if this.id != document || this.draft_pending != Some(revision) {
                    return;
                }
                this.draft_pending = None;
                if this.closed || this.document_closing {
                    return;
                }
                match result {
                    Ok(Outcome::DocumentOpened { .. }) => {
                        this.draft_notice = "Recovery draft saved locally.".into()
                    }
                    Err(error) => {
                        if matches!(error, OxynError::Serialization(_)) {
                            this.save_conflict = true;
                            this.dirty = true;
                        }
                        this.draft_notice = format!("Draft not saved: {error}");
                    }
                    _ => this.draft_notice = "Unexpected draft response.".into(),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn save_document(&mut self, cx: &mut Context<'_, Self>) {
        if self.result_only
            || self.closed
            || self.save_conflict
            || self.save_active.is_some()
            || self.document_closing
        {
            return;
        }
        let Some(revision) = self.document_revision.checked_add(1) else {
            self.save_notice = "Document revision is exhausted.".into();
            cx.notify();
            return;
        };
        self.title = self.name.read(cx).text().to_owned();
        let update = QueryDocumentUpdate {
            expected_revision: None,
            document: self.id,
            revision,
            title: self.title.clone(),
            language: self.document_language,
            text: self.editor.read(cx).text(),
            connection: self.connection,
            save_named: true,
            is_open: true,
            provenance: self.provenance.clone(),
        };
        if let Err(error) = update.validate() {
            self.save_problem = true;
            self.save_notice = error.to_string();
            cx.notify();
            return;
        }
        self.document_revision = revision;
        let id = CommandId::new();
        let cancel = CancelToken::new();
        self.save_active = Some((id, cancel.clone()));
        self.save_problem = false;
        self.save_notice = "Saving named query…".into();
        let response = self.writer.save(update.clone(), cancel);
        cx.spawn(async move |this, cx| {
            let outcome = response.await.unwrap_or_else(|_| Err(OxynError::Internal("Query save worker stopped answering".into())));
            let _ = this.update(cx, |this, cx| {
                if this.save_active.as_ref().map(|request| request.0) != Some(id) { return; }
                this.save_active = None;
                let mut saved = false;
                match outcome {
                    Ok(Outcome::DocumentOpened { document }) if document.saved_revision == revision
                        && document.saved_content.as_deref() == Some(&update.text)
                        && document.saved_title.as_deref() == Some(&update.title) => {
                        saved = true;
                        this.save_conflict = false;
                        this.saved_text = update.text; this.saved_title = update.title;
                        this.has_saved_copy = true;
                        if this.document_revision == revision { this.draft_notice = "Recovery draft saved locally.".into(); }
                        this.document_revision = this.document_revision.max(document.revision);
                        this.refresh_document_state(cx);
                        this.save_notice = if this.dirty { "Saved the requested version. Newer edits are not saved." } else { "Saved in query library." }.into();
                    }
                    Ok(Outcome::DocumentOpened { document }) => {
                        this.document_revision = this.document_revision.max(document.revision).max(document.saved_revision);
                        this.save_conflict = true;
                        this.dirty = true;
                        this.save_notice = "The stored query changed elsewhere. Save a new query to preserve both versions.".into();
                    }
                    Ok(Outcome::NeedsApproval { command, .. }) => {
                        drop(this.backend.decide(command, false, CancelToken::new()));
                        this.save_notice = "The policy does not allow this local save.".into();
                    }
                    Ok(Outcome::Denied { reason, .. }) => this.save_notice = reason,
                    Err(error) => {
                        if matches!(error, OxynError::Serialization(_)) { this.save_conflict = true; this.dirty = true; }
                        this.save_notice = error.to_string();
                    },
                    _ => this.save_notice = "Unexpected query save response.".into(),
                }
                this.save_problem = !saved;
                let close = std::mem::take(&mut this.close_after_save);
                if saved && close && !this.dirty { this.close_document(false, cx); }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    pub(crate) fn save_as_new_query(&mut self, cx: &mut Context<'_, Self>) {
        if self.closed || self.document_closing || self.save_active.is_some() {
            return;
        }
        self.id = DocumentId::new();
        self.document_revision = 0;
        self.writer = self.backend.document_writer(self.id, 0);
        self.draft_pending = None;
        self.has_saved_copy = false;
        self.saved_text.clear();
        self.saved_title.clear();
        self.save_conflict = false;
        self.refresh_document_state(cx);
        self.save_document(cx);
    }

    pub(crate) fn save_and_close(&mut self, cx: &mut Context<'_, Self>) {
        if self.save_conflict {
            self.save_as_new_query(cx);
        } else {
            self.save_document(cx);
        }
        self.close_after_save = self.save_active.is_some();
    }

    pub(crate) fn cancel_save(&mut self, cx: &mut Context<'_, Self>) {
        self.close_after_save = false;
        if let Some((_, cancel)) = &self.save_active {
            cancel.cancel();
            self.save_notice = "Cancelling save…".into();
            cx.notify();
        }
    }

    pub(crate) fn close_document(&mut self, discard: bool, cx: &mut Context<'_, Self>) {
        if self.closed || self.document_closing || self.save_active.is_some() {
            return;
        }
        // Échappée immédiate d'ADR-0024. Sans elle, fermer une console dans les
        // 250 ms qui suivent la dernière touche perdrait ces caractères — et
        // `discard` déciderait du sort d'un brouillon qui n'a jamais été écrit.
        if !discard {
            self.write_draft_now(cx);
        }
        if self.result_only || self.save_conflict {
            self.editor
                .update(cx, |editor, cx| editor.set_read_only(true, cx));
            self.name
                .update(cx, |name, cx| name.set_read_only(true, cx));
            self.shutdown();
            cx.emit(ConsoleEvent::Closed);
            return;
        }
        let Some(revision) = self.document_revision.checked_add(1) else {
            self.save_notice = "Document revision is exhausted.".into();
            cx.notify();
            return;
        };
        self.document_revision = revision;
        self.document_closing = true;
        self.editor
            .update(cx, |editor, cx| editor.set_read_only(true, cx));
        self.name
            .update(cx, |name, cx| name.set_read_only(true, cx));
        self.save_notice = "Closing the saved query…".into();
        let cancel = CancelToken::new();
        self.document_close_token = Some(cancel.clone());
        let response = self
            .writer
            .close(revision, discard || !self.has_saved_copy, cancel);
        cx.spawn(async move |this, cx| {
            let outcome = response.await.unwrap_or_else(|_| {
                Err(OxynError::Internal(
                    "Query close worker stopped answering".into(),
                ))
            });
            let _ = this.update(cx, |this, cx| {
                this.document_closing = false;
                this.document_close_token = None;
                match outcome {
                    Ok(Outcome::DocumentClosed { document }) if document == this.id => {
                        this.shutdown();
                        cx.emit(ConsoleEvent::Closed);
                    }
                    Ok(Outcome::Denied { reason, .. }) => this.save_notice = reason,
                    Err(error) => {
                        if matches!(error, OxynError::Serialization(_)) {
                            this.save_conflict = true;
                            this.dirty = true;
                        }
                        this.save_notice = error.to_string();
                    }
                    _ => this.save_notice = "Unexpected query close response.".into(),
                }
                if !this.closed {
                    this.editor
                        .update(cx, |editor, cx| editor.set_read_only(false, cx));
                    this.name
                        .update(cx, |name, cx| name.set_read_only(false, cx));
                    if !this.save_conflict {
                        this.document_changed(cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(crate) fn cancel_document_close(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(cancel) = &self.document_close_token {
            cancel.cancel();
            self.save_notice = "Cancelling close…".into();
            cx.notify();
        }
    }
}
