//! Ce qui traduit entre l'écran de configuration et le domaine.
//!
//! `oxyn-ui` ne connaît ni `oxyn-llm`, ni les dates, ni le trousseau : c'est ce
//! qui lui permet d'être testable sans rien de tout cela. Le prix est cette
//! traduction, et elle vit ici plutôt que dans la vue.
//!
//! # Ce qui se traduit par variante, et jamais par sa chaîne
//!
//! [`ProviderReach`] est le miroir de [`Reach`] côté interface. Passer par
//! `Reach::as_str()` marcherait aujourd'hui — les deux rendent les mêmes mots —
//! et casserait en silence le jour où l'un des deux change un libellé. Une
//! correspondance de variantes, elle, ne compile plus ce jour-là.

use chrono::{DateTime, Local, Utc};
use gpui::SharedString;
use oxyn_core::{AiProviderConfig, ProviderId};
use oxyn_llm::Reach;
use oxyn_ui::provider_settings::{DeclaredProvider, KeyState, ProviderDraft, ProviderReach};

/// Traduit le classement d'un point d'accès.
///
/// `Unresolved` reste `Unresolved` : l'arrondir à « distant » perdrait ce que
/// l'écran doit dire — Oxyn n'a pas su classer ce point d'accès. Il compte
/// **comme** distant pour toute décision, ce que `Reach::leaves_machine` tient
/// déjà, mais l'afficher comme tel serait affirmer une mesure qui n'a pas eu
/// lieu ([ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md)).
pub(crate) const fn reach_of(reach: Reach) -> ProviderReach {
    match reach {
        Reach::Local => ProviderReach::Local,
        Reach::Remote => ProviderReach::Remote,
        Reach::Unresolved => ProviderReach::Unresolved,
    }
}

/// Met en forme l'instant d'une mesure, dans le fuseau de l'utilisateur.
///
/// L'heure seule, sans la date : la mesure date de cette session — le
/// classement n'est jamais persisté — et écrire la date laisserait croire qu'un
/// relevé d'hier a été conservé.
pub(crate) fn measured_at(instant: DateTime<Utc>) -> SharedString {
    instant
        .with_timezone(&Local)
        .format("%H:%M")
        .to_string()
        .into()
}

/// Ce que l'écran montre d'une déclaration.
///
/// Le `secret_ref` ne traverse pas : l'écran dit « une clé est enregistrée » ou
/// « aucune », jamais où elle est rangée ni ce qu'elle vaut
/// ([I-03](../../../CLAUDE.md#i-03)).
pub(crate) fn declared(
    config: &AiProviderConfig,
    reach: Reach,
    measured: DateTime<Utc>,
) -> DeclaredProvider {
    DeclaredProvider {
        label: config.label.clone().into(),
        kind: config.kind,
        base_url: config.base_url.clone().into(),
        model: config.model.clone().into(),
        key: if config.secret_ref.is_some() {
            KeyState::Configured
        } else {
            KeyState::Absent
        },
        reach: reach_of(reach),
        measured_at: measured_at(measured),
    }
}

/// Fabrique la déclaration à persister à partir de ce que l'utilisateur a rempli.
///
/// L'identité est **reçue** et non frappée ici. La raison est le trousseau : la
/// clé est rangée sous une référence dérivée de cet identifiant, et
/// l'appelant doit donc l'avoir en main **avant** d'écrire la clé. La frapper
/// une seconde fois ici, pour l'écraser ensuite, faisait tenir tout l'appariement
/// à une ligne d'affectation qu'aucun test ne gardait : la supprimer compilait,
/// passait la porte de qualité, et produisait une déclaration dont la clé était
/// introuvable — signalée au premier message, longtemps après un enregistrement
/// annoncé réussi.
///
/// `secret_ref` est celle que le trousseau a rendue, ou `None` pour un point
/// d'accès qui ne demande pas de clé.
///
/// La configuration rendue n'est **pas** validée ici : elle l'est par
/// `AiProviderConfig::validate`, que la commande appelle avant d'écrire. Un
/// second appel ici donnerait deux endroits où la règle vit.
pub(crate) fn config_of(
    id: ProviderId,
    draft: &ProviderDraft,
    secret_ref: Option<String>,
) -> AiProviderConfig {
    let config = AiProviderConfig::new(
        id,
        draft.kind,
        draft.label.trim(),
        draft.base_url.trim(),
        draft.model.trim(),
    );
    match secret_ref {
        Some(reference) => config.with_secret_ref(reference),
        None => config,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::AiProviderKind;

    fn brouillon() -> ProviderDraft {
        ProviderDraft {
            kind: AiProviderKind::Anthropic,
            label: "  Poste de travail  ".to_owned(),
            base_url: " https://api.anthropic.com ".to_owned(),
            model: " claude-sonnet-5 ".to_owned(),
            key: Some("sk-secret".to_owned()),
        }
    }

    #[test]
    fn un_point_dacces_non_resolu_ne_sarrondit_pas_a_distant() {
        // Il compte comme distant pour décider, et il se dit « non résolu » à
        // l'écran : affirmer une mesure qui n'a pas eu lieu serait mentir sur
        // la seule chose que cet écran a à dire.
        assert_eq!(reach_of(Reach::Unresolved), ProviderReach::Unresolved);
        assert!(Reach::Unresolved.leaves_machine());
        assert_eq!(reach_of(Reach::Local), ProviderReach::Local);
        assert_eq!(reach_of(Reach::Remote), ProviderReach::Remote);
    }

    #[test]
    fn letat_de_la_cle_se_lit_sans_que_la_reference_traverse() {
        let identite = ProviderId::for_new_declaration(AiProviderKind::Anthropic);
        let config = config_of(
            identite,
            &brouillon(),
            Some("oxyn:llm:anthropic-1234abcd".to_owned()),
        );
        let montre = declared(&config, Reach::Remote, Utc::now());
        assert_eq!(montre.key, KeyState::Configured);

        // Rien de ce que l'écran reçoit ne nomme le trousseau. Le test lit tous
        // les champs montrables : c'est ce qui le fera rougir si un champ est
        // ajouté plus tard en y recopiant la référence.
        let montrable = format!(
            "{} {} {} {}",
            montre.label, montre.base_url, montre.model, montre.measured_at
        );
        assert!(
            !montrable.contains("oxyn:llm"),
            "aucune référence de trousseau ne rejoint l'écran : {montrable}"
        );
    }

    #[test]
    fn un_point_dacces_sans_cle_nest_pas_un_defaut() {
        // Ollama, LM Studio et llama.cpp n'en demandent pas : « absente » est un
        // état normal, que l'écran doit distinguer de « configurée ».
        let config = config_of(
            ProviderId::for_new_declaration(AiProviderKind::Anthropic),
            &brouillon(),
            None,
        );
        assert_eq!(
            declared(&config, Reach::Local, Utc::now()).key,
            KeyState::Absent
        );
    }

    #[test]
    fn les_espaces_de_saisie_ne_survivent_pas_a_la_declaration() {
        // Un libellé collé depuis un gestionnaire de mots de passe arrive
        // souvent entouré d'espaces. Les garder ferait de « Prod » et de
        // « Prod  » deux noms différents à l'œil identiques.
        let config = config_of(
            ProviderId::for_new_declaration(AiProviderKind::Anthropic),
            &brouillon(),
            None,
        );
        assert_eq!(config.label, "Poste de travail");
        assert_eq!(config.base_url, "https://api.anthropic.com");
        assert_eq!(config.model, "claude-sonnet-5");
        config
            .validate()
            .expect("la déclaration nettoyée est valide");
    }

    #[test]
    fn deux_declarations_du_meme_brouillon_restent_deux_declarations() {
        // Le piège : une identité dérivée du libellé ferait de la seconde un
        // remplacement silencieux de la première, clé du trousseau comprise.
        let premiere = config_of(
            ProviderId::for_new_declaration(AiProviderKind::Anthropic),
            &brouillon(),
            None,
        );
        let seconde = config_of(
            ProviderId::for_new_declaration(AiProviderKind::Anthropic),
            &brouillon(),
            None,
        );
        assert_ne!(premiere.id, seconde.id);
        assert_eq!(premiere.label, seconde.label, "le libellé, lui, est libre");
    }
}
