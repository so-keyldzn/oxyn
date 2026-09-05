//! Classer une erreur `sqlx`, et surtout : ne pas la classer transitoire quand
//! elle est ambiguë.
//!
//! [DRIVER-CONTRACT §4](../../../docs/DRIVER-CONTRACT.md) donne trois familles.
//! La seule qui coûte cher est l'**ambiguë** : une coupure pendant un `INSERT`
//! ne dit pas si le serveur a appliqué l'écriture. Classée transitoire, elle est
//! rejouée par un appelant consciencieux, et crée un doublon que personne ne
//! verra jamais dans un message d'erreur ([I-13](../../../CLAUDE.md#i-13)).
//!
//! **C'est pourquoi la classification prend l'intention en paramètre.** La même
//! coupure réseau est transitoire pendant un `SELECT` — rien n'a pu changer — et
//! ambiguë pendant un `UPDATE`. Une classification qui ignorerait l'intention
//! devrait choisir, et choisir « transitoire » est le mauvais côté.
//!
//! # Ce que ce module ne fait pas
//!
//! Il ne retente rien. La politique de reprise appartient à l'appelant, seul à
//! savoir si l'opération est rejouable
//! ([DRIVER-CONTRACT](../../../docs/DRIVER-CONTRACT.md)).

use oxyn_core::{DriverId, ErrorClass, OxynError, StatementIntent};
use sqlx::error::DatabaseError;

/// `query_canceled` : le serveur confirme l'annulation qu'on lui a demandée.
const SQLSTATE_QUERY_CANCELED: &str = "57014";
/// `admin_shutdown` : la connexion a été coupée par l'administrateur.
const SQLSTATE_ADMIN_SHUTDOWN: &str = "57P01";
/// `crash_shutdown`.
const SQLSTATE_CRASH_SHUTDOWN: &str = "57P02";
/// `cannot_connect_now` : le serveur démarre encore.
const SQLSTATE_CANNOT_CONNECT_NOW: &str = "57P03";
/// `idle_session_timeout` / `idle_in_transaction_session_timeout`.
const SQLSTATE_IDLE_TIMEOUT: &str = "57P05";
/// `serialization_failure`.
const SQLSTATE_SERIALIZATION_FAILURE: &str = "40001";
/// `deadlock_detected`.
const SQLSTATE_DEADLOCK: &str = "40P01";
/// `lock_not_available`.
const SQLSTATE_LOCK_NOT_AVAILABLE: &str = "55P03";
/// `too_many_connections`.
const SQLSTATE_TOO_MANY_CONNECTIONS: &str = "53300";
/// `configuration_limit_exceeded`.
const SQLSTATE_CONFIG_LIMIT: &str = "53400";
/// `out_of_memory` côté serveur.
const SQLSTATE_OUT_OF_MEMORY: &str = "53200";
/// `disk_full` côté serveur.
const SQLSTATE_DISK_FULL: &str = "53100";
/// Classe `08` : `connection_exception`.
const SQLSTATE_CLASS_CONNECTION: &str = "08";
/// Classe `28` : `invalid_authorization_specification`.
const SQLSTATE_CLASS_AUTHORIZATION: &str = "28";
/// Classe `53` : `insufficient_resources`.
const SQLSTATE_CLASS_RESOURCES: &str = "53";
/// `read_only_sql_transaction` : la transaction en lecture seule a fait son
/// travail.
const SQLSTATE_READ_ONLY_TRANSACTION: &str = "25006";

/// L'erreur de driver telle qu'elle est emballée dans [`OxynError::Driver`].
///
/// Un type nommé plutôt qu'un `Box<dyn Error>` anonyme : l'appelant qui veut le
/// SQLSTATE peut le lire par `downcast_ref`, sans analyser un message.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct PostgresError {
    /// Le message du serveur, ou celui du transport. Ne porte jamais de valeur
    /// liée : `sqlx` ne les recopie pas dans ses messages, et le serveur ne
    /// renvoie que ce qu'il a reçu dans le texte de la requête.
    message: String,
    /// Le SQLSTATE, quand l'erreur vient du serveur.
    sqlstate: Option<String>,
}

impl PostgresError {
    /// Le SQLSTATE à cinq caractères, quand le serveur en a donné un.
    #[must_use]
    pub fn sqlstate(&self) -> Option<&str> {
        self.sqlstate.as_deref()
    }

    /// Le message tel que le serveur l'a formulé.
    ///
    /// « Le public lit les messages d'erreur de PostgreSQL » : on ne les
    /// paraphrase pas.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Traduit une erreur `sqlx` survenue **pendant une exécution**.
///
/// `intent` décide du sort des erreurs de transport : ambiguës si l'instruction
/// pouvait écrire, transitoires sinon. Dans le doute — [`StatementIntent::Unknown`]
/// — l'instruction compte pour mutante, comme partout ailleurs.
#[must_use]
pub(crate) fn map_exec_error(
    driver: &DriverId,
    intent: StatementIntent,
    err: sqlx::Error,
) -> OxynError {
    // Une annulation confirmée par le serveur n'est pas une panne : c'est le
    // bouton « Annuler » qui a fonctionné.
    if sqlstate_of(&err).as_deref() == Some(SQLSTATE_QUERY_CANCELED) {
        return OxynError::Cancelled;
    }
    if let Some(erreur) = as_config_error(&err) {
        return erreur;
    }
    let classe = classify(&err, intent);
    OxynError::driver(driver.clone(), classe, wrap(err))
}

/// Traduit une erreur `sqlx` survenue **pendant l'ouverture d'une session**.
///
/// Aucune écriture n'a pu avoir lieu : rien n'est ambigu ici. En revanche, un
/// refus d'identifiants doit se distinguer d'un serveur injoignable — l'un se
/// corrige dans le formulaire, l'autre pas.
#[must_use]
pub(crate) fn map_connect_error(err: &sqlx::Error) -> OxynError {
    if let Some(erreur) = as_config_error(err) {
        return erreur;
    }
    match sqlstate_of(err) {
        Some(code) if code.starts_with(SQLSTATE_CLASS_AUTHORIZATION) => {
            OxynError::Authentication(message_of(err))
        }
        // `invalid_catalog_name` : la base n'existe pas. C'est une erreur de
        // configuration, pas un refus d'identifiants.
        Some(code) if code == "3D000" => OxynError::Config(message_of(err)),
        _ => OxynError::Connection(message_of(err)),
    }
}

/// La famille d'une erreur `sqlx`, au sens de [`ErrorClass`].
#[must_use]
pub(crate) fn classify(err: &sqlx::Error, intent: StatementIntent) -> ErrorClass {
    // Une expiration ou une coupure pendant une instruction qui pouvait écrire
    // laisse l'effet inconnu. C'est le seul endroit du driver où l'intention
    // change une décision.
    let transport = if intent.is_mutating() {
        ErrorClass::Ambiguous
    } else {
        ErrorClass::Transient
    };

    match err {
        // Le serveur a parlé : son SQLSTATE fait foi.
        sqlx::Error::Database(_) => classify_sqlstate(sqlstate_of(err).as_deref(), transport),

        // Le transport a lâché. Ce qui a été envoyé a pu être exécuté.
        sqlx::Error::Io(_) | sqlx::Error::Protocol(_) | sqlx::Error::WorkerCrashed => transport,

        // Rien n'a été envoyé : l'acquisition d'une connexion a échoué avant.
        sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed => ErrorClass::Transient,

        // Tout le reste vient de ce qui a été demandé : rejouer donnerait le
        // même résultat.
        _ => ErrorClass::Permanent,
    }
}

/// La famille associée à un SQLSTATE.
fn classify_sqlstate(code: Option<&str>, transport: ErrorClass) -> ErrorClass {
    let Some(code) = code else {
        return ErrorClass::Permanent;
    };
    match code {
        // Conflits de concurrence : rejouer est exactement la bonne réponse.
        SQLSTATE_SERIALIZATION_FAILURE
        | SQLSTATE_DEADLOCK
        | SQLSTATE_LOCK_NOT_AVAILABLE
        | SQLSTATE_TOO_MANY_CONNECTIONS
        | SQLSTATE_CONFIG_LIMIT
        | SQLSTATE_CANNOT_CONNECT_NOW
        | SQLSTATE_IDLE_TIMEOUT
        | SQLSTATE_OUT_OF_MEMORY
        | SQLSTATE_DISK_FULL => ErrorClass::Transient,

        // Le serveur s'est arrêté au milieu : comme une coupure.
        SQLSTATE_ADMIN_SHUTDOWN | SQLSTATE_CRASH_SHUTDOWN => transport,

        autre if autre.starts_with(SQLSTATE_CLASS_CONNECTION) => transport,
        autre if autre.starts_with(SQLSTATE_CLASS_RESOURCES) => ErrorClass::Transient,

        // Syntaxe, objet absent, contrainte violée, droits, transaction en
        // lecture seule : on affiche, on ne retente pas.
        _ => ErrorClass::Permanent,
    }
}

/// Une erreur qui relève de la configuration plutôt que du serveur.
fn as_config_error(err: &sqlx::Error) -> Option<OxynError> {
    match err {
        sqlx::Error::Configuration(_) => Some(OxynError::Config(message_of(err))),
        // Un échec TLS est un problème de configuration ou de confiance, pas une
        // panne réseau à retenter en boucle.
        sqlx::Error::Tls(_) => Some(OxynError::Connection(message_of(err))),
        _ => None,
    }
}

/// Le SQLSTATE porté par l'erreur, quand elle vient du serveur.
#[must_use]
pub(crate) fn sqlstate_of(err: &sqlx::Error) -> Option<String> {
    match err {
        sqlx::Error::Database(base) => base.code().map(|code| code.into_owned()),
        _ => None,
    }
}

/// Le message d'une erreur, en préférant celui du serveur au nôtre.
fn message_of(err: &sqlx::Error) -> String {
    match err {
        sqlx::Error::Database(base) => base.message().to_owned(),
        autre => autre.to_string(),
    }
}

/// Emballe l'erreur `sqlx` dans le type nommé du driver.
fn wrap(err: sqlx::Error) -> PostgresError {
    PostgresError {
        sqlstate: sqlstate_of(&err),
        message: message_of(&err),
    }
}

/// L'erreur est-elle celle d'une transaction en lecture seule ayant refusé une
/// écriture ?
#[must_use]
pub(crate) fn is_read_only_rejection(err: &sqlx::Error) -> bool {
    sqlstate_of(err).as_deref() == Some(SQLSTATE_READ_ONLY_TRANSACTION)
}

/// Ce qu'on dit à l'utilisateur quand ses bornes d'exécution ont refusé une
/// écriture.
///
/// Le message existe pour que personne ne conclue à un défaut de droits sur sa
/// base : c'est Oxyn qui a demandé la lecture seule, pas l'administrateur.
const READ_ONLY_MESSAGE: &str = "cette exécution est bornée en lecture seule : \
                                 le serveur a refusé une instruction qui écrit";

/// Traduit une erreur d'exécution en tenant compte des bornes demandées.
///
/// Un seul endroit décide de ce message, parce qu'il est produit sur deux
/// chemins — la préparation et le flux — et que deux formulations divergeraient.
#[must_use]
pub(crate) fn map_stream_error(
    driver: &DriverId,
    intent: StatementIntent,
    read_only: bool,
    err: sqlx::Error,
) -> OxynError {
    if read_only && is_read_only_rejection(&err) {
        return OxynError::Query(READ_ONLY_MESSAGE.to_owned());
    }
    map_exec_error(driver, intent, err)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Une erreur de base de données factice portant un SQLSTATE donné.
    ///
    /// `sqlx` ne permet pas de construire un `PgDatabaseError` depuis
    /// l'extérieur ; on implémente donc le trait, ce qui suffit à `classify`.
    #[derive(Debug)]
    struct BaseFactice {
        code: &'static str,
    }

    impl std::fmt::Display for BaseFactice {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("erreur factice")
        }
    }

    impl std::error::Error for BaseFactice {}

    impl DatabaseError for BaseFactice {
        fn message(&self) -> &str {
            "erreur factice"
        }

        fn code(&self) -> Option<std::borrow::Cow<'_, str>> {
            Some(std::borrow::Cow::Borrowed(self.code))
        }

        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }

        fn kind(&self) -> sqlx::error::ErrorKind {
            sqlx::error::ErrorKind::Other
        }
    }

    fn erreur_serveur(code: &'static str) -> sqlx::Error {
        sqlx::Error::Database(Box::new(BaseFactice { code }))
    }

    fn coupure() -> sqlx::Error {
        sqlx::Error::Io(std::io::Error::from(std::io::ErrorKind::ConnectionReset))
    }

    #[test]
    fn une_coupure_pendant_une_ecriture_est_ambigue() {
        // C'est le test le plus important du module : classée transitoire, cette
        // erreur produit un doublon silencieux dans les données de
        // l'utilisateur (I-13).
        assert_eq!(
            classify(&coupure(), StatementIntent::Write),
            ErrorClass::Ambiguous
        );
        assert!(!ErrorClass::Ambiguous.is_retryable());
    }

    #[test]
    fn une_coupure_pendant_une_lecture_est_transitoire() {
        assert_eq!(
            classify(&coupure(), StatementIntent::Read),
            ErrorClass::Transient
        );
    }

    #[test]
    fn une_intention_inconnue_compte_pour_mutante() {
        // Le défaut d'`ExecRequest` : une instruction qu'aucun analyseur n'a su
        // classer peut écrire.
        assert_eq!(
            classify(&coupure(), StatementIntent::Unknown),
            ErrorClass::Ambiguous
        );
    }

    #[test]
    fn un_interblocage_se_retente() {
        assert_eq!(
            classify(&erreur_serveur(SQLSTATE_DEADLOCK), StatementIntent::Write),
            ErrorClass::Transient
        );
        assert_eq!(
            classify(
                &erreur_serveur(SQLSTATE_SERIALIZATION_FAILURE),
                StatementIntent::Write
            ),
            ErrorClass::Transient
        );
    }

    #[test]
    fn une_erreur_de_syntaxe_ne_se_retente_jamais() {
        // 42601 : syntax_error.
        assert_eq!(
            classify(&erreur_serveur("42601"), StatementIntent::Read),
            ErrorClass::Permanent
        );
        // 42P01 : undefined_table.
        assert_eq!(
            classify(&erreur_serveur("42P01"), StatementIntent::Read),
            ErrorClass::Permanent
        );
        // 23505 : unique_violation.
        assert_eq!(
            classify(&erreur_serveur("23505"), StatementIntent::Write),
            ErrorClass::Permanent
        );
    }

    #[test]
    fn une_coupure_annoncee_par_le_serveur_suit_l_intention() {
        // 08006 : connection_failure.
        assert_eq!(
            classify(&erreur_serveur("08006"), StatementIntent::Read),
            ErrorClass::Transient
        );
        assert_eq!(
            classify(&erreur_serveur("08006"), StatementIntent::Write),
            ErrorClass::Ambiguous
        );
    }

    #[test]
    fn une_attente_de_connexion_expiree_reste_transitoire_meme_en_ecriture() {
        // Rien n'a été envoyé : l'acquisition a échoué avant la requête.
        assert_eq!(
            classify(&sqlx::Error::PoolTimedOut, StatementIntent::Write),
            ErrorClass::Transient
        );
    }

    #[test]
    fn une_annulation_confirmee_par_le_serveur_n_est_pas_une_panne() {
        let erreur = map_exec_error(
            &DriverId::postgres(),
            StatementIntent::Read,
            erreur_serveur(SQLSTATE_QUERY_CANCELED),
        );
        assert!(erreur.is_cancelled(), "{erreur:?}");
    }

    #[test]
    fn un_refus_d_identifiants_se_distingue_d_un_serveur_injoignable() {
        // 28P01 : invalid_password. L'un se corrige dans le formulaire.
        let refus = map_connect_error(&erreur_serveur("28P01"));
        assert!(matches!(refus, OxynError::Authentication(_)), "{refus:?}");
        assert!(refus.is_user_error());

        let injoignable = map_connect_error(&coupure());
        assert!(
            matches!(injoignable, OxynError::Connection(_)),
            "{injoignable:?}"
        );
        assert!(injoignable.is_retryable());
    }

    #[test]
    fn une_base_inexistante_est_une_erreur_de_configuration() {
        let erreur = map_connect_error(&erreur_serveur("3D000"));
        assert!(matches!(erreur, OxynError::Config(_)), "{erreur:?}");
        assert!(!erreur.is_retryable(), "rejouer ne créera pas la base");
    }

    #[test]
    fn la_classe_traverse_l_emballage_jusqu_a_l_appelant() {
        // L'appelant lit `ErrorClass`, il ne relit pas le message.
        let erreur = map_exec_error(&DriverId::postgres(), StatementIntent::Write, coupure());
        assert_eq!(erreur.class(), ErrorClass::Ambiguous);
        assert!(!erreur.is_retryable());
    }

    #[test]
    fn le_sqlstate_reste_lisible_sans_analyser_le_message() {
        let erreur = map_exec_error(
            &DriverId::postgres(),
            StatementIntent::Read,
            erreur_serveur("42P01"),
        );
        let OxynError::Driver { source, .. } = &erreur else {
            panic!("attendu une erreur de driver : {erreur:?}");
        };
        let postgres = source
            .downcast_ref::<PostgresError>()
            .expect("le driver emballe ses erreurs dans PostgresError");
        assert_eq!(postgres.sqlstate(), Some("42P01"));
        assert_eq!(postgres.message(), "erreur factice");
    }

    #[test]
    fn le_refus_d_une_transaction_en_lecture_seule_se_reconnait() {
        assert!(is_read_only_rejection(&erreur_serveur(
            SQLSTATE_READ_ONLY_TRANSACTION
        )));
        assert!(!is_read_only_rejection(&erreur_serveur("42601")));
    }

    #[test]
    fn une_ecriture_refusee_par_les_bornes_nomme_les_bornes_pas_les_droits() {
        // Sans ce message, l'utilisateur conclut à un défaut de droits sur sa
        // base et va voir son administrateur.
        let erreur = map_stream_error(
            &DriverId::postgres(),
            StatementIntent::Write,
            true,
            erreur_serveur(SQLSTATE_READ_ONLY_TRANSACTION),
        );
        assert!(matches!(erreur, OxynError::Query(_)), "{erreur:?}");
        assert!(erreur.to_string().contains("lecture seule"), "{erreur}");
        assert!(erreur.is_user_error());
    }

    #[test]
    fn le_meme_refus_hors_bornes_reste_une_erreur_du_serveur() {
        // La base peut être en lecture seule pour ses propres raisons — un
        // secondaire, un `default_transaction_read_only`. Ce n'est alors pas à
        // Oxyn de s'attribuer le refus.
        let erreur = map_stream_error(
            &DriverId::postgres(),
            StatementIntent::Write,
            false,
            erreur_serveur(SQLSTATE_READ_ONLY_TRANSACTION),
        );
        assert!(matches!(erreur, OxynError::Driver { .. }), "{erreur:?}");
    }
}
