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
//!
//! Le message du moteur, lui, **cite ce qu'on vient de lier** : un déclencheur
//! `RAISE(ABORT, 'solde : ' || NEW.montant)` ou une contrainte `CHECK` violée
//! reprennent la valeur passée à `sqlite3_bind_*`. Ce message est affiché,
//! journalisé et persisté par l'historique. Il n'est donc propagé que si
//! l'instruction ne portait **aucune** valeur liée par l'appelant : sinon, le
//! code de résultat étendu remplace le texte du moteur.

use oxyn_core::{DriverId, ErrorClass, OxynError, ScalarValue};
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

/// D'où viennent les valeurs liées à l'instruction concernée.
///
/// Le message du moteur peut citer une valeur liée ; il n'est donc propagé que
/// lorsque rien de ce qu'il peut citer ne vient de l'appelant
/// ([I-03](../../../CLAUDE.md#i-03)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Bound {
    /// Aucune valeur venue d'une [`ExecRequest`](oxyn_core::ExecRequest). Une
    /// requête d'introspection lie bien des identifiants — un nom de schéma, un
    /// nom de table —, mais ceux-là viennent du catalogue déjà affiché, pas de
    /// ce que l'utilisateur a saisi. La propriété tenue est donc « rien de ce
    /// que le moteur peut citer ne vient de l'appelant », pas « rien n'était
    /// lié » : l'écrire autrement rendrait l'audit faux au premier lecteur qui
    /// ouvrirait `catalog.rs`.
    Internal,
    /// Au moins une valeur de l'[`ExecRequest`](oxyn_core::ExecRequest) de
    /// l'appelant était liée.
    Caller,
}

impl Bound {
    /// Ce que les paramètres d'une demande impliquent pour le message du moteur.
    pub(crate) const fn of(params: &[ScalarValue]) -> Self {
        if params.is_empty() {
            Self::Internal
        } else {
            Self::Caller
        }
    }
}

/// Le code de résultat d'un échec, seule part d'un message du moteur qui ne
/// puisse pas citer une valeur.
///
/// `ffi::Error` rend « Error code 19: constraint failed » : le libellé est
/// dérivé du **nombre**, pas du texte que SQLite a composé.
fn code_of(code: &Option<rusqlite::ffi::Error>) -> String {
    match code {
        Some(failure) => failure.to_string(),
        // Les variantes de `rusqlite` qui ne viennent pas du moteur (index de
        // colonne, conversion refusée) n'ont pas de code de résultat.
        None => "SQLite driver error".to_owned(),
    }
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
    ///
    /// Le message étendu de SQLite y figure : il n'est produit que pour une
    /// instruction sans valeur liée par l'appelant, où il ne peut citer que le
    /// SQL soumis. Sinon, c'est [`Withheld`](Self::Withheld).
    /// **Pas de `#[from]`** : un `?` sur un `rusqlite::Error` construirait cette
    /// variante non rédigée sans jamais consulter l'origine des valeurs. Le
    /// premier `?` ajouté dans `params.rs` — la fonction qui lie les valeurs —
    /// sauterait ainsi la rédaction sans qu'aucun relecteur ne le voie. La
    /// construction passe donc par les deux fonctions de traduction du module,
    /// où le choix se pose à l'écriture.
    #[error("{0}")]
    Engine(rusqlite::Error),

    /// Le moteur a refusé une instruction qui portait des valeurs liées par
    /// l'appelant : son message est retenu.
    ///
    /// Retenu et non filtré : SQLite peut citer une valeur liée tronquée,
    /// échappée ou transformée — un déclencheur `RAISE(ABORT, …)` la concatène,
    /// une contrainte `CHECK` la reprend —, et chercher le texte de la valeur
    /// dans le message aurait l'apparence d'une protection sans en être une
    /// ([I-03](../../../CLAUDE.md#i-03)). L'erreur d'origine n'est pas non plus
    /// conservée dans la variante : `#[derive(Debug)]` la ré-exposerait au
    /// premier `tracing::debug!` venu.
    ///
    /// Ne survit que le code de résultat, qui est un nombre.
    #[error(
        "{}: the SQLite message is withheld because the statement carried \
         bound values",
        code_of(.code)
    )]
    Withheld {
        /// Le code de résultat étendu, quand l'échec vient du moteur.
        code: Option<rusqlite::ffi::Error>,
    },

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

/// Traduit une erreur du moteur pour une instruction **composée par le driver**.
///
/// Le message du moteur est propagé tel quel : une telle instruction ne lie que
/// des littéraux que le driver a écrits, donc rien de ce que le moteur peut
/// citer ne vient de l'appelant. Pour l'exécution d'une
/// [`ExecRequest`](oxyn_core::ExecRequest), c'est [`engine_bound`] qu'il faut
/// appeler : là, le moteur cite des valeurs liées
/// ([I-03](../../../CLAUDE.md#i-03)).
pub(crate) fn engine(error: rusqlite::Error, effect: Effect) -> OxynError {
    engine_bound(error, effect, Bound::Internal)
}

/// Traduit une erreur du moteur dans le vocabulaire du domaine.
///
/// `effect` dit ce que l'instruction pouvait faire ; il ne sert qu'au cas de
/// l'interruption, voir la documentation du module. `bound` décide du sort du
/// message : retenu dès qu'une valeur de l'appelant était liée.
///
/// La famille de l'erreur, elle, ne dépend **pas** de `bound` : elle se lit sur
/// le code de résultat, relevé avant que l'erreur d'origine soit abandonnée.
pub(crate) fn engine_bound(error: rusqlite::Error, effect: Effect, bound: Bound) -> OxynError {
    let code = match &error {
        rusqlite::Error::SqliteFailure(inner, _) => Some(*inner),
        _ => None,
    };
    if code.is_some_and(|inner| inner.code == ErrorCode::OperationInterrupted) {
        return match effect {
            Effect::ReadOnly => OxynError::Cancelled,
            Effect::Mutating => driver(hide(error, code, bound), ErrorClass::Ambiguous),
        };
    }
    if matches!(error, rusqlite::Error::InvalidPath(_)) {
        // Le `Display` de cette variante contient le chemin (I-03).
        return driver(SqliteError::Path, ErrorClass::Permanent);
    }
    let class = classify(&error);
    driver(hide(error, code, bound), class)
}

/// L'erreur que le driver rend : celle du moteur, ou son code seul quand le
/// message pourrait citer une valeur de l'appelant.
///
/// Seul le texte venu de `sqlite3_errmsg` peut reprendre ce qui vient d'être
/// lié — c'est lui que porte `SqliteFailure(_, Some(_))`. Les autres variantes
/// de `rusqlite` sont composées par la bibliothèque à partir d'indices et de
/// noms de types : les retenir ferait disparaître le diagnostic d'un **bug du
/// driver** sans rien protéger, et personne ne saurait le reproduire. Le driver
/// PostgreSQL fait la même distinction pour la même raison.
pub(crate) fn hide(
    error: rusqlite::Error,
    code: Option<rusqlite::ffi::Error>,
    bound: Bound,
) -> SqliteError {
    match bound {
        Bound::Caller if matches!(error, rusqlite::Error::SqliteFailure(_, Some(_))) => {
            SqliteError::Withheld { code }
        }
        Bound::Internal | Bound::Caller => SqliteError::Engine(error),
    }
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

    /// Un échec du moteur dont le message cite une valeur liée, comme le fait
    /// `RAISE(ABORT, 'solde : ' || NEW.montant)`.
    fn echec_bavard() -> rusqlite::Error {
        rusqlite::Error::SqliteFailure(
            ffi::Error {
                code: ErrorCode::ConstraintViolation,
                // `SQLITE_CONSTRAINT_TRIGGER`.
                extended_code: 1_811,
            },
            Some("solde : S3NT1NELLE-42".to_owned()),
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
    fn le_message_du_moteur_est_retenu_des_qu_une_valeur_etait_liee() {
        // I-03 : ce message est affiché, journalisé et persisté par
        // l'historique. SQLite y recopie ce qu'on vient de lier.
        let err = engine_bound(echec_bavard(), Effect::Mutating, Bound::Caller);
        for rendu in [format!("{err}"), format!("{err:?}")] {
            assert!(!rendu.contains("S3NT1NELLE-42"), "valeur liée : {rendu}");
            assert!(!rendu.contains("solde"), "message du moteur : {rendu}");
        }
        // Le retrait se dit, plutôt que de laisser croire à une erreur muette.
        assert!(err.to_string().contains("withheld"), "{err}");
        // Le code de résultat étendu survit : c'est un nombre, il ne cite rien.
        assert!(err.to_string().contains("1811"), "{err}");
    }

    #[test]
    fn sans_valeur_liee_le_message_du_moteur_passe_inchange() {
        // Le public d'Oxyn lit les messages de son moteur ; une paraphrase
        // rassurante serait un défaut.
        let err = engine_bound(echec_bavard(), Effect::Mutating, Bound::Internal);
        assert!(err.to_string().contains("solde : S3NT1NELLE-42"), "{err}");
        assert_eq!(
            err.to_string(),
            engine(echec_bavard(), Effect::Mutating).to_string(),
            "`engine` est le cas sans valeur liée"
        );
    }

    #[test]
    fn le_retrait_du_message_ne_change_ni_la_famille_ni_l_annulation() {
        // La classe se lit sur le code de résultat, relevé avant d'abandonner
        // l'erreur d'origine : la retenir ne doit rien déplacer.
        for (code, attendue) in [
            (ErrorCode::DatabaseBusy, ErrorClass::Transient),
            (ErrorCode::ConstraintViolation, ErrorClass::Permanent),
        ] {
            let err = engine_bound(echec(code), Effect::Mutating, Bound::Caller);
            assert_eq!(err.class(), attendue, "{err:?}");
        }

        let lecture = engine_bound(
            echec(ErrorCode::OperationInterrupted),
            Effect::ReadOnly,
            Bound::Caller,
        );
        assert!(lecture.is_cancelled(), "{lecture:?}");

        // I-13 : une écriture interrompue reste ambiguë, donc non rejouable.
        let ecriture = engine_bound(
            echec(ErrorCode::OperationInterrupted),
            Effect::Mutating,
            Bound::Caller,
        );
        assert_eq!(ecriture.class(), ErrorClass::Ambiguous, "{ecriture:?}");
        assert!(!ecriture.is_retryable());
        assert!(!format!("{ecriture:?}").contains("message du moteur"));
    }

    #[test]
    fn une_demande_sans_parametre_ne_retient_rien() {
        assert_eq!(Bound::of(&[]), Bound::Internal);
        assert_eq!(
            Bound::of(&[ScalarValue::Text("S3NT1NELLE-42".to_owned())]),
            Bound::Caller
        );
    }

    #[test]
    fn une_session_fermee_est_une_erreur_permanente() {
        let err = closed();
        assert_eq!(err.class(), ErrorClass::Permanent);
        assert!(!err.is_retryable());
    }

    /// Retenir le message d'un défaut du driver ne protège rien et efface le
    /// diagnostic : `InvalidColumnIndex` est composé par `rusqlite` à partir
    /// d'un index, il ne peut citer aucune valeur liée.
    #[test]
    fn seul_le_texte_du_moteur_est_retenu_quand_des_valeurs_sont_liees() {
        let usage = hide(rusqlite::Error::InvalidColumnIndex(3), None, Bound::Caller);
        assert!(
            matches!(usage, SqliteError::Engine(_)),
            "une erreur d'usage garde son message : {usage}"
        );
        assert!(usage.to_string().contains('3'));

        let moteur = hide(
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(19),
                Some("CHECK constraint failed: S3NT1NELLE-42".to_owned()),
            ),
            Some(rusqlite::ffi::Error::new(19)),
            Bound::Caller,
        );
        let rendu = format!("{moteur} {moteur:?}");
        assert!(
            !rendu.contains("S3NT1NELLE"),
            "le texte du moteur ne sort pas : {rendu}"
        );
        assert!(rendu.contains("19"), "le code de résultat survit : {rendu}");
    }
}
