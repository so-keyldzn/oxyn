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
//! # Le message du serveur cite ce qu'on lui a lié
//!
//! `invalid input syntax for type integer: "…"` sur un `$1` mal typé, un
//! `RAISE` qui concatène `NEW.colonne` : PostgreSQL recopie une valeur liée dans
//! son message primaire. Ce message est affiché, journalisé et **persisté** par
//! l'historique et le journal. Il n'est donc propagé que si l'instruction ne
//! portait aucune valeur liée par l'appelant — voir [`Bound`].
//!
//! # Ce que ce module ne fait pas
//!
//! Il ne retente rien. La politique de reprise appartient à l'appelant, seul à
//! savoir si l'opération est rejouable
//! ([DRIVER-CONTRACT](../../../docs/DRIVER-CONTRACT.md)).
//!
//! Il ne **filtre** pas non plus le texte des valeurs dans le message : le
//! serveur les tronque, les échappe et les transforme, si bien qu'un `replace`
//! aurait l'apparence d'une protection sans en être une.

use oxyn_core::{DriverId, ErrorClass, OxynError, ScalarValue, StatementIntent};

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

/// D'où viennent les valeurs liées à l'instruction concernée.
///
/// Le message primaire du serveur peut citer une valeur liée ; il n'est donc
/// propagé que lorsque rien de ce qu'il peut citer ne vient de l'appelant
/// ([I-03](../../../CLAUDE.md#i-03)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Bound {
    /// Aucune valeur venue d'une [`ExecRequest`](oxyn_core::ExecRequest). Les
    /// requêtes d'introspection lient bien des identifiants — un nom de schéma,
    /// un nom de relation —, mais ceux-là viennent du catalogue déjà affiché.
    /// La propriété tenue est « rien de ce que le serveur peut citer ne vient de
    /// l'appelant », pas « rien n'était lié ».
    Internal,
    /// Au moins une valeur de l'[`ExecRequest`](oxyn_core::ExecRequest) de
    /// l'appelant était liée.
    Caller,
}

impl Bound {
    /// Ce que les paramètres d'une demande impliquent pour le message du
    /// serveur.
    pub(crate) const fn of(params: &[ScalarValue]) -> Self {
        if params.is_empty() {
            Self::Internal
        } else {
            Self::Caller
        }
    }
}

/// Ce qui remplace le message du serveur quand l'instruction portait des valeurs
/// liées.
///
/// Explicite plutôt que rassurant : sans cette phrase, l'utilisateur croirait à
/// une erreur muette et chercherait un défaut dans son SQL.
const WITHHELD_MESSAGE: &str = "the server message is withheld because the statement carried \
                                bound values, which PostgreSQL can quote verbatim";

/// L'erreur de driver telle qu'elle est emballée dans [`OxynError::Driver`].
///
/// Un type nommé plutôt qu'un `Box<dyn Error>` anonyme : l'appelant qui veut le
/// SQLSTATE peut le lire par `downcast_ref`, sans analyser un message.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct PostgresError {
    /// Ce qui sera montré, journalisé et persisté.
    ///
    /// C'est le message du serveur — ou celui du transport — **seulement** si
    /// l'instruction ne portait aucune valeur liée par l'appelant. Sinon, c'est
    /// [`WITHHELD_MESSAGE`] suivi du SQLSTATE : le serveur recopie une valeur
    /// liée dans son message primaire, et ce champ finit dans l'historique et
    /// le journal ([I-03](../../../CLAUDE.md#i-03)).
    ///
    /// L'erreur `sqlx` d'origine n'est jamais conservée à côté :
    /// `#[derive(Debug)]` la ré-exposerait au premier `tracing::debug!` venu.
    message: String,
    /// Le SQLSTATE, quand l'erreur vient du serveur.
    sqlstate: Option<String>,
}

impl PostgresError {
    /// Le SQLSTATE à cinq caractères, quand le serveur en a donné un.
    ///
    /// Toujours présent, y compris quand le message est retenu : c'est un code,
    /// il ne cite rien.
    #[must_use]
    pub fn sqlstate(&self) -> Option<&str> {
        self.sqlstate.as_deref()
    }

    /// Le message tel qu'il sera montré.
    ///
    /// « Le public lit les messages d'erreur de PostgreSQL » : celui du serveur
    /// n'est jamais paraphrasé. Il est en revanche **retenu**, et le dit, quand
    /// l'instruction portait des valeurs liées.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Traduit une erreur `sqlx` survenue sur une instruction **composée par le
/// driver**.
///
/// Le message du serveur est propagé tel quel : une telle instruction ne lie que
/// des littéraux que le driver a écrits. Pour l'exécution d'une
/// [`ExecRequest`](oxyn_core::ExecRequest), passer par [`map_stream_error`], qui
/// prend le relevé des valeurs liées ([I-03](../../../CLAUDE.md#i-03)).
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
    map_bound_error(driver, intent, Bound::Internal, err)
}

/// Traduit une erreur `sqlx` survenue **pendant une exécution**, en sachant si
/// l'instruction portait des valeurs liées par l'appelant.
///
/// `bound` ne change **que** le message : la famille se lit sur le SQLSTATE et
/// l'intention, tous deux relevés sur l'erreur d'origine.
#[must_use]
fn map_bound_error(
    driver: &DriverId,
    intent: StatementIntent,
    bound: Bound,
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
    OxynError::driver(driver.clone(), classe, wrap(err, bound))
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
///
/// Deux variantes seulement sont retenues quand l'appelant avait lié des
/// valeurs : celle **du serveur**, dont le message primaire les recopie, et
/// celle de l'**encodeur**, qui est composée à partir de la valeur qu'il n'a pas
/// su encoder. Les autres — coupure, TLS, bassin épuisé — sont écrites par
/// `sqlx` à partir du transport, sans jamais y verser les arguments : les
/// retenir coûterait un diagnostic sans rien protéger.
fn wrap(err: sqlx::Error, bound: Bound) -> PostgresError {
    let sqlstate = sqlstate_of(&err);
    let message = match (bound, &err) {
        (Bound::Caller, sqlx::Error::Database(_) | sqlx::Error::Encode(_)) => {
            withheld(sqlstate.as_deref())
        }
        _ => message_of(&err),
    };
    PostgresError { message, sqlstate }
}

/// Le message de remplacement, qui garde le seul identifiant sûr : le SQLSTATE.
fn withheld(sqlstate: Option<&str>) -> String {
    match sqlstate {
        Some(code) => format!("{WITHHELD_MESSAGE} (SQLSTATE {code})"),
        None => WITHHELD_MESSAGE.to_owned(),
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

/// Traduit une erreur d'exécution en tenant compte des bornes demandées et des
/// valeurs liées.
///
/// Un seul endroit décide de ce message, parce qu'il est produit sur deux
/// chemins — la préparation et le flux — et que deux formulations divergeraient.
///
/// C'est la porte de l'instruction **de l'appelant** : `bound` y est obligatoire
/// pour qu'aucun de ces deux chemins ne puisse l'oublier.
#[must_use]
pub(crate) fn map_stream_error(
    driver: &DriverId,
    intent: StatementIntent,
    read_only: bool,
    bound: Bound,
    err: sqlx::Error,
) -> OxynError {
    if read_only && is_read_only_rejection(&err) {
        return OxynError::Query(READ_ONLY_MESSAGE.to_owned());
    }
    map_bound_error(driver, intent, bound, err)
}

#[cfg(test)]
mod tests {
    use super::*;
    // Seul le test implémente ce trait : à la racine du module, il serait un
    // import inutilisé dans la cible `lib`.
    use sqlx::error::DatabaseError;

    /// Une erreur de base de données factice portant un SQLSTATE donné.
    ///
    /// `sqlx` ne permet pas de construire un `PgDatabaseError` depuis
    /// l'extérieur ; on implémente donc le trait, ce qui suffit à `classify`.
    #[derive(Debug)]
    struct BaseFactice {
        code: &'static str,
        message: &'static str,
    }

    impl std::fmt::Display for BaseFactice {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.message)
        }
    }

    impl std::error::Error for BaseFactice {}

    impl DatabaseError for BaseFactice {
        fn message(&self) -> &str {
            self.message
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
        sqlx::Error::Database(Box::new(BaseFactice {
            code,
            message: "erreur factice",
        }))
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
            Bound::Internal,
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
            Bound::Internal,
            erreur_serveur(SQLSTATE_READ_ONLY_TRANSACTION),
        );
        assert!(matches!(erreur, OxynError::Driver { .. }), "{erreur:?}");
    }

    /// Une erreur serveur dont le message primaire cite une valeur liée, comme
    /// le fait un cast invalide de `$1`.
    fn erreur_bavarde() -> sqlx::Error {
        sqlx::Error::Database(Box::new(BaseFactice {
            code: "22P02",
            message: "invalid input syntax for type integer: \"S3NT1NELLE-42\"",
        }))
    }

    #[test]
    fn le_message_du_serveur_est_retenu_des_qu_une_valeur_etait_liee() {
        // I-03 : ce message est affiché, et persisté par `HistoryRecord::failed`
        // et `JournalRecord::failed`, qui appellent `error.to_string()`.
        let erreur = map_stream_error(
            &DriverId::postgres(),
            StatementIntent::Read,
            false,
            Bound::Caller,
            erreur_bavarde(),
        );
        for rendu in [format!("{erreur}"), format!("{erreur:?}")] {
            assert!(!rendu.contains("S3NT1NELLE-42"), "valeur liée : {rendu}");
            assert!(!rendu.contains("invalid input syntax"), "{rendu}");
        }
        assert!(erreur.to_string().contains("withheld"), "{erreur}");
        // Le SQLSTATE survit : c'est un code, il ne cite rien.
        assert!(erreur.to_string().contains("22P02"), "{erreur}");
        let OxynError::Driver { source, .. } = &erreur else {
            panic!("attendu une erreur de driver : {erreur:?}");
        };
        let postgres = source
            .downcast_ref::<PostgresError>()
            .expect("le driver emballe ses erreurs dans PostgresError");
        assert_eq!(postgres.sqlstate(), Some("22P02"));
    }

    #[test]
    fn sans_valeur_liee_le_message_du_serveur_passe_inchange() {
        // Le public d'Oxyn lit les messages de PostgreSQL : une paraphrase
        // rassurante serait un défaut.
        let erreur = map_stream_error(
            &DriverId::postgres(),
            StatementIntent::Read,
            false,
            Bound::Internal,
            erreur_bavarde(),
        );
        assert!(
            erreur
                .to_string()
                .contains("invalid input syntax for type integer"),
            "{erreur}"
        );
    }

    #[test]
    fn le_retrait_du_message_ne_change_ni_la_famille_ni_l_annulation() {
        // La famille se lit sur le SQLSTATE et l'intention, pas sur le message.
        let coupure_en_ecriture = map_stream_error(
            &DriverId::postgres(),
            StatementIntent::Write,
            false,
            Bound::Caller,
            coupure(),
        );
        assert_eq!(coupure_en_ecriture.class(), ErrorClass::Ambiguous);
        assert!(!coupure_en_ecriture.is_retryable(), "I-13");

        let interblocage = map_stream_error(
            &DriverId::postgres(),
            StatementIntent::Write,
            false,
            Bound::Caller,
            erreur_serveur(SQLSTATE_DEADLOCK),
        );
        assert_eq!(interblocage.class(), ErrorClass::Transient);

        let annulee = map_stream_error(
            &DriverId::postgres(),
            StatementIntent::Read,
            false,
            Bound::Caller,
            erreur_serveur(SQLSTATE_QUERY_CANCELED),
        );
        assert!(annulee.is_cancelled(), "{annulee:?}");
    }

    #[test]
    fn une_erreur_d_encodeur_est_retenue_comme_celle_du_serveur() {
        // L'encodeur compose son message à partir de la valeur qu'il a refusée.
        let erreur = map_stream_error(
            &DriverId::postgres(),
            StatementIntent::Read,
            false,
            Bound::Caller,
            sqlx::Error::Encode("`S3NT1NELLE-42` is out of range".into()),
        );
        assert!(!erreur.to_string().contains("S3NT1NELLE-42"), "{erreur}");
        assert!(erreur.to_string().contains("withheld"), "{erreur}");
    }

    #[test]
    fn une_demande_sans_parametre_ne_retient_rien() {
        assert_eq!(Bound::of(&[]), Bound::Internal);
        assert_eq!(
            Bound::of(&[oxyn_core::ScalarValue::Text("S3NT1NELLE-42".to_owned())]),
            Bound::Caller
        );
    }
}
