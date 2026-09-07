//! L'introspection, par `pg_catalog` et non par `information_schema`.
//!
//! # Pourquoi pas `information_schema`
//!
//! Les vues d'`information_schema` sont normalisées, lisibles… et construites
//! par-dessus `pg_catalog` avec des jointures et des appels de fonction qui ne
//! se planifient pas bien. Sur un schéma à 20 000 objets, la même liste de
//! tables se paie en dizaines de secondes plutôt qu'en dizaines de
//! millisecondes. `pg_catalog` est le chemin direct.
//!
//! # Paresseuse et hiérarchique
//!
//! Une requête par palier, jamais l'arbre entier : on descend quand
//! l'utilisateur ouvre un nœud (ARCHITECTURE §6). Rien n'est mis en cache ici —
//! c'est le rôle de [`CatalogCache`](oxyn_catalog::CatalogCache).
//!
//! # Ce que ce module ne compose jamais
//!
//! **Aucun identifiant reçu n'entre dans le texte d'une requête.** Les noms de
//! schéma et de relation sont des **valeurs liées** (`$1`, `$2`). Une table
//! nommée `"users"; DROP TABLE audit; --` existe légalement dans PostgreSQL, et
//! un aperçu construit par concaténation exécuterait la suppression au simple
//! clic dans l'arborescence ([I-10](../../../CLAUDE.md#i-10)).
//!
//! # Une connexion ne voit qu'une base
//!
//! PostgreSQL n'autorise pas l'introspection croisée : depuis une connexion à
//! `caisse`, les tables de `entrepot` sont inaccessibles. Demander l'un depuis
//! l'autre rend [`OxynError::CatalogUnavailable`] — pas une liste vide, qui
//! affirmerait qu'il n'y a rien.

use std::sync::Arc;

use async_trait::async_trait;
use oxyn_catalog::CatalogProvider;
use oxyn_catalog::model::{
    CatalogRef, Field, ForeignKey, ForeignKeyTarget, Index, LogicalType, NamespaceRef,
    ReferentialAction, Relation, RelationKind, RelationRef, ServerInfo,
};
use oxyn_catalog::path::CatalogPath;
use oxyn_core::{CancelToken, Capabilities, DriverId, OxynError, Result, StatementIntent};
use sqlx::Row as _;
use sqlx::postgres::{PgPool, PgRow};

use crate::error::{map_connect_error, map_exec_error};
use crate::session::{BackendCanceller, backend_pid};
use crate::variant::PostgresVariant;

/// Les bases accessibles sur ce serveur.
const SQL_CATALOGS: &str = "\
SELECT d.datname::text, (d.datname = current_database()) \
FROM pg_catalog.pg_database d \
WHERE d.datallowconn AND NOT d.datistemplate \
ORDER BY d.datname";

/// Les schémas de la base courante, avec leur commentaire et leur caractère
/// système.
const SQL_NAMESPACES: &str = "\
SELECT n.nspname::text, \
       pg_catalog.obj_description(n.oid, 'pg_namespace'), \
       (n.nspname = 'information_schema' OR n.nspname LIKE 'pg\\_%') \
FROM pg_catalog.pg_namespace n \
ORDER BY n.nspname";

/// Les relations d'un schéma.
///
/// Les genres retenus sont des littéraux : tables ordinaires et partitionnées,
/// vues, vues matérialisées, tables distantes et séquences. Les index et les
/// types composites n'ont pas leur place dans une arborescence de données.
const SQL_RELATIONS: &str = "\
SELECT c.relname::text, c.relkind::text, pg_catalog.obj_description(c.oid, 'pg_class') \
FROM pg_catalog.pg_class c \
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
WHERE n.nspname = $1 AND c.relkind IN ('r', 'p', 'v', 'm', 'f', 'S') \
ORDER BY c.relname";

/// Le genre, le commentaire et la volumétrie estimée d'une relation.
///
/// `reltuples` est une **estimation** tenue par `ANALYZE`, pas un compte : elle
/// vaut −1 sur une table jamais analysée, ce que le décodage traduit par
/// « inconnu » plutôt que par zéro.
const SQL_RELATION: &str = "\
SELECT c.relkind::text, pg_catalog.obj_description(c.oid, 'pg_class'), c.reltuples \
FROM pg_catalog.pg_class c \
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
WHERE n.nspname = $1 AND c.relname = $2";

/// Les colonnes d'une relation, dans l'ordre de déclaration.
///
/// `attnum > 0` écarte les colonnes système (`ctid`, `xmin`…) ;
/// `NOT attisdropped` écarte celles qu'un `ALTER TABLE DROP COLUMN` a laissées
/// dans le catalogue.
const SQL_FIELDS: &str = "\
SELECT a.attname::text, \
       a.attnum, \
       pg_catalog.format_type(a.atttypid, a.atttypmod), \
       a.attnotnull, \
       pg_catalog.pg_get_expr(ad.adbin, ad.adrelid), \
       pg_catalog.col_description(c.oid, a.attnum), \
       COALESCE(i.indisprimary, false) \
FROM pg_catalog.pg_attribute a \
JOIN pg_catalog.pg_class c ON c.oid = a.attrelid \
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
LEFT JOIN pg_catalog.pg_attrdef ad ON ad.adrelid = a.attrelid AND ad.adnum = a.attnum \
LEFT JOIN pg_catalog.pg_index i \
       ON i.indrelid = a.attrelid AND i.indisprimary AND a.attnum = ANY(i.indkey) \
WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped \
ORDER BY a.attnum";

/// Les index d'une relation, avec leurs expressions de colonne et leur méthode.
///
/// `pg_get_indexdef(oid, rang, true)` rend l'expression de la colonne de rang
/// donné : c'est la seule forme qui décrive correctement un index fonctionnel,
/// où la « colonne » est un calcul et non un nom.
const SQL_INDEXES: &str = "\
SELECT ic.relname::text, \
       i.indisunique, \
       am.amname::text, \
       pg_catalog.pg_get_expr(i.indpred, i.indrelid), \
       ARRAY(SELECT pg_catalog.pg_get_indexdef(i.indexrelid, s.i::int, true) \
             FROM generate_series(1, i.indnatts) AS s(i)) \
FROM pg_catalog.pg_index i \
JOIN pg_catalog.pg_class ic ON ic.oid = i.indexrelid \
JOIN pg_catalog.pg_class tc ON tc.oid = i.indrelid \
JOIN pg_catalog.pg_namespace n ON n.oid = tc.relnamespace \
JOIN pg_catalog.pg_am am ON am.oid = ic.relam \
WHERE n.nspname = $1 AND tc.relname = $2 \
ORDER BY ic.relname";

/// Les clés étrangères portées par une relation.
///
/// `WITH ORDINALITY` conserve l'ordre des colonnes de la contrainte : sans lui,
/// une clé composite `(a, b)` pourrait être décrite comme `(b, a)`, ce qui
/// tracerait un diagramme faux en le présentant comme un fait.
const SQL_FOREIGN_KEYS: &str = "\
SELECT con.conname::text, \
       ARRAY(SELECT a.attname::text \
             FROM unnest(con.conkey) WITH ORDINALITY AS k(attnum, ord) \
             JOIN pg_catalog.pg_attribute a \
               ON a.attrelid = con.conrelid AND a.attnum = k.attnum \
             ORDER BY k.ord), \
       fn.nspname::text, \
       fc.relname::text, \
       ARRAY(SELECT a.attname::text \
             FROM unnest(con.confkey) WITH ORDINALITY AS k(attnum, ord) \
             JOIN pg_catalog.pg_attribute a \
               ON a.attrelid = con.confrelid AND a.attnum = k.attnum \
             ORDER BY k.ord), \
       con.confdeltype::text \
FROM pg_catalog.pg_constraint con \
JOIN pg_catalog.pg_class c ON c.oid = con.conrelid \
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
JOIN pg_catalog.pg_class fc ON fc.oid = con.confrelid \
JOIN pg_catalog.pg_namespace fn ON fn.oid = fc.relnamespace \
WHERE n.nspname = $1 AND c.relname = $2 AND con.contype = 'f' \
ORDER BY con.conname";

/// L'introspection d'une session PostgreSQL.
#[derive(Debug)]
pub struct PostgresCatalog {
    driver: DriverId,
    pool: PgPool,
    /// La base à laquelle la session est connectée. Toute autre est
    /// inaccessible.
    database: String,
    variant: PostgresVariant,
    capabilities: Capabilities,
    canceller: Arc<BackendCanceller>,
}

impl PostgresCatalog {
    /// Construit l'introspection d'une session déjà ouverte.
    #[must_use]
    pub(crate) fn new(
        driver: DriverId,
        pool: PgPool,
        database: String,
        variant: PostgresVariant,
        capabilities: Capabilities,
        canceller: Arc<BackendCanceller>,
    ) -> Self {
        Self {
            driver,
            pool,
            database,
            variant,
            capabilities,
            canceller,
        }
    }

    /// La base à laquelle cette session est connectée.
    #[must_use]
    pub fn database(&self) -> &str {
        &self.database
    }

    /// Exécute une requête d'introspection, annulable jusqu'au serveur.
    ///
    /// Le pid est capturé avant la requête : une introspection de quatre minutes
    /// qu'on abandonnerait sans le connaître resterait en cours côté serveur,
    /// connexion prise et verrou posé
    /// ([DRIVER-CONTRACT §2](../../../docs/DRIVER-CONTRACT.md)).
    ///
    // TODO(phase 1) : l'aller-retour supplémentaire pour `pg_backend_pid()` se
    // paie une fois par nœud ouvert. `sqlx` connaît déjà le pid — il arrive dans
    // le message `BackendKeyData` de la poignée de main — mais ne l'expose pas.
    // Débloque : une ouverture de nœud à un seul aller-retour.
    async fn fetch(
        &self,
        cancel: &CancelToken,
        sql: &'static str,
        params: &[&str],
    ) -> Result<Vec<PgRow>> {
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        let mut connexion = self
            .pool
            .acquire()
            .await
            .map_err(|erreur| map_connect_error(&erreur))?;
        let pid = backend_pid(&mut connexion).await.ok();

        let mut requete = sqlx::query(sql);
        for parametre in params {
            requete = requete.bind(*parametre);
        }

        let issue = tokio::select! {
            biased;
            () = cancel.cancelled() => None,
            resultat = requete.fetch_all(&mut *connexion) => Some(resultat),
        };

        match issue {
            Some(Ok(lignes)) => Ok(lignes),
            Some(Err(erreur)) => Err(map_exec_error(&self.driver, StatementIntent::Read, erreur)),
            None => {
                // Le flux a été abandonné en cours : la connexion peut porter
                // des octets non lus, elle ne retourne pas au bassin.
                connexion.close_on_drop();
                if let Some(pid) = pid
                    && let Err(erreur) = self.canceller.cancel_backend(pid).await
                {
                    tracing::warn!(
                        target: "oxyn::driver::postgres",
                        erreur = %erreur,
                        "l'introspection n'a pas pu être annulée côté serveur"
                    );
                }
                Err(OxynError::Cancelled)
            }
        }
    }

    pub(crate) fn preview_request(
        &self,
        path: &CatalogPath,
        limit: u32,
    ) -> Result<oxyn_core::ExecRequest> {
        crate::preview::request(&self.database, self.variant.flavor.dialect(), path, limit)
    }

    /// Vérifie qu'un chemin vise bien la base de cette session.
    ///
    /// # Erreurs
    /// [`OxynError::CatalogUnavailable`] si le chemin nomme une autre base :
    /// PostgreSQL n'autorise pas l'introspection croisée, et rendre une liste
    /// vide laisserait croire que la base est vide.
    fn check_catalog(&self, catalog: Option<&str>) -> Result<()> {
        match catalog {
            None => Ok(()),
            Some(nom) if nom == self.database => Ok(()),
            Some(_) => Err(OxynError::CatalogUnavailable(format!(
                "cette session est connectée à `{}` ; PostgreSQL n'autorise pas \
                 l'introspection d'une autre base — ouvrir une connexion vers elle",
                self.database
            ))),
        }
    }

    /// Le schéma désigné par un chemin, ou une erreur qui dit ce qui manque.
    fn require_namespace<'a>(&self, path: &'a CatalogPath) -> Result<&'a str> {
        self.check_catalog(path.catalog())?;
        path.namespace().ok_or_else(|| {
            OxynError::CatalogUnavailable(
                "un chemin PostgreSQL doit nommer un schéma : les relations n'existent \
                 pas au niveau du serveur"
                    .to_owned(),
            )
        })
    }

    /// Le couple (schéma, relation) désigné par un chemin.
    fn require_relation<'a>(&self, path: &'a CatalogPath) -> Result<(&'a str, &'a str)> {
        let espace = self.require_namespace(path)?;
        let relation = path.relation().ok_or_else(|| {
            OxynError::CatalogUnavailable("ce chemin ne nomme pas de relation".to_owned())
        })?;
        Ok((espace, relation))
    }
}

#[async_trait]
impl CatalogProvider for PostgresCatalog {
    /// L'identité du serveur et les capacités de la session.
    ///
    /// Aucun aller-retour : tout a été appris à la connexion. C'est appelé à
    /// chaque ouverture de l'arborescence.
    async fn server_info(&self, _cancel: &CancelToken) -> Result<ServerInfo> {
        Ok(ServerInfo::new(
            self.variant.product(),
            self.variant.server_version.clone(),
            self.capabilities,
        ))
    }

    /// Les bases du serveur.
    ///
    /// Toutes sont listées, mais une seule est introspectable : celle de la
    /// session. Les autres apparaissent pour que l'utilisateur sache qu'elles
    /// existent et puisse ouvrir une connexion vers elles.
    ///
    /// # Erreurs
    /// Toute erreur de la session, ou [`OxynError::Cancelled`].
    async fn list_catalogs(&self, cancel: &CancelToken) -> Result<Vec<CatalogRef>> {
        let lignes = self.fetch(cancel, SQL_CATALOGS, &[]).await?;
        let mut bases = Vec::with_capacity(lignes.len());
        for ligne in &lignes {
            let nom: String = read_text(ligne, 0)?;
            let courante: bool = ligne.try_get(1).unwrap_or(false);
            let mut base = CatalogRef::new(nom)?;
            if courante {
                base = base.as_default();
            }
            bases.push(base);
        }
        Ok(bases)
    }

    /// Les schémas de la base courante.
    ///
    /// Les schémas système sont **listés et marqués**, pas filtrés : c'est à
    /// l'interface de décider de les replier, et à l'utilisateur de pouvoir les
    /// ouvrir quand il en a besoin.
    ///
    /// # Erreurs
    /// [`OxynError::CatalogUnavailable`] si `catalog` nomme une autre base ;
    /// toute erreur de la session.
    async fn list_namespaces(
        &self,
        catalog: Option<&str>,
        cancel: &CancelToken,
    ) -> Result<Vec<NamespaceRef>> {
        self.check_catalog(catalog)?;
        let parent = CatalogPath::for_catalog(self.database.clone())?;

        let lignes = self.fetch(cancel, SQL_NAMESPACES, &[]).await?;
        let mut espaces = Vec::with_capacity(lignes.len());
        for ligne in &lignes {
            let nom: String = read_text(ligne, 0)?;
            let commentaire: Option<String> = ligne.try_get(1).unwrap_or(None);
            let systeme: bool = ligne.try_get(2).unwrap_or(false);

            let mut espace = NamespaceRef::new(parent.clone(), nom)?;
            if let Some(texte) = commentaire {
                espace = espace.with_comment(texte);
            }
            if systeme {
                espace = espace.as_system();
            }
            espaces.push(espace);
        }
        Ok(espaces)
    }

    /// Les relations d'un schéma.
    ///
    /// # Erreurs
    /// [`OxynError::CatalogUnavailable`] si le chemin ne nomme pas de schéma ou
    /// nomme une autre base ; toute erreur de la session.
    async fn list_relations(
        &self,
        namespace: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<RelationRef>> {
        let espace = self.require_namespace(namespace)?;
        let lignes = self.fetch(cancel, SQL_RELATIONS, &[espace]).await?;

        let mut relations = Vec::with_capacity(lignes.len());
        for ligne in &lignes {
            let nom: String = read_text(ligne, 0)?;
            let genre: String = read_text(ligne, 1)?;
            let commentaire: Option<String> = ligne.try_get(2).unwrap_or(None);

            let Some(genre) = relation_kind(&genre) else {
                // Un `relkind` inconnu vient d'une version plus récente que ce
                // driver : l'ignorer vaut mieux que le ranger au hasard.
                continue;
            };
            let mut relation = RelationRef::new(namespace.clone(), nom, genre)?;
            if let Some(texte) = commentaire {
                relation = relation.with_comment(texte);
            }
            relations.push(relation);
        }
        Ok(relations)
    }

    /// La description complète d'une relation.
    ///
    /// Deux allers-retours : la relation, puis ses colonnes. Les fusionner ferait
    /// répéter le commentaire et la volumétrie sur chaque ligne de colonne.
    ///
    /// # Erreurs
    /// [`OxynError::CatalogUnavailable`] si le chemin ne nomme pas de relation,
    /// ou si la relation n'existe pas ; toute erreur de la session.
    async fn describe_relation(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Relation> {
        let (espace, nom) = self.require_relation(relation)?;

        let entetes = self.fetch(cancel, SQL_RELATION, &[espace, nom]).await?;
        let Some(entete) = entetes.first() else {
            return Err(OxynError::CatalogUnavailable(
                "cette relation n'existe pas, ou le compte n'a pas le droit de la voir".to_owned(),
            ));
        };

        let genre: String = read_text(entete, 0)?;
        let commentaire: Option<String> = entete.try_get(1).unwrap_or(None);
        let estimation: Option<f32> = entete.try_get(2).ok();

        let mut decrite = Relation::new(nom, relation_kind(&genre).unwrap_or(RelationKind::Table));
        if let Some(texte) = commentaire {
            decrite = decrite.with_comment(texte);
        }
        // `reltuples` vaut −1 sur une table jamais analysée : c'est « inconnu »,
        // et l'annoncer comme zéro ferait croire à une table vide.
        if let Some(lignes) = estimation.filter(|valeur| *valeur >= 0.0) {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let arrondi = lignes.round().max(0.0) as u64;
            decrite = decrite.with_estimated_rows(arrondi);
        }

        let colonnes = self.fetch(cancel, SQL_FIELDS, &[espace, nom]).await?;
        let mut champs = Vec::with_capacity(colonnes.len());
        for ligne in &colonnes {
            let nom_champ: String = read_text(ligne, 0)?;
            let rang: i16 = ligne.try_get(1).unwrap_or(0);
            let brut: String = ligne
                .try_get::<Option<String>, _>(2)
                .ok()
                .flatten()
                .unwrap_or_else(|| "unknown".to_owned());
            let non_nul: bool = ligne.try_get(3).unwrap_or(false);
            let defaut: Option<String> = ligne.try_get(4).unwrap_or(None);
            let commentaire: Option<String> = ligne.try_get(5).unwrap_or(None);
            let cle_primaire: bool = ligne.try_get(6).unwrap_or(false);

            // `attnum` commence à 1 ; les positions du modèle commencent à 0.
            let position = u32::try_from(rang.max(1).saturating_sub(1)).unwrap_or(0);
            let mut champ = Field::new(nom_champ, position, logical_type(&brut), brut);
            if non_nul {
                champ = champ.not_null();
            }
            if cle_primaire {
                champ = champ.primary_key();
            }
            if let Some(texte) = defaut {
                champ = champ.with_default(texte);
            }
            if let Some(texte) = commentaire {
                champ = champ.with_comment(texte);
            }
            champs.push(champ);
        }

        Ok(decrite.with_fields(champs))
    }

    /// Les index d'une relation.
    ///
    /// # Erreurs
    /// [`OxynError::NotSupported`] si la session ne déclare pas
    /// [`Capabilities::INDEXES`] ; toute erreur de la session.
    async fn list_indexes(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<Index>> {
        self.capabilities.require(Capabilities::INDEXES)?;
        let (espace, nom) = self.require_relation(relation)?;
        let lignes = self.fetch(cancel, SQL_INDEXES, &[espace, nom]).await?;

        let mut index = Vec::with_capacity(lignes.len());
        for ligne in &lignes {
            let nom_index: String = read_text(ligne, 0)?;
            let unique: bool = ligne.try_get(1).unwrap_or(false);
            let methode: Option<String> = ligne.try_get(2).unwrap_or(None);
            let predicat: Option<String> = ligne.try_get(3).unwrap_or(None);
            let colonnes: Vec<String> = ligne.try_get(4).unwrap_or_default();

            let mut decrit = Index::new(nom_index, colonnes);
            if unique {
                decrit = decrit.unique();
            }
            decrit.method = methode;
            decrit.predicate = predicat;
            index.push(decrit);
        }
        Ok(index)
    }

    /// Les clés étrangères portées par une relation.
    ///
    /// C'est ce qui permet de tracer un diagramme de relations sans le deviner ;
    /// le deviner à partir des noms de colonnes produirait des liens faux
    /// présentés comme des faits.
    ///
    /// # Erreurs
    /// [`OxynError::NotSupported`] si la session ne déclare pas
    /// [`Capabilities::FOREIGN_KEYS`] ; toute erreur de la session.
    async fn list_foreign_keys(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<ForeignKey>> {
        self.capabilities.require(Capabilities::FOREIGN_KEYS)?;
        let (espace, nom) = self.require_relation(relation)?;
        let lignes = self.fetch(cancel, SQL_FOREIGN_KEYS, &[espace, nom]).await?;

        let mut cles = Vec::with_capacity(lignes.len());
        for ligne in &lignes {
            let nom_contrainte: String = read_text(ligne, 0)?;
            let colonnes: Vec<String> = ligne.try_get(1).unwrap_or_default();
            let espace_cible: String = read_text(ligne, 2)?;
            let relation_cible: String = read_text(ligne, 3)?;
            let colonnes_cibles: Vec<String> = ligne.try_get(4).unwrap_or_default();
            let suppression: String = read_text(ligne, 5)?;

            let cible = CatalogPath::for_relation(
                Some(&self.database),
                Some(&espace_cible),
                relation_cible,
            )?;
            let mut cle = ForeignKey::new(
                nom_contrainte,
                colonnes,
                ForeignKeyTarget {
                    relation: cible,
                    fields: colonnes_cibles,
                },
            );
            cle.on_delete = referential_action(&suppression);
            cles.push(cle);
        }
        Ok(cles)
    }
}

/// Lit une colonne textuelle obligatoire.
///
/// Une colonne du catalogue qui devrait porter un nom et n'en porte pas est une
/// incohérence du serveur, pas une donnée : on le dit, sans reprendre la valeur.
fn read_text(row: &PgRow, ordinal: usize) -> Result<String> {
    row.try_get::<String, _>(ordinal).map_err(|_| {
        OxynError::CatalogUnavailable(format!(
            "la colonne {ordinal} du catalogue n'a pas rendu de texte lisible"
        ))
    })
}

/// Traduit un `relkind` de `pg_class`.
///
/// Une valeur inconnue rend `None` : elle vient d'une version de PostgreSQL plus
/// récente que ce driver, et la ranger au hasard vaudrait moins que l'ignorer.
#[must_use]
fn relation_kind(relkind: &str) -> Option<RelationKind> {
    let genre = match relkind {
        // `r` ordinaire, `p` partitionnée, `f` distante : trois façons d'être
        // une table du point de vue de qui la lit.
        "r" | "p" | "f" => RelationKind::Table,
        "v" => RelationKind::View,
        "m" => RelationKind::MaterializedView,
        "S" => RelationKind::Sequence,
        "i" | "I" => RelationKind::Index,
        _ => return None,
    };
    Some(genre)
}

/// Traduit un `confdeltype` de `pg_constraint`.
///
/// Une valeur inconnue vaut [`ReferentialAction::NoAction`], qui est le défaut
/// de la norme SQL et le comportement le moins destructeur.
#[must_use]
fn referential_action(confdeltype: &str) -> ReferentialAction {
    match confdeltype {
        "r" => ReferentialAction::Restrict,
        "c" => ReferentialAction::Cascade,
        "n" => ReferentialAction::SetNull,
        "d" => ReferentialAction::SetDefault,
        _ => ReferentialAction::NoAction,
    }
}

/// Traduit le rendu de `format_type` vers le type logique du catalogue.
///
/// `format_type` est la façon dont PostgreSQL **écrit** un type : `integer`,
/// `character varying(50)`, `numeric(10,2)`, `timestamp with time zone`,
/// `integer[]`. C'est un rendu stable, et le seul qui porte la précision et
/// l'échelle — que l'OID seul ne donne pas.
///
/// Ce qui n'est pas reconnu vaut [`LogicalType::Unknown`], jamais un type
/// approchant : `raw_type` conserve de toute façon le rendu exact.
#[must_use]
pub fn logical_type(raw: &str) -> LogicalType {
    let normalise = raw.trim().to_ascii_lowercase();

    if let Some(element) = normalise.strip_suffix("[]") {
        return LogicalType::Array(Box::new(logical_type(element)));
    }

    // Les types temporels se reconnaissent **avant** le découpage sur la
    // parenthèse : `format_type` écrit `timestamp(3) with time zone`, dont la
    // partie qui compte — le fuseau — est après la précision. La perdre
    // décalerait la donnée de deux heures sans que rien ne le signale
    // (DRIVER-CONTRACT §7).
    if normalise.starts_with("timestamp") {
        return LogicalType::Timestamp {
            tz: normalise.contains("with time zone"),
        };
    }
    if normalise.starts_with("time") {
        return LogicalType::Time;
    }

    // La partie avant la première parenthèse : `numeric(10,2)` → `numeric`.
    let (base, parametres) = match normalise.split_once('(') {
        Some((base, reste)) => (base.trim(), reste.strip_suffix(')').unwrap_or(reste)),
        None => (normalise.as_str(), ""),
    };

    match base {
        "boolean" | "bool" => LogicalType::Boolean,
        "smallint" | "int2" | "smallserial" => LogicalType::Integer { bits: 16 },
        "integer" | "int" | "int4" | "serial" => LogicalType::INT32,
        "bigint" | "int8" | "bigserial" => LogicalType::INT64,
        "real" | "float4" => LogicalType::Float { bits: 32 },
        "double precision" | "float8" => LogicalType::FLOAT64,
        "numeric" | "decimal" => {
            let (precision, scale) = decimal_params(parametres);
            LogicalType::Decimal { precision, scale }
        }
        "text" | "character varying" | "varchar" | "character" | "char" | "name" | "citext"
        | "xml" | "inet" | "cidr" | "macaddr" | "macaddr8" | "money" => LogicalType::Text,
        "bytea" => LogicalType::Bytes,
        "uuid" => LogicalType::Uuid,
        "date" => LogicalType::Date,
        "interval" => LogicalType::Interval,
        "json" | "jsonb" => LogicalType::Json,
        "vector" | "halfvec" => LogicalType::Vector {
            dims: parametres.trim().parse().ok(),
        },
        "geometry" | "geography" => LogicalType::Geometry,
        _ => LogicalType::Unknown,
    }
}

/// Lit la précision et l'échelle d'un `numeric(p, s)`.
fn decimal_params(parametres: &str) -> (Option<u16>, Option<i16>) {
    let mut morceaux = parametres.split(',');
    let precision = morceaux.next().and_then(|p| p.trim().parse().ok());
    let scale = morceaux.next().and_then(|s| s.trim().parse().ok());
    (precision, scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_sql_d_introspection_ne_concatene_aucun_identifiant() {
        // I-10 : une table nommée `"users"; DROP TABLE audit; --` est légale.
        // Toutes les requêtes qui visent un objet nommé le font par `$1`/`$2`.
        for (nom, requete) in [
            ("relations", SQL_RELATIONS),
            ("relation", SQL_RELATION),
            ("champs", SQL_FIELDS),
            ("index", SQL_INDEXES),
            ("clés étrangères", SQL_FOREIGN_KEYS),
        ] {
            assert!(
                requete.contains("$1"),
                "{nom} : le schéma doit être un paramètre lié"
            );
            assert!(!requete.contains("{}"), "{nom} : rien n'est formaté");
            assert!(!requete.contains("' ||"), "{nom} : rien n'est concaténé");
        }
        for requete in [SQL_CATALOGS, SQL_NAMESPACES] {
            assert!(
                !requete.contains("$1"),
                "ces deux-là ne visent rien de nommé"
            );
        }
    }

    #[test]
    fn l_introspection_passe_par_pg_catalog_pas_par_information_schema() {
        // Des dizaines de secondes contre des dizaines de millisecondes sur un
        // schéma à 20 000 objets.
        for requete in [
            SQL_CATALOGS,
            SQL_NAMESPACES,
            SQL_RELATIONS,
            SQL_RELATION,
            SQL_FIELDS,
            SQL_INDEXES,
            SQL_FOREIGN_KEYS,
        ] {
            assert!(
                !requete.contains("information_schema.")
                    || requete.contains("nspname = 'information_schema'"),
                "{requete}"
            );
        }
    }

    #[test]
    fn les_genres_de_relation_se_traduisent() {
        assert_eq!(relation_kind("r"), Some(RelationKind::Table));
        assert_eq!(relation_kind("p"), Some(RelationKind::Table));
        assert_eq!(relation_kind("f"), Some(RelationKind::Table));
        assert_eq!(relation_kind("v"), Some(RelationKind::View));
        assert_eq!(relation_kind("m"), Some(RelationKind::MaterializedView));
        assert_eq!(relation_kind("S"), Some(RelationKind::Sequence));
    }

    #[test]
    fn un_genre_inconnu_est_ignore_plutot_que_range_au_hasard() {
        // `c` est un type composite : ce n'est pas une relation de données.
        assert_eq!(relation_kind("c"), None);
        assert_eq!(relation_kind("z"), None);
        assert_eq!(relation_kind(""), None);
    }

    #[test]
    fn les_actions_referentielles_se_traduisent_et_le_defaut_est_le_moins_destructeur() {
        assert_eq!(referential_action("c"), ReferentialAction::Cascade);
        assert_eq!(referential_action("n"), ReferentialAction::SetNull);
        assert_eq!(referential_action("r"), ReferentialAction::Restrict);
        assert_eq!(referential_action("d"), ReferentialAction::SetDefault);
        assert_eq!(referential_action("a"), ReferentialAction::NoAction);
        assert_eq!(referential_action("?"), ReferentialAction::NoAction);
        assert!(!ReferentialAction::NoAction.propagates_delete());
    }

    #[test]
    fn les_types_usuels_se_lisent_dans_le_rendu_de_format_type() {
        assert_eq!(logical_type("integer"), LogicalType::INT32);
        assert_eq!(logical_type("bigint"), LogicalType::INT64);
        assert_eq!(logical_type("smallint"), LogicalType::Integer { bits: 16 });
        assert_eq!(logical_type("boolean"), LogicalType::Boolean);
        assert_eq!(logical_type("text"), LogicalType::Text);
        assert_eq!(logical_type("character varying(50)"), LogicalType::Text);
        assert_eq!(logical_type("bytea"), LogicalType::Bytes);
        assert_eq!(logical_type("uuid"), LogicalType::Uuid);
        assert_eq!(logical_type("jsonb"), LogicalType::Json);
        assert_eq!(logical_type("interval"), LogicalType::Interval);
    }

    #[test]
    fn un_horodatage_garde_la_distinction_avec_ou_sans_fuseau() {
        // C'est la distinction dont la perte décale des données de deux heures.
        assert_eq!(
            logical_type("timestamp without time zone"),
            LogicalType::Timestamp { tz: false }
        );
        assert_eq!(
            logical_type("timestamp with time zone"),
            LogicalType::TIMESTAMPTZ
        );
        // La précision se glisse **entre** le mot et le fuseau : découper sur la
        // parenthèse avant de chercher « with time zone » perdrait le fuseau.
        assert_eq!(
            logical_type("timestamp(3) with time zone"),
            LogicalType::TIMESTAMPTZ
        );
        assert_eq!(logical_type("time(6) without time zone"), LogicalType::Time);
    }

    #[test]
    fn un_numeric_conserve_sa_precision_et_son_echelle() {
        assert_eq!(
            logical_type("numeric(10,2)"),
            LogicalType::Decimal {
                precision: Some(10),
                scale: Some(2),
            }
        );
        assert_eq!(
            logical_type("numeric"),
            LogicalType::Decimal {
                precision: None,
                scale: None,
            }
        );
    }

    #[test]
    fn un_tableau_se_lit_comme_un_tableau_de_son_element() {
        assert_eq!(
            logical_type("integer[]"),
            LogicalType::Array(Box::new(LogicalType::INT32))
        );
        assert_eq!(
            logical_type("character varying(20)[]"),
            LogicalType::Array(Box::new(LogicalType::Text))
        );
    }

    #[test]
    fn pgvector_se_lit_avec_sa_dimension() {
        assert_eq!(
            logical_type("vector(1536)"),
            LogicalType::Vector { dims: Some(1536) }
        );
        assert_eq!(logical_type("vector"), LogicalType::Vector { dims: None });
    }

    #[test]
    fn un_type_inconnu_reste_inconnu_plutot_qu_approche() {
        // `raw_type` conserve le rendu exact ; inventer un type logique
        // proche ferait des promesses que le type ne tient pas.
        assert_eq!(logical_type("hstore"), LogicalType::Unknown);
        assert_eq!(logical_type("mon_type_maison"), LogicalType::Unknown);
    }
}
