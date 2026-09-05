//! Ce qui peut mal tourner dans la couche données.
//!
//! Une énumération par frontière ([`rust.md`](../../../.claude/rules/rust.md)) :
//! celle-ci est la frontière entre un flux de `RecordBatch` et le reste du
//! workspace. Elle se convertit en [`OxynError`] pour remonter, mais l'appelant
//! immédiat — `oxyn-exec`, `oxyn-driver` — distingue les cas **par variante**,
//! jamais en analysant un message.
//!
//! La distinction qui compte ici est celle entre *le tampon a refusé* et *le
//! disque a refusé* : la première est une décision de politique, la seconde une
//! panne de la machine. Les confondre conduit à retenter un débordement disque
//! sur un système de fichiers plein.

use std::io;

use arrow::error::ArrowError;
use oxyn_core::OxynError;

/// Résultat d'une opération de la couche données.
pub type Result<T> = std::result::Result<T, DataError>;

/// Défaillance d'un tampon de résultats, d'un puits de lots ou d'un export.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DataError {
    /// Un lot ne porte pas le schéma annoncé par le tampon.
    ///
    /// C'est toujours un défaut de driver : le schéma est fixé par
    /// `Cursor::schema()` avant le premier lot et ne change plus. On refuse
    /// plutôt que de laisser la grille lire une colonne qui n'est pas celle
    /// qu'elle dessine.
    #[error("batch schema does not match the result schema: expected {expected}, found {found}")]
    SchemaMismatch {
        /// Schéma du tampon, en notation Arrow.
        expected: String,
        /// Schéma du lot rejeté.
        found: String,
    },

    /// Un lot est arrivé après [`mark_complete`](crate::ResultBuffer::mark_complete).
    ///
    /// Accepter reviendrait à publier des lignes qu'aucun consommateur ne
    /// verra : le compte de lignes affiché est déjà figé.
    #[error("the result is already complete; no further batch can be accepted")]
    AlreadyComplete,

    /// Le tampon a atteint une de ses bornes et refuse d'en accueillir plus.
    ///
    /// Ce n'est **pas** une erreur au sens d'une panne : c'est de la
    /// contre-pression. `BatchSink` la traite en tronquant proprement le
    /// résultat, marqué comme tel dans [`ExecStats`](oxyn_core::ExecStats).
    #[error("the result buffer is full: {reason}")]
    Full {
        /// Quelle borne a été atteinte.
        reason: &'static str,
    },

    /// L'écriture ou la relecture du fichier de débordement a échoué.
    ///
    /// Distincte d'[`Io`](Self::Io) : ici, le résultat déjà accumulé reste
    /// lisible, seule l'extension a échoué.
    #[error("spill file failure: {0}")]
    Spill(#[source] io::Error),

    /// Arrow a refusé l'encodage ou le décodage.
    #[error("arrow failure: {0}")]
    Arrow(#[from] ArrowError),

    /// Entrée-sortie locale : fichier d'export, fichier temporaire.
    #[error("i/o failure: {0}")]
    Io(#[from] io::Error),

    /// Le format d'export demandé n'est pas encore implémenté.
    ///
    /// « Ne pas savoir faire est une réponse acceptable ; laisser croire ne
    /// l'est pas » — un export silencieusement dégradé produit un fichier que
    /// l'utilisateur croit fidèle.
    #[error("export format `{format}` is not supported yet")]
    UnsupportedFormat {
        /// Nom du format, tel qu'il apparaît dans
        /// [`ExportFormat`](oxyn_core::ExportFormat).
        format: &'static str,
    },

    /// Un export a été demandé sur un résultat encore en cours de réception.
    ///
    /// Écrire un fichier partiel qui ressemble à un fichier complet est une
    /// perte de données silencieuse. L'appelant qui l'accepte le déclare via
    /// [`ExportOptions::allow_incomplete`](crate::ExportOptions::allow_incomplete).
    #[error("the result is still streaming; exporting it now would truncate it silently")]
    IncompleteResult,

    /// L'opération a été interrompue par un
    /// [`CancelToken`](oxyn_core::CancelToken).
    #[error("cancelled")]
    Cancelled,
}

impl DataError {
    /// L'opération peut-elle être retentée telle quelle ?
    ///
    /// Conservateur par construction ([I-13](../../../CLAUDE.md#i-13)) : dans le
    /// doute, `false`. Seule une panne d'écriture du fichier de débordement est
    /// déclarée rejouable, parce qu'elle ne touche pas les données déjà reçues.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(self, Self::Spill(_))
    }

    /// L'erreur vient-elle d'une annulation demandée par l'utilisateur ?
    #[must_use]
    pub const fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}

impl From<DataError> for OxynError {
    fn from(erreur: DataError) -> Self {
        match erreur {
            DataError::Cancelled => Self::Cancelled,
            DataError::Io(source) => Self::Io(source),
            DataError::Spill(source) => Self::Io(source),
            DataError::UnsupportedFormat { format } => Self::NotSupported {
                capability: format!("export:{format}"),
            },
            // `Arrow` et `SchemaMismatch` sont des défauts d'encodage : les
            // ranger dans `Internal` masquerait qu'ils viennent d'une donnée
            // reçue, et donc qu'ils peuvent se reproduire à la prochaine
            // exécution de la même requête.
            autre @ (DataError::Arrow(_) | DataError::SchemaMismatch { .. }) => {
                Self::Serialization(autre.to_string())
            }
            autre => Self::Internal(autre.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn une_annulation_reste_une_annulation_apres_conversion() {
        let erreur: OxynError = DataError::Cancelled.into();
        assert!(erreur.is_cancelled());
    }

    #[test]
    fn un_format_absent_devient_une_capacite_absente() {
        let erreur: OxynError = DataError::UnsupportedFormat { format: "parquet" }.into();
        match erreur {
            OxynError::NotSupported { capability } => assert_eq!(capability, "export:parquet"),
            autre => panic!("variante inattendue : {autre:?}"),
        }
    }

    /// Un défaut de schéma ne doit pas être classé rejouable : rejouer la même
    /// requête produira le même schéma incohérent.
    #[test]
    fn un_defaut_de_schema_n_est_pas_rejouable() {
        let erreur = DataError::SchemaMismatch {
            expected: "a: Int32".into(),
            found: "a: Utf8".into(),
        };
        assert!(!erreur.is_retryable());
    }
}
