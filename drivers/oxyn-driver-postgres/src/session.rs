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
//! **Le pid est capturé par exécution, pas par session.** Une session s'appuie
//! sur un bassin, donc chaque exécution tourne sur un processus serveur
//! différent. Un pid retenu à la connexion viserait, à l'annulation, une requête
//! qui n'est pas celle qu'on veut couper — au mieux personne, au pire le voisin.
//! `StatementRegistry` associe donc chaque [`StatementHandle`] au pid qui
//! l'exécute.
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

use std::collections::HashMap;
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
use sqlx::{Arguments as _, AssertSqlSafe, ConnectOptions as _, Connection as _, Executor as _};
use sqlx::{Row as _, SqlSafeStr as _, Statement as _};

use crate::catalog::PostgresCatalog;
use crate::cursor::{self, StreamRequest};
use crate::error::{Bound, map_connect_error, map_exec_error, map_stream_error};
use crate::options::ConnectSpec;
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
/// Le pid du processus serveur qui exécute sur cette connexion.
const SQL_BACKEND_PID: &str = "SELECT pg_catalog.pg_backend_pid()";

/// Le schéma demandé existe-t-il, et ce rôle le voit-il ?
///
/// `SET search_path` accepte un schéma absent sans rien dire ; c'est cette
/// requête qui fait la différence entre un contexte appliqué et un contexte
/// affiché. Le nom voyage **lié**, jamais concaténé (I-10).
const SQL_NAMESPACE_EXISTS: &str = "SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname = $1 \
     AND pg_catalog.has_schema_privilege(oid, 'USAGE')";
/// Demande au serveur d'interrompre la requête d'un autre processus.
const SQL_CANCEL_BACKEND: &str = "SELECT pg_catalog.pg_cancel_backend($1)";

/// Associe chaque exécution au processus serveur qui la porte.
///
/// Partagé entre la session — qui interroge — et les tâches de flux — qui
/// s'effacent en partant. Sans cet effacement, une session ouverte une journée
/// accumulerait une entrée par requête exécutée.
#[derive(Debug, Default)]
pub(crate) struct StatementRegistry {
    entries: Mutex<HashMap<StatementHandle, i32>>,
}

impl StatementRegistry {
    /// Retient le pid d'une exécution qui démarre.
    pub(crate) fn remember(&self, handle: StatementHandle, backend_pid: i32) {
        self.lock().insert(handle, backend_pid);
    }

    /// Oublie une exécution terminée.
    pub(crate) fn forget(&self, handle: StatementHandle) {
        self.lock().remove(&handle);
    }

    /// Le pid d'une exécution en cours, s'il en reste une.
    pub(crate) fn backend_pid(&self, handle: StatementHandle) -> Option<i32> {
        self.lock().get(&handle).copied()
    }

    /// Nombre d'exécutions en cours. Réservé au diagnostic et aux tests.
    #[doc(hidden)]
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Un verrou empoisonné ne doit pas propager la panique d'une autre tâche :
    /// la table reste exploitable, et perdre une entrée coûte moins qu'une
    /// session inutilisable ([I-09](../../../CLAUDE.md#i-09)).
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<StatementHandle, i32>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// De quoi demander au serveur d'arrêter une requête.
///
/// Porte les paramètres de connexion parce que l'annulation ouvre sa **propre**
/// connexion : emprunter celle du bassin ferait attendre l'annulation derrière
/// les requêtes qu'elle doit couper.
#[derive(Debug)]
pub(crate) struct BackendCanceller {
    spec: ConnectSpec,
    driver: DriverId,
}

impl BackendCanceller {
    /// Prépare l'annulateur d'une session.
    pub(crate) fn new(spec: ConnectSpec, driver: DriverId) -> Self {
        Self { spec, driver }
    }

    /// Demande au serveur d'interrompre la requête du processus `backend_pid`.
    ///
    /// Interrompre une requête déjà terminée n'est **pas** une erreur : le
    /// serveur rend `false` et on n'en fait rien. Ce qui compte est qu'aucune
    /// requête ne survive à la fermeture d'un onglet.
    ///
    /// # Erreurs
    /// [`OxynError::Connection`] si la seconde connexion ne s'ouvre pas,
    /// [`OxynError::Driver`] si le serveur refuse l'appel — typiquement faute de
    /// droits sur un pid appartenant à un autre rôle.
    pub(crate) async fn cancel_backend(&self, backend_pid: i32) -> Result<()> {
        let mut connexion = self
            .spec
            .options()
            .connect()
            .await
            .map_err(|erreur| map_connect_error(&erreur))?;

        let issue = sqlx::query(SQL_CANCEL_BACKEND)
            .bind(backend_pid)
            .fetch_optional(&mut connexion)
            .await;

        // Fermée dans tous les cas : cette connexion n'a plus d'usage, et la
        // laisser filer en ouvrirait une par annulation.
        let _ = connexion.close().await;

        issue.map_err(|erreur| map_exec_error(&self.driver, StatementIntent::Read, erreur))?;
        Ok(())
    }
}

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

        let mut connexion = self.acquire(cancel).await?;

        let pid = race_cancel(cancel, async {
            backend_pid(&mut connexion)
                .await
                .map_err(|erreur| map_exec_error(&self.driver, StatementIntent::Read, erreur))
        })
        .await?;

        // Avant la transaction : posé à l'intérieur, un `SET` serait défait par
        // le `ROLLBACK` qui clôt une lecture seule, et l'instruction suivante
        // sur la même connexion résoudrait ailleurs.
        let restore_context = self.context_statement().is_some();
        if let Some(statement) = self.context_statement() {
            let sql = AssertSqlSafe(statement).into_sql_str();
            race_cancel(cancel, async {
                sqlx::raw_sql(sql)
                    .execute(&mut *connexion)
                    .await
                    .map_err(|erreur| map_exec_error(&self.driver, StatementIntent::Read, erreur))
            })
            .await?;
        }

        // À partir d'ici la connexion porte peut-être un `search_path` : c'est
        // le curseur qui la remet au défaut, et lui seul. Tout départ anticipé
        // la ferme donc au lieu de la rendre au bassin — sinon le contexte
        // d'une console voyagerait vers l'introspection, et une faute de frappe
        // sur un nom de table suffirait à l'y envoyer
        // ([ADR-0019](../../../docs/adr/0019-contexte-de-session.md)). Rouvrir
        // une connexion coûte moins qu'un catalogue qui se décrit autrement
        // d'une lecture à l'autre.
        if limits.read_only {
            let ouverte = race_cancel(cancel, async {
                sqlx::raw_sql(SQL_BEGIN_READ_ONLY)
                    .execute(&mut *connexion)
                    .await
                    .map_err(|erreur| map_exec_error(&self.driver, StatementIntent::Read, erreur))
            })
            .await;
            if let Err(erreur) = ouverte {
                if restore_context {
                    connexion.close_on_drop();
                }
                return Err(erreur);
            }
        }

        // Le SQL de l'utilisateur part **tel quel** : c'est la fonctionnalité
        // d'un outil professionnel, et la distinction avec le SQL qu'Oxyn
        // compose est ce que garde I-10. Rien n'est concaténé ici.
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

        let statement = match prepare {
            Ok(statement) => statement,
            Err(erreur) => {
                // La transaction ouverte juste au-dessus ne doit pas repartir au
                // bassin : une connexion `idle in transaction` garde des verrous
                // et bloque le `VACUUM` de toute la base. Un `search_path` posé
                // pour cette exécution ne le doit pas non plus, et c'est le cas
                // le plus atteignable des deux : il suffit d'une faute de frappe
                // sur un nom de table dans une console en écriture.
                if limits.read_only || restore_context {
                    connexion.close_on_drop();
                }
                return Err(erreur);
            }
        };

        let (schema, decodings) = schema_for(statement.columns());
        let handle = StatementHandle::new();
        self.statements.remember(handle, pid);

        let curseur = cursor::spawn(
            StreamRequest {
                driver: self.driver.clone(),
                connection: connexion,
                statement,
                arguments,
                bound,
                restore_context,
                schema,
                decodings,
                limits,
                intent,
                backend_pid: pid,
                canceller: Arc::clone(&self.canceller),
                statements: Arc::clone(&self.statements),
                handle,
            },
            cancel,
        );
        Ok(Box::new(curseur))
    }

    /// Demande au serveur d'interrompre une exécution.
    ///
    /// Annuler une instruction déjà terminée n'est pas une erreur : la tâche de
    /// flux a effacé son entrée en partant, et il n'y a plus rien à couper.
    ///
    /// # Erreurs
    /// Celles de `BackendCanceller::cancel_backend`.
    async fn cancel(&self, statement: StatementHandle) -> Result<()> {
        let Some(backend_pid) = self.statements.backend_pid(statement) else {
            return Ok(());
        };
        self.canceller.cancel_backend(backend_pid).await
    }

    async fn preview_request(
        &self,
        path: &oxyn_catalog::CatalogPath,
        limit: u32,
        cancel: &CancelToken,
    ) -> Result<ExecRequest> {
        self.catalog.preview_request(path, limit, cancel).await
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
                        "un intervalle plus fin que la microseconde",
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
                return Err(unsupported_param(rang, "une valeur décimale exacte"));
            }
            // Un tableau vide n'a pas de type d'élément, et un tableau
            // hétérogène n'en a pas un seul.
            ScalarValue::Array(_) => {
                return Err(unsupported_param(rang, "un tableau"));
            }
        };
        // Le message de l'encodeur `sqlx` est abandonné : il est composé à
        // partir de la valeur, et cette erreur est affichée puis persistée
        // (I-03). Le rang et le type suffisent à situer le défaut, qui est un
        // bug du driver et non une donnée de l'utilisateur.
        issue.map_err(|_| {
            OxynError::Internal(format!(
                "le paramètre ${} de type `{}` n'a pas pu être encodé",
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
            "lier {quoi} au paramètre ${} — le convertir dans la requête",
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
    use oxyn_core::{ConnectionConfig, StatementHandle};
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
    fn le_registre_associe_une_execution_a_son_processus() {
        // Un pid retenu à la connexion viserait, au moment d'annuler, une
        // requête qui n'est pas celle qu'on veut couper.
        let registre = StatementRegistry::default();
        let une = StatementHandle::new();
        let autre = StatementHandle::new();

        registre.remember(une, 4_242);
        registre.remember(autre, 4_243);
        assert_eq!(registre.backend_pid(une), Some(4_242));
        assert_eq!(registre.backend_pid(autre), Some(4_243));
        assert_eq!(registre.len(), 2);
    }

    #[test]
    fn une_execution_terminee_ne_reste_pas_dans_le_registre() {
        // Sans cet effacement, une session ouverte une journée accumule une
        // entrée par requête exécutée.
        let registre = StatementRegistry::default();
        let handle = StatementHandle::new();
        registre.remember(handle, 7);
        registre.forget(handle);
        assert_eq!(registre.backend_pid(handle), None);
        assert_eq!(registre.len(), 0);
    }

    #[test]
    fn annuler_une_execution_inconnue_ne_doit_rien_couter() {
        // « Annuler une instruction déjà terminée n'est pas une erreur. »
        let registre = StatementRegistry::default();
        assert_eq!(registre.backend_pid(StatementHandle::new()), None);
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
        // I-10 : ces cinq littéraux sont tout ce que le driver compose ici.
        // Un identifiant ou une valeur qui s'y glisserait serait un `{}` visible.
        for compose in [
            SQL_BEGIN_READ_ONLY,
            SQL_ROLLBACK,
            SQL_RESET_SEARCH_PATH,
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
