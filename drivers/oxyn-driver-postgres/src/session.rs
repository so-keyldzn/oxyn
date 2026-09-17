//! Une connexion ouverte : exécuter, annuler, sonder, fermer.
//!
//! # Ce qui gouverne ce fichier
//!
//! **L'annulation part d'une seconde connexion.** `pg_cancel_backend` est une
//! fonction SQL ordinaire : l'appeler demande une connexion libre. Or c'est
//! exactement quand toutes les connexions sont prises par des requêtes longues
//! qu'on veut annuler. `BackendCanceller` ouvre donc une connexion **neuve**,
//! hors du bassin, plutôt que d'attendre qu'une place se libère
//! ([DRIVER-CONTRACT §2](../../../docs/DRIVER-CONTRACT.md)).
//!
//! **L'annulation est envoyée par qui tient la connexion.** Une session
//! s'appuie sur un bassin, donc chaque exécution tourne sur un processus serveur
//! différent, et un même processus sert une requête après l'autre.
//! [`Session::cancel`] ne vise donc pas un pid : il réveille la tâche de flux de
//! l'exécution, qui connaît son pid et **garde sa connexion** jusqu'à ce que
//! l'annulation soit partie. Envoyée d'ici, elle pourrait tomber sur la requête
//! suivante, lancée sur le même processus pendant la poignée de main de
//! l'annulation. Le détail est dans le module `cancel`.
//!
//! **Le SQL de l'utilisateur part tel quel ; celui d'Oxyn ne concatène rien.**
//! Le texte d'une [`ExecRequest`] est préparé sans être analysé ni réécrit :
//! c'est la fonctionnalité d'un outil professionnel. Les requêtes que le driver
//! **compose** — introspection, annulation — sont des littéraux à paramètres
//! liés ([I-10](../../../CLAUDE.md#i-10)).
//!
//! **La lecture seule est imposée par le serveur.** Quand
//! [`ExecLimits::read_only`](oxyn_core::ExecLimits::read_only) est vrai,
//! l'exécution est encadrée par
//! `BEGIN READ ONLY` : c'est le serveur qui refuse l'écriture, pas un filtre
//! côté client. C'est ce que déclare
//! [`Capabilities::READ_ONLY_SESSION`](oxyn_core::Capabilities::READ_ONLY_SESSION).

use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use oxyn_catalog::CatalogProvider;
use oxyn_core::{
    CancelToken, Capabilities, DriverId, ExecRequest, OxynError, Result, ScalarValue, SqlDialect,
    StatementHandle, StatementIntent,
};
use oxyn_driver::{Cursor, Session};
use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgArguments, PgConnection, PgPool, Postgres};
use sqlx::{Arguments as _, AssertSqlSafe, Connection as _, Executor as _};
use sqlx::{Row as _, SqlSafeStr as _, Statement as _};

use crate::cancel::{BackendCanceller, StatementRegistry};
use crate::catalog::PostgresCatalog;
use crate::cursor::{self, StreamRequest};
use crate::error::{Bound, map_connect_error, map_exec_error, map_stream_error};
use crate::lease::Lease;
use crate::options::ConnectSpec;
use crate::transaction_text::{controls_transaction, opens_transaction};

/// Ce que dit le refus d'une instruction de contrôle de transaction.
///
/// Nomme la capacité manquante, puis ce que l'utilisateur doit savoir pour ne
/// pas se tromper sur ses données : sans transaction, chaque instruction est
/// validée seule.
const TRANSACTIONS_REFUSED: &str = "TRANSACTIONS (transactions are not supported in the console \
                                    yet: each statement commits on its own)";
use crate::types::schema_for;
use crate::variant::PostgresVariant;
use oxyn_catalog::path::{QuoteStyle, quote_identifier};
use oxyn_driver::SessionContext;

/// Ouvre une transaction en lecture seule pour la durée d'une exécution.
///
/// Littéral : aucun élément n'y est composé.
pub(crate) const SQL_BEGIN_READ_ONLY: &str = "BEGIN READ ONLY";
/// Referme la transaction ouverte par [`SQL_BEGIN_READ_ONLY`].
pub(crate) const SQL_ROLLBACK: &str = "ROLLBACK";
/// Rend au `search_path` sa valeur par défaut avant que la connexion reparte
/// au bassin.
///
/// Littéral : le contexte d'une console ne doit pas être emporté par la
/// connexion vers l'introspection ou vers une autre console.
pub(crate) const SQL_RESET_SEARCH_PATH: &str = "SET search_path TO DEFAULT";
/// Remet l'état de session au défaut après une exécution **inscriptible**, qui
/// n'a pas de `ROLLBACK` pour défaire ce que l'utilisateur a pu poser.
///
/// Littéral, deux instructions en un seul aller-retour (protocole simple) :
///
/// * `standard_conforming_strings` : le découpeur d'`oxyn-query` suppose `on`.
///   Une connexion rendue au bassin avec `off` ferait lire au serveur un
///   `DELETE` là où le découpeur a vu une chaîne, donc une écriture classée
///   lecture, sans la confirmation qui nomme la connexion
///   ([I-02](../../../CLAUDE.md#i-02)). Voir [`ConnectSpec`](crate::ConnectSpec)
///   pour la valeur posée à l'ouverture ;
/// * `search_path` : un `SET` tapé dans une console voyagerait sinon vers
///   l'emprunteur suivant. Il n'est de toute façon pas fiable pour
///   l'utilisateur, dont la requête suivante peut partir sur une autre
///   connexion ; le schéma d'une console passe par son contexte de session.
pub(crate) const SQL_RESET_AFTER_WRITE: &str =
    "SET standard_conforming_strings TO on; SET search_path TO DEFAULT";
/// Le pid du processus serveur qui exécute sur cette connexion.
const SQL_BACKEND_PID: &str = "SELECT pg_catalog.pg_backend_pid()";

/// Le schéma demandé existe-t-il, et ce rôle le voit-il ?
///
/// `SET search_path` accepte un schéma absent sans rien dire ; c'est cette
/// requête qui fait la différence entre un contexte appliqué et un contexte
/// affiché. Le nom voyage **lié**, jamais concaténé (I-10).
const SQL_NAMESPACE_EXISTS: &str = "SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname = $1 \
     AND pg_catalog.has_schema_privilege(oid, 'USAGE')";

/// Une session PostgreSQL ouverte.
#[derive(Debug)]
pub struct PostgresSession {
    driver: DriverId,
    pool: PgPool,
    canceller: Arc<BackendCanceller>,
    statements: Arc<StatementRegistry>,
    variant: PostgresVariant,
    capabilities: Capabilities,
    catalog: PostgresCatalog,
    /// La base de la connexion, pour refuser un contexte qui en désigne une
    /// autre : une session PostgreSQL ne change pas de base.
    database: String,
    /// Ce que le serveur a confirmé, ou `None` tant que rien n'a été déclaré.
    ///
    /// Sous un verrou synchrone : il n'est jamais tenu à travers un `await`,
    /// seulement lu le temps de composer une instruction.
    context: Mutex<Option<SessionContext>>,
}

impl PostgresSession {
    /// Assemble une session à partir d'un bassin déjà ouvert et d'une variante
    /// déjà détectée.
    ///
    /// Appelée par [`PostgresDriver::connect`](crate::driver::PostgresDriver) ;
    /// la détection y est faite parce qu'elle a besoin d'une connexion, et que
    /// c'est le seul moment où l'échec peut encore se traduire par un refus de
    /// connexion plutôt que par une capacité fausse.
    #[must_use]
    pub(crate) fn new(
        driver: DriverId,
        pool: PgPool,
        spec: ConnectSpec,
        variant: PostgresVariant,
        database: String,
    ) -> Self {
        let capabilities = variant.capabilities();
        // L'annulateur est partagé avec le catalogue : une introspection longue
        // doit pouvoir être coupée côté serveur au même titre qu'une requête.
        let canceller = Arc::new(BackendCanceller::new(spec, driver.clone()));
        let catalog = PostgresCatalog::new(
            driver.clone(),
            pool.clone(),
            database.clone(),
            variant.clone(),
            capabilities,
            Arc::clone(&canceller),
        );
        Self {
            canceller,
            statements: Arc::new(StatementRegistry::default()),
            driver,
            pool,
            variant,
            capabilities,
            catalog,
            database,
            context: Mutex::new(None),
        }
    }

    /// L'instruction qui pose le contexte sur la connexion d'une exécution.
    ///
    /// `None` quand il n'y a rien à poser et que rien n'a jamais été posé : la
    /// connexion est alors dans l'état où le serveur l'a ouverte. Dès qu'un
    /// contexte a été déclaré, même pour revenir au défaut, l'instruction est
    /// émise — une connexion du bassin peut porter l'état d'une exécution
    /// précédente, et `search_path` est un état **par connexion**.
    fn context_statement(&self) -> Option<String> {
        let guard = self
            .context
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let context = guard?;
        Some(match context.namespace() {
            // Cité par le driver : un nom de schéma vient du catalogue, donc du
            // serveur, et le concaténer exécuterait ce qu'il contient (I-10).
            Some(namespace) => format!(
                "SET search_path TO {}",
                quote_identifier(namespace, QuoteStyle::for_dialect(SqlDialect::Postgres))
            ),
            None => "SET search_path TO DEFAULT".to_owned(),
        })
    }

    /// Ce que la session a appris de son serveur.
    #[must_use]
    pub const fn variant(&self) -> &PostgresVariant {
        &self.variant
    }

    /// Exécutions en cours, c'est-à-dire annulables. Diagnostic et tests.
    #[doc(hidden)]
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.statements.len()
    }

    /// L'annulateur, pour les tests qui retiennent une annulation en vol.
    #[cfg(test)]
    pub(crate) fn canceller(&self) -> &BackendCanceller {
        &self.canceller
    }

    /// Déclare une capacité que le driver n'implémente pas, pour les tests qui
    /// éprouvent ce qui se passe **derrière** un refus : le filet de sécurité
    /// qui ferme une connexion laissée en transaction.
    #[cfg(test)]
    pub(crate) fn declare_for_test(&mut self, capabilities: Capabilities) {
        self.capabilities.insert(capabilities);
    }

    /// Emprunte une connexion au bassin, sans jamais devenir inannulable.
    async fn acquire(&self, cancel: &CancelToken) -> Result<PoolConnection<Postgres>> {
        race_cancel(cancel, async {
            self.pool
                .acquire()
                .await
                .map_err(|erreur| map_connect_error(&erreur))
        })
        .await
    }
}

#[async_trait]
impl Session for PostgresSession {
    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// Déclare où les noms non qualifiés se résolvent, après vérification.
    ///
    /// PostgreSQL **accepte en silence** un `SET search_path` vers un schéma qui
    /// n'existe pas : sans la vérification préalable, Oxyn afficherait un
    /// contexte que le serveur n'applique pas. Le nom est comparé par une valeur
    /// liée, pas concaténé.
    ///
    /// # Erreurs
    /// [`OxynError::Config`] si le contexte désigne une autre base — une session
    /// PostgreSQL ne change pas de base — ou un schéma absent ;
    /// [`OxynError::Cancelled`] si `cancel` se déclenche.
    async fn set_context(&self, context: &SessionContext, cancel: &CancelToken) -> Result<()> {
        if let Some(catalog) = context.catalog()
            && catalog != self.database
        {
            return Err(OxynError::Config(
                "a PostgreSQL session cannot change database; \
                 open a connection to that database instead"
                    .to_owned(),
            ));
        }
        if let Some(namespace) = context.namespace() {
            let mut connexion = self.acquire(cancel).await?;
            let found: Option<i32> = race_cancel(cancel, async {
                sqlx::query_scalar(SQL_NAMESPACE_EXISTS)
                    .bind(namespace)
                    .fetch_optional(&mut *connexion)
                    .await
                    .map_err(|erreur| map_exec_error(&self.driver, StatementIntent::Read, erreur))
            })
            .await?;
            if found.is_none() {
                // Le nom demandé vient du catalogue affiché : le citer ici ne
                // révèle rien que l'utilisateur ne voie déjà, et sans lui le
                // message ne dirait pas quoi corriger.
                return Err(OxynError::Config(format!(
                    "schema `{namespace}` does not exist or is not visible to this role"
                )));
            }
        }
        *self.context.lock().unwrap_or_else(PoisonError::into_inner) = Some(context.clone());
        Ok(())
    }

    fn context(&self) -> Option<SessionContext> {
        self.context
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Prépare, puis lance le flux.
    ///
    /// Rend la main **dès que le schéma est connu** : celui-ci vient de
    /// l'instruction préparée, pas de la première ligne. C'est ce qui permet à
    /// la grille de dessiner ses colonnes pendant que les données arrivent
    /// encore.
    ///
    /// # Erreurs
    /// [`OxynError::NotSupported`] si le langage n'est pas du SQL, ou si un
    /// paramètre lié porte un type que le driver ne sait pas encoder ;
    /// [`OxynError::Cancelled`] si `cancel` se déclenche avant le départ ;
    /// [`OxynError::Query`] si le serveur rejette l'instruction ; toute erreur de
    /// transport, classée.
    async fn execute(&self, request: ExecRequest, cancel: &CancelToken) -> Result<Box<dyn Cursor>> {
        self.capabilities.require_language(request.language)?;
        // Avant tout emprunt : rien ne part au serveur. Voir
        // `transaction_text::controls_transaction` pour le mensonge que ce
        // refus empêche — un `ROLLBACK` qui « réussit » sans rien annuler.
        if !self.capabilities.contains(Capabilities::TRANSACTIONS)
            && controls_transaction(&request.text)
        {
            return Err(OxynError::NotSupported {
                capability: TRANSACTIONS_REFUSED.to_owned(),
            });
        }
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }

        let ExecRequest {
            text,
            params,
            intent,
            limits,
            ..
        } = request;

        // Relevé **avant** que les valeurs partent dans `PgArguments`, puis dans
        // `query_with` : à partir de là, plus personne sur le chemin ne sait
        // qu'il y avait des valeurs liées, et le message du serveur peut en
        // citer une (I-03).
        let bound = Bound::of(&params);

        // Les paramètres sont encodés avant tout aller-retour : un type non
        // encodable doit être refusé sans avoir occupé de connexion.
        let arguments = bind_params(&params)?;

        let mut connexion = Lease::new(self.acquire(cancel).await?);

        let pid = race_cancel(cancel, async {
            backend_pid(&mut connexion)
                .await
                .map_err(|erreur| map_exec_error(&self.driver, StatementIntent::Read, erreur))
        })
        .await?;

        // À partir du `SET` ou du `BEGIN`, la connexion porte un état qui n'est
        // pas celui du bassin, et c'est le curseur qui la remet au défaut, lui
        // seul. L'emprunt est donc marqué **avant** l'envoi : tout départ
        // anticipé — une erreur, un `?`, et surtout un futur abandonné, qui ne
        // passe par aucun des deux — ferme la connexion au lieu de la rendre.
        // Sinon le contexte d'une console voyagerait vers l'introspection ou
        // vers une autre console
        // ([ADR-0019](../../../docs/adr/0019-contexte-de-session.md)). Rouvrir
        // une connexion coûte moins qu'un `DELETE` résolu dans un autre schéma.

        // Avant la transaction : posé à l'intérieur, un `SET` serait défait par
        // le `ROLLBACK` qui clôt une lecture seule, et l'instruction suivante
        // sur la même connexion résoudrait ailleurs.
        let restore_context = self.context_statement().is_some();
        if let Some(statement) = self.context_statement() {
            let sql = AssertSqlSafe(statement).into_sql_str();
            connexion.taint();
            race_cancel(cancel, async {
                sqlx::raw_sql(sql)
                    .execute(&mut *connexion)
                    .await
                    .map_err(|erreur| map_exec_error(&self.driver, StatementIntent::Read, erreur))
            })
            .await?;
        }

        if limits.read_only {
            connexion.taint();
            race_cancel(cancel, async {
                sqlx::raw_sql(SQL_BEGIN_READ_ONLY)
                    .execute(&mut *connexion)
                    .await
                    .map_err(|erreur| map_exec_error(&self.driver, StatementIntent::Read, erreur))
            })
            .await?;
        }

        // Le SQL de l'utilisateur part **tel quel** : c'est la fonctionnalité
        // d'un outil professionnel, et la distinction avec le SQL qu'Oxyn
        // compose est ce que garde I-10. Rien n'est concaténé ici : le texte
        // est seulement lu, pour savoir si sa connexion pourra être rendue.
        let opens_transaction = opens_transaction(&text);
        let sql = AssertSqlSafe(text).into_sql_str();
        let prepare = race_cancel(cancel, async {
            (&mut *connexion)
                .prepare(sql)
                .await
                // `Bound::Internal`, quelles que soient les valeurs de la
                // demande : `prepare` n'émet que Parse et Describe, qui portent
                // le texte SQL et les OID des paramètres — jamais leur contenu,
                // qui ne part qu'au Bind, dans `cursor.rs`. Le serveur ne peut
                // donc rien citer ici, et retirer son message coûterait le
                // diagnostic le plus utile d'une requête paramétrée : le nom de
                // l'objet qui n'existe pas.
                .map_err(|erreur| {
                    map_stream_error(
                        &self.driver,
                        intent,
                        limits.read_only,
                        Bound::Internal,
                        erreur,
                    )
                })
        })
        .await;

        // Une préparation qui échoue ferme la connexion si elle est déjà sale :
        // la transaction ouverte au-dessus garderait des verrous et bloquerait
        // le `VACUUM` de toute la base, et le `search_path` d'une console
        // partirait avec elle. Il suffit d'une faute de frappe sur un nom de
        // table dans une console en écriture.
        let statement = prepare?;
        // L'instruction de l'utilisateur va s'exécuter, et elle peut elle-même
        // changer l'état de session (`SET standard_conforming_strings = off`) :
        // c'est le curseur qui décidera si la connexion peut repartir.
        connexion.taint();

        let (schema, decodings) = schema_for(statement.columns());
        let handle = StatementHandle::new();
        // Un **enfant** du jeton de l'appelant : annuler l'appelant annule cette
        // exécution, mais annuler celle-ci n'annule pas les autres onglets.
        let execution = cancel.child();
        let verdict = self.statements.register(handle, execution.clone());
        let curseur = cursor::spawn(
            StreamRequest {
                driver: self.driver.clone(),
                connection: connexion,
                statement,
                arguments,
                bound,
                restore_context,
                opens_transaction,
                schema,
                decodings,
                limits,
                intent,
                backend_pid: pid,
                canceller: Arc::clone(&self.canceller),
                statements: Arc::clone(&self.statements),
                verdict,
                handle,
            },
            execution,
        );
        Ok(Box::new(curseur))
    }

    /// Demande au serveur d'interrompre une exécution.
    ///
    /// L'annulation part de la tâche de flux de l'exécution, qui tient sa
    /// connexion pendant l'envoi : aucune autre requête ne peut démarrer sur ce
    /// processus serveur avant qu'elle soit partie. L'appel rend quand la tâche
    /// a statué.
    ///
    /// Annuler une instruction déjà terminée n'est pas une erreur, et n'envoie
    /// rien au serveur.
    ///
    /// # Erreurs
    /// Celles de `BackendCanceller::cancel_backend`, rapportées par la tâche.
    async fn cancel(&self, statement: StatementHandle) -> Result<()> {
        self.statements.cancel(&self.driver, statement).await
    }

    async fn preview_request(
        &self,
        path: &oxyn_catalog::CatalogPath,
        limit: u32,
        shape: &oxyn_core::PreviewShape,
        cancel: &CancelToken,
    ) -> Result<ExecRequest> {
        self.catalog
            .preview_request(path, limit, shape, cancel)
            .await
    }

    fn catalog(&self) -> &dyn CatalogProvider {
        &self.catalog
    }

    /// Mesure un aller-retour complet vers le serveur.
    ///
    /// # Erreurs
    /// Toute erreur de transport. Une session dont le `ping` échoue est
    /// considérée comme perdue.
    async fn ping(&self) -> Result<Duration> {
        let depart = Instant::now();
        let mut connexion = self
            .pool
            .acquire()
            .await
            .map_err(|erreur| map_connect_error(&erreur))?;
        connexion
            .ping()
            .await
            .map_err(|erreur| map_connect_error(&erreur))?;
        Ok(depart.elapsed())
    }

    /// Ferme le bassin.
    ///
    /// Les curseurs encore vivants ont chacun leur connexion, marquée à fermer :
    /// ils s'arrêtent d'eux-mêmes quand on les détruit.
    ///
    /// # Erreurs
    /// Aucune : `sqlx` ferme au mieux et ne signale rien. La signature reste
    /// faillible parce que le trait la partage avec des drivers qui, eux, ont
    /// quelque chose à dire.
    async fn close(self: Box<Self>) -> Result<()> {
        self.pool.close().await;
        Ok(())
    }
}

/// Fait courir un futur contre l'annulation.
///
/// Le jeton gagne les égalités (`biased`) : une annulation déjà demandée ne doit
/// pas attendre qu'une opération lente veuille bien se terminer.
pub(crate) async fn race_cancel<T, F>(cancel: &CancelToken, operation: F) -> Result<T>
where
    F: Future<Output = Result<T>>,
{
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(OxynError::Cancelled),
        issue = operation => issue,
    }
}

/// Encode les paramètres liés d'une requête.
///
/// Le sens « écriture » de la table de correspondance des types. Ce qui n'y
/// figure pas est **refusé**, jamais converti au jugé : lier un décimal exact
/// comme un flottant corromprait des montants, et lier un tableau hétérogène
/// n'aurait pas de type d'élément à déclarer.
///
/// # Erreurs
/// [`OxynError::NotSupported`] pour un type que le driver ne sait pas encoder,
/// [`OxynError::Internal`] si `sqlx` refuse l'encodage — ce qui serait un bug.
pub(crate) fn bind_params(params: &[ScalarValue]) -> Result<PgArguments> {
    let mut arguments = PgArguments::default();
    arguments.reserve(params.len(), 0);

    for (rang, valeur) in params.iter().enumerate() {
        let issue = match valeur {
            // Un `NULL` s'encode par une longueur de −1, indépendamment du type :
            // le type déclaré ici n'atteint jamais le serveur, puisque
            // l'instruction est préparée séparément et que c'est le serveur qui
            // a inféré les types de ses paramètres.
            ScalarValue::Null => arguments.add(Option::<&str>::None),
            ScalarValue::Bool(v) => arguments.add(*v),
            ScalarValue::Int64(v) => arguments.add(*v),
            ScalarValue::Float64(v) => arguments.add(*v),
            ScalarValue::Text(v) => arguments.add(v.as_str()),
            ScalarValue::Bytes(v) => arguments.add(v.as_slice()),
            ScalarValue::Uuid(v) => arguments.add(*v),
            ScalarValue::Date(v) => arguments.add(*v),
            ScalarValue::Time(v) => arguments.add(*v),
            ScalarValue::Timestamp(v) => arguments.add(*v),
            ScalarValue::TimestampNaive(v) => arguments.add(*v),
            ScalarValue::Json(v) => arguments.add(sqlx::types::Json(v)),
            ScalarValue::Interval {
                months,
                days,
                nanos,
            } => {
                // PostgreSQL stocke les intervalles à la microseconde. Tronquer
                // les nanosecondes restantes changerait la valeur en silence ;
                // refuser le dit.
                if nanos % 1_000 != 0 {
                    return Err(unsupported_param(
                        rang,
                        "an interval finer than a microsecond",
                    ));
                }
                let microseconds = nanos / 1_000;
                arguments.add(sqlx::postgres::types::PgInterval {
                    months: *months,
                    days: *days,
                    microseconds,
                })
            }
            // Un décimal exact n'a pas d'encodeur dans `sqlx` sans `bigdecimal`
            // ni `rust_decimal`, aucun des deux au contrat de dépendances. Le
            // lier comme du texte marcherait par hasard sur certaines colonnes
            // et échouerait sur les autres.
            ScalarValue::Decimal(_) => {
                return Err(unsupported_param(rang, "an exact decimal value"));
            }
            // Un tableau vide n'a pas de type d'élément, et un tableau
            // hétérogène n'en a pas un seul.
            ScalarValue::Array(_) => {
                return Err(unsupported_param(rang, "an array"));
            }
        };
        // Le message de l'encodeur `sqlx` est abandonné : il est composé à
        // partir de la valeur, et cette erreur est affichée puis persistée
        // (I-03). Le rang et le type suffisent à situer le défaut, qui est un
        // bug du driver et non une donnée de l'utilisateur.
        issue.map_err(|_| {
            OxynError::Internal(format!(
                "parameter ${} of type `{}` could not be encoded",
                rang.saturating_add(1),
                valeur.type_name()
            ))
        })?;
    }
    Ok(arguments)
}

/// L'erreur d'un paramètre que le driver ne sait pas encoder.
///
/// Nomme le **rang** du paramètre, jamais sa valeur
/// ([I-03](../../../CLAUDE.md#i-03)).
fn unsupported_param(rang: usize, quoi: &str) -> OxynError {
    OxynError::NotSupported {
        capability: format!(
            "binding {quoi} as parameter ${} — cast it in the query instead",
            rang.saturating_add(1)
        ),
    }
}

/// Le pid du processus serveur d'une connexion.
///
/// Extrait pour que la connexion d'annulation et celle d'exécution partagent
/// exactement la même requête.
pub(crate) async fn backend_pid(
    connection: &mut PgConnection,
) -> std::result::Result<i32, sqlx::Error> {
    let ligne = sqlx::query(SQL_BACKEND_PID).fetch_one(connection).await?;
    ligne.try_get::<i32, _>(0)
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, NaiveTime};
    use oxyn_core::{ConnectionConfig, QueryLanguage};

    use crate::cancel::SQL_CANCEL_BACKEND;
    use oxyn_driver::Credentials;
    use sqlx::postgres::PgPoolOptions;

    use super::*;
    use crate::driver::postgres_metadata;

    /// Une session dont le bassin n'a jamais rien ouvert.
    ///
    /// `connect_lazy_with` n'établit aucune connexion : ce qui se teste ici est
    /// l'instruction que la session **compose**, et la composer ne demande pas
    /// de serveur. Le comportement contre un vrai serveur est éprouvé par les
    /// tests `#[ignore]` de [`crate::integration`].
    fn session_hors_ligne() -> PostgresSession {
        let config = ConnectionConfig::new("essai", DriverId::postgres())
            .with_param("host", "127.0.0.1")
            .with_param("database", "caisse")
            .with_param("user", "lecture");
        let spec = ConnectSpec::from_config(&postgres_metadata(), &config, &Credentials::new())
            .expect("configuration complète");
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy_with(spec.options().clone());
        PostgresSession::new(
            DriverId::postgres(),
            pool,
            spec,
            PostgresVariant::detect("PostgreSQL 17.11", "17.11", Vec::new()),
            "caisse".to_owned(),
        )
    }

    /// Force le contexte retenu sans passer par le serveur.
    ///
    /// `set_context` vérifie l'existence du schéma, donc demande une connexion ;
    /// ce test-ci ne porte que sur la composition de l'instruction.
    fn poser(session: &PostgresSession, contexte: Option<SessionContext>) {
        *session
            .context
            .lock()
            .expect("aucun autre fil ne tient ce verrou") = contexte;
    }

    #[tokio::test]
    async fn sans_contexte_declare_aucune_instruction_n_est_posee() {
        // La connexion reste dans l'état où le serveur l'a ouverte : poser un
        // `SET` ici serait exactement l'état de session invisible que le contrat
        // refuse.
        let session = session_hors_ligne();
        assert_eq!(session.context_statement(), None);
    }

    #[tokio::test]
    async fn un_namespace_declare_est_cite_jamais_concatene() {
        // I-10 : un nom de schéma vient du catalogue, donc du serveur. Le
        // guillemet doublé est ce qui sépare un `SET` d'une exécution arbitraire.
        let session = session_hors_ligne();

        poser(
            &session,
            Some(SessionContext::new(None, Some("analytics".to_owned()))),
        );
        assert_eq!(
            session.context_statement().as_deref(),
            Some(r#"SET search_path TO "analytics""#)
        );

        poser(
            &session,
            Some(SessionContext::new(
                None,
                Some(r#"oxyn_ctx"weird"#.to_owned()),
            )),
        );
        assert_eq!(
            session.context_statement().as_deref(),
            Some(r#"SET search_path TO "oxyn_ctx""weird""#)
        );
    }

    #[tokio::test]
    async fn le_controle_de_transaction_est_refuse_sans_rien_envoyer() {
        // Sans `TRANSACTIONS`, un `ROLLBACK` accepté « réussirait » côté
        // serveur sans rien annuler. Le refus part avant tout emprunt : le
        // bassin paresseux n'ouvre jamais la moindre connexion.
        let session = session_hors_ligne();
        assert!(!session.capabilities().contains(Capabilities::TRANSACTIONS));
        for texte in [
            "BEGIN",
            "START TRANSACTION",
            "COMMIT",
            "END",
            "ROLLBACK",
            "ABORT",
            "SAVEPOINT s",
            "RELEASE SAVEPOINT s",
            "PREPARE TRANSACTION 'x'",
            "/* annuler */ rollback",
        ] {
            let demande = ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), texte);
            let erreur = match session.execute(demande, &CancelToken::new()).await {
                Ok(_) => panic!("`{texte}` doit être refusé"),
                Err(erreur) => erreur,
            };
            assert!(
                matches!(erreur, OxynError::NotSupported { .. }),
                "`{texte}` : {erreur:?}"
            );
            assert!(
                erreur
                    .to_string()
                    .contains("transactions are not supported in the console yet: each statement commits on its own"),
                "{erreur}"
            );
        }
        assert_eq!(
            session.pool.size(),
            0,
            "aucune connexion ne doit avoir été ouverte"
        );
    }

    #[tokio::test]
    async fn revenir_au_defaut_pose_une_instruction_plutot_que_rien() {
        // Ne rien poser laisserait la connexion du bassin sur le `search_path`
        // d'une exécution précédente : une requête sur deux résoudrait ailleurs.
        let session = session_hors_ligne();
        poser(&session, Some(SessionContext::server_default()));
        assert_eq!(
            session.context_statement().as_deref(),
            Some("SET search_path TO DEFAULT")
        );
        // Poser le défaut et le **défaire** au retour au bassin sont deux
        // chemins distincts, portés par deux textes distincts. Les laisser
        // diverger ne casserait rien à la compilation.
        assert_eq!(
            session.context_statement().as_deref(),
            Some(SQL_RESET_SEARCH_PATH)
        );
    }

    #[test]
    fn les_types_scalaires_usuels_se_lient() {
        let params = vec![
            ScalarValue::Null,
            ScalarValue::Bool(true),
            ScalarValue::Int64(42),
            ScalarValue::Float64(1.5),
            ScalarValue::Text("caisse".to_owned()),
            ScalarValue::Bytes(vec![1, 2, 3]),
            ScalarValue::Uuid(uuid::Uuid::nil()),
            ScalarValue::Date(NaiveDate::from_ymd_opt(2026, 9, 5).expect("date de test valide")),
            ScalarValue::Time(NaiveTime::from_hms_opt(14, 30, 0).expect("heure de test valide")),
        ];
        let arguments = bind_params(&params).expect("tous ces types s'encodent");
        assert_eq!(arguments.len(), params.len());
    }

    #[test]
    fn un_intervalle_a_la_microseconde_se_lie() {
        let params = vec![ScalarValue::Interval {
            months: 1,
            days: 2,
            nanos: 3_000_000,
        }];
        assert!(bind_params(&params).is_ok());
    }

    #[test]
    fn un_intervalle_plus_fin_que_la_microseconde_est_refuse_pas_tronque() {
        // Tronquer changerait la valeur sans que rien ne le signale.
        let params = vec![ScalarValue::Interval {
            months: 0,
            days: 0,
            nanos: 1,
        }];
        let erreur = bind_params(&params).expect_err("refus attendu");
        assert!(
            matches!(erreur, OxynError::NotSupported { .. }),
            "{erreur:?}"
        );
        assert!(erreur.is_user_error());
    }

    #[test]
    fn un_decimal_exact_est_refuse_plutot_que_lie_comme_un_flottant() {
        // C'est la perte que la table de types interdit dans les deux sens.
        let params = vec![ScalarValue::Decimal("12345678901234567890.12".to_owned())];
        let erreur = bind_params(&params).expect_err("refus attendu");
        assert!(
            matches!(erreur, OxynError::NotSupported { .. }),
            "{erreur:?}"
        );
        assert!(erreur.to_string().contains("$1"), "{erreur}");
    }

    #[test]
    fn un_message_de_refus_ne_reprend_jamais_la_valeur() {
        // I-03 : un paramètre lié est exactement ce qu'on ne journalise pas.
        let secret = "4111111111111111";
        let params = vec![ScalarValue::Decimal(secret.to_owned())];
        let erreur = bind_params(&params).expect_err("refus attendu");
        assert!(!erreur.to_string().contains(secret), "fuite : {erreur}");
    }

    #[test]
    fn un_tableau_est_refuse_faute_de_type_d_element() {
        let params = vec![ScalarValue::Array(vec![ScalarValue::Int64(1)])];
        let erreur = bind_params(&params).expect_err("refus attendu");
        assert!(
            matches!(erreur, OxynError::NotSupported { .. }),
            "{erreur:?}"
        );
    }

    #[test]
    fn le_rang_du_parametre_fautif_est_celui_que_lit_l_utilisateur() {
        // PostgreSQL numérote ses emplacements à partir de $1.
        let params = vec![
            ScalarValue::Int64(1),
            ScalarValue::Decimal("1.5".to_owned()),
        ];
        let erreur = bind_params(&params).expect_err("refus attendu");
        assert!(erreur.to_string().contains("$2"), "{erreur}");
    }

    #[test]
    fn le_sql_compose_par_le_driver_ne_concatene_rien() {
        // I-10 : ces six littéraux sont tout ce que le driver compose ici.
        // Un identifiant ou une valeur qui s'y glisserait serait un `{}` visible.
        for compose in [
            SQL_BEGIN_READ_ONLY,
            SQL_ROLLBACK,
            SQL_RESET_SEARCH_PATH,
            SQL_RESET_AFTER_WRITE,
            SQL_BACKEND_PID,
            SQL_CANCEL_BACKEND,
        ] {
            assert!(!compose.contains('{'), "{compose}");
            assert!(!compose.contains("' ||"), "{compose}");
        }
        assert!(
            SQL_CANCEL_BACKEND.contains("$1"),
            "le pid doit être un paramètre lié"
        );
    }
}
