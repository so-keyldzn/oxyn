//! L'éditeur de requêtes.
//!
//! # Ce que ce fichier **ne fait pas**, et qu'il faut savoir avant de le lire
//!
//! [ADR-0001](../../../docs/adr/0001-ui-toolkit.md) désigne l'éditeur comme le
//! poste de coût numéro un du projet, et
//! [ARCHITECTURE §12](../../../docs/ARCHITECTURE.md) le classe en risque élevé.
//! Ce qui est ici est un éditeur **complet sur ce qu'il fait** et
//! **explicitement incomplet** sur le reste. La liste est courte, elle est
//! exacte, et elle n'est pas une politesse :
//!
//! * `// TODO(phase 0, suite)` — **coloration syntaxique**. Aucune. Le texte est
//!   d'une seule couleur. Débloqué par le choix d'un analyseur : `sqlparser`
//!   est déjà au graphe de dépendances d'`oxyn-query`, mais il analyse une
//!   instruction complète et échoue sur du SQL en cours de frappe ; il faut un
//!   analyseur lexical tolérant, qui reste à écrire.
//! * `// TODO(phase 0, suite)` — **complétion**. Aucune. Débloquée par le
//!   catalogue *et* par l'analyse lexicale ci-dessus : compléter demande de
//!   savoir si le curseur est après un `FROM` ou dans une chaîne littérale.
//! * `// TODO(phase 0, suite)` — **curseurs multiples**. Un seul curseur, une
//!   seule sélection.
//! * `// TODO(phase 1)` — **retour à la ligne visuel**. Une ligne longue déborde
//!   horizontalement au lieu d'être repliée.
//!
//! Ce qui fonctionne : tampon de texte, curseur, sélection, saisie, retour à la
//! ligne, effacement, presse-papiers, annulation, gouttière de numéros, et
//! `Cmd+Entrée` pour exécuter.
//!
//! # Pourquoi le curseur n'est pas positionné en pixels
//!
//! Une ligne est dessinée comme une suite de morceaux ([`LinePiece`]) posés en
//! ligne : texte avant, curseur, texte après. C'est le moteur de texte qui place
//! le curseur, au pixel près, pour n'importe quelle police — là où une
//! multiplication `colonne × largeur_de_caractère` ne serait juste qu'à chasse
//! fixe et se décalerait à chaque frappe sur tout le reste.
//!
//! # Ce que l'éditeur ne décide pas
//!
//! Execution shortcuts emit [`EditorEvent::ExecuteRequested`]. `oxyn-app`
//! resolves the exact selection or the current statement using the dialect,
//! then submits a [`Command`](oxyn_core::Command) through the `PolicyGate`
//! ([I-01](../../../CLAUDE.md#i-01)).

use gpui::{Bounds, MouseButton, Pixels, ShapedLine, TextRun, canvas};
use std::collections::BTreeMap;
use std::ops::Range;
mod input;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClipboardItem, Context, ElementId, EventEmitter, FocusHandle, Focusable,
    KeyDownEvent, SharedString, UniformListScrollHandle, Window, div, px, uniform_list,
};

use crate::theme::Theme;

/// Profondeur de la pile d'annulation.
///
/// Une requête tient dans quelques kilo-octets ; deux cents états, c'est une
/// session de travail entière et une empreinte négligeable.
pub const UNDO_DEPTH: usize = 200;

/// Espaces insérés par la touche de tabulation.
///
/// Des espaces et non une tabulation : le SQL est relu dans des outils qui ne
/// s'accordent pas sur la largeur d'une tabulation, et un plan d'exécution
/// recopié depuis Oxyn doit rester aligné ailleurs.
pub const TAB_WIDTH: usize = 2;

/// Une position dans le tampon.
///
/// La colonne est un **indice de caractère**, pas d'octet : un `é` compte pour
/// un. Les décalages en octets ne servent qu'au découpage interne, et ne sortent
/// jamais de ce module — un `column` en octets finirait par être utilisé comme
/// une colonne d'affichage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct TextPosition {
    /// Ligne, à partir de 0.
    pub line: usize,
    /// Caractère dans la ligne, à partir de 0.
    pub column: usize,
}

impl TextPosition {
    /// Une position.
    #[must_use]
    pub const fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }
}

/// Le texte édité, ligne par ligne.
///
/// Un `Vec<String>` et non un rope : une requête fait quelques kilo-octets, un
/// rope se justifie à partir du méga-octet, et une structure qu'on ne sait pas
/// déboguer coûte plus qu'elle ne rapporte à cette taille.
/// TODO(phase 2) : passer à un rope si l'éditeur reçoit des scripts de
/// migration de plusieurs mégaoctets ; le déclencheur est mesurable, pas
/// esthétique.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextBuffer {
    lines: Vec<String>,
}

impl Default for TextBuffer {
    fn default() -> Self {
        Self {
            lines: vec![String::new()],
        }
    }
}

impl TextBuffer {
    /// Un tampon à partir d'un texte.
    ///
    /// `\r\n` est ramené à `\n` : un fichier SQL venu de Windows ne doit pas
    /// laisser un caractère invisible en fin de chaque ligne, qui se retrouverait
    /// dans l'instruction envoyée au serveur.
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        let lignes: Vec<String> = text
            .replace("\r\n", "\n")
            .split('\n')
            .map(str::to_owned)
            .collect();
        Self {
            lines: if lignes.is_empty() {
                vec![String::new()]
            } else {
                lignes
            },
        }
    }

    /// Le texte complet.
    #[must_use]
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// Nombre de lignes. Toujours au moins 1.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.lines.len().max(1)
    }

    /// Une ligne.
    #[must_use]
    pub fn line(&self, index: usize) -> Option<&str> {
        self.lines.get(index).map(String::as_str)
    }

    /// Nombre de caractères d'une ligne, `0` si elle n'existe pas.
    #[must_use]
    pub fn line_len(&self, index: usize) -> usize {
        self.line(index).map_or(0, |ligne| ligne.chars().count())
    }

    /// La position, ramenée dans les bornes du tampon.
    ///
    /// Toutes les mutations passent par là : une position hors bornes vient
    /// d'un clic ou d'une touche, donc d'une entrée, et une entrée ne panique
    /// pas ([I-09](../../../CLAUDE.md#i-09)).
    #[must_use]
    pub fn clamp(&self, position: TextPosition) -> TextPosition {
        let ligne = position.line.min(self.line_count().saturating_sub(1));
        TextPosition {
            line: ligne,
            column: position.column.min(self.line_len(ligne)),
        }
    }

    /// La dernière position du tampon.
    #[must_use]
    pub fn end(&self) -> TextPosition {
        let ligne = self.line_count().saturating_sub(1);
        TextPosition::new(ligne, self.line_len(ligne))
    }

    /// Le décalage en octets d'une colonne dans une ligne.
    fn byte_offset(ligne: &str, colonne: usize) -> usize {
        ligne
            .char_indices()
            .nth(colonne)
            .map_or(ligne.len(), |(octet, _)| octet)
    }

    /// Insère du texte et rend la position d'après.
    ///
    /// Les retours à la ligne du texte inséré sont respectés : c'est ce qui fait
    /// marcher le collage d'une requête multiligne.
    pub fn insert(&mut self, at: TextPosition, text: &str) -> TextPosition {
        let at = self.clamp(at);
        let normalise = text.replace("\r\n", "\n");
        let segments: Vec<&str> = normalise.split('\n').collect();
        let Some(premier) = segments.first().copied() else {
            return at;
        };

        // La queue de la ligne courante est mise de côté, puis recollée à la
        // fin du dernier segment inséré. Le bloc borne l'emprunt mutable sur
        // `self.lines`, que la suite doit pouvoir reprendre.
        let queue = {
            let Some(ligne) = self.lines.get_mut(at.line) else {
                return at;
            };
            let coupe = Self::byte_offset(ligne, at.column);
            let queue = ligne.split_off(coupe);
            ligne.push_str(premier);
            queue
        };

        if segments.len() == 1 {
            if let Some(ligne) = self.lines.get_mut(at.line) {
                ligne.push_str(&queue);
            }
            return TextPosition::new(at.line, at.column.saturating_add(premier.chars().count()));
        }

        let dernier = segments.len().saturating_sub(1);
        let mut position = at;
        for (decalage, segment) in segments.iter().enumerate().skip(1) {
            let index = at.line.saturating_add(decalage);
            let mut nouvelle = (*segment).to_owned();
            if decalage == dernier {
                position = TextPosition::new(index, nouvelle.chars().count());
                nouvelle.push_str(&queue);
            }
            self.lines.insert(index.min(self.lines.len()), nouvelle);
        }
        position
    }

    /// Efface le caractère avant la position, ou fusionne avec la ligne d'avant.
    pub fn delete_backward(&mut self, at: TextPosition) -> TextPosition {
        let at = self.clamp(at);
        if at.column > 0 {
            let avant = TextPosition::new(at.line, at.column.saturating_sub(1));
            self.delete_range(avant, at);
            return avant;
        }
        if at.line == 0 {
            return at;
        }
        let precedente = at.line.saturating_sub(1);
        let fin = TextPosition::new(precedente, self.line_len(precedente));
        self.delete_range(fin, at);
        fin
    }

    /// Efface le caractère après la position, ou colle la ligne suivante.
    pub fn delete_forward(&mut self, at: TextPosition) -> TextPosition {
        let at = self.clamp(at);
        if at.column < self.line_len(at.line) {
            self.delete_range(at, TextPosition::new(at.line, at.column.saturating_add(1)));
        } else if at.line.saturating_add(1) < self.line_count() {
            self.delete_range(at, TextPosition::new(at.line.saturating_add(1), 0));
        }
        at
    }

    /// Efface l'intervalle `[start, end)` et rend son début.
    ///
    /// Les bornes peuvent arriver dans n'importe quel ordre : une sélection
    /// tirée vers le haut a son ancre après son curseur.
    pub fn delete_range(&mut self, start: TextPosition, end: TextPosition) -> TextPosition {
        let (debut, fin) = ordered(self.clamp(start), self.clamp(end));
        if debut == fin {
            return debut;
        }
        // `get` et non l'indexation : un décalage hors borne panique, et cette
        // fonction est atteinte depuis un glissement de souris (I-09).
        let queue = self
            .line(fin.line)
            .map(|ligne| {
                let coupe = Self::byte_offset(ligne, fin.column);
                ligne.get(coupe..).unwrap_or_default().to_owned()
            })
            .unwrap_or_default();

        if let Some(ligne) = self.lines.get_mut(debut.line) {
            let coupe = Self::byte_offset(ligne, debut.column);
            ligne.truncate(coupe);
            ligne.push_str(&queue);
        }
        if fin.line > debut.line {
            let premiere = debut.line.saturating_add(1);
            let derniere = fin.line.min(self.lines.len().saturating_sub(1));
            if premiere <= derniere {
                self.lines.drain(premiere..=derniere);
            }
        }
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        debut
    }

    /// Le texte de l'intervalle `[start, end)`.
    #[must_use]
    pub fn slice(&self, start: TextPosition, end: TextPosition) -> String {
        let (debut, fin) = ordered(self.clamp(start), self.clamp(end));
        if debut.line == fin.line {
            return self
                .line(debut.line)
                .map(|ligne| {
                    let a = Self::byte_offset(ligne, debut.column);
                    let b = Self::byte_offset(ligne, fin.column);
                    ligne.get(a..b).unwrap_or_default().to_owned()
                })
                .unwrap_or_default();
        }
        let mut sortie = String::new();
        for index in debut.line..=fin.line {
            let Some(ligne) = self.line(index) else {
                continue;
            };
            if index == debut.line {
                let a = Self::byte_offset(ligne, debut.column);
                sortie.push_str(ligne.get(a..).unwrap_or_default());
            } else if index == fin.line {
                let b = Self::byte_offset(ligne, fin.column);
                sortie.push_str(ligne.get(..b).unwrap_or_default());
            } else {
                sortie.push_str(ligne);
            }
            if index < fin.line {
                sortie.push('\n');
            }
        }
        sortie
    }
}

/// Les deux positions, remises dans l'ordre du texte.
#[must_use]
fn ordered(a: TextPosition, b: TextPosition) -> (TextPosition, TextPosition) {
    if a <= b { (a, b) } else { (b, a) }
}

/// Un morceau d'une ligne dessinée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinePiece {
    /// Du texte, sélectionné ou non.
    Text {
        /// Le texte du morceau.
        text: String,
        /// Le morceau fait-il partie de la sélection ?
        selected: bool,
    },
    /// Le curseur, entre deux morceaux de texte.
    Cursor,
}

/// Découpe une ligne en morceaux à poser côte à côte.
///
/// `selection` est en colonnes **de cette ligne**, déjà ramenée dans ses bornes.
/// `include_newline` marque la sélection du retour à la ligne : sans lui, une
/// sélection multiligne ne montrerait rien à la fin des lignes intermédiaires,
/// et l'utilisateur ne saurait pas ce qu'il s'apprête à supprimer.
#[must_use]
pub fn line_pieces(
    line: &str,
    selection: Option<Range<usize>>,
    cursor: Option<usize>,
    include_newline: bool,
) -> Vec<LinePiece> {
    let longueur = line.chars().count();
    let selection = selection.map(|plage| plage.start.min(longueur)..plage.end.min(longueur));
    let curseur = cursor.map(|colonne| colonne.min(longueur));

    let mut bornes = vec![0usize, longueur];
    if let Some(plage) = &selection {
        bornes.push(plage.start);
        bornes.push(plage.end);
    }
    if let Some(colonne) = curseur {
        bornes.push(colonne);
    }
    bornes.sort_unstable();
    bornes.dedup();

    let mut morceaux = Vec::new();
    for fenetre in bornes.windows(2) {
        let (Some(debut), Some(fin)) = (fenetre.first().copied(), fenetre.get(1).copied()) else {
            continue;
        };
        if curseur == Some(debut) {
            morceaux.push(LinePiece::Cursor);
        }
        let texte: String = line
            .chars()
            .skip(debut)
            .take(fin.saturating_sub(debut))
            .collect();
        if !texte.is_empty() {
            morceaux.push(LinePiece::Text {
                text: texte,
                selected: selection.as_ref().is_some_and(|plage| {
                    debut >= plage.start && fin <= plage.end && plage.start < plage.end
                }),
            });
        }
    }
    if curseur == Some(longueur) {
        morceaux.push(LinePiece::Cursor);
    }
    if include_newline {
        // Un blanc sélectionné qui matérialise le retour à la ligne.
        morceaux.push(LinePiece::Text {
            text: " ".to_owned(),
            selected: true,
        });
    }
    morceaux
}

/// Ce que l'éditeur demande, sans jamais le faire lui-même.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EditorEvent {
    /// `Cmd+Entrée` : l'utilisateur veut exécuter.
    ///
    /// Le texte à exécuter est [`QueryEditor::statement_text`] ; l'éditeur ne
    /// construit ni `ExecRequest` ni `Command`.
    ExecuteRequested,
    /// Cancel the active execution without changing the text.
    CancelRequested,
    /// Le contenu a changé.
    Changed,
}

/// L'éditeur de requêtes.
#[derive(Debug)]
pub struct QueryEditor {
    focus: FocusHandle,
    buffer: TextBuffer,
    cursor: TextPosition,
    /// Origine de la sélection. `None` : pas de sélection.
    anchor: Option<TextPosition>,
    undo: Vec<(TextBuffer, TextPosition)>,
    redo: Vec<(TextBuffer, TextPosition)>,
    scroll: UniformListScrollHandle,
    /// Vrai quand une exécution est en cours : l'éditeur reste modifiable, mais
    /// il le signale.
    running: bool,
    read_only: bool,
    selecting: bool,
    marked: Option<Range<usize>>,
    line_layouts: BTreeMap<usize, (ShapedLine, Bounds<Pixels>)>,
}

impl EventEmitter<EditorEvent> for QueryEditor {}

impl Focusable for QueryEditor {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl QueryEditor {
    /// Un éditeur vide.
    pub fn new(cx: &mut Context<'_, Self>) -> Self {
        Self::with_text("", cx)
    }

    /// Un éditeur préchargé — restauration d'un onglet, requête d'un agent
    /// soumise à relecture.
    pub fn with_text(text: &str, cx: &mut Context<'_, Self>) -> Self {
        let buffer = TextBuffer::from_text(text);
        let cursor = buffer.end();
        Self {
            focus: cx.focus_handle(),
            buffer,
            cursor,
            anchor: None,
            undo: Vec::new(),
            redo: Vec::new(),
            scroll: UniformListScrollHandle::new(),
            running: false,
            read_only: false,
            selecting: false,
            marked: None,
            line_layouts: BTreeMap::new(),
        }
    }

    /// Byte position in `text()`, derived from the editor's Unicode cursor.
    #[must_use]
    pub fn cursor_byte_offset(&self) -> usize {
        let cursor = self.buffer.clamp(self.cursor);
        let before = self
            .buffer
            .lines
            .iter()
            .take(cursor.line)
            .fold(0usize, |offset, line| {
                offset.saturating_add(line.len()).saturating_add(1)
            });
        let current = self.buffer.line(cursor.line).unwrap_or("");
        let within = current
            .char_indices()
            .nth(cursor.column)
            .map_or(current.len(), |(offset, _)| offset);
        before.saturating_add(within)
    }

    /// Le texte complet.
    #[must_use]
    pub fn text(&self) -> String {
        self.buffer.text()
    }

    /// Le tampon.
    #[must_use]
    pub fn buffer(&self) -> &TextBuffer {
        &self.buffer
    }

    /// La position du curseur.
    #[must_use]
    pub fn cursor(&self) -> TextPosition {
        self.cursor
    }

    /// La sélection, remise dans l'ordre du texte.
    #[must_use]
    pub fn selection(&self) -> Option<(TextPosition, TextPosition)> {
        let ancre = self.anchor?;
        if ancre == self.cursor {
            return None;
        }
        Some(ordered(ancre, self.cursor))
    }

    /// Returns the exact selection, or the full buffer when none is selected.
    /// The application resolves the current SQL statement using its dialect
    /// and [`Self::cursor_byte_offset`] before submitting a command.
    #[must_use]
    pub fn statement_text(&self) -> String {
        match self.selection() {
            Some((debut, fin)) => self.buffer.slice(debut, fin),
            None => self.buffer.text(),
        }
    }

    /// Remplace tout le texte.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<'_, Self>) {
        self.snapshot();
        self.buffer = TextBuffer::from_text(text);
        self.cursor = self.buffer.end();
        self.anchor = None;
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    /// Signale qu'une exécution est en cours, ou terminée.
    pub fn set_running(&mut self, running: bool, cx: &mut Context<'_, Self>) {
        self.running = running;
        cx.notify();
    }

    /// Keeps selection and copying available, but refuses edits and execution requests.
    pub fn set_read_only(&mut self, read_only: bool, cx: &mut Context<'_, Self>) {
        self.read_only = read_only;
        self.marked = None;
        cx.notify();
    }

    /// Empile l'état courant pour l'annulation.
    fn snapshot(&mut self) {
        self.undo.push((self.buffer.clone(), self.cursor));
        if self.undo.len() > UNDO_DEPTH {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn undo(&mut self, cx: &mut Context<'_, Self>) {
        let Some((tampon, curseur)) = self.undo.pop() else {
            return;
        };
        self.redo.push((self.buffer.clone(), self.cursor));
        self.buffer = tampon;
        self.cursor = self.buffer.clamp(curseur);
        self.anchor = None;
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    fn redo(&mut self, cx: &mut Context<'_, Self>) {
        let Some((tampon, curseur)) = self.redo.pop() else {
            return;
        };
        self.undo.push((self.buffer.clone(), self.cursor));
        self.buffer = tampon;
        self.cursor = self.buffer.clamp(curseur);
        self.anchor = None;
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    /// Supprime la sélection, s'il y en a une. Rend `true` si elle existait.
    fn delete_selection(&mut self) -> bool {
        let Some((debut, fin)) = self.selection() else {
            return false;
        };
        self.cursor = self.buffer.delete_range(debut, fin);
        self.anchor = None;
        true
    }

    /// Insère du texte à la place de la sélection.
    pub fn insert(&mut self, text: &str, cx: &mut Context<'_, Self>) {
        if self.read_only {
            return;
        }
        self.snapshot();
        self.delete_selection();
        self.cursor = self.buffer.insert(self.cursor, text);
        self.anchor = None;
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    /// Déplace le curseur, en étendant la sélection si `extend`.
    fn move_to(&mut self, position: TextPosition, extend: bool) {
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor);
            }
        } else {
            self.anchor = None;
        }
        self.cursor = self.buffer.clamp(position);
        self.scroll
            .scroll_to_item(self.cursor.line, gpui::ScrollStrategy::Center);
    }

    /// La position à gauche du curseur, en franchissant le début de ligne.
    fn position_left(&self) -> TextPosition {
        if self.cursor.column > 0 {
            return TextPosition::new(self.cursor.line, self.cursor.column.saturating_sub(1));
        }
        if self.cursor.line == 0 {
            return self.cursor;
        }
        let precedente = self.cursor.line.saturating_sub(1);
        TextPosition::new(precedente, self.buffer.line_len(precedente))
    }

    /// La position à droite du curseur, en franchissant la fin de ligne.
    fn position_right(&self) -> TextPosition {
        if self.cursor.column < self.buffer.line_len(self.cursor.line) {
            return TextPosition::new(self.cursor.line, self.cursor.column.saturating_add(1));
        }
        if self.cursor.line.saturating_add(1) >= self.buffer.line_count() {
            return self.cursor;
        }
        TextPosition::new(self.cursor.line.saturating_add(1), 0)
    }

    fn on_key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<'_, Self>) {
        let touche = event.keystroke.key.as_str();
        let modificateurs = event.keystroke.modifiers;
        let etend = modificateurs.shift;
        // `secondary` est Cmd sur macOS et Ctrl ailleurs : coder « Cmd » en dur
        // rendrait l'éditeur inutilisable sur Linux, cible de la phase 0.
        let commande = modificateurs.secondary();

        if self.read_only
            && ((commande && matches!(touche, "enter" | "x" | "v" | "z"))
                || matches!(touche, "enter" | "backspace" | "delete"))
        {
            cx.stop_propagation();
            return;
        }
        if self.read_only && touche == "tab" {
            return;
        }
        if commande {
            if matches!(touche, "enter" | "a" | "c" | "x" | "v" | "z") {
                cx.stop_propagation();
            }
            match touche {
                "enter" => {
                    cx.emit(EditorEvent::ExecuteRequested);
                    return;
                }
                "a" => {
                    self.anchor = Some(TextPosition::default());
                    self.cursor = self.buffer.end();
                    cx.notify();
                    return;
                }
                "c" | "x" => {
                    if let Some((debut, fin)) = self.selection() {
                        let texte = self.buffer.slice(debut, fin);
                        cx.write_to_clipboard(ClipboardItem::new_string(texte));
                        if touche == "x" {
                            self.snapshot();
                            self.delete_selection();
                            cx.emit(EditorEvent::Changed);
                        }
                        cx.notify();
                    }
                    return;
                }
                "v" => {
                    if let Some(texte) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        self.insert(&texte, cx);
                    }
                    return;
                }
                "z" => {
                    if modificateurs.shift {
                        self.redo(cx);
                    } else {
                        self.undo(cx);
                    }
                    return;
                }
                _ => {}
            }
        }

        match touche {
            "left" => {
                let cible = self.position_left();
                self.move_to(cible, etend);
                cx.notify();
            }
            "right" => {
                let cible = self.position_right();
                self.move_to(cible, etend);
                cx.notify();
            }
            "up" => {
                let cible =
                    TextPosition::new(self.cursor.line.saturating_sub(1), self.cursor.column);
                self.move_to(cible, etend);
                cx.notify();
            }
            "down" => {
                let cible =
                    TextPosition::new(self.cursor.line.saturating_add(1), self.cursor.column);
                self.move_to(cible, etend);
                cx.notify();
            }
            "home" => {
                let cible = TextPosition::new(self.cursor.line, 0);
                self.move_to(cible, etend);
                cx.notify();
            }
            "end" => {
                let cible =
                    TextPosition::new(self.cursor.line, self.buffer.line_len(self.cursor.line));
                self.move_to(cible, etend);
                cx.notify();
            }
            "enter" => self.insert("\n", cx),
            "tab" => self.insert(&" ".repeat(TAB_WIDTH), cx),
            "backspace" => {
                self.snapshot();
                if !self.delete_selection() {
                    self.cursor = self.buffer.delete_backward(self.cursor);
                }
                cx.emit(EditorEvent::Changed);
                cx.notify();
            }
            "delete" => {
                self.snapshot();
                if !self.delete_selection() {
                    self.cursor = self.buffer.delete_forward(self.cursor);
                }
                cx.emit(EditorEvent::Changed);
                cx.notify();
            }
            "escape" => {
                if self.running {
                    cx.emit(EditorEvent::CancelRequested);
                }
                self.anchor = None;
                cx.notify();
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Rendu
// ─────────────────────────────────────────────────────────────────────────────

impl Render for QueryEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let lignes = self.buffer.line_count();
        let input = cx.entity();

        div()
            .relative()
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, (), window, cx| {
                        let focus = input.read(cx).focus.clone();
                        window.handle_input(
                            &focus,
                            gpui::ElementInputHandler::new(bounds, input.clone()),
                            cx,
                        );
                    },
                )
                .absolute()
                .inset_0(),
            )
            .key_context("QueryEditor")
            .track_focus(&self.focus)
            .when(self.read_only, |el| el.tab_index(0))
            .id("oxyn-query-editor")
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.colors.background)
            .text_color(theme.colors.text)
            .font_family(theme.typography.mono_family.clone())
            .text_size(theme.typography.mono_size)
            .cursor_text()
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.selecting = false),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.selecting = false),
            )
            .child(
                div().flex_1().overflow_hidden().child(
                    uniform_list(
                        "oxyn-editor-lines",
                        lignes,
                        cx.processor(|editeur: &mut Self, plage: Range<usize>, _window, cx| {
                            let theme = Theme::of(cx).clone();
                            editeur.line_layouts.clear();
                            let mut sorties = Vec::with_capacity(plage.len());
                            for index in plage {
                                sorties.push(editeur.render_line(index, &theme, cx));
                            }
                            sorties
                        }),
                    )
                    .track_scroll(self.scroll.clone())
                    .size_full(),
                ),
            )
            .child(self.render_hint(cx))
    }
}

impl QueryEditor {
    fn mouse_column(&self, index: usize, x: Pixels) -> usize {
        let Some((line, bounds)) = self.line_layouts.get(&index) else {
            return 0;
        };
        // Match the gutter and padding used by render_line, using the shaped text
        // rather than an assumed fixed character width.
        let theme = self.mouse_metrics();
        let byte = line.closest_index_for_x(x - bounds.left() - theme.0 - theme.1);
        line.text.get(..byte).unwrap_or_default().chars().count()
    }

    fn mouse_metrics(&self) -> (Pixels, Pixels) {
        // Metrics currently do not vary between theme modes.
        let theme = Theme::default();
        (theme.metrics.gutter_width, theme.metrics.cell_padding)
    }

    /// Une ligne : gouttière figée, puis les morceaux de texte.
    fn render_line(&self, index: usize, theme: &Theme, cx: &Context<'_, Self>) -> AnyElement {
        let metrics = theme.metrics;
        let ligne = self.buffer.line(index).unwrap_or_default();
        let courante = self.cursor.line == index;

        let selection = self.selection().and_then(|(debut, fin)| {
            if index < debut.line || index > fin.line {
                return None;
            }
            let a = if index == debut.line { debut.column } else { 0 };
            let b = if index == fin.line {
                fin.column
            } else {
                ligne.chars().count()
            };
            Some(a..b)
        });
        let retour_selectionne = self
            .selection()
            .is_some_and(|(debut, fin)| index >= debut.line && index < fin.line);

        let morceaux = line_pieces(
            ligne,
            selection,
            courante.then_some(self.cursor.column),
            retour_selectionne,
        );

        let mut piste = div().flex().flex_row().items_center().h_full();
        for morceau in morceaux {
            piste = piste.child(match morceau {
                LinePiece::Cursor => div()
                    .w(px(2.0))
                    .h(theme.typography.line_height)
                    .flex_none()
                    .bg(theme.colors.accent)
                    .into_any_element(),
                LinePiece::Text { text, selected } => div()
                    .flex_none()
                    .whitespace_nowrap()
                    .when(selected, |element| element.bg(theme.colors.selection))
                    .child(SharedString::from(text))
                    .into_any_element(),
            });
        }

        let text: SharedString = ligne.to_owned().into();
        let entity = cx.entity();
        let layout_entity = entity.clone();
        div()
            .relative()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                    window.focus(&this.focus);
                    let column = this.mouse_column(index, event.position.x);
                    if event.click_count >= 2 {
                        this.anchor = Some(TextPosition::new(index, 0));
                        this.cursor = TextPosition::new(index, this.buffer.line_len(index));
                    } else {
                        this.move_to(TextPosition::new(index, column), event.modifiers.shift);
                    }
                    this.selecting = true;
                    cx.notify();
                    cx.stop_propagation();
                }),
            )
            .on_mouse_move(
                cx.listener(move |this, event: &gpui::MouseMoveEvent, _, cx| {
                    if this.selecting {
                        let column = this.mouse_column(index, event.position.x);
                        this.move_to(TextPosition::new(index, column), true);
                        cx.notify();
                    }
                }),
            )
            .child(
                canvas(
                    move |bounds, window, _cx| {
                        let style = window.text_style();
                        let run = TextRun {
                            len: text.len(),
                            font: style.font(),
                            color: style.color,
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        };
                        (
                            window.text_system().shape_line(
                                text.clone(),
                                style.font_size.to_pixels(window.rem_size()),
                                &[run],
                                None,
                            ),
                            bounds,
                        )
                    },
                    move |_, (line, bounds), _, cx| {
                        layout_entity.update(cx, |this, _| {
                            this.line_layouts.insert(index, (line, bounds));
                        });
                    },
                )
                .absolute()
                .inset_0(),
            )
            .id(ElementId::named_usize("oxyn-editor-line", index))
            .h(metrics.row_height)
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .when(courante, |element| element.bg(theme.colors.grid_stripe))
            .child(
                div()
                    .w(metrics.gutter_width)
                    .h_full()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_end()
                    .px(metrics.cell_padding)
                    .bg(theme.colors.grid_gutter)
                    .border_r_1()
                    .border_color(theme.colors.grid_line)
                    .text_color(if courante {
                        theme.colors.text_muted
                    } else {
                        theme.colors.text_faint
                    })
                    .child(SharedString::from(index.saturating_add(1).to_string())),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .overflow_hidden()
                    .px(metrics.cell_padding)
                    .child(piste),
            )
            .into_any_element()
    }

    /// Le rappel du raccourci d'exécution, et l'état d'exécution.
    fn render_hint(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let position = SharedString::from(format!(
            "L{} C{}",
            self.cursor.line.saturating_add(1),
            self.cursor.column.saturating_add(1)
        ));
        let portee = match self.selection() {
            Some(_) => "selection",
            None => "current statement",
        };
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap_3()
            .h(theme.metrics.header_height)
            .px_3()
            .bg(theme.colors.surface_raised)
            .border_t_1()
            .border_color(theme.colors.border)
            .font_family(theme.typography.ui_family.clone())
            .text_size(theme.typography.small_size)
            .text_color(theme.colors.text_faint)
            .child(position)
            .child(SharedString::from(if self.read_only {
                "Read only · select and copy".to_owned()
            } else {
                format!("⌘Enter executes: {portee}")
            }))
            .when(self.running, |element| {
                element.child(
                    div()
                        .text_color(theme.colors.warning)
                        .child("running — Esc cancels"),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_tampon_vide_a_une_ligne() {
        let tampon = TextBuffer::default();
        assert_eq!(tampon.line_count(), 1);
        assert_eq!(tampon.text(), "");
        assert_eq!(tampon.end(), TextPosition::new(0, 0));
    }

    #[test]
    fn les_fins_de_ligne_windows_sont_normalisees() {
        // Un `\r` résiduel partirait dans l'instruction envoyée au serveur.
        let tampon = TextBuffer::from_text("SELECT 1\r\nFROM t");
        assert_eq!(tampon.line_count(), 2);
        assert_eq!(tampon.line(0), Some("SELECT 1"));
        assert_eq!(tampon.text(), "SELECT 1\nFROM t");
    }

    #[test]
    fn linsertion_simple_avance_le_curseur_en_caracteres() {
        let mut tampon = TextBuffer::from_text("SELECT");
        let position = tampon.insert(TextPosition::new(0, 6), " *");
        assert_eq!(tampon.text(), "SELECT *");
        assert_eq!(position, TextPosition::new(0, 8));
    }

    #[test]
    fn linsertion_compte_en_caracteres_et_non_en_octets() {
        // Le piège classique : `é` fait deux octets et une colonne.
        let mut tampon = TextBuffer::from_text("café");
        let position = tampon.insert(TextPosition::new(0, 4), "!");
        assert_eq!(tampon.text(), "café!");
        assert_eq!(position, TextPosition::new(0, 5));
    }

    #[test]
    fn linsertion_multiligne_coupe_la_ligne_courante() {
        let mut tampon = TextBuffer::from_text("SELECT a, b");
        let position = tampon.insert(TextPosition::new(0, 9), "\nFROM t\n");
        assert_eq!(tampon.text(), "SELECT a,\nFROM t\n b");
        assert_eq!(position, TextPosition::new(2, 0));
    }

    #[test]
    fn effacer_en_debut_de_ligne_fusionne_avec_la_precedente() {
        let mut tampon = TextBuffer::from_text("SELECT\nFROM");
        let position = tampon.delete_backward(TextPosition::new(1, 0));
        assert_eq!(tampon.text(), "SELECTFROM");
        assert_eq!(position, TextPosition::new(0, 6));
    }

    #[test]
    fn effacer_au_tout_debut_ne_fait_rien() {
        let mut tampon = TextBuffer::from_text("SELECT");
        let position = tampon.delete_backward(TextPosition::new(0, 0));
        assert_eq!(tampon.text(), "SELECT");
        assert_eq!(position, TextPosition::new(0, 0));
    }

    #[test]
    fn effacer_vers_lavant_en_fin_de_ligne_colle_la_suivante() {
        let mut tampon = TextBuffer::from_text("SELECT\nFROM");
        tampon.delete_forward(TextPosition::new(0, 6));
        assert_eq!(tampon.text(), "SELECTFROM");
    }

    #[test]
    fn effacer_un_intervalle_multiligne_recolle_les_bords() {
        let mut tampon = TextBuffer::from_text("SELECT a\nFROM t\nWHERE x");
        let position = tampon.delete_range(TextPosition::new(0, 7), TextPosition::new(2, 6));
        assert_eq!(tampon.text(), "SELECT x");
        assert_eq!(position, TextPosition::new(0, 7));
    }

    #[test]
    fn un_intervalle_inverse_est_remis_dans_lordre() {
        // Une sélection tirée vers le haut a son ancre après son curseur.
        let mut tampon = TextBuffer::from_text("SELECT a");
        tampon.delete_range(TextPosition::new(0, 8), TextPosition::new(0, 6));
        assert_eq!(tampon.text(), "SELECT");
    }

    #[test]
    fn une_position_hors_bornes_est_ramenee_au_lieu_de_paniquer() {
        let tampon = TextBuffer::from_text("SELECT");
        assert_eq!(
            tampon.clamp(TextPosition::new(99, 99)),
            TextPosition::new(0, 6)
        );
    }

    #[test]
    fn extraire_un_intervalle_multiligne_conserve_les_retours() {
        let tampon = TextBuffer::from_text("SELECT a\nFROM t\nWHERE x");
        let texte = tampon.slice(TextPosition::new(0, 7), TextPosition::new(2, 5));
        assert_eq!(texte, "a\nFROM t\nWHERE");
    }

    #[test]
    fn une_ligne_sans_selection_ni_curseur_est_un_seul_morceau() {
        let morceaux = line_pieces("SELECT", None, None, false);
        assert_eq!(
            morceaux,
            vec![LinePiece::Text {
                text: "SELECT".to_owned(),
                selected: false
            }]
        );
    }

    #[test]
    fn le_curseur_coupe_la_ligne_en_deux_morceaux() {
        // C'est ce découpage qui place le curseur au pixel près sans mesurer
        // le texte à la main.
        let morceaux = line_pieces("SELECT", None, Some(3), false);
        assert_eq!(
            morceaux,
            vec![
                LinePiece::Text {
                    text: "SEL".to_owned(),
                    selected: false
                },
                LinePiece::Cursor,
                LinePiece::Text {
                    text: "ECT".to_owned(),
                    selected: false
                },
            ]
        );
    }

    #[test]
    fn le_curseur_en_fin_de_ligne_est_le_dernier_morceau() {
        let morceaux = line_pieces("ab", None, Some(2), false);
        assert_eq!(
            morceaux,
            vec![
                LinePiece::Text {
                    text: "ab".to_owned(),
                    selected: false
                },
                LinePiece::Cursor,
            ]
        );
    }

    #[test]
    fn une_ligne_vide_avec_curseur_rend_le_seul_curseur() {
        assert_eq!(
            line_pieces("", None, Some(0), false),
            vec![LinePiece::Cursor]
        );
    }

    #[test]
    fn la_selection_marque_le_bon_morceau() {
        let morceaux = line_pieces("SELECT", Some(2..4), None, false);
        assert_eq!(
            morceaux,
            vec![
                LinePiece::Text {
                    text: "SE".to_owned(),
                    selected: false
                },
                LinePiece::Text {
                    text: "LE".to_owned(),
                    selected: true
                },
                LinePiece::Text {
                    text: "CT".to_owned(),
                    selected: false
                },
            ]
        );
    }

    #[test]
    fn le_retour_a_la_ligne_selectionne_est_materialise() {
        let morceaux = line_pieces("ab", Some(0..2), None, true);
        assert_eq!(
            morceaux.last(),
            Some(&LinePiece::Text {
                text: " ".to_owned(),
                selected: true
            })
        );
    }

    #[test]
    fn une_selection_vide_ne_marque_rien() {
        let morceaux = line_pieces("ab", Some(1..1), None, false);
        assert!(matches!(
            morceaux.as_slice(),
            [
                LinePiece::Text {
                    selected: false,
                    ..
                },
                LinePiece::Text {
                    selected: false,
                    ..
                }
            ]
        ));
    }

    #[test]
    fn les_bornes_de_selection_hors_ligne_sont_ramenees() {
        // Une sélection multiligne donne une borne au-delà de la ligne courte.
        let morceaux = line_pieces("ab", Some(0..99), None, false);
        assert_eq!(
            morceaux,
            vec![LinePiece::Text {
                text: "ab".to_owned(),
                selected: true
            }]
        );
    }
}

#[cfg(test)]
mod read_only_tests {
    use super::*;

    #[gpui::test]
    fn cursor_offsets_match_utf8_bytes_across_lines(cx: &mut gpui::TestAppContext) {
        let editor = cx.new(|cx| QueryEditor::with_text("éα\n😀SELECT 1;", cx));
        editor.update(cx, |editor, _| {
            editor.cursor = TextPosition::new(1, 1);
            assert_eq!(editor.cursor_byte_offset(), 9);
            editor.cursor = TextPosition::new(0, 1);
            assert_eq!(editor.cursor_byte_offset(), 2);
            editor.cursor = TextPosition::new(usize::MAX, usize::MAX);
            assert_eq!(editor.cursor_byte_offset(), editor.text().len());
        });
    }

    #[gpui::test]
    fn read_only_text_accepts_selection_and_copy_but_neither_edit_nor_execution(
        cx: &mut gpui::TestAppContext,
    ) {
        let requests = std::rc::Rc::new(std::cell::Cell::new(0));
        let observed = requests.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            let mut view = QueryEditor::with_text("SELECT 'é🐾'", cx);
            view.set_read_only(true, cx);
            window.focus(&view.focus);
            let entity = cx.entity();
            cx.subscribe(&entity, move |_, _, event, _| {
                if matches!(event, EditorEvent::ExecuteRequested | EditorEvent::Changed) {
                    observed.set(observed.get() + 1);
                }
            })
            .detach();
            view
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("cmd-a");
        cx.simulate_keystrokes("cmd-c");
        cx.update(|_, cx| {
            assert_eq!(
                cx.read_from_clipboard()
                    .and_then(|item| item.text())
                    .as_deref(),
                Some("SELECT 'é🐾'")
            )
        });
        for keys in [
            "cmd-x",
            "cmd-v",
            "backspace",
            "delete",
            "enter",
            "cmd-enter",
            "cmd-z",
        ] {
            cx.simulate_keystrokes(keys);
        }
        cx.simulate_input("DROP TABLE data");
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                gpui::EntityInputHandler::replace_and_mark_text_in_range(
                    view,
                    Some(0..6),
                    "INSERT",
                    None,
                    window,
                    cx,
                );
            })
        });
        assert_eq!(view.read_with(cx, |view, _| view.text()), "SELECT 'é🐾'");
        assert_eq!(requests.get(), 0);
        view.update(cx, |view, cx| {
            view.set_read_only(false, cx);
            view.insert("SELECT 2", cx);
        });
        assert_eq!(view.read_with(cx, |view, _| view.text()), "SELECT 2");
    }
}
