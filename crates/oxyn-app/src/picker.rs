//! Turning what a driver declares into what the connection screen shows.
//!
//! `oxyn-ui` must not depend on `oxyn-driver`: a view that could name `Driver`
//! or `Session` would be one call away from the second execution path
//! [I-01](../../../CLAUDE.md#i-01) forbids. So the screen speaks in bare types —
//! [`DriverChoice`], [`FormField`] — and this module is the single translation
//! between the two vocabularies.
//!
//! It holds **no** GPUI and no state, which is the point: every rule below is
//! testable without opening a window.

use std::collections::BTreeMap;

use oxyn_core::{ConnectionConfig, DriverId, IdParseError};
use oxyn_driver::{ConnectionField, DriverMetadata, FieldKind};
use oxyn_ui::connection_form::{
    ConnectionDraft, DriverChoice, FormField, FormFieldKind, SavedConnection,
};

/// Translates a driver's declared field into the one the form renders.
fn champ(field: &ConnectionField) -> FormField {
    let kind = match &field.kind {
        FieldKind::Text => FormFieldKind::Text,
        FieldKind::Password => FormFieldKind::Password,
        FieldKind::Number => FormFieldKind::Number,
        FieldKind::Bool => FormFieldKind::Bool,
        FieldKind::Choice(valeurs) => {
            FormFieldKind::Choice(valeurs.iter().map(|v| v.clone().into()).collect())
        }
        FieldKind::Path => FormFieldKind::Path,
        // `FieldKind` est `#[non_exhaustive]` : un genre que cette version ne
        // sait pas rendre devient un champ texte plutôt que de disparaître du
        // formulaire. Un champ absent est un champ qu'on ne peut pas remplir, et
        // la connexion échouerait sans dire pourquoi.
        autre => {
            tracing::warn!(kind = %autre, field = %field.key, "unknown field kind, rendered as text");
            FormFieldKind::Text
        }
    };

    FormField {
        key: field.key.clone().into(),
        label: field.label.clone().into(),
        kind,
        required: field.required,
        // Jamais de valeur par défaut sur un secret, quoi que le driver
        // déclare : elle serait écrite en clair dans l'interface et dans le
        // binaire ([I-03](../../../CLAUDE.md#i-03)). `DriverMetadata::check`
        // refuse déjà cette déclaration ; le filtre est ici parce qu'un driver
        // tiers n'est pas obligé d'appeler `check`.
        default: if field.is_secret() {
            None
        } else {
            field.default.clone().map(Into::into)
        },
        help: field.help.clone().map(Into::into),
    }
}

/// Translates a driver's metadata into a choice on the connection screen.
#[must_use]
pub fn driver_choice(metadata: &DriverMetadata) -> DriverChoice {
    DriverChoice {
        id: metadata.id.to_string().into(),
        display_name: metadata.display_name.clone().into(),
        family: metadata.family.to_string().into(),
        fields: metadata.connection_fields.iter().map(champ).collect(),
    }
}

/// What the screen shows about a connection that already exists.
///
/// Deliberately drops the [`ConnectionId`](oxyn_core::ConnectionId) and every
/// parameter value: the view is given the rank in the list, and that is enough
/// to designate one ([I-03](../../../CLAUDE.md#i-03)).
#[must_use]
pub fn saved_connection(config: &ConnectionConfig) -> SavedConnection {
    SavedConnection {
        name: config.name.clone().into(),
        driver: config.driver.to_string().into(),
        environment: config.environment,
    }
}

/// Builds the configuration a draft describes — **without** its secrets.
///
/// The secrets stay in [`ConnectionDraft::secrets`] and go to the keyring; what
/// comes back here is what may be written to the workspace file. Splitting the
/// two here rather than at the call site is what makes the rule checkable: a
/// caller cannot accidentally persist a password it was never handed.
///
/// # Errors
/// If the draft names a driver whose identifier is not well formed. That cannot
/// happen through the screen — the identifiers come from the registry — but the
/// alternative is an `expect` on a value that crossed a view boundary
/// ([I-09](../../../CLAUDE.md#i-09)).
pub fn config_from_draft(draft: &ConnectionDraft) -> Result<ConnectionConfig, IdParseError> {
    let mut config = ConnectionConfig::new(draft.name.clone(), DriverId::new(&draft.driver)?)
        .with_environment(draft.environment);

    for (cle, valeur) in &draft.values {
        config = config.with_param(cle, valeur);
    }

    Ok(config)
}

/// The secrets of a draft, keyed by field key.
///
/// A borrow rather than a clone: the fewer copies of a password exist, the
/// fewer places have to be zeroed.
#[must_use]
pub fn secrets_of(draft: &ConnectionDraft) -> &BTreeMap<String, String> {
    &draft.secrets
}

#[cfg(test)]
mod tests {
    // `Environment` n'est utilisé que par les tests : importé plus haut, il
    // devient un avertissement « inutilisé » sur la cible bibliothèque, que
    // `-D warnings` transforme en échec.
    use oxyn_core::Environment;
    use oxyn_driver::{DriverFamily, DriverMetadata};

    use super::*;

    fn metadonnees() -> DriverMetadata {
        DriverMetadata::new(DriverId::postgres(), "PostgreSQL", DriverFamily::Relational)
            .with_default_port(5432)
            .with_field(
                ConnectionField::new("host", "Hôte", FieldKind::Text)
                    .required()
                    .with_default("localhost"),
            )
            .with_field(ConnectionField::new(
                "password",
                "Mot de passe",
                FieldKind::Password,
            ))
            .with_field(ConnectionField::new(
                "sslmode",
                "Mode TLS",
                FieldKind::Choice(vec!["disable".to_owned(), "prefer".to_owned()]),
            ))
    }

    #[test]
    fn les_champs_gardent_lordre_du_driver() {
        // L'ordre est une décision du driver : `host` avant `password` parce
        // que c'est l'ordre dans lequel on remplit une connexion. Un tri
        // alphabétique ici détruirait cette information sans que rien n'échoue.
        let choix = driver_choice(&metadonnees());
        let cles: Vec<_> = choix.fields.iter().map(|c| c.key.as_ref()).collect();
        assert_eq!(cles, ["host", "password", "sslmode"]);
    }

    #[test]
    fn un_champ_secret_perd_sa_valeur_par_defaut() {
        // Même si un driver tiers en déclarait une : elle finirait affichée à
        // l'écran et écrite en clair dans le binaire.
        let mut metadata = metadonnees();
        metadata.connection_fields[1].default = Some("hunter2".to_owned());

        let choix = driver_choice(&metadata);
        assert_eq!(choix.fields[1].default, None);
        assert_eq!(
            choix.fields[0].default.as_ref().map(AsRef::as_ref),
            Some("localhost"),
            "un champ ordinaire garde la sienne"
        );
    }

    #[test]
    fn un_choix_ferme_garde_ses_valeurs_dans_lordre() {
        let choix = driver_choice(&metadonnees());
        match &choix.fields[2].kind {
            FormFieldKind::Choice(valeurs) => {
                let vues: Vec<&str> = valeurs.iter().map(AsRef::as_ref).collect();
                assert_eq!(vues, ["disable", "prefer"]);
            }
            autre => panic!("expected a closed choice, got {autre:?}"),
        }
    }

    #[test]
    fn la_configuration_ne_porte_aucun_secret() {
        // Le test qui compte : c'est cette valeur-là qui est écrite dans le
        // fichier de workspace ([I-03](../../../CLAUDE.md#i-03)).
        let mut values = BTreeMap::new();
        values.insert("host".to_owned(), "db.interne".to_owned());
        let mut secrets = BTreeMap::new();
        secrets.insert("password".to_owned(), "hunter2".to_owned());

        let draft = ConnectionDraft {
            driver: "postgres".into(),
            name: "production".to_owned(),
            environment: Environment::Production,
            values,
            secrets,
        };

        let config = config_from_draft(&draft).expect("« postgres » est un identifiant valide");
        assert_eq!(
            config.params.get("host").map(String::as_str),
            Some("db.interne")
        );
        assert!(config.params.get("password").is_none());
        assert!(
            config.secret_ref.is_none(),
            "la référence s'ajoute après l'écriture au trousseau"
        );

        let rendu = format!("{config:?}");
        assert!(
            !rendu.contains("hunter2"),
            "un secret a fuité dans la configuration : {rendu}"
        );
    }

    #[test]
    fn lenvironnement_du_brouillon_est_repris_tel_quel() {
        // Jamais deviné à partir du nom ni de l'hôte : une connexion marquée
        // `Production` doit l'être parce que l'utilisateur l'a dit
        // ([I-02](../../../CLAUDE.md#i-02)).
        let draft = ConnectionDraft {
            driver: "sqlite".into(),
            name: "local".to_owned(),
            environment: Environment::Production,
            values: BTreeMap::new(),
            secrets: BTreeMap::new(),
        };
        let config = config_from_draft(&draft).expect("« sqlite » est un identifiant valide");
        assert_eq!(config.environment, Environment::Production);
    }

    #[test]
    fn une_connexion_enregistree_ne_montre_pas_son_identifiant() {
        let config = ConnectionConfig::new("prod", DriverId::postgres())
            .with_environment(Environment::Production)
            .with_param("host", "db.interne");

        let vue = saved_connection(&config);
        let rendu = format!("{vue:?}");
        assert!(
            !rendu.contains(&config.id.to_string()),
            "identifiant exposé : {rendu}"
        );
        assert!(!rendu.contains("db.interne"), "paramètre exposé : {rendu}");
    }
}
