//! Choisir un type de base, renseigner ses paramètres, ouvrir la connexion.
//!
//! C'est le premier écran du produit : sans connexion, aucune autre surface n'a
//! de sens. Il est **conditionnel aux capacités** au sens le plus littéral —
//! il ne montre que les types de base réellement enregistrés dans le registre
//! de drivers, et pour chacun exactement les champs qu'il déclare
//! ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)).
//!
//! # Ce que cette vue ne connaît pas
//!
//! Ni `Driver`, ni `Session`, ni `DriverMetadata` : `oxyn-ui` ne dépend pas
//! d'`oxyn-driver`, et lui donner accès à ces traits ouvrirait le second chemin
//! d'exécution qu'[I-01](../../../CLAUDE.md#i-01) interdit. Elle reçoit une
//! description en types nus — [`DriverChoice`] et [`FormField`] — que
//! `oxyn-app` construit à partir des métadonnées du driver.
//!
//! Elle ne construit pas non plus de `Command` : elle émet un
//! [`ConnectionDraft`], et c'est `oxyn-app` qui en fait une
//! `Command::CreateConnection` puis une `Command::Connect`.
//!
//! # Les cinq états
//!
//! | État | Ce qu'il montre |
//! |---|---|
//! | Initial | la liste des types de base, groupés par famille |
//! | Vide | aucun driver enregistré — dit que c'est un défaut de câblage, pas une absence de bases |
//! | Peuplé | le formulaire du type choisi, champ par champ |
//! | En cours | la connexion est tentée, avec un moyen d'annuler |
//! | Erreur | le message du serveur, et s'il est retentable |
//!
//! # Le mot de passe
//!
//! Un champ [`FormFieldKind::Password`] est saisi masqué et sa valeur part dans
//! [`ConnectionDraft::secrets`], séparée des paramètres ordinaires. `oxyn-app`
//! l'écrit au trousseau et ne met qu'une **référence** dans la configuration
//! persistée. C'est la raison pour laquelle le brouillon sépare les deux cartes
//! plutôt que de tout mettre dans une seule : un champ oublié du mauvais côté
//! finirait en clair dans un fichier de workspace
//! ([I-03](../../../CLAUDE.md#i-03)).

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent,
    PathPromptOptions, SharedString, Window, div, px,
};
use oxyn_core::Environment;

use crate::text_field::{FieldEvent, TextField};
use crate::theme::Theme;

mod fields;
mod view;

/// Le genre d'un champ de formulaire, tel que la vue doit le rendre.
///
/// Miroir sans dépendance de `oxyn_driver::FieldKind` : la traduction se fait
/// dans `oxyn-app`, seule crate à connaître les deux.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FormFieldKind {
    /// Texte libre sur une ligne.
    Text,
    /// Secret : saisi masqué, jamais persisté avec la configuration.
    Password,
    /// Entier. La validation reste au driver.
    Number,
    /// Case à cocher.
    Bool,
    /// Choix fermé, dans l'ordre proposé par le driver.
    Choice(Vec<SharedString>),
    /// Chemin d'un fichier local.
    Path,
}

impl FormFieldKind {
    /// La valeur de ce champ est-elle un secret ?
    #[must_use]
    pub const fn is_secret(&self) -> bool {
        matches!(self, Self::Password)
    }
}

/// Un champ du formulaire de connexion d'un driver.
#[derive(Debug, Clone)]
pub struct FormField {
    /// Clé technique, celle que le driver attend.
    pub key: SharedString,
    /// Libellé affiché.
    pub label: SharedString,
    /// Genre, qui gouverne le rendu et le stockage.
    pub kind: FormFieldKind,
    /// Le champ doit-il être renseigné pour que la connexion soit tentable ?
    pub required: bool,
    /// Valeur proposée. Jamais renseignée sur un champ secret.
    pub default: Option<SharedString>,
    /// Aide contextuelle, en une phrase.
    pub help: Option<SharedString>,
}

/// Un type de base proposé à l'utilisateur.
#[derive(Debug, Clone)]
pub struct DriverChoice {
    /// Identifiant du protocole, tel que le registre le connaît.
    pub id: SharedString,
    /// Nom affiché : « PostgreSQL », pas « postgres ».
    pub display_name: SharedString,
    /// Famille, pour le groupement.
    pub family: SharedString,
    /// Les champs, dans l'ordre de saisie voulu par le driver.
    pub fields: Vec<FormField>,
}

/// Ce que l'utilisateur a rempli, prêt à devenir une `Command`.
///
/// **Pas de `#[derive(Debug)]`** : `secrets` porte des mots de passe en clair,
/// et un `Debug` dérivé les mettrait à un `{:?}` de distance
/// ([I-03](../../../CLAUDE.md#i-03)). L'implémentation manuelle plus bas ne rend
/// que les clés.
#[derive(Clone)]
pub struct ConnectionDraft {
    /// Le protocole choisi.
    pub driver: SharedString,
    /// Le nom que l'utilisateur donne à la connexion.
    pub name: String,
    /// Le marquage d'environnement. Jamais deviné : voir [`I-02`].
    ///
    /// [`I-02`]: ../../../CLAUDE.md#i-02
    pub environment: Environment,
    /// Les paramètres ordinaires, persistés avec la configuration.
    pub values: BTreeMap<String, String>,
    /// Les secrets, destinés au trousseau et à lui seul.
    pub secrets: BTreeMap<String, String>,
}

impl fmt::Debug for ConnectionDraft {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionDraft")
            .field("driver", &self.driver)
            .field("name", &self.name)
            .field("environment", &self.environment)
            // Les clés, jamais les valeurs : savoir qu'un `host` est renseigné
            // aide au diagnostic, savoir lequel ne le fait pas.
            .field("values", &self.values.keys().collect::<Vec<_>>())
            .field("secrets", &self.secrets.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

/// Ce que la vue demande, sans jamais le faire elle-même.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ConnectionFormEvent {
    /// L'utilisateur veut ouvrir cette connexion. Encadré : c'est la plus
    /// grosse charge utile de l'énumération, et de loin.
    ConnectRequested(Box<ConnectionDraft>),
    /// L'utilisateur veut rouvrir une connexion déjà enregistrée, par son rang
    /// dans la liste qu'on lui a donnée.
    SavedChosen(usize),
    /// L'utilisateur renonce à la connexion en cours d'ouverture.
    CancelRequested,
    /// L'utilisateur veut consulter les copies de travail sauvegardées.
    RecoveryRequested,
    /// L'utilisateur revient au workspace resté ouvert derrière cet écran.
    ReturnRequested,
}

/// Ce que la barre de titre de l'accueil propose, à droite de la marque.
///
/// Ces actions dépendent de l'état de la fenêtre — un workspace déjà ouvert,
/// des copies locales à reprendre — que seule `oxyn-app` connaît. Elles vivent
/// dans la barre plutôt qu'en superposition : un calque flottant posé sur
/// l'écran recouvrait la marque et ses deux lignes de titre.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HeaderActions {
    /// Des copies de travail locales sont consultables.
    pub saved_copies: bool,
    /// Un workspace connecté attend derrière cet écran.
    pub return_to_workspace: bool,
}

impl HeaderActions {
    /// Aucune action à dessiner : la barre garde alors la seule marque.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        !self.saved_copies && !self.return_to_workspace
    }
}

/// Une connexion déjà enregistrée, telle que l'écran la présente.
///
/// Ne porte **pas** de `ConnectionId` : la vue n'a pas à pouvoir l'afficher
/// ([I-03](../../../CLAUDE.md#i-03)). Le rang dans la liste suffit à la
/// désigner.
#[derive(Debug, Clone)]
pub struct SavedConnection {
    /// Le nom donné par l'utilisateur.
    pub name: SharedString,
    /// Le protocole.
    pub driver: SharedString,
    /// Le marquage d'environnement.
    pub environment: Environment,
}

/// Où en est l'écran.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum FormState {
    /// Le choix du type de base n'est pas fait.
    #[default]
    ChoosingDriver,
    /// Le formulaire du driver de rang `driver` est en cours de saisie.
    Filling {
        /// Rang dans la liste des types proposés.
        driver: usize,
    },
    /// La connexion est tentée.
    Connecting,
    /// La tentative a échoué.
    Failed {
        /// Le message du serveur, code compris.
        message: SharedString,
        /// L'erreur est-elle retentable telle quelle ?
        retryable: bool,
    },
}

/// Les rangs de champ réservés au bloc commun, avant les champs du driver.
const CHAMP_NOM: usize = 0;
const CHAMP_ENVIRONNEMENT: usize = 1;
const CHAMPS_COMMUNS: usize = 2;

/// Les environnements proposés, dans l'ordre du moins au plus contraignant.
const ENVIRONNEMENTS: [Environment; 4] = [
    Environment::Local,
    Environment::Development,
    Environment::Staging,
    Environment::Production,
];

/// Ce que l'écran sait, sans rien de GPUI.
///
/// Séparé de [`ConnectionForm`] pour une raison pratique : une `FocusHandle` ne
/// se construit que depuis un `Context`, donc depuis une fenêtre. Toute la
/// logique qui décide — quels champs sont obligatoires, ce qui part en secret,
/// quel environnement est retenu — vit ici et se teste sans écran. La vue ne
/// garde que le rendu et le clavier.
#[derive(Default)]
pub struct FormModel {
    drivers: Vec<DriverChoice>,
    saved: Vec<SavedConnection>,
    state: FormState,
    /// Le nom donné à la connexion.
    name: String,
    /// Rang dans [`ENVIRONNEMENTS`].
    environment: usize,
    /// Valeurs saisies, par clé de champ. Secrets compris : la séparation se
    /// fait à la construction du brouillon, à partir du genre du champ.
    values: BTreeMap<String, String>,
    /// Le champ qui a le curseur, tous champs confondus.
    focused: usize,
}

impl fmt::Debug for FormModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Pas de `values` : la carte contient les mots de passe en cours de
        // frappe (I-03).
        f.debug_struct("FormModel")
            .field("state", &self.state)
            .field("drivers", &self.drivers.len())
            .field("saved", &self.saved.len())
            .field("focused", &self.focused)
            .finish_non_exhaustive()
    }
}

/// L'écran de connexion.
pub struct ConnectionForm {
    focus: FocusHandle,
    model: FormModel,
    inputs: BTreeMap<usize, Entity<TextField>>,
    pending_focus: bool,
    last_driver: Option<usize>,
    header: HeaderActions,
}

impl fmt::Debug for ConnectionForm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionForm")
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl EventEmitter<ConnectionFormEvent> for ConnectionForm {}

impl ConnectionForm {
    /// L'écran, sur la liste des types de base disponibles.
    pub fn new(
        drivers: Vec<DriverChoice>,
        saved: Vec<SavedConnection>,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        Self {
            focus: cx.focus_handle(),
            model: FormModel::new(drivers, saved),
            inputs: BTreeMap::new(),
            pending_focus: true,
            last_driver: None,
            header: HeaderActions::default(),
        }
    }

    /// Ce que l'écran sait, pour le lire depuis `oxyn-app`.
    #[must_use]
    pub const fn model(&self) -> &FormModel {
        &self.model
    }

    /// Déclare les actions que la barre de titre doit proposer.
    ///
    /// Redessine seulement quand elles changent : cet écran est reconstruit à
    /// chaque retour de connexion, et notifier sans changement ferait battre
    /// la vue pour rien.
    pub fn set_header_actions(&mut self, actions: HeaderActions, cx: &mut Context<'_, Self>) {
        if self.header != actions {
            self.header = actions;
            cx.notify();
        }
    }

    /// Les actions actuellement dessinées dans la barre de titre.
    #[must_use]
    pub const fn header_actions(&self) -> HeaderActions {
        self.header
    }

    /// Adds a successfully opened connection without reading persistent storage.
    pub fn add_saved(&mut self, connection: SavedConnection, cx: &mut Context<'_, Self>) {
        self.model.saved.push(connection);
        cx.notify();
    }

    /// Choisit un type de base et redessine.
    pub fn choose_driver(&mut self, index: usize, cx: &mut Context<'_, Self>) {
        self.model.choose_driver(index);
        self.last_driver = Some(index);
        self.build_inputs(cx);
        self.pending_focus = true;
        cx.notify();
    }

    /// Revient à la liste des types de base.
    pub fn back_to_drivers(&mut self, cx: &mut Context<'_, Self>) {
        self.model.back_to_drivers();
        self.pending_focus = true;
        cx.notify();
    }

    /// Signale que la connexion est en cours.
    pub fn set_connecting(&mut self, cx: &mut Context<'_, Self>) {
        self.model.set_connecting();
        self.pending_focus = true;
        cx.notify();
    }

    /// Signale l'échec de la connexion, sans paraphraser le serveur.
    pub fn set_failed(
        &mut self,
        message: impl Into<SharedString>,
        retryable: bool,
        cx: &mut Context<'_, Self>,
    ) {
        self.model.set_failed(message, retryable);
        self.pending_focus = true;
        cx.notify();
    }
}

impl FormModel {
    /// Le modèle, sur la liste des types de base disponibles.
    #[must_use]
    pub fn new(drivers: Vec<DriverChoice>, saved: Vec<SavedConnection>) -> Self {
        Self {
            drivers,
            saved,
            state: FormState::ChoosingDriver,
            name: String::new(),
            environment: ENVIRONNEMENTS.len().saturating_sub(1),
            values: BTreeMap::new(),
            focused: CHAMP_NOM,
        }
    }

    /// L'état courant.
    #[must_use]
    pub const fn state(&self) -> &FormState {
        &self.state
    }

    /// Les types de base proposés.
    #[must_use]
    pub fn drivers(&self) -> &[DriverChoice] {
        &self.drivers
    }

    /// Les connexions déjà enregistrées.
    #[must_use]
    pub fn saved(&self) -> &[SavedConnection] {
        &self.saved
    }

    /// Choisit un type de base et prépare son formulaire.
    ///
    /// Les valeurs par défaut du driver sont posées ici, et **jamais** sur un
    /// champ secret : une valeur par défaut de mot de passe serait écrite en
    /// clair dans le binaire.
    pub fn choose_driver(&mut self, index: usize) {
        let Some(choix) = self.drivers.get(index) else {
            return;
        };
        self.values.clear();
        for champ in &choix.fields {
            if champ.kind.is_secret() {
                continue;
            }
            if let Some(defaut) = &champ.default {
                self.values
                    .insert(champ.key.to_string(), defaut.to_string());
            }
        }
        if self.name.is_empty() {
            self.name = choix.display_name.to_string();
        }
        self.state = FormState::Filling { driver: index };
        self.focused = CHAMP_NOM;
    }

    /// Revient à la liste des types de base.
    pub fn back_to_drivers(&mut self) {
        self.state = FormState::ChoosingDriver;
    }

    /// Signale que la connexion est en cours.
    pub fn set_connecting(&mut self) {
        self.state = FormState::Connecting;
    }

    /// Signale l'échec de la connexion, sans paraphraser le serveur.
    pub fn set_failed(&mut self, message: impl Into<SharedString>, retryable: bool) {
        self.state = FormState::Failed {
            message: message.into(),
            retryable,
        };
    }

    /// Le driver en cours de saisie, s'il y en a un.
    fn current(&self) -> Option<&DriverChoice> {
        match &self.state {
            FormState::Filling { driver } => self.drivers.get(*driver),
            _ => None,
        }
    }

    /// Le nombre de champs saisissables, bloc commun compris.
    fn field_count(&self) -> usize {
        CHAMPS_COMMUNS + self.current().map_or(0, |d| d.fields.len())
    }

    /// Tous les champs obligatoires sont-ils renseignés ?
    ///
    /// Le nom en fait partie : une connexion sans nom ne peut pas être nommée
    /// dans une confirmation d'écriture, ce que [I-02] exige.
    ///
    /// [I-02]: ../../../CLAUDE.md#i-02
    #[must_use]
    pub fn is_complete(&self) -> bool {
        if self.name.trim().is_empty() {
            return false;
        }
        let Some(choix) = self.current() else {
            return false;
        };
        choix.fields.iter().all(|champ| {
            !champ.required
                || self
                    .values
                    .get(champ.key.as_ref())
                    .is_some_and(|v| !v.trim().is_empty())
        })
    }

    /// Construit le brouillon à partir de ce qui est saisi.
    ///
    /// C'est **ici** que les secrets sont séparés des paramètres, sur le genre
    /// déclaré par le driver et non sur le nom du champ : un driver qui
    /// appellerait son secret `token` plutôt que `password` serait tout de même
    /// traité correctement.
    #[must_use]
    pub fn draft(&self) -> Option<ConnectionDraft> {
        let choix = self.current()?;
        let mut values = BTreeMap::new();
        let mut secrets = BTreeMap::new();
        for champ in &choix.fields {
            let Some(valeur) = self.values.get(champ.key.as_ref()) else {
                continue;
            };
            if valeur.is_empty() {
                continue;
            }
            let cible = if champ.kind.is_secret() {
                &mut secrets
            } else {
                &mut values
            };
            cible.insert(champ.key.to_string(), valeur.clone());
        }
        Some(ConnectionDraft {
            driver: choix.id.clone(),
            name: self.name.trim().to_owned(),
            environment: ENVIRONNEMENTS
                .get(self.environment)
                .copied()
                .unwrap_or(Environment::Production),
            values,
            secrets,
        })
    }

    /// Demande l'ouverture, si le formulaire est complet.
    /// La clé du champ de rang `index`, hors bloc commun.
    fn field_key(&self, index: usize) -> Option<String> {
        let choix = self.current()?;
        let champ = choix.fields.get(index.checked_sub(CHAMPS_COMMUNS)?)?;
        Some(champ.key.to_string())
    }

    /// La clé du champ courant, s'il attend un chemin de fichier.
    ///
    /// Sert au sélecteur natif : taper un chemin à la main reste possible, mais
    /// personne ne connaît par cœur le chemin de son fichier SQLite, et l'écrire
    /// de mémoire produit un « unable to open database file » qui ne dit pas
    /// lequel des vingt caractères est faux.
    #[must_use]
    pub fn focused_path_field(&self) -> Option<SharedString> {
        let choix = self.current()?;
        let champ = choix
            .fields
            .get(self.focused.checked_sub(CHAMPS_COMMUNS)?)?;
        matches!(champ.kind, FormFieldKind::Path).then(|| champ.key.clone())
    }

    /// Pose la valeur d'un champ, en écrasant ce qui s'y trouvait.
    ///
    /// Le sélecteur de fichier rend un chemin entier : l'ajouter à ce qui était
    /// déjà saisi produirait une concaténation absurde.
    pub fn set_value(&mut self, key: &str, value: impl Into<String>) {
        self.values.insert(key.to_owned(), value.into());
    }

    /// Insère du texte dans le champ qui a le curseur.
    fn insert(&mut self, texte: &str) {
        match self.focused {
            CHAMP_NOM => self.name.push_str(texte),
            CHAMP_ENVIRONNEMENT => {}
            index => {
                let Some(cle) = self.field_key(index) else {
                    return;
                };
                self.values.entry(cle).or_default().push_str(texte);
            }
        }
    }

    /// Efface le dernier caractère du champ qui a le curseur.
    fn backspace(&mut self) {
        match self.focused {
            CHAMP_NOM => {
                self.name.pop();
            }
            CHAMP_ENVIRONNEMENT => {}
            index => {
                let Some(cle) = self.field_key(index) else {
                    return;
                };
                if let Some(valeur) = self.values.get_mut(&cle) {
                    valeur.pop();
                }
            }
        }
    }

    /// Déplace le curseur d'un champ.
    fn move_focus(&mut self, avant: bool) {
        let total = self.field_count();
        if total == 0 {
            return;
        }
        self.focused = if avant {
            self.focused.saturating_add(1) % total
        } else {
            self.focused
                .checked_sub(1)
                .unwrap_or(total.saturating_sub(1))
        };
    }

    /// Fait tourner la valeur d'un champ à choix fermé, ou l'environnement.
    fn cycle(&mut self) {
        if self.focused == CHAMP_ENVIRONNEMENT {
            self.environment = self.environment.saturating_add(1) % ENVIRONNEMENTS.len();
            return;
        }
        let Some(choix) = self.current() else {
            return;
        };
        let Some(rang) = self.focused.checked_sub(CHAMPS_COMMUNS) else {
            return;
        };
        let Some(champ) = choix.fields.get(rang) else {
            return;
        };
        let (cle, options) = match &champ.kind {
            FormFieldKind::Choice(options) if !options.is_empty() => {
                (champ.key.to_string(), options.clone())
            }
            FormFieldKind::Bool => {
                let cle = champ.key.to_string();
                let actuel = self.values.get(&cle).map(String::as_str) == Some("true");
                self.values
                    .insert(cle, if actuel { "false" } else { "true" }.to_owned());
                return;
            }
            _ => return,
        };
        let actuel = self.values.get(&cle).cloned().unwrap_or_default();
        let suivant = options
            .iter()
            .position(|o| o.as_ref() == actuel)
            .map_or(0, |rang| rang.saturating_add(1) % options.len());
        if let Some(valeur) = options.get(suivant) {
            self.values.insert(cle, valeur.to_string());
        }
    }

    /// Applique une frappe. Rend `Some` si l'utilisateur demande l'ouverture.
    ///
    /// Le modèle n'émet rien : c'est la vue qui traduit ce retour en événement.
    /// La conséquence utile est que **tout le clavier se teste sans fenêtre**.
    fn key(&mut self, touche: &str, shift: bool, saisie: Option<&str>) -> Option<ConnectionDraft> {
        // Dans la liste des types, seules la navigation et la validation ont
        // un sens : le reste ne doit pas se mettre à saisir du texte.
        if matches!(self.state, FormState::ChoosingDriver) {
            match touche {
                "down" | "tab" => self.move_driver(true),
                "up" => self.move_driver(false),
                "enter" => self.choose_driver(self.focused),
                _ => {}
            }
            return None;
        }

        if touche == "enter" {
            return self.draft().filter(|_| self.is_complete());
        }

        match touche {
            "tab" => self.move_focus(!shift),
            "down" => self.move_focus(true),
            "up" => self.move_focus(false),
            "space" if self.focused == CHAMP_ENVIRONNEMENT => self.cycle(),
            "left" | "right" => self.cycle(),
            "escape" => self.back_to_drivers(),
            "backspace" => self.backspace(),
            _ => {
                if let Some(texte) = saisie
                    && !texte.is_empty()
                    && !texte.chars().any(char::is_control)
                {
                    self.insert(texte);
                }
            }
        }
        None
    }

    /// Déplace la sélection dans la liste des types de base.
    fn move_driver(&mut self, avant: bool) {
        let total = self.drivers.len() + self.saved.len();
        if total == 0 {
            return;
        }
        self.focused = if avant {
            self.focused.saturating_add(1) % total
        } else {
            self.focused
                .checked_sub(1)
                .unwrap_or(total.saturating_sub(1))
        };
    }
}

impl ConnectionForm {
    fn build_inputs(&mut self, cx: &mut Context<'_, Self>) {
        self.inputs.clear();
        let mut fields = vec![(CHAMP_NOM, self.model.name.clone(), false)];
        if let Some(driver) = self.model.current() {
            for (rank, field) in driver.fields.iter().enumerate() {
                if matches!(field.kind, FormFieldKind::Choice(_) | FormFieldKind::Bool) {
                    continue;
                }
                fields.push((
                    rank + CHAMPS_COMMUNS,
                    self.model
                        .values
                        .get(field.key.as_ref())
                        .cloned()
                        .unwrap_or_default(),
                    field.kind.is_secret(),
                ));
            }
        }
        for (index, value, secret) in fields {
            let input = cx.new(|cx| TextField::new(value, secret, cx).with_managed_tab_order());
            cx.subscribe(&input, move |this, input, event, cx| {
                match event {
                    FieldEvent::Changed => {
                        let text = input.read(cx).text().to_owned();
                        if index == CHAMP_NOM {
                            this.model.name = text;
                        } else if let Some(key) = this.model.field_key(index) {
                            this.model.values.insert(key, text);
                        }
                    }
                    FieldEvent::LimitReached => {}
                    FieldEvent::Focused => this.model.focused = index,
                    FieldEvent::Next(backwards) => {
                        this.model.focused = index;
                        this.model.move_focus(!backwards);
                        this.pending_focus = true;
                    }
                    FieldEvent::Submit => this.submit(cx),
                    FieldEvent::Escape => this.back_to_drivers(cx),
                    FieldEvent::Browse(create) => {
                        this.model.focused = index;
                        if let Some(key) = this.model.focused_path_field() {
                            if *create {
                                this.nommer_un_fichier(key, cx);
                            } else {
                                this.parcourir(key, cx);
                            }
                        }
                    }
                }
                cx.notify();
            })
            .detach();
            self.inputs.insert(index, input);
        }
    }

    fn submit(&mut self, cx: &mut Context<'_, Self>) {
        if !matches!(self.model.state, FormState::Filling { .. }) {
            return;
        }
        if self.model.is_complete()
            && let Some(draft) = self.model.draft()
        {
            cx.emit(ConnectionFormEvent::ConnectRequested(Box::new(draft)));
        }
    }

    fn correct(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(driver) = self.last_driver {
            self.model.state = FormState::Filling { driver };
            self.pending_focus = true;
            cx.notify();
        } else {
            self.back_to_drivers(cx);
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        // Unhandled printable keys must reach the platform text-input handler.
        // The form only owns keys while its own focus handle is active: a text
        // field or a title-bar action holding focus keeps its own keys, and Tab
        // must move between them instead of walking the driver list.
        if !self.focus.is_focused(window)
            || self
                .inputs
                .values()
                .any(|input| input.read(cx).focus_handle(cx).is_focused(window))
        {
            return;
        }
        let key = event.keystroke.key.as_str();
        match self.model.state {
            FormState::Connecting => {
                if key == "escape" {
                    cx.emit(ConnectionFormEvent::CancelRequested);
                }
            }
            FormState::Failed { .. } => {
                if key == "enter" {
                    self.correct(cx);
                } else if key == "escape" {
                    self.back_to_drivers(cx);
                }
            }
            FormState::ChoosingDriver => {
                if key == "enter" {
                    let rank = self.model.focused;
                    if rank < self.model.drivers.len() {
                        self.choose_driver(rank, cx);
                    } else if rank - self.model.drivers.len() < self.model.saved.len() {
                        cx.emit(ConnectionFormEvent::SavedChosen(
                            rank - self.model.drivers.len(),
                        ));
                    }
                } else if key == "tab" && event.keystroke.modifiers.shift {
                    self.model.move_driver(false);
                } else {
                    self.model.key(key, event.keystroke.modifiers.shift, None);
                }
            }
            FormState::Filling { .. } => {
                if key == "enter" {
                    self.submit(cx);
                } else if key == "escape" {
                    self.back_to_drivers(cx);
                } else {
                    self.model.key(key, event.keystroke.modifiers.shift, None);
                    self.pending_focus = true;
                }
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Ouvre le sélecteur de la plateforme sur un fichier existant.
    ///
    /// Le dialogue est modal côté système mais **pas** bloquant ici : la
    /// fenêtre continue de se dessiner pendant qu'il est ouvert
    /// ([I-05](../../CLAUDE.md#i-05)).
    fn parcourir(&mut self, cle: SharedString, cx: &mut Context<'_, Self>) {
        let attente = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(SharedString::new_static("Ouvrir")),
        });
        cx.spawn(async move |this, cx| {
            // Annulation, échec du dialogue, sélection vide : dans les trois cas
            // le champ garde ce qu'il avait. Rien à signaler — l'utilisateur
            // vient de fermer la fenêtre qu'il avait ouverte.
            let Ok(Ok(Some(chemins))) = attente.await else {
                return;
            };
            let Some(chemin) = chemins.first() else {
                return;
            };
            Self::appliquer(&this, cx, &cle, chemin);
        })
        .detach();
    }

    /// Ouvre le sélecteur sur un fichier **à créer**.
    ///
    /// Sans ce geste, une base neuve serait impossible à créer autrement qu'en
    /// tapant son chemin : le sélecteur d'ouverture ne propose que ce qui
    /// existe déjà, et une base SQLite neuve n'existe pas encore.
    fn nommer_un_fichier(&mut self, cle: SharedString, cx: &mut Context<'_, Self>) {
        let depart = std::env::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let attente = cx.prompt_for_new_path(&depart, Some("base.sqlite"));

        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(chemin))) = attente.await else {
                return;
            };
            Self::appliquer(&this, cx, &cle, &chemin);
        })
        .detach();
    }

    /// Écrit le chemin choisi dans le champ, si la vue existe encore.
    fn appliquer(
        this: &gpui::WeakEntity<Self>,
        cx: &mut gpui::AsyncApp,
        cle: &SharedString,
        chemin: &Path,
    ) {
        // `to_string_lossy` : un chemin macOS qui ne serait pas de l'UTF-8 est
        // rare mais légal, et le remplacer par une erreur empêcherait d'ouvrir
        // un fichier que le système sait pourtant ouvrir.
        let texte = chemin.to_string_lossy().into_owned();
        let _ = this.update(cx, |form, cx| {
            form.model.set_value(cle, texte.clone());
            if let Some(driver) = form.model.current()
                && let Some(rank) = driver
                    .fields
                    .iter()
                    .position(|field| field.key.as_ref() == cle.as_ref())
                && let Some(input) = form.inputs.get(&(rank + CHAMPS_COMMUNS))
            {
                input.update(cx, |input, cx| input.set_text(texte, cx));
            }
            form.pending_focus = true;
            cx.notify();
        });
    }
}

impl Focusable for ConnectionForm {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Le libellé d'un environnement, en toutes lettres.
#[must_use]
pub fn environment_choice_label(environment: Environment) -> &'static str {
    match environment {
        Environment::Local => "local",
        Environment::Development => "développement",
        Environment::Staging => "préproduction",
        Environment::Production => "production",
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Rendu
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn champ(cle: &str, kind: FormFieldKind, obligatoire: bool) -> FormField {
        FormField {
            key: cle.to_owned().into(),
            label: cle.to_owned().into(),
            kind,
            required: obligatoire,
            default: None,
            help: None,
        }
    }

    fn postgres() -> DriverChoice {
        DriverChoice {
            id: "postgres".into(),
            display_name: "PostgreSQL".into(),
            family: "Relationnel".into(),
            fields: vec![
                FormField {
                    default: Some("localhost".into()),
                    ..champ("host", FormFieldKind::Text, true)
                },
                champ("password", FormFieldKind::Password, false),
                champ(
                    "sslmode",
                    FormFieldKind::Choice(vec!["prefer".into()]),
                    false,
                ),
            ],
        }
    }

    fn sqlite() -> DriverChoice {
        DriverChoice {
            id: "sqlite".into(),
            display_name: "SQLite".into(),
            family: "Relationnel".into(),
            fields: vec![champ("path", FormFieldKind::Path, true)],
        }
    }

    /// Un modèle prêt à être interrogé. Aucune fenêtre, aucun `FocusHandle` :
    /// c'est précisément ce que la séparation modèle / vue achète.
    fn forme(drivers: Vec<DriverChoice>) -> FormModel {
        FormModel::new(drivers, Vec::new())
    }

    #[test]
    fn le_champ_fichier_de_sqlite_accepte_le_selecteur() {
        // La demande explicite : choisir le fichier SQLite soi-même. Le modèle
        // doit reconnaître le champ comme un chemin, sinon ⌘O ne fait rien et
        // rien ne le signale.
        let mut modele = forme(vec![DriverChoice {
            id: "sqlite".into(),
            display_name: "SQLite".into(),
            family: "relational".into(),
            fields: vec![FormField {
                key: "path".into(),
                label: "Fichier de base".into(),
                kind: FormFieldKind::Path,
                required: true,
                default: None,
                help: None,
            }],
        }]);
        modele.choose_driver(0);

        modele.focused = CHAMPS_COMMUNS;
        assert_eq!(
            modele.focused_path_field().as_ref().map(AsRef::as_ref),
            Some("path"),
            "le champ de SQLite doit être reconnu comme un chemin"
        );

        modele.focused = CHAMP_NOM;
        assert!(
            modele.focused_path_field().is_none(),
            "le nom de la connexion n'est pas un chemin"
        );
    }

    #[test]
    fn un_chemin_choisi_remplace_la_saisie_au_lieu_de_sy_ajouter() {
        // Le sélecteur rend un chemin entier. L'ajouter à ce qui était tapé
        // produirait « /tmp/a.db/Users/nicolas/b.db », que SQLite refuserait
        // avec un message qui ne dit pas d'où vient la concaténation.
        let mut modele = forme(vec![DriverChoice {
            id: "sqlite".into(),
            display_name: "SQLite".into(),
            family: "relational".into(),
            fields: vec![FormField {
                key: "path".into(),
                label: "Fichier de base".into(),
                kind: FormFieldKind::Path,
                required: true,
                default: None,
                help: None,
            }],
        }]);
        modele.choose_driver(0);
        modele.set_value("path", "/tmp/ancien.db");
        modele.set_value("path", "/Users/nicolas/base.sqlite");

        assert_eq!(
            modele.values.get("path").map(String::as_str),
            Some("/Users/nicolas/base.sqlite")
        );
    }

    #[test]
    fn un_mot_de_passe_ne_recoit_jamais_de_valeur_par_defaut() {
        // Une valeur par défaut sur un champ secret serait écrite en clair dans
        // le binaire et affichée à l'écran. `choose_driver` pose les défauts ;
        // il doit sauter les secrets même si le driver en propose un.
        let mut f = forme(vec![DriverChoice {
            fields: vec![FormField {
                default: Some("hunter2".into()),
                ..champ("password", FormFieldKind::Password, false)
            }],
            ..postgres()
        }]);
        f.state = FormState::Filling { driver: 0 };
        for champ in &f.drivers[0].fields {
            if champ.kind.is_secret() {
                continue;
            }
            if let Some(defaut) = &champ.default {
                f.values.insert(champ.key.to_string(), defaut.to_string());
            }
        }
        assert!(!f.values.contains_key("password"), "{:?}", f.values.keys());
    }

    #[test]
    fn le_brouillon_separe_les_secrets_des_parametres() {
        // La panne visée : un mot de passe qui rejoint `values` finit dans le
        // fichier de workspace, en clair (I-03).
        let mut f = forme(vec![postgres()]);
        f.state = FormState::Filling { driver: 0 };
        f.name = "base client".to_owned();
        f.values.insert("host".to_owned(), "db.example".to_owned());
        f.values.insert("password".to_owned(), "hunter2".to_owned());

        let brouillon = f.draft().expect("un driver est choisi");
        assert!(brouillon.values.contains_key("host"));
        assert!(!brouillon.values.contains_key("password"), "secret fuité");
        assert_eq!(
            brouillon.secrets.get("password").map(String::as_str),
            Some("hunter2")
        );
    }

    #[test]
    fn le_debug_du_brouillon_ne_rend_aucune_valeur() {
        let mut f = forme(vec![postgres()]);
        f.state = FormState::Filling { driver: 0 };
        f.name = "base".to_owned();
        f.values
            .insert("host".to_owned(), "db.interne.example".to_owned());
        f.values.insert("password".to_owned(), "hunter2".to_owned());

        let rendu = format!("{:?}", f.draft().expect("brouillon"));
        assert!(!rendu.contains("hunter2"), "secret fuité : {rendu}");
        assert!(
            !rendu.contains("db.interne.example"),
            "valeur fuitée : {rendu}"
        );
        assert!(rendu.contains("password"), "la clé reste utile : {rendu}");
    }

    #[test]
    fn un_champ_obligatoire_vide_bloque_la_connexion() {
        let mut f = forme(vec![sqlite()]);
        f.state = FormState::Filling { driver: 0 };
        f.name = "locale".to_owned();
        assert!(!f.is_complete(), "`path` est obligatoire et vide");

        f.values.insert("path".to_owned(), "   ".to_owned());
        assert!(!f.is_complete(), "des espaces ne renseignent rien");

        f.values
            .insert("path".to_owned(), "/tmp/base.sqlite".to_owned());
        assert!(f.is_complete());
    }

    #[test]
    fn une_connexion_sans_nom_est_refusee() {
        // I-02 : une confirmation d'écriture doit pouvoir nommer la connexion.
        let mut f = forme(vec![sqlite()]);
        f.state = FormState::Filling { driver: 0 };
        f.values
            .insert("path".to_owned(), "/tmp/base.sqlite".to_owned());
        assert!(!f.is_complete(), "le nom manque");
        f.name = "  ".to_owned();
        assert!(!f.is_complete(), "un nom d'espaces n'en est pas un");
    }

    #[test]
    fn new_connections_default_to_production_until_explicitly_changed() {
        let mut f = forme(vec![sqlite()]);
        f.state = FormState::Filling { driver: 0 };
        f.name = "n".to_owned();
        f.values.insert("path".to_owned(), "p".to_owned());
        assert_eq!(
            f.draft().expect("brouillon").environment,
            Environment::Production
        );

        f.focused = CHAMP_ENVIRONNEMENT;
        f.cycle();
        assert_eq!(
            f.draft().expect("brouillon").environment,
            Environment::Local
        );
    }

    #[test]
    fn un_rang_d_environnement_aberrant_retombe_sur_production() {
        // Le repli le plus contraignant, jamais le plus permissif.
        let mut f = forme(vec![sqlite()]);
        f.state = FormState::Filling { driver: 0 };
        f.name = "n".to_owned();
        f.environment = 99;
        assert_eq!(
            f.draft().expect("brouillon").environment,
            Environment::Production
        );
    }

    #[test]
    fn sans_driver_choisi_il_n_y_a_pas_de_brouillon() {
        let f = forme(vec![sqlite()]);
        assert!(f.draft().is_none());
        assert!(!f.is_complete());
    }

    #[test]
    fn un_champ_vide_n_entre_pas_dans_le_brouillon() {
        // Écrire `sslmode = ""` ferait échouer la connexion avec un message que
        // l'utilisateur ne pourrait pas relier à un champ qu'il a laissé vide.
        let mut f = forme(vec![postgres()]);
        f.state = FormState::Filling { driver: 0 };
        f.name = "n".to_owned();
        f.values.insert("host".to_owned(), "h".to_owned());
        f.values.insert("sslmode".to_owned(), String::new());
        let brouillon = f.draft().expect("brouillon");
        assert!(!brouillon.values.contains_key("sslmode"), "{:?}", brouillon);
    }
}
