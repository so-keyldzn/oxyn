//! Les erreurs du driver, et la **famille** à laquelle il les rattache.
//!
//! [`DRIVER-CONTRACT` §4](../../../docs/DRIVER-CONTRACT.md) exige qu'un driver
//! classe ses erreurs : transitoire, permanente, ambiguë. La classe est une
//! **donnée** portée par l'erreur, jamais une déduction faite par l'appelant à
//! partir du message.
//!
//! # Le cas qui coûte le plus cher
//!
//! `sqlite3_interrupt` pendant une écriture. SQLite garantit l'atomicité au
//! niveau de l'instruction, mais pas la restauration d'une transaction
//! explicite : une instruction interrompue **hors** transaction est annulée, une
//! transaction interrompue peut laisser les instructions déjà validées en place.
//! Une interruption est donc rendue :
//!
//! * [`OxynError::Cancelled`] si l'instruction ne pouvait rien modifier
//!   (`sqlite3_stmt_readonly`) — l'effet est connu : aucun ;
//! * [`ErrorClass::Ambiguous`] sinon. L'ambiguïté ne se retente **jamais**
//!   ([I-13](../../../CLAUDE.md#i-13)).
//!
//! # Ce qui ne sort jamais d'ici
//!
//! Aucun message ne reprend une **valeur liée** ni le **chemin du fichier de
//! base** : le chemin est une valeur de paramètre de connexion, et un driver
//! n'a pas le droit de la journaliser ([I-03](../../../CLAUDE.md#i-03)). C'est
//! la raison d'être de [`SqliteError::Path`], qui remplace
//! `rusqlite::Error::InvalidPath` — la seule variante de `rusqlite` dont le
//! `Display` contient le chemin.

use oxyn_core::{DriverId, ErrorClass, OxynError};
use rusqlite::ErrorCode;

/// Ce que l'instruction concernée pouvait faire à la base.
///
/// Sert uniquement à décider si une interruption est une annulation propre ou
/// une ambiguïté. La valeur vient de `sqlite3_stmt_readonly`, pas d'une analyse
/// du texte : c'est le moteur qui répond, pas nous.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Effect {
    /// L'instruction ne pouvait rien modifier.
    ReadOnly,
    /// L'instruction pouvait écrire.
    Mutating,
}

/// Une erreur propre au driver SQLite.
///
/// Elle circule comme `source` d'[`OxynError::Driver`], qui porte la famille.
/// Elle est publique parce qu'un appelant peut vouloir la retrouver par
/// `downcast_ref` — pour distinguer un conflit de type d'une erreur du moteur,
/// par exemple.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SqliteError {
    /// Erreur rendue par le moteur SQLite lui-même.
    #[error("{0}")]
    Engine(#[from] rusqlite::Error),

    /// Le chemin du fichier de base est inutilisable.
    ///
    /// Le chemin **n'est pas** repris dans le message : c'est une valeur de
    /// paramètre de connexion (I-03).
    #[error("the database path is not usable")]
    Path,

    /// Le thread qui détient la connexion n'est plus là : la session est
    /// fermée, ou son thread a terminé.
    #[error("the connection thread is gone: the session is closed")]
    Closed,

    /// Une valeur n'a aucune représentation sans perte dans le type Arrow
    /// retenu pour sa colonne.
    ///
    /// Le nom de la colonne n'est pas repris — il vient du serveur, et un
    /// message d'erreur finit dans un journal. L'index suffit à retrouver le
    /// champ dans le schéma du curseur.
    #[error(
        "column #{column} was typed as `{resolved}` from the first batch, \
         but a `{found}` value appeared further in the stream and cannot be \
         rendered there without loss"
    )]
    ColumnConflict {
        /// Index de la colonne dans le schéma, à partir de zéro.
        column: usize,
        /// Type Arrow retenu pour la colonne.
        resolved: &'static str,
        /// Classe de stockage rencontrée.
        found: &'static str,
    },

    /// Un paramètre lié n'a pas de classe de stockage SQLite.
    #[error("bound parameter #{index} is a `{type_name}`, which SQLite cannot store")]
    Parameter {
        /// Position du paramètre, à partir de 1.
        index: usize,
        /// Nom du type scalaire, tel que `ScalarValue::type_name` le donne.
        type_name: &'static str,
    },

    /// Le nombre de paramètres liés ne correspond pas à l'instruction.
    #[error("the statement expects {expected} bound parameter(s), {given} given")]
    ParameterCount {
        /// Ce que l'instruction attend.
        expected: usize,
        /// Ce qui a été fourni.
        given: usize,
    },

    /// Des paramètres liés accompagnent un lot de plusieurs instructions.
    ///
    /// Rien ne dit à quelle instruction ils se rapportent : le découpage
    /// appartient à `oxyn-query`, et le driver refuse plutôt que de deviner.
    #[error("bound parameters cannot be used with a multi-statement batch: split it first")]
    ParametersWithBatch,

    /// La construction du `RecordBatch` a échoué.
    #[error("arrow: {0}")]
    Arrow(#[from] arrow::error::ArrowError),
}

/// La famille d'une erreur du moteur.
///
/// Seules quatre situations sont **transitoires** : le verrou de fichier tenu
/// par un autre processus (`SQLITE_BUSY`), le verrou de table tenu par une autre
/// connexion (`SQLITE_LOCKED`), l'échec de protocole de verrouillage, et le
/// changement de schéma sous les pieds d'une instruction préparée. Tout le reste
/// — syntaxe, contrainte violée, base illisible, disque plein — ne s'améliore
/// pas en réessayant.
///
/// `SQLITE_INTERRUPT` n'apparaît pas ici : il est traité en amont par
/// `engine` — nommée et non liée : la fonction est interne à la crate —,
/// parce que sa famille dépend de ce que faisait l'instruction.
#[must_use]
pub fn classify(error: &rusqlite::Error) -> ErrorClass {
    let rusqlite::Error::SqliteFailure(inner, _) = error else {
        // Les autres variantes sont des erreurs d'usage de la bibliothèque
        // (mauvais index de colonne, conversion refusée) : rejouer ne les
        // corrige pas.
        return ErrorClass::Permanent;
    };
    match inner.code {
        ErrorCode::DatabaseBusy
        | ErrorCode::DatabaseLocked
        | ErrorCode::FileLockingProtocolFailed
        | ErrorCode::SchemaChanged => ErrorClass::Transient,
        _ => ErrorClass::Permanent,
    }
}

/// Traduit une erreur du moteur dans le vocabulaire du domaine.
///
/// `effect` dit ce que l'instruction pouvait faire ; il ne sert qu'au cas de
/// l'interruption, voir la documentation du module.
pub(crate) fn engine(error: rusqlite::Error, effect: Effect) -> OxynError {
    let interrupted = matches!(
        &error,
        rusqlite::Error::SqliteFailure(inner, _) if inner.code == ErrorCode::OperationInterrupted
    );
    if interrupted {
        return match effect {
            Effect::ReadOnly => OxynError::Cancelled,
            Effect::Mutating => driver(SqliteError::Engine(error), ErrorClass::Ambiguous),
        };
    }
    if matches!(error, rusqlite::Error::InvalidPath(_)) {
        // Le `Display` de cette variante contient le chemin (I-03).
        return driver(SqliteError::Path, ErrorClass::Permanent);
    }
    let class = classify(&error);
    driver(SqliteError::Engine(error), class)
}

/// Emballe une erreur du driver, en nommant sa famille.
pub(crate) fn driver(error: SqliteError, class: ErrorClass) -> OxynError {
    OxynError::driver(DriverId::sqlite(), class, error)
}

/// L'erreur d'une session dont le thread porteur a disparu.
pub(crate) fn closed() -> OxynError {
    driver(SqliteError::Closed, ErrorClass::Permanent)
}

/// L'erreur d'ouverture d'une base, classée comme une erreur de connexion.
///
/// Rendue par [`Driver::connect`](oxyn_driver::Driver::connect), qui a sa propre
/// variante : l'appelant distingue « le serveur est injoignable » de « le
/// serveur a rejeté l'instruction » sans lire de message. Le chemin n'y figure
/// jamais.
pub(crate) fn open(error: rusqlite::Error) -> OxynError {
    OxynError::Connection(match &error {
        rusqlite::Error::InvalidPath(_) => "the database path is not usable".to_owned(),
        // Le message détaillé de SQLite embarque le chemin du fichier
        // (« unable to open database file: /home/… »). Un chemin est un
        // paramètre de connexion : il n'a pas sa place dans un message qui peut
        // finir dans un journal, un rapport de plantage ou une invite IA
        // ([I-03](../../../CLAUDE.md#i-03)). Le `ffi::Error` seul rend
        // « Error code 14: unable to open database file » — le code étendu et
        // son libellé canonique, sans rien de l'installation de l'utilisateur.
        // Le code reste une donnée exploitable, pas une déduction à faire sur le
        // texte ([rust.md](../../../.claude/rules/rust.md)).
        rusqlite::Error::SqliteFailure(code, _) => code.to_string(),
        other => other.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rusqlite::ffi;

    use super::*;

    fn echec(code: ErrorCode) -> rusqlite::Error {
        rusqlite::Error::SqliteFailure(
            ffi::Error {
                code,
                extended_code: 0,
            },
            Some("message du moteur".to_owned()),
        )
    }

    #[test]
    fn un_verrou_est_transitoire_une_syntaxe_ne_l_est_pas() {
        assert_eq!(
            classify(&echec(ErrorCode::DatabaseBusy)),
            ErrorClass::Transient
        );
        assert_eq!(
            classify(&echec(ErrorCode::DatabaseLocked)),
            ErrorClass::Transient
        );
        assert_eq!(classify(&echec(ErrorCode::Unknown)), ErrorClass::Permanent);
        assert_eq!(
            classify(&echec(ErrorCode::ConstraintViolation)),
            ErrorClass::Permanent
        );
        assert_eq!(
            classify(&echec(ErrorCode::DatabaseCorrupt)),
            ErrorClass::Permanent,
            "une base corrompue ne se répare pas en réessayant"
        );
    }

    #[test]
    fn une_lecture_interrompue_est_une_annulation() {
        let err = engine(echec(ErrorCode::OperationInterrupted), Effect::ReadOnly);
        assert!(err.is_cancelled(), "{err:?}");
        assert!(!err.is_retryable());
    }

    #[test]
    fn une_ecriture_interrompue_est_ambigue_et_ne_se_retente_pas() {
        // I-13 : le serveur a peut-être appliqué. Rejouer crée un doublon
        // silencieux dans les données de l'utilisateur.
        let err = engine(echec(ErrorCode::OperationInterrupted), Effect::Mutating);
        assert_eq!(err.class(), ErrorClass::Ambiguous, "{err:?}");
        assert!(!err.is_retryable());
        assert!(
            !err.is_cancelled(),
            "l'effet n'est pas connu : ce n'est pas une annulation propre"
        );
    }

    #[test]
    fn un_chemin_fautif_ne_ressort_jamais_dans_le_message() {
        // I-03 : le chemin du fichier est une valeur de paramètre de connexion.
        let secret = PathBuf::from("/Users/quelqu-un/bases/clients-2026.sqlite");
        let err = engine(
            rusqlite::Error::InvalidPath(secret.clone()),
            Effect::ReadOnly,
        );
        let rendu = format!("{err}");
        assert!(!rendu.contains("clients-2026"), "chemin fuité : {rendu}");
        assert!(!rendu.contains("quelqu-un"), "chemin fuité : {rendu}");

        let rendu = format!("{}", open(rusqlite::Error::InvalidPath(secret)));
        assert!(!rendu.contains("clients-2026"), "chemin fuité : {rendu}");
    }

    #[test]
    fn une_erreur_du_moteur_porte_le_nom_du_driver() {
        let err = engine(echec(ErrorCode::Unknown), Effect::ReadOnly);
        assert!(err.to_string().contains("sqlite"), "{err}");
        assert!(err.to_string().contains("message du moteur"), "{err}");
    }

    #[test]
    fn un_conflit_de_colonne_ne_nomme_pas_la_colonne() {
        // Un nom de colonne vient du serveur : il peut porter une séquence
        // d'échappement de terminal, et un message d'erreur finit dans un
        // journal.
        let err = SqliteError::ColumnConflict {
            column: 3,
            resolved: "int64",
            found: "blob",
        };
        let rendu = err.to_string();
        assert!(rendu.contains("#3"), "{rendu}");
        assert!(rendu.contains("int64") && rendu.contains("blob"), "{rendu}");
    }

    #[test]
    fn une_session_fermee_est_une_erreur_permanente() {
        let err = closed();
        assert_eq!(err.class(), ErrorClass::Permanent);
        assert!(!err.is_retryable());
    }
}
