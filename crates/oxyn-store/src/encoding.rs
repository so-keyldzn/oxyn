//! Représentation des types du domaine dans les colonnes SQLite.
//!
//! Un seul endroit décide comment un identifiant, une intention ou un risque
//! s'écrit dans une colonne. Disperser ces conversions dans les six modules de
//! tables garantirait qu'un jour deux d'entre eux ne s'accordent plus, et
//! l'endroit où cela se verrait est la piste d'audit.
//!
//! # Deux encodages, et pourquoi il y en a deux
//!
//! * **Nom stable** pour les énumérations **fermées** du domaine
//!   ([`StatementIntent`], [`Environment`]) : elles portent déjà un `as_str()`
//!   qui fait partie de leur contrat, et une valeur de plus y est une rupture
//!   visible à la compilation.
//! * **Étiquette JSON** (via `serde`) pour les énumérations `#[non_exhaustive]`
//!   (`MutationRisk`, `QueryLanguage`). Un `match` exhaustif y est
//!   impossible, et une branche `_ => "unknown"` étiquetterait silencieusement
//!   de la même façon deux risques différents — dans le journal d'audit,
//!   précisément. `serde` garde le nom aligné sur le type sans intervention.
//!
//! # Relire est plus permissif qu'écrire
//!
//! Une valeur inattendue **en lecture** ne fait pas échouer la lecture : elle
//! retombe sur la valeur la plus contraignante et émet un `warn`. Un journal
//! d'audit qu'on ne peut plus ouvrir parce qu'une ligne est étrange ne protège
//! personne, et « le plus contraignant » est le défaut de tout `oxyn-core` :
//! [`StatementIntent::Unknown`] compte pour mutant,
//! [`Environment::Production`] est le défaut.

use std::str::FromStr;
use std::time::Duration;

use oxyn_core::{Environment, ErrorClass, IdParseError, StatementIntent};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{Result, StoreError};

/// Relit un identifiant UUID écrit en TEXT.
///
/// # Erreurs
/// [`StoreError::Corrupted`] si la colonne ne contient pas un UUID. Le message
/// nomme la colonne, jamais la valeur (I-03).
pub(crate) fn parse_id<T>(raw: &str, field: &'static str) -> Result<T>
where
    T: FromStr<Err = IdParseError>,
{
    T::from_str(raw).map_err(|err| StoreError::Corrupted {
        field,
        detail: err.detail().to_owned(),
    })
}

/// Variante optionnelle de [`parse_id`] : une colonne `NULL` rend `None`.
///
/// # Erreurs
/// Voir [`parse_id`].
pub(crate) fn parse_id_opt<T>(raw: Option<String>, field: &'static str) -> Result<Option<T>>
where
    T: FromStr<Err = IdParseError>,
{
    raw.as_deref().map(|s| parse_id(s, field)).transpose()
}

/// Encode une valeur `serde` en étiquette JSON destinée à une colonne TEXT.
///
/// Pour une énumération à variantes unitaires, cela produit `"truncate"` —
/// guillemets compris. C'est volontaire : la colonne reste lisible par un
/// humain **et** relisible sans ambiguïté par `serde` (I-11).
///
/// # Erreurs
/// [`StoreError::Json`] si la valeur n'est pas sérialisable.
pub(crate) fn tag_to_json<T: Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

/// Relit une étiquette écrite par [`tag_to_json`].
///
/// # Erreurs
/// [`StoreError::Corrupted`] si l'étiquette ne correspond à aucune variante
/// connue — cas d'un état local écrit par une version plus récente.
pub(crate) fn tag_from_json<T: DeserializeOwned>(raw: &str, field: &'static str) -> Result<T> {
    serde_json::from_str(raw).map_err(|err| StoreError::Corrupted {
        field,
        detail: err.to_string(),
    })
}

/// Relit une intention d'instruction.
///
/// Une étiquette inconnue rend [`StatementIntent::Unknown`], qui compte pour
/// **mutant** : c'est la seule retombée qui ne fait pas passer une écriture
/// pour une lecture dans la piste d'audit.
pub(crate) fn intent_from_text(raw: &str) -> StatementIntent {
    match raw {
        "read" => StatementIntent::Read,
        "write" => StatementIntent::Write,
        "ddl" => StatementIntent::Ddl,
        "grant" => StatementIntent::Grant,
        "unknown" => StatementIntent::Unknown,
        _ => {
            tracing::warn!(
                column = "intent",
                "unknown statement intent in local state, falling back to `unknown`"
            );
            StatementIntent::Unknown
        }
    }
}

/// Relit une famille d'erreur.
///
/// Une valeur inconnue rend [`ErrorClass::Ambiguous`], et c'est le sens de
/// I-13 appliqué jusqu'à la relecture : une erreur dont on ne sait plus dire
/// si le serveur a appliqué l'écriture ne se retente pas.
pub(crate) fn error_class_from_text(raw: &str) -> ErrorClass {
    match raw {
        "transitoire" => ErrorClass::Transient,
        "permanente" => ErrorClass::Permanent,
        "ambiguë" => ErrorClass::Ambiguous,
        _ => {
            tracing::warn!(
                column = "error_class",
                "unknown error class in local state, falling back to `ambiguous`"
            );
            ErrorClass::Ambiguous
        }
    }
}

/// Relit un marquage d'environnement.
///
/// Une étiquette inconnue rend [`Environment::Production`]. C'est la règle de
/// SECURITY appliquée jusque dans la relecture : un environnement qu'on ne sait
/// pas lire n'est pas un environnement de développement.
pub(crate) fn environment_from_text(raw: &str) -> Environment {
    Environment::from_str(raw).unwrap_or_else(|_| {
        tracing::warn!(
            column = "environment",
            "unknown environment in local state, falling back to `production`"
        );
        Environment::Production
    })
}

/// Convertit une durée en millisecondes stockables.
///
/// Sature plutôt que de déborder : une durée absurde dans le journal vaut mieux
/// qu'une panique ou qu'un `as` qui tronquerait en silence.
pub(crate) fn duration_to_ms(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

/// Convertit un compte du domaine (`u64`) vers l'entier signé de SQLite.
///
/// Sature à [`i64::MAX`] : SQLite n'a pas d'entier non signé, et aucun compte
/// de lignes réel n'approche cette borne.
pub(crate) fn count_to_i64(count: u64) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}

/// Convertit un compte relu depuis SQLite vers le domaine.
///
/// Une valeur négative — impossible par construction, donc signe d'une
/// modification hors d'Oxyn — rend `0` plutôt qu'un nombre gigantesque.
pub(crate) fn count_from_i64(count: i64) -> u64 {
    u64::try_from(count).unwrap_or(0)
}

/// Convertit une limite de requête vers l'entier de SQLite.
pub(crate) fn limit_to_i64(limit: usize) -> i64 {
    i64::try_from(limit).unwrap_or(i64::MAX)
}

/// Échappe un motif `LIKE` saisi par l'utilisateur.
///
/// `%`, `_` et `\` y sont des métacaractères. Chercher `100%` dans
/// l'historique doit trouver `100%`, pas toutes les lignes. Le motif est
/// ensuite **lié** comme paramètre, jamais concaténé (I-10) ; la clause doit
/// porter `ESCAPE '\'`.
pub(crate) fn escape_like(needle: &str) -> String {
    let mut out = String::with_capacity(needle.len() + 2);
    for c in needle.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::{ConnectionId, MutationRisk, QueryLanguage, SqlDialect};

    #[test]
    fn un_identifiant_illisible_nomme_la_colonne_pas_la_valeur() {
        let erreur = parse_id::<ConnectionId>("base-de-production-banque", "connection_id")
            .expect_err("ce n'est pas un UUID");
        let message = erreur.to_string();
        assert!(message.contains("connection_id"));
        assert!(!message.contains("banque"), "la valeur a fuité : {message}");
    }

    #[test]
    fn aller_retour_des_etiquettes_non_exhaustives() {
        // MutationRisk et QueryLanguage sont `#[non_exhaustive]` : leur nom
        // stable vient de serde, pas d'un `match` que l'on oublierait d'étendre.
        for risque in [
            MutationRisk::None,
            MutationRisk::UnboundedUpdate,
            MutationRisk::UnboundedDelete,
            MutationRisk::Truncate,
            MutationRisk::DropObject,
        ] {
            let brut = tag_to_json(&risque).expect("sérialisation");
            let relu: MutationRisk = tag_from_json(&brut, "risk").expect("désérialisation");
            assert_eq!(risque, relu);
        }

        let langage = QueryLanguage::Sql(SqlDialect::Postgres);
        let brut = tag_to_json(&langage).expect("sérialisation");
        let relu: QueryLanguage = tag_from_json(&brut, "language").expect("désérialisation");
        assert_eq!(
            langage, relu,
            "le dialecte ne doit pas être perdu par l'encodage"
        );
    }

    #[test]
    fn une_intention_inconnue_compte_pour_mutante() {
        assert_eq!(intent_from_text("read"), StatementIntent::Read);
        assert_eq!(intent_from_text("grant"), StatementIntent::Grant);

        let inconnue = intent_from_text("vaporize");
        assert_eq!(inconnue, StatementIntent::Unknown);
        assert!(
            inconnue.is_mutating(),
            "une intention illisible ne doit jamais passer pour une lecture"
        );
    }

    #[test]
    fn un_environnement_inconnu_vaut_production() {
        assert_eq!(environment_from_text("local"), Environment::Local);
        assert_eq!(environment_from_text("staging"), Environment::Staging);
        assert!(
            environment_from_text("preprod-bis").is_production(),
            "SECURITY : ce qu'on ne sait pas lire est de la production"
        );
    }

    #[test]
    fn les_metacaracteres_like_sont_echappes() {
        assert_eq!(escape_like("100%"), "100\\%");
        assert_eq!(escape_like("a_b"), "a\\_b");
        assert_eq!(escape_like("c:\\tmp"), "c:\\\\tmp");
        assert_eq!(escape_like("SELECT 1"), "SELECT 1");
    }

    #[test]
    fn les_comptes_saturent_au_lieu_de_deborder() {
        assert_eq!(count_to_i64(42), 42);
        assert_eq!(count_to_i64(u64::MAX), i64::MAX);
        assert_eq!(count_from_i64(-1), 0);
        assert_eq!(duration_to_ms(Duration::from_millis(1_500)), 1_500);
        assert_eq!(duration_to_ms(Duration::MAX), i64::MAX);
    }
}
