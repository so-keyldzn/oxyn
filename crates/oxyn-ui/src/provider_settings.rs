//! Déclarer, lister et retirer les fournisseurs de modèles de cette machine.
//!
//! Autorité : [ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md)
//! sur la structure d'une déclaration,
//! [UX-SPEC](../../../docs/UX-SPEC.md) § « Configuration des fournisseurs » sur
//! ce que l'écran montre.
//!
//! # Ce que cette vue ne connaît pas
//!
//! Ni le trousseau, ni le bus, ni le store. Elle émet un
//! [`ProviderSettingsEvent`] et `oxyn-app` le traduit — écriture de la clé au
//! trousseau, puis déclaration par le bus, dans cet ordre
//! ([I-01](../../../CLAUDE.md#i-01)). Elle ne connaît pas non plus
//! `oxyn_llm::Reach` : le classement local/distant lui **arrive** déjà calculé,
//! parce qu'il demande une résolution DNS, qui est bloquante et n'a rien à faire
//! sur le fil d'interface ([I-05](../../../CLAUDE.md#i-05)).
//!
//! # L'identité d'une déclaration ne se décide pas ici
//!
//! [`ProviderDraft`] ne porte **pas** de [`ProviderId`] : c'est `oxyn-app` qui
//! frappe l'identité, parce que lui seul sait ce qui existe déjà et ce que le
//! trousseau référence. La validation, elle, est celle du domaine —
//! [`AiProviderConfig::validate`] — et non des règles réécrites ici, qui
//! divergeraient au premier changement.
//!
//! # La clé
//!
//! Elle vit dans son champ de saisie le temps de la frappe, part **une fois**
//! dans [`ProviderSettingsEvent::SaveRequested`], et l'écran l'oublie au moment
//! de l'envoi : ni dans l'état de la vue, ni dans un `Debug`, ni réaffichée
//! ([I-03](../../../CLAUDE.md#i-03)). Ce que la liste remontre, c'est
//! « configurée » ou « absente ».
//!
//! Conséquence assumée : un enregistrement qui échoue oblige à ressaisir la
//! clé. La garder « pour le cas où » serait exactement la ligne qu'I-03
//! interdit ; l'écran le dit dans le bandeau d'erreur plutôt que de laisser
//! l'utilisateur découvrir un champ vide.
//!
//! # Aucune condition de capacité
//!
//! Cet écran ne suppose ni table, ni schéma, ni SQL
//! ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)) : il ne parle
//! pas à une base et n'a pas de session. Il existe même sans aucune connexion
//! ouverte. Ce qui est conditionnel, c'est l'**entrée d'IA** ailleurs dans
//! l'interface — elle n'existe que si la liste rendue ici est non vide —, et
//! cette condition appartient à `oxyn-app` (ADR-0023).
//!
//! # Les cinq états
//!
//! | État | Ce qu'il montre |
//! |---|---|
//! | Initial | [`ProviderSettingsState::Loading`] — la liste n'a pas encore été lue |
//! | Vide | la liste est lue et vide : « aucun fournisseur déclaré », et Oxyn fonctionne sans |
//! | Peuplé | une ligne par déclaration, avec l'état de sa clé et son classement daté |
//! | En cours | l'enregistrement touche le trousseau puis le bus ; l'abandon reste offert |
//! | Erreur | le message tel quel, s'il est retentable, et si la clé est à ressaisir |

use std::fmt;

use gpui::prelude::*;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, SharedString, Window,
    div,
};
use oxyn_core::ai::{
    MAX_PROVIDER_BASE_URL_BYTES, MAX_PROVIDER_LABEL_BYTES, MAX_PROVIDER_MODEL_BYTES,
};
use oxyn_core::{AiProviderConfig, AiProviderKind, ProviderId};

use crate::select_field::{SelectEvent, SelectField};
use crate::text_field::{FieldEvent, TextField};

mod form;
pub mod row;
mod view;

/// Le classement d'un point d'accès, tel que l'écran le montre.
///
/// Miroir sans dépendance d'`oxyn_llm::Reach` — `oxyn-ui` ne dépend pas
/// d'`oxyn-llm`, et la traduction se fait dans `oxyn-app`, seule crate à
/// connaître les deux. Même précédent que
/// [`FormFieldKind`](crate::connection_form::FormFieldKind) vis-à-vis de
/// `oxyn_driver::FieldKind`.
///
/// [`Unresolved`](Self::Unresolved) s'affiche **comme tel** et jamais arrondi à
/// « local » : le doute ne profite pas à l'envoi
/// ([UX-SPEC](../../../docs/UX-SPEC.md)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ProviderReach {
    /// L'hôte est sur la boucle locale : rien ne quitte la machine.
    Local,
    /// L'hôte est joignable ailleurs : les données quittent la machine.
    Remote,
    /// L'hôte n'a pas pu être résolu. Compte comme distant partout où une
    /// décision se prend.
    Unresolved,
}

impl ProviderReach {
    /// Le mot affiché.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
            Self::Unresolved => "unresolved",
        }
    }

    /// Les données quittent-elles la machine, en l'état de ce qu'on sait ?
    ///
    /// `true` pour [`Unresolved`](Self::Unresolved) : un point d'accès qu'on
    /// n'a pas su classer n'obtient pas le bénéfice du doute.
    #[must_use]
    pub const fn leaves_machine(self) -> bool {
        !matches!(self, Self::Local)
    }
}

/// Ce que l'écran sait de la clé d'un fournisseur : qu'elle existe, ou non.
///
/// Il n'y a **pas** de troisième variante portant la valeur, et c'est le point :
/// un type qui pourrait la porter finirait par la rendre.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyState {
    /// Une clé est enregistrée au trousseau du système.
    Configured,
    /// Aucune clé n'est enregistrée. Ce n'est pas un défaut : un point d'accès
    /// local n'en demande pas.
    Absent,
}

impl KeyState {
    /// Le mot affiché.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Configured => "key configured",
            Self::Absent => "no key",
        }
    }
}

/// Une déclaration existante, telle que l'écran la présente.
///
/// Ne porte **ni** [`ProviderId`], **ni** référence de trousseau : la vue n'a
/// pas à pouvoir les afficher ([I-03](../../../CLAUDE.md#i-03)). Le rang dans
/// la liste suffit à désigner la déclaration à retirer — c'est le précédent de
/// [`SavedConnection`](crate::connection_form::SavedConnection).
/// Un agent externe déclaré, tel que l'écran le montre.
///
/// Trois champs, et pas un de plus : il n'y a **ni clé, ni point d'accès, ni
/// portée** à afficher ([ADR-0026](../../../docs/adr/0026-agents-externes-acp.md)).
/// C'est pourquoi ce type n'est pas un [`DeclaredProvider`] amputé : cinq de ses
/// champs n'auraient aucun sens ici.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredAgent {
    /// Le nom donné par l'utilisateur. C'est **lui** qui est montré.
    pub label: SharedString,
    /// Le programme déclaré.
    pub command: SharedString,
    /// Combien d'arguments l'accompagnent. Le compte, pas les valeurs : la
    /// ligne reste lisible, et le détail vit dans le formulaire.
    pub args: usize,
}

#[derive(Debug, Clone)]
pub struct DeclaredProvider {
    /// Le nom donné par l'utilisateur. C'est **lui** qui est montré.
    pub label: SharedString,
    /// La famille de protocole.
    pub kind: AiProviderKind,
    /// Le point d'accès. Montrable : le domaine refuse d'écrire une URL
    /// portant des identifiants.
    pub base_url: SharedString,
    /// Le modèle par défaut de cette déclaration.
    pub model: SharedString,
    /// L'état de la clé, jamais sa valeur.
    pub key: KeyState,
    /// Le classement, recalculé et jamais persisté (ADR-0023).
    pub reach: ProviderReach,
    /// L'instant de la mesure, **déjà mis en forme** par `oxyn-app`.
    ///
    /// Une chaîne et non un instant : `oxyn-ui` ne porte pas de dépendance de
    /// dates, et l'écriture lisible d'un instant dépend du fuseau et de la
    /// locale, que la couche applicative connaît et pas celle-ci.
    pub measured_at: SharedString,
}

/// Ce que l'utilisateur a rempli, prêt à devenir une déclaration.
///
/// **Pas de `#[derive(Debug)]`** : `key` porte une clé d'API en clair, et
/// `base_url` n'a pas encore été validée — elle peut donc porter le couple
/// `utilisateur:motdepasse` que l'écran est justement en train de refuser. Un
/// `Debug` dérivé mettrait les deux à un `{:?}` de distance
/// ([I-03](../../../CLAUDE.md#i-03)).
#[derive(Clone)]
pub struct ProviderDraft {
    /// La famille de protocole choisie.
    pub kind: AiProviderKind,
    /// Le nom donné à la déclaration.
    pub label: String,
    /// Le point d'accès, tel que saisi.
    pub base_url: String,
    /// Le modèle par défaut.
    pub model: String,
    /// La clé, destinée au trousseau et à lui seul. `None` pour un point
    /// d'accès qui n'en demande pas.
    pub key: Option<String>,
}

impl fmt::Debug for ProviderDraft {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderDraft")
            .field("kind", &self.kind)
            .field("label", &self.label)
            // Ni la clé, ni le point d'accès : savoir qu'une clé est saisie
            // suffit au diagnostic, la lire ne sert à personne.
            .field("model", &self.model)
            .field("key", &self.key.as_ref().map(|_| "<clé masquée>"))
            .finish_non_exhaustive()
    }
}

/// Ce que l'écran demande, sans jamais le faire lui-même.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ProviderSettingsEvent {
    /// Déclarer ce fournisseur. Encadré comme
    /// [`ConnectRequested`](crate::connection_form::ConnectionFormEvent::ConnectRequested) :
    /// c'est de loin la plus grosse charge utile de l'énumération.
    ///
    /// La clé qu'il porte est la **seule** fois où elle traverse quoi que ce
    /// soit : l'écran ne l'a plus après l'envoi.
    SaveRequested(Box<ProviderDraft>),
    /// Retirer la déclaration de ce rang, dans la liste donnée à l'écran.
    ///
    /// « Confirmed » et non « Requested » : la confirmation a déjà eu lieu ici,
    /// et elle **nommait** le fournisseur. `oxyn-app` n'en redemande pas une
    /// seconde — une confirmation qu'on voit deux fois finit par être cliquée
    /// deux fois sans être lue.
    RemovalConfirmed(usize),
    /// Retirer l'agent externe de ce rang, **dans la liste des agents**.
    ///
    /// Événement distinct plutôt qu'un rang global : l'appelant aurait sinon à
    /// refaire le découpage « au-delà de tant, c'est un agent », et il se
    /// tromperait le jour où l'ordre d'affichage change.
    AgentRemovalConfirmed(usize),
    /// Déclarer cet agent externe.
    ///
    /// Ne porte **aucun secret**, à la différence de
    /// [`SaveRequested`](Self::SaveRequested) : c'est ce qui fonde ce mode.
    AgentSaveRequested(Box<row::AgentDraft>),
    /// L'utilisateur renonce à l'opération en cours. C'est `oxyn-app` qui porte
    /// le jeton d'annulation ; l'écran ne fait que le demander.
    CancelRequested,
}

/// L'opération qui occupe l'écran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Operation {
    /// La clé part au trousseau, puis la déclaration au bus.
    Saving,
    /// La déclaration est retirée.
    Removing,
}

impl Operation {
    /// Ce que l'écran annonce pendant l'attente.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Saving => "Saving the provider…",
            Self::Removing => "Removing the declaration…",
        }
    }
}

/// Où en est l'écran.
///
/// L'état **vide** n'est pas une variante : c'est
/// [`Ready`](Self::Ready) avec une liste vide. Une variante séparée se
/// désynchroniserait de la liste au premier enregistrement.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ProviderSettingsState {
    /// Les déclarations n'ont pas encore été lues.
    #[default]
    Loading,
    /// La liste est à jour — peuplée ou vide.
    Ready,
    /// Une opération est en cours.
    Working {
        /// Ce qui occupe l'écran.
        operation: Operation,
    },
    /// La dernière opération a échoué. La liste et le formulaire restent
    /// visibles : l'action suivante est de corriger, pas de recommencer
    /// d'ailleurs.
    Failed {
        /// Le message tel qu'il est venu, code compris. Jamais une paraphrase.
        message: SharedString,
        /// L'opération est-elle retentable telle quelle ? Vient de la classe
        /// d'erreur, jamais d'une lecture du message
        /// ([I-13](../../../CLAUDE.md#i-13)).
        retryable: bool,
        /// La clé de l'enregistrement perdu est-elle à ressaisir ?
        key_must_be_retyped: bool,
    },
}

impl ProviderSettingsState {
    /// L'écran accepte-t-il une modification ?
    #[must_use]
    pub const fn accepts_edits(&self) -> bool {
        !matches!(self, Self::Working { .. } | Self::Loading)
    }
}

/// Les familles proposées, dans l'ordre où elles s'affichent.
///
/// Une table et non un `match` : [`AiProviderKind`] est `#[non_exhaustive]`,
/// donc une famille ajoutée dans `oxyn-core` n'aurait pas de branche ici. Avec
/// une table, elle n'a simplement pas d'entrée tant que personne ne l'a
/// ajoutée — une famille manquante se voit à l'écran, alors qu'une branche
/// attrape-tout aurait affiché un libellé faux.
const FAMILLES: [(AiProviderKind, &str); 4] = [
    (AiProviderKind::Anthropic, "Anthropic"),
    (AiProviderKind::OpenAi, "OpenAI"),
    (AiProviderKind::Gemini, "Gemini"),
    (
        AiProviderKind::OpenAiCompatible,
        "OpenAI-compatible (Ollama, LM Studio, Azure…)",
    ),
];

/// Le libellé d'une famille de protocole, si l'écran sait la présenter.
#[must_use]
pub fn kind_label(kind: AiProviderKind) -> Option<&'static str> {
    FAMILLES
        .iter()
        .find(|(famille, _)| *famille == kind)
        .map(|(_, libelle)| *libelle)
}

/// Les libellés des familles, dans l'ordre du sélecteur.
fn kind_options() -> Vec<SharedString> {
    let mut choix: Vec<SharedString> = FAMILLES
        .iter()
        .map(|(_, libelle)| SharedString::new_static(libelle))
        .collect();
    // La seconde sorte de déclaration est un **choix de ce sélecteur**, et non
    // un second écran : c'est ce qu'ADR-0026 veut dire par « une variante, pas
    // un système parallèle ». Elle vient en dernier pour que l'ordre des
    // familles existantes ne bouge pas.
    choix.push(SharedString::new_static(AGENT_CHOICE));
    choix
}

/// Le libellé du choix « agent externe » dans le sélecteur de sorte.
pub const AGENT_CHOICE: &str = "External agent (no key)";

/// Le classement et l'instant de sa mesure, en une ligne.
///
/// L'instant est exigé par UX-SPEC : un point d'accès classé local hier peut
/// résoudre ailleurs aujourd'hui. Quand l'appelant n'en a pas, l'écran le dit
/// plutôt que de montrer un classement qui aurait l'air éternel.
#[must_use]
pub fn reach_summary(reach: ProviderReach, measured_at: &str) -> String {
    if measured_at.trim().is_empty() {
        format!("{} · measurement time unknown", reach.label())
    } else {
        format!("{} · measured {measured_at}", reach.label())
    }
}

/// La question posée avant un retrait. Elle **nomme** le fournisseur.
#[must_use]
pub fn removal_question(label: &str) -> String {
    format!("Remove the “{label}” declaration?")
}

/// Construit la déclaration candidate que le domaine sait valider.
///
/// L'identifiant est un remplissage qui ne quitte jamais cette fonction :
/// [`AiProviderConfig::validate`] porte sur les champs saisis, et l'identité
/// d'une déclaration est frappée par `oxyn-app`. Le test
/// `la_validation_du_domaine_ignore_l_identifiant` tient cette hypothèse.
fn candidate(draft: &ProviderDraft) -> AiProviderConfig {
    AiProviderConfig::new(
        ProviderId::openai_compatible(),
        draft.kind,
        draft.label.trim(),
        draft.base_url.trim(),
        draft.model.trim(),
    )
}

/// Ce qui empêche d'enregistrer ce brouillon, dit par le domaine.
///
/// Le message vient d'[`AiProviderConfig::validate`] et n'est pas reformulé :
/// réécrire ses règles ici les ferait diverger au premier changement. Aucun de
/// ces messages ne recopie la valeur fautive — c'est une garantie d'`oxyn-core`,
/// et le test `un_mot_de_passe_colle_dans_l_url_ne_revient_pas_dans_le_message`
/// la vérifie depuis ce côté-ci.
#[must_use]
pub fn draft_error(draft: &ProviderDraft) -> Option<SharedString> {
    candidate(draft)
        .validate()
        .err()
        .map(|erreur| erreur.to_string().into())
}

/// Ce qui, dans le seul point d'accès, empêche d'enregistrer.
///
/// Sépare le jugement de l'URL de celui du reste pour que le refus des
/// identifiants soit **immédiat** : un mot de passe collé dans le champ « point
/// d'accès » ne doit pas attendre que le formulaire soit complet pour produire
/// un message ([I-03](../../../CLAUDE.md#i-03)).
#[must_use]
pub fn base_url_error(base_url: &str) -> Option<SharedString> {
    draft_error(&ProviderDraft {
        kind: AiProviderKind::OpenAiCompatible,
        // Deux valeurs de remplissage que le domaine accepte : seul le point
        // d'accès est jugé ici.
        label: "?".to_owned(),
        base_url: base_url.to_owned(),
        model: "?".to_owned(),
        key: None,
    })
}

/// Ce que l'écran doit dire de la saisie en cours, s'il doit dire quelque chose.
///
/// Rien tant que le formulaire n'est qu'à moitié rempli : signaler « le modèle
/// est vide » sur un champ qu'on n'a pas encore atteint apprend à ignorer les
/// messages. L'URL fait exception, pour la raison qu'explique
/// [`base_url_error`].
#[must_use]
pub fn draft_notice(draft: &ProviderDraft) -> Option<SharedString> {
    if !draft.base_url.trim().is_empty()
        && let Some(message) = base_url_error(&draft.base_url)
    {
        return Some(message);
    }
    if draft.label.trim().is_empty()
        || draft.base_url.trim().is_empty()
        || draft.model.trim().is_empty()
    {
        return None;
    }
    draft_error(draft)
}

/// L'écran de configuration des fournisseurs.
pub struct ProviderSettings {
    focus: FocusHandle,
    providers: Vec<DeclaredProvider>,
    /// Les agents externes déclarés, montrés dans la **même** liste.
    agents: Vec<DeclaredAgent>,
    state: ProviderSettingsState,
    /// Le rang dont le retrait attend une confirmation.
    confirming: Option<usize>,
    /// Le focus de la liste. Une poignée pour la liste entière et un rang
    /// courant, et non une poignée par ligne : les lignes sont des données que
    /// le bus renouvelle, et une poignée qui meurt avec sa ligne emporterait le
    /// curseur de qui la parcourait. C'est le parti de la liste des types de
    /// base ([`ConnectionForm`](crate::connection_form::ConnectionForm)).
    list_focus: FocusHandle,
    /// La ligne qui a le curseur dans la liste.
    focused_row: usize,
    /// Le focus du refus, sur lequel s'ouvre une confirmation : le bouton par
    /// défaut n'est jamais l'action destructrice.
    cancel_focus: FocusHandle,
    /// Le focus du retrait confirmé. Atteignable, mais jamais par défaut.
    confirm_focus: FocusHandle,
    /// Le focus du bouton d'enregistrement.
    save_focus: FocusHandle,
    /// Le focus du bouton d'abandon, offert pendant l'attente.
    abandon_focus: FocusHandle,
    /// Le focus du bouton qui referme le bandeau d'échec. Une poignée à lui :
    /// un bouton qu'on atteint par tabulation et qui n'obéit qu'à `Échap` est
    /// un cul-de-sac pour qui n'a pas vu le libellé.
    dismiss_focus: FocusHandle,
    kind: Entity<SelectField>,
    kind_index: usize,
    label: Entity<TextField>,
    base_url: Entity<TextField>,
    model: Entity<TextField>,
    /// Le programme d'un agent externe. Distinct de `base_url` : les deux
    /// ne se remplissent jamais ensemble, et réutiliser le champ ferait
    /// porter à un nom de variable deux sens selon l'écran.
    command: Entity<TextField>,
    /// Les arguments, un par ligne.
    args: Entity<TextField>,
    /// Le champ de la clé. Vidé à l'envoi ; jamais relu ailleurs qu'ici.
    key: Entity<TextField>,
    /// Ce que la saisie a de fautif, recalculé à chaque frappe et non à chaque
    /// trame.
    notice: Option<SharedString>,
    /// Le formulaire est-il enregistrable ? Calculé avec [`Self::notice`], et
    /// pour la même raison.
    submittable: bool,
    /// Le dernier enregistrement portait-il une clé ? Un booléen, jamais la
    /// valeur : il sert à dire, en cas d'échec, que la clé est à ressaisir.
    last_draft_had_key: bool,
}

impl fmt::Debug for ProviderSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderSettings")
            .field("state", &self.state)
            .field("providers", &self.providers.len())
            .field("confirming", &self.confirming)
            .finish_non_exhaustive()
    }
}

impl EventEmitter<ProviderSettingsEvent> for ProviderSettings {}

impl Focusable for ProviderSettings {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ProviderSettings {
    /// L'écran, avant que la liste n'ait été lue.
    pub fn new(cx: &mut Context<'_, Self>) -> Self {
        let kind = cx.new(|cx| SelectField::new(kind_options(), 0, cx));
        let command = cx.new(|cx| TextField::new(String::new(), false, cx));
        let args = cx.new(|cx| TextField::new(String::new(), false, cx));
        let label = cx.new(|cx| {
            TextField::new(String::new(), false, cx).with_byte_limit(MAX_PROVIDER_LABEL_BYTES)
        });
        let base_url = cx.new(|cx| {
            TextField::new(String::new(), false, cx).with_byte_limit(MAX_PROVIDER_BASE_URL_BYTES)
        });
        let model = cx.new(|cx| {
            TextField::new(String::new(), false, cx).with_byte_limit(MAX_PROVIDER_MODEL_BYTES)
        });
        let key = cx.new(|cx| {
            let mut champ = TextField::new(String::new(), true, cx);
            // Un champ secret masque déjà son rendu ; couper l'export du
            // presse-papiers ferme le second chemin, celui de l'extraction de
            // texte de la plateforme (I-03).
            champ.set_clipboard_export_allowed(false);
            champ
        });

        for champ in [&label, &base_url, &model, &key] {
            cx.subscribe(champ, |ecran, _, event: &FieldEvent, cx| match event {
                FieldEvent::Changed => ecran.revalidate(cx),
                FieldEvent::Submit => ecran.submit(cx),
                // Échap est consommé par le champ qui a le curseur : sans cette
                // branche, la confirmation de retrait resterait ouverte pendant
                // que l'utilisateur croit l'avoir refermée.
                FieldEvent::Escape => ecran.dismiss(cx),
                _ => {}
            })
            .detach();
        }
        cx.subscribe(&kind, |ecran, _, event: &SelectEvent, cx| {
            ecran.kind_index = event.index;
            ecran.revalidate(cx);
        })
        .detach();

        Self {
            focus: cx.focus_handle(),
            providers: Vec::new(),
            agents: Vec::new(),
            state: ProviderSettingsState::Loading,
            confirming: None,
            list_focus: cx.focus_handle(),
            focused_row: 0,
            cancel_focus: cx.focus_handle(),
            confirm_focus: cx.focus_handle(),
            save_focus: cx.focus_handle(),
            abandon_focus: cx.focus_handle(),
            dismiss_focus: cx.focus_handle(),
            kind,
            kind_index: 0,
            command,
            args,
            label,
            base_url,
            model,
            key,
            notice: None,
            submittable: false,
            last_draft_had_key: false,
        }
    }

    /// Les déclarations connues.
    #[must_use]
    pub fn providers(&self) -> &[DeclaredProvider] {
        &self.providers
    }

    /// Où en est l'écran.
    #[must_use]
    pub const fn state(&self) -> &ProviderSettingsState {
        &self.state
    }

    /// Ce que l'écran reproche à la saisie en cours.
    #[must_use]
    pub fn notice(&self) -> Option<&SharedString> {
        self.notice.as_ref()
    }

    /// Le rang dont le retrait attend une confirmation.
    #[must_use]
    pub const fn confirming(&self) -> Option<usize> {
        self.confirming
    }

    /// Pose la liste rendue par le bus. C'est ce qui fait passer l'écran de
    /// l'état initial à l'état peuplé — ou vide.
    ///
    /// Vide le formulaire si un enregistrement vient d'aboutir : ce qui a été
    /// saisi est désormais dans la liste, et le laisser dans les champs
    /// inviterait à le déclarer deux fois.
    pub fn set_providers(&mut self, providers: Vec<DeclaredProvider>, cx: &mut Context<'_, Self>) {
        let enregistrait = matches!(
            self.state,
            ProviderSettingsState::Working {
                operation: Operation::Saving
            }
        );
        self.providers = providers;
        self.state = ProviderSettingsState::Ready;
        self.confirming = None;
        // Une liste qui rétrécit sous le rang courant laisserait le curseur sur
        // une ligne qui n'existe plus, et le retrait suivant porterait sur une
        // autre déclaration que celle qu'on croit désigner.
        self.focused_row = self.focused_row.min(self.providers.len().saturating_sub(1));
        if enregistrait {
            self.clear_form(cx);
        }
        self.revalidate(cx);
    }

    /// Signale qu'une opération est en cours.
    pub fn set_working(&mut self, operation: Operation, cx: &mut Context<'_, Self>) {
        self.state = ProviderSettingsState::Working { operation };
        self.set_fields_read_only(true, cx);
        self.revalidate(cx);
    }

    /// Signale l'échec, sans paraphraser ce qui est venu.
    pub fn set_failed(
        &mut self,
        message: impl Into<SharedString>,
        retryable: bool,
        cx: &mut Context<'_, Self>,
    ) {
        let enregistrait = matches!(
            self.state,
            ProviderSettingsState::Working {
                operation: Operation::Saving
            }
        );
        self.state = ProviderSettingsState::Failed {
            message: message.into(),
            retryable,
            key_must_be_retyped: enregistrait && self.last_draft_had_key,
        };
        self.confirming = None;
        self.set_fields_read_only(false, cx);
        self.revalidate(cx);
    }

    /// Le brouillon tel que les champs le portent à cet instant.
    /// L'écran déclare-t-il un agent externe plutôt qu'un fournisseur ?
    ///
    /// Déduit du sélecteur, et non d'un drapeau à part : deux sources pour la
    /// même question finissent par se contredire, et c'est le formulaire qui
    /// afficherait alors des champs sans rapport avec ce qu'il enregistre.
    #[must_use]
    pub fn declares_agent(&self) -> bool {
        self.kind_index >= FAMILLES.len()
    }

    /// La saisie d'un agent, telle que le formulaire la porte.
    pub(super) fn agent_draft(&self, cx: &App) -> row::AgentDraft {
        row::AgentDraft {
            label: self.label.read(cx).text().trim().to_owned(),
            command: self.command.read(cx).text().trim().to_owned(),
            args: self.args.read(cx).text().to_owned(),
        }
    }

    fn draft(&self, cx: &App) -> ProviderDraft {
        let saisie = self.key.read(cx).text().to_owned();
        ProviderDraft {
            kind: FAMILLES
                .get(self.kind_index)
                .map_or(AiProviderKind::OpenAiCompatible, |(famille, _)| *famille),
            label: self.label.read(cx).text().trim().to_owned(),
            base_url: self.base_url.read(cx).text().trim().to_owned(),
            model: self.model.read(cx).text().trim().to_owned(),
            // La clé n'est pas rognée : ce qui est entre les espaces d'un
            // secret ne nous appartient pas. Seule une saisie entièrement
            // blanche compte comme absente.
            key: (!saisie.trim().is_empty()).then_some(saisie),
        }
    }

    /// Recalcule ce que l'écran reproche à la saisie, et s'il peut l'enregistrer.
    ///
    /// À la frappe et au changement d'état, **pas** à la trame : construire un
    /// brouillon pendant le rendu recopierait la clé soixante fois par seconde
    /// pour n'en regarder que la présence ([I-03](../../../CLAUDE.md#i-03)), et
    /// mettrait dans `render` un calcul qui ne s'y teste pas.
    fn revalidate(&mut self, cx: &mut Context<'_, Self>) {
        if self.declares_agent() {
            // La validation d'un agent est celle du **domaine**, posée plus tôt.
            let faute = row::agent_draft_error(&self.agent_draft(cx));
            self.submittable = self.state.accepts_edits() && faute.is_none();
            self.notice = faute.map(SharedString::from);
            cx.notify();
            return;
        }
        let draft = self.draft(cx);
        self.notice = draft_notice(&draft);
        self.submittable = self.state.accepts_edits() && draft_error(&draft).is_none();
        cx.notify();
    }

    /// Le formulaire est-il enregistrable en l'état ?
    #[must_use]
    pub const fn is_submittable(&self) -> bool {
        self.submittable
    }

    /// Demande l'enregistrement, si la saisie passe la validation du domaine.
    ///
    /// La clé quitte le champ **avant** que l'événement ne parte : après cette
    /// ligne, l'écran ne la porte plus nulle part.
    pub fn submit(&mut self, cx: &mut Context<'_, Self>) {
        if !self.state.accepts_edits() {
            return;
        }
        if self.declares_agent() {
            let brouillon = self.agent_draft(cx);
            if let Some(message) = row::agent_draft_error(&brouillon) {
                self.notice = Some(message.into());
                cx.notify();
                return;
            }
            self.set_working(Operation::Saving, cx);
            cx.emit(ProviderSettingsEvent::AgentSaveRequested(Box::new(
                brouillon,
            )));
            return;
        }
        let draft = self.draft(cx);
        if let Some(message) = draft_error(&draft) {
            self.notice = Some(message);
            cx.notify();
            return;
        }
        self.last_draft_had_key = draft.key.is_some();
        self.key
            .update(cx, |champ, cx| champ.set_text(String::new(), cx));
        self.notice = None;
        self.set_working(Operation::Saving, cx);
        cx.emit(ProviderSettingsEvent::SaveRequested(Box::new(draft)));
    }

    /// Ouvre la confirmation de retrait sur le rang donné, focus sur le refus.
    pub fn ask_removal(&mut self, index: usize, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.state.accepts_edits() || index >= self.declaration_count() {
            return;
        }
        self.confirming = Some(index);
        // Le geste sûr est celui qui a le focus : une frappe qui continue ne
        // retire rien.
        window.focus(&self.cancel_focus);
        cx.notify();
    }

    /// Referme ce qui est ouvert : confirmation, puis bandeau d'erreur.
    pub fn dismiss(&mut self, cx: &mut Context<'_, Self>) {
        if self.confirming.take().is_some() {
            cx.notify();
            return;
        }
        if matches!(self.state, ProviderSettingsState::Failed { .. }) {
            self.state = ProviderSettingsState::Ready;
            cx.notify();
        }
    }

    /// Demande le retrait confirmé.
    pub fn confirm_removal(&mut self, cx: &mut Context<'_, Self>) {
        let Some(index) = self.confirming.take() else {
            return;
        };
        if index >= self.declaration_count() {
            cx.notify();
            return;
        }
        self.set_working(Operation::Removing, cx);
        // Le rang global se traduit **ici**, une seule fois. L'appelant reçoit
        // un rang qui indexe la liste qu'il connaît, et n'a jamais à refaire le
        // découpage — c'est ce qui l'empêche de se tromper le jour où l'ordre
        // d'affichage change.
        if index < self.providers.len() {
            cx.emit(ProviderSettingsEvent::RemovalConfirmed(index));
        } else {
            cx.emit(ProviderSettingsEvent::AgentRemovalConfirmed(
                index - self.providers.len(),
            ));
        }
    }

    /// Demande l'abandon de l'opération en cours.
    pub fn cancel(&mut self, cx: &mut Context<'_, Self>) {
        if matches!(self.state, ProviderSettingsState::Working { .. }) {
            cx.emit(ProviderSettingsEvent::CancelRequested);
        }
    }

    /// Rend les champs modifiables ou non, sans les reconstruire.
    ///
    /// Reconstruire les entités ferait sauter le focus de qui était en train de
    /// saisir, et l'échec se lirait comme un défaut du clavier.
    fn set_fields_read_only(&mut self, read_only: bool, cx: &mut Context<'_, Self>) {
        for champ in [
            self.label.clone(),
            self.base_url.clone(),
            self.model.clone(),
            self.key.clone(),
        ] {
            champ.update(cx, |champ, cx| champ.set_read_only(read_only, cx));
        }
    }

    /// Vide le formulaire après un enregistrement abouti.
    fn clear_form(&mut self, cx: &mut Context<'_, Self>) {
        for champ in [
            self.label.clone(),
            self.base_url.clone(),
            self.model.clone(),
            self.key.clone(),
        ] {
            champ.update(cx, |champ, cx| {
                champ.set_read_only(false, cx);
                champ.set_text(String::new(), cx);
            });
        }
        self.notice = None;
        self.last_draft_had_key = false;
    }

    /// Le clavier de l'écran.
    ///
    /// GPUI **ne donne pas** l'activation clavier d'un contrôle qui a le
    /// curseur : une sonde écrite pendant ce lot montre qu'un
    /// [`control`](crate::controls::control) focalisé n'ouvre rien sur `Entrée`
    /// ni sur `Espace`, malgré son `tab_stop`. L'écran la fournit donc
    /// lui-même, en regardant lequel de ses gestes a le curseur — c'est ce que
    /// font déjà [`ApprovalDialog`](crate::approval::ApprovalDialog) et le
    /// formulaire de connexion. [ADR-0001](../../../docs/adr/0001-ui-toolkit.md)
    /// classe l'accessibilité de ce toolkit comme un risque **structurel**, pas
    /// comme une finition.
    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        // Un champ de saisie garde ses touches : sans cette réserve, la barre
        // d'espace tapée dans un nom de fournisseur ouvrirait une confirmation
        // de retrait.
        if self.a_field_is_focused(window, cx) {
            return;
        }
        let touche = event.keystroke.key.as_str();
        let active = matches!(touche, "enter" | "space");

        if self.confirming.is_some() {
            // `Entrée` sur le refus referme : la sortie par réflexe est la
            // sortie sûre. Le retrait, lui, demande d'aller chercher son
            // bouton.
            if touche == "escape" || (active && self.cancel_focus.is_focused(window)) {
                self.dismiss(cx);
            } else if active && self.confirm_focus.is_focused(window) {
                self.confirm_removal(cx);
            } else {
                return;
            }
            // Le curseur revient à la liste : le bouton qu'il occupait vient de
            // disparaître, et un curseur nulle part est un cul-de-sac.
            window.focus(&self.list_focus);
            cx.stop_propagation();
            cx.notify();
            return;
        }

        if active {
            if self.abandon_focus.is_focused(window) {
                self.cancel(cx);
            } else if self.dismiss_focus.is_focused(window) {
                self.dismiss(cx);
            } else if self.save_focus.is_focused(window) {
                self.submit(cx);
            } else if self.list_focus.is_focused(window) {
                self.ask_removal(self.focused_row, window, cx);
            } else {
                return;
            }
        } else {
            match touche {
                "escape" if matches!(self.state, ProviderSettingsState::Working { .. }) => {
                    self.cancel(cx);
                }
                "escape" => self.dismiss(cx),
                "down" | "up" if self.list_focus.is_focused(window) => {
                    self.move_row(touche == "down");
                }
                _ => return,
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Un champ de saisie ou le sélecteur de famille tient-il le curseur ?
    fn a_field_is_focused(&self, window: &Window, cx: &App) -> bool {
        self.kind.read(cx).focus_handle(cx).is_focused(window)
            || [&self.label, &self.base_url, &self.model, &self.key]
                .iter()
                .any(|champ| champ.read(cx).focus_handle(cx).is_focused(window))
    }

    /// Déplace le curseur d'une ligne, en bouclant.
    fn move_row(&mut self, avant: bool) {
        let total = self.providers.len();
        if total == 0 {
            return;
        }
        self.focused_row = if avant {
            self.focused_row.saturating_add(1) % total
        } else {
            self.focused_row
                .checked_sub(1)
                .unwrap_or(total.saturating_sub(1))
        };
    }

    /// La ligne qui a le curseur dans la liste.
    #[must_use]
    pub const fn focused_row(&self) -> usize {
        self.focused_row
    }

    /// Combien de lignes la liste porte, les deux sortes confondues.
    ///
    /// Les agents suivent les fournisseurs : un rang inférieur à
    /// `providers.len()` désigne un fournisseur, au-delà un agent. C'est ce
    /// découpage qui permet à l'écran de n'avoir qu'**un** curseur et qu'une
    /// confirmation, plutôt que deux listes qui se disputeraient le clavier.
    #[must_use]
    pub const fn declaration_count(&self) -> usize {
        self.providers.len() + self.agents.len()
    }

    /// Les agents déclarés, tels que l'écran les montre.
    #[must_use]
    pub fn agents(&self) -> &[DeclaredAgent] {
        &self.agents
    }

    /// Pose la liste des agents rendue par le bus.
    pub fn set_agents(&mut self, agents: Vec<DeclaredAgent>, cx: &mut Context<'_, Self>) {
        self.agents = agents;
        self.confirming = None;
        self.focused_row = self
            .focused_row
            .min(self.declaration_count().saturating_sub(1));
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brouillon() -> ProviderDraft {
        ProviderDraft {
            kind: AiProviderKind::OpenAiCompatible,
            label: "Ollama du portable".to_owned(),
            base_url: "http://localhost:11434/v1".to_owned(),
            model: "llama3.2".to_owned(),
            key: None,
        }
    }

    fn declaration(label: &str, key: KeyState, reach: ProviderReach) -> DeclaredProvider {
        DeclaredProvider {
            label: label.to_owned().into(),
            kind: AiProviderKind::OpenAiCompatible,
            base_url: "http://localhost:11434/v1".into(),
            model: "llama3.2".into(),
            key,
            reach,
            measured_at: "at 14:02".into(),
        }
    }

    #[test]
    fn un_mot_de_passe_colle_dans_l_url_est_refuse_avec_un_message() {
        // ADR-0023 : refusée, jamais nettoyée en silence. Un nettoyage
        // rendrait à l'utilisateur une configuration qu'il n'a pas saisie et
        // laisserait sa clé sans propriétaire.
        let mut draft = brouillon();
        draft.base_url = "https://alice:hunter2@api.example.com/v1".to_owned();

        let message = draft_error(&draft).expect("une URL avec identifiants ne s'enregistre pas");
        assert!(!message.contains("hunter2"), "{message}");
        assert!(!message.contains("alice"), "{message}");
        assert!(message.contains("credentials"), "{message}");

        // Et le refus ne dépend pas du reste du formulaire : il tombe dès que
        // l'URL est saisie, même si rien d'autre ne l'est.
        let mut nu = ProviderDraft {
            label: String::new(),
            model: String::new(),
            ..draft
        };
        assert!(draft_notice(&nu).is_some(), "le refus doit être immédiat");

        // Un formulaire à moitié rempli, lui, ne reproche rien : signaler un
        // champ qu'on n'a pas encore atteint apprend à ignorer les messages.
        nu.base_url = "http://localhost:11434/v1".to_owned();
        assert!(draft_notice(&nu).is_none());
    }

    #[test]
    fn la_validation_du_domaine_ignore_l_identifiant() {
        // `candidate` construit une identité de remplissage pour appeler
        // `AiProviderConfig::validate`. Ce test tient l'hypothèse : le jour où
        // le domaine se mettrait à juger l'identifiant, la validation de cet
        // écran divergerait de celle du bus, en silence.
        for draft in [
            brouillon(),
            ProviderDraft {
                model: String::new(),
                ..brouillon()
            },
        ] {
            let verdicts: Vec<bool> = [ProviderId::ollama(), ProviderId::anthropic()]
                .into_iter()
                .map(|id| {
                    AiProviderConfig::new(
                        id,
                        draft.kind,
                        draft.label.clone(),
                        draft.base_url.clone(),
                        draft.model.clone(),
                    )
                    .validate()
                    .is_ok()
                })
                .collect();
            assert_eq!(verdicts.first(), verdicts.last());
        }
    }

    #[test]
    fn les_bornes_du_domaine_sont_celles_de_l_ecran() {
        // Les champs portent les bornes de `oxyn-core` et non des nombres
        // recopiés : une borne d'écran plus large laisserait saisir ce que le
        // bus refuse ensuite, loin de la saisie.
        let mut draft = brouillon();
        draft.label = "a".repeat(MAX_PROVIDER_LABEL_BYTES + 1);
        assert!(draft_error(&draft).is_some());

        draft = brouillon();
        draft.model = "m".repeat(MAX_PROVIDER_MODEL_BYTES + 1);
        assert!(draft_error(&draft).is_some());

        assert!(draft_error(&brouillon()).is_none());
    }

    #[test]
    fn le_debug_du_brouillon_ne_rend_ni_la_cle_ni_l_url() {
        // I-03 : la clé est évidente, l'URL l'est moins — un brouillon n'a pas
        // encore été validé, donc son point d'accès peut porter exactement le
        // couple que l'écran est en train de refuser.
        let draft = ProviderDraft {
            base_url: "https://alice:hunter2@api.example.com/v1".to_owned(),
            key: Some("sk-CECINEDOITPASFUIR".to_owned()),
            ..brouillon()
        };

        let rendu = format!("{draft:?}");
        assert!(!rendu.contains("hunter2"), "URL fuitée : {rendu}");
        assert!(!rendu.contains("api.example.com"), "URL fuitée : {rendu}");
        assert!(!rendu.contains("CECINEDOITPASFUIR"), "clé fuitée : {rendu}");
        assert!(
            rendu.contains("Ollama du portable"),
            "le nom reste diagnosticable : {rendu}"
        );
    }

    #[test]
    fn un_classement_non_resolu_ne_devient_jamais_local() {
        // UX-SPEC : le doute ne profite pas à l'envoi.
        assert!(ProviderReach::Unresolved.leaves_machine());
        assert!(ProviderReach::Remote.leaves_machine());
        assert!(!ProviderReach::Local.leaves_machine());
        assert_eq!(ProviderReach::Unresolved.label(), "unresolved");
    }

    #[test]
    fn le_classement_est_toujours_dit_avec_l_instant_de_sa_mesure() {
        // Le classement est recalculé et jamais persisté : sans son instant, il
        // aurait l'air éternel (ADR-0023).
        let ligne = reach_summary(ProviderReach::Local, "at 14:02");
        assert!(ligne.contains("local"), "{ligne}");
        assert!(ligne.contains("14:02"), "{ligne}");

        let sans = reach_summary(ProviderReach::Remote, "   ");
        assert!(sans.contains("remote"), "{sans}");
        assert!(sans.contains("unknown"), "{sans}");
    }

    #[test]
    fn la_confirmation_de_retrait_nomme_le_fournisseur() {
        let question = removal_question("Ollama du portable");
        assert!(question.contains("Ollama du portable"), "{question}");
    }

    #[test]
    fn chaque_famille_offerte_a_son_libelle_et_son_rang() {
        for (rang, (famille, libelle)) in FAMILLES.iter().enumerate() {
            assert_eq!(kind_label(*famille), Some(*libelle));
            assert_eq!(
                kind_options().get(rang).map(SharedString::as_ref),
                Some(*libelle)
            );
            for (autre, autre_libelle) in FAMILLES.iter().skip(rang + 1) {
                assert_ne!(famille, autre, "deux entrées pour la même famille");
                assert_ne!(libelle, autre_libelle);
            }
        }
    }

    #[gpui::test]
    fn la_cle_ne_survit_pas_a_son_envoi(cx: &mut gpui::TestAppContext) {
        // I-03, et la règle de ce lot : la clé traverse l'événement
        // d'enregistrement, et rien d'autre. Ce test rougit si `submit` cesse
        // de vider le champ.
        let recues = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let observees = std::rc::Rc::clone(&recues);
        let (ecran, cx) = cx.add_window_view(|_, cx| {
            let ecran = ProviderSettings::new(cx);
            cx.subscribe(
                &cx.entity(),
                move |_, _, event: &ProviderSettingsEvent, _| {
                    if let ProviderSettingsEvent::SaveRequested(draft) = event
                        && let Some(cle) = &draft.key
                    {
                        observees.borrow_mut().push(cle.clone());
                    }
                },
            )
            .detach();
            ecran
        });
        ecran.update(cx, |ecran, cx| {
            ecran.set_providers(Vec::new(), cx);
            ecran
                .label
                .update(cx, |champ, cx| champ.set_text("Ollama".to_owned(), cx));
            ecran.base_url.update(cx, |champ, cx| {
                champ.set_text("http://localhost:11434/v1".to_owned(), cx);
            });
            ecran
                .model
                .update(cx, |champ, cx| champ.set_text("llama3.2".to_owned(), cx));
            ecran
                .key
                .update(cx, |champ, cx| champ.set_text("sk-secret".to_owned(), cx));
            ecran.submit(cx);
        });
        cx.run_until_parked();

        assert_eq!(recues.borrow().as_slice(), ["sk-secret"]);
        ecran.read_with(cx, |ecran, cx| {
            assert!(
                ecran.key.read(cx).text().is_empty(),
                "la clé est restée dans le champ après l'envoi"
            );
            assert!(
                !format!("{ecran:?}").contains("sk-secret"),
                "la clé est restée dans l'état de la vue"
            );
            assert!(matches!(
                ecran.state,
                ProviderSettingsState::Working {
                    operation: Operation::Saving
                }
            ));
        });

        // L'échec le dit plutôt que de laisser découvrir un champ vide.
        ecran.update(cx, |ecran, cx| {
            ecran.set_failed("keyring: access denied", true, cx);
        });
        ecran.read_with(cx, |ecran, _| {
            assert!(matches!(
                ecran.state,
                ProviderSettingsState::Failed {
                    key_must_be_retyped: true,
                    retryable: true,
                    ..
                }
            ));
        });
    }

    #[gpui::test]
    fn un_retrait_demande_une_confirmation_qui_ne_part_pas_a_la_touche_entree(
        cx: &mut gpui::TestAppContext,
    ) {
        // Le bouton par défaut n'est jamais l'action destructrice : la
        // confirmation s'ouvre sur le refus, `Entrée` ne retire rien, `Échap`
        // referme.
        let retires = std::rc::Rc::new(std::cell::Cell::new(0_usize));
        let observes = std::rc::Rc::clone(&retires);
        let (ecran, cx) = cx.add_window_view(|_, cx| {
            let ecran = ProviderSettings::new(cx);
            cx.subscribe(
                &cx.entity(),
                move |_, _, event: &ProviderSettingsEvent, _| {
                    if matches!(event, ProviderSettingsEvent::RemovalConfirmed(_)) {
                        observes.set(observes.get() + 1);
                    }
                },
            )
            .detach();
            ecran
        });
        cx.update(|window, cx| {
            ecran.update(cx, |ecran, cx| {
                ecran.set_providers(
                    vec![declaration(
                        "Ollama du portable",
                        KeyState::Absent,
                        ProviderReach::Local,
                    )],
                    cx,
                );
                ecran.ask_removal(0, window, cx);
            });
        });
        cx.run_until_parked();

        cx.update(|window, cx| {
            assert!(
                ecran.read(cx).cancel_focus.is_focused(window),
                "la confirmation doit s'ouvrir sur le refus"
            );
        });
        // La touche qu'on presse sans lire referme, elle ne retire pas.
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert_eq!(retires.get(), 0, "`Entrée` ne doit rien retirer");
        assert_eq!(
            ecran.read_with(cx, |ecran, _| ecran.confirming),
            None,
            "`Entrée` sur le refus referme la confirmation"
        );

        // `Échap` referme aussi, depuis l'écran comme depuis le refus.
        cx.update(|window, cx| {
            ecran.update(cx, |ecran, cx| ecran.ask_removal(0, window, cx));
            window.focus(&ecran.read(cx).focus);
        });
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert_eq!(ecran.read_with(cx, |ecran, _| ecran.confirming), None);
        assert_eq!(retires.get(), 0);

        // Et le retrait reste **atteignable au clavier** : il demande d'aller
        // chercher son bouton, pas une souris.
        cx.update(|window, cx| {
            ecran.update(cx, |ecran, cx| ecran.ask_removal(0, window, cx));
            window.focus(&ecran.read(cx).confirm_focus);
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert_eq!(retires.get(), 1);
        assert_eq!(ecran.read_with(cx, |ecran, _| ecran.confirming), None);

        // Une seconde confirmation sans demande ne produit pas un second
        // retrait : un double-clic ne doit pas retirer deux déclarations.
        ecran.update(cx, |ecran, cx| ecran.confirm_removal(cx));
        cx.run_until_parked();
        assert_eq!(retires.get(), 1);
    }

    #[gpui::test]
    /// Le rang global se traduit dans la bonne liste, et une seule fois.
    ///
    /// C'est ici que le défaut vivrait : les deux sortes partagent une liste et
    /// un curseur, mais l'appelant reçoit un rang qui indexe **sa** liste. Se
    /// tromper de traduction retirerait une déclaration pour une autre — sans
    /// message, puisque les deux gestes réussissent.
    #[gpui::test]
    fn le_rang_dun_retrait_designe_la_bonne_liste(cx: &mut gpui::TestAppContext) {
        let (ecran, cx) = cx.add_window_view(|_, cx| ProviderSettings::new(cx));
        let recus = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let temoin = std::rc::Rc::clone(&recus);
        cx.update(|_, cx| {
            cx.subscribe(
                &ecran,
                move |_, event: &ProviderSettingsEvent, _| match event {
                    ProviderSettingsEvent::RemovalConfirmed(rang) => {
                        temoin.borrow_mut().push(format!("provider:{rang}"));
                    }
                    ProviderSettingsEvent::AgentRemovalConfirmed(rang) => {
                        temoin.borrow_mut().push(format!("agent:{rang}"));
                    }
                    _ => {}
                },
            )
            .detach();
        });

        ecran.update(cx, |ecran, cx| {
            ecran.set_providers(
                vec![
                    declaration("Ollama", KeyState::Absent, ProviderReach::Local),
                    declaration("OpenAI", KeyState::Configured, ProviderReach::Remote),
                ],
                cx,
            );
            ecran.set_agents(
                vec![
                    DeclaredAgent {
                        label: "Claude Code".into(),
                        command: "claude".into(),
                        args: 1,
                    },
                    DeclaredAgent {
                        label: "Gemini CLI".into(),
                        command: "gemini".into(),
                        args: 0,
                    },
                ],
                cx,
            );
            assert_eq!(ecran.declaration_count(), 4);
        });

        // Rang 1 : le second fournisseur.
        cx.update(|window, cx| {
            ecran.update(cx, |ecran, cx| {
                ecran.ask_removal(1, window, cx);
                ecran.confirm_removal(cx);
            });
        });
        // Rang 2 : le **premier** agent, pas le troisième fournisseur.
        cx.update(|window, cx| {
            ecran.update(cx, |ecran, cx| {
                // Le bus repose la liste après chaque retrait : c'est ce qui
                // fait repasser l'écran de « en cours » à « prêt ».
                ecran.set_providers(
                    vec![
                        declaration("Ollama", KeyState::Absent, ProviderReach::Local),
                        declaration("OpenAI", KeyState::Configured, ProviderReach::Remote),
                    ],
                    cx,
                );
                ecran.ask_removal(2, window, cx);
                ecran.confirm_removal(cx);
            });
        });
        // Rang 3 : le second agent.
        cx.update(|window, cx| {
            ecran.update(cx, |ecran, cx| {
                ecran.set_providers(
                    vec![
                        declaration("Ollama", KeyState::Absent, ProviderReach::Local),
                        declaration("OpenAI", KeyState::Configured, ProviderReach::Remote),
                    ],
                    cx,
                );
                ecran.ask_removal(3, window, cx);
                ecran.confirm_removal(cx);
            });
        });

        assert_eq!(
            recus.borrow().as_slice(),
            ["provider:1", "agent:0", "agent:1"],
            "un rang au-delà des fournisseurs désigne un agent, recalé sur sa propre liste"
        );
    }

    #[gpui::test]
    /// Le formulaire bascule de sorte, et n'enregistre jamais l'autre.
    ///
    /// Le mode se déduit du sélecteur, pas d'un drapeau à part : deux sources
    /// pour la même question finiraient par se contredire, et le formulaire
    /// afficherait alors des champs sans rapport avec ce qu'il enregistre.
    #[gpui::test]
    fn le_formulaire_denregistre_que_la_sorte_choisie(cx: &mut gpui::TestAppContext) {
        let (ecran, cx) = cx.add_window_view(|_, cx| ProviderSettings::new(cx));
        let recus = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let temoin = std::rc::Rc::clone(&recus);
        cx.update(|_, cx| {
            cx.subscribe(
                &ecran,
                move |_, event: &ProviderSettingsEvent, _| match event {
                    ProviderSettingsEvent::SaveRequested(_) => {
                        temoin.borrow_mut().push("provider");
                    }
                    ProviderSettingsEvent::AgentSaveRequested(_) => {
                        temoin.borrow_mut().push("agent");
                    }
                    _ => {}
                },
            )
            .detach();
        });

        ecran.update(cx, |ecran, cx| {
            assert!(!ecran.declares_agent(), "un fournisseur par défaut");
            ecran.set_providers(Vec::new(), cx);

            // L'utilisateur choisit la dernière entrée du sélecteur.
            ecran.kind_index = kind_options().len() - 1;
            assert!(ecran.declares_agent());

            // Une saisie d'agent valide.
            ecran.label.update(cx, |champ, cx| {
                champ.set_text("Claude Code".to_owned(), cx);
            });
            ecran.command.update(cx, |champ, cx| {
                champ.set_text("claude".to_owned(), cx);
            });
            ecran.args.update(cx, |champ, cx| {
                champ.set_text("--acp".to_owned(), cx);
            });
            ecran.submit(cx);
        });

        assert_eq!(
            recus.borrow().as_slice(),
            ["agent"],
            "le mode agent n'émet jamais une déclaration de fournisseur"
        );
    }

    #[gpui::test]
    fn la_liste_et_le_formulaire_se_conduisent_au_clavier(cx: &mut gpui::TestAppContext) {
        // GPUI n'active pas un contrôle focalisé sur `Entrée` : l'écran doit le
        // faire lui-même. Ce test rougit si cette activation disparaît, et la
        // panne serait silencieuse — l'écran resterait beau et inutilisable
        // sans souris (ADR-0001).
        let (ecran, cx) = cx.add_window_view(|_, cx| ProviderSettings::new(cx));
        ecran.update(cx, |ecran, cx| {
            ecran.set_providers(
                vec![
                    declaration("Ollama", KeyState::Absent, ProviderReach::Local),
                    declaration("OpenAI", KeyState::Configured, ProviderReach::Remote),
                ],
                cx,
            );
        });
        cx.update(|window, cx| window.focus(&ecran.read(cx).list_focus));
        cx.run_until_parked();

        cx.simulate_keystrokes("down");
        cx.run_until_parked();
        assert_eq!(ecran.read_with(cx, |ecran, _| ecran.focused_row()), 1);
        cx.simulate_keystrokes("up");
        cx.simulate_keystrokes("up");
        cx.run_until_parked();
        assert_eq!(
            ecran.read_with(cx, |ecran, _| ecran.focused_row()),
            1,
            "le curseur boucle plutôt que de se coincer"
        );

        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert_eq!(
            ecran.read_with(cx, |ecran, _| ecran.confirming),
            Some(1),
            "`Entrée` sur la liste ouvre la confirmation de la ligne choisie"
        );
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();

        // Le bouton d'enregistrement, lui aussi, s'active au clavier.
        ecran.update(cx, |ecran, cx| {
            ecran
                .label
                .update(cx, |champ, cx| champ.set_text("Ollama".to_owned(), cx));
            ecran.base_url.update(cx, |champ, cx| {
                champ.set_text("http://localhost:11434/v1".to_owned(), cx);
            });
            ecran
                .model
                .update(cx, |champ, cx| champ.set_text("llama3.2".to_owned(), cx));
        });
        cx.update(|window, cx| window.focus(&ecran.read(cx).save_focus));
        cx.run_until_parked();
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(matches!(
            ecran.read_with(cx, |ecran, _| ecran.state.clone()),
            ProviderSettingsState::Working {
                operation: Operation::Saving
            }
        ));

        // Et le bandeau d'échec se referme au clavier, pas seulement à `Échap`.
        ecran.update(cx, |ecran, cx| {
            ecran.set_failed("keyring: access denied", true, cx);
        });
        cx.update(|window, cx| window.focus(&ecran.read(cx).dismiss_focus));
        cx.run_until_parked();
        cx.simulate_keystrokes("space");
        cx.run_until_parked();
        assert_eq!(
            ecran.read_with(cx, |ecran, _| ecran.state.clone()),
            ProviderSettingsState::Ready
        );
    }

    #[gpui::test]
    fn le_bouton_denregistrement_suit_la_validation_du_domaine(cx: &mut gpui::TestAppContext) {
        // Le bouton est disponible si, et seulement si, le domaine accepterait
        // la déclaration. Un bouton disponible sur une saisie que le bus
        // refuse ensuite fait découvrir l'erreur loin de la saisie.
        let (ecran, cx) = cx.add_window_view(|_, cx| ProviderSettings::new(cx));
        ecran.update(cx, |ecran, cx| {
            ecran
                .label
                .update(cx, |champ, cx| champ.set_text("Ollama".to_owned(), cx));
            ecran
                .model
                .update(cx, |champ, cx| champ.set_text("llama3.2".to_owned(), cx));
            ecran.base_url.update(cx, |champ, cx| {
                champ.set_text("http://localhost:11434/v1".to_owned(), cx);
            });
            // La liste rendue par le bus fait aussi revoir la saisie.
            ecran.set_providers(Vec::new(), cx);
        });
        ecran.read_with(cx, |ecran, _| {
            assert!(ecran.is_submittable());
            assert!(ecran.notice().is_none());
        });

        ecran.update(cx, |ecran, cx| {
            ecran.base_url.update(cx, |champ, cx| {
                champ.set_text("https://alice:hunter2@api.example.com/v1".to_owned(), cx);
            });
            ecran.set_providers(Vec::new(), cx);
        });
        ecran.read_with(cx, |ecran, _| {
            assert!(!ecran.is_submittable(), "le domaine refuse cette URL");
            let message = ecran.notice().expect("un refus se dit").to_string();
            assert!(!message.contains("hunter2"), "{message}");
        });
    }

    #[gpui::test]
    fn l_ecran_distingue_le_vide_de_l_attente_et_de_l_echec(cx: &mut gpui::TestAppContext) {
        // L'état qu'on oublie est le vide, et sur une installation neuve c'est
        // le seul que l'utilisateur voit. Il ne doit se confondre ni avec
        // « pas encore lu » ni avec « échec ».
        let (ecran, cx) = cx.add_window_view(|_, cx| ProviderSettings::new(cx));
        ecran.read_with(cx, |ecran, _| {
            assert_eq!(ecran.state, ProviderSettingsState::Loading);
            assert!(
                !ecran.state.accepts_edits(),
                "rien à modifier avant lecture"
            );
        });

        ecran.update(cx, |ecran, cx| ecran.set_providers(Vec::new(), cx));
        ecran.read_with(cx, |ecran, _| {
            assert_eq!(ecran.state, ProviderSettingsState::Ready);
            assert!(ecran.providers().is_empty());
            assert!(ecran.state.accepts_edits());
        });

        ecran.update(cx, |ecran, cx| {
            ecran.set_providers(
                vec![declaration(
                    "Ollama",
                    KeyState::Configured,
                    ProviderReach::Unresolved,
                )],
                cx,
            );
        });
        ecran.read_with(cx, |ecran, _| assert_eq!(ecran.providers().len(), 1));
    }

    #[gpui::test]
    fn une_operation_en_cours_fige_la_saisie_et_laisse_abandonner(cx: &mut gpui::TestAppContext) {
        let abandons = std::rc::Rc::new(std::cell::Cell::new(0_usize));
        let observes = std::rc::Rc::clone(&abandons);
        let (ecran, cx) = cx.add_window_view(|window, cx| {
            let ecran = ProviderSettings::new(cx);
            window.focus(&ecran.focus);
            cx.subscribe(
                &cx.entity(),
                move |_, _, event: &ProviderSettingsEvent, _| {
                    if matches!(event, ProviderSettingsEvent::CancelRequested) {
                        observes.set(observes.get() + 1);
                    }
                },
            )
            .detach();
            ecran
        });
        ecran.update(cx, |ecran, cx| {
            ecran.set_providers(Vec::new(), cx);
            ecran.set_working(Operation::Saving, cx);
        });
        cx.run_until_parked();

        ecran.read_with(cx, |ecran, cx| {
            assert!(!ecran.state.accepts_edits());
            assert!(
                ecran.label.read(cx).is_read_only(),
                "les champs restent lisibles mais ne se modifient plus"
            );
            assert!(!ecran.is_submittable());
        });

        // Le focus est sur l'écran lui-même : `Échap` y demande l'abandon.
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert_eq!(abandons.get(), 1);

        ecran.update(cx, |ecran, cx| {
            ecran.set_providers(
                vec![declaration(
                    "Ollama",
                    KeyState::Absent,
                    ProviderReach::Local,
                )],
                cx,
            );
        });
        ecran.read_with(cx, |ecran, cx| {
            assert!(!ecran.label.read(cx).is_read_only());
            assert!(
                ecran.label.read(cx).text().is_empty(),
                "un enregistrement abouti vide le formulaire"
            );
        });
    }
}
