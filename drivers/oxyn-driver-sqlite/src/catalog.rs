//! L'introspection : `sqlite_master` et les `PRAGMA`.
//!
//! # Où SQLite se range dans la hiérarchie à cinq paliers
//!
//! [ARCHITECTURE §6](../../../docs/ARCHITECTURE.md) pose
//! `Server → Catalog → Namespace → Relation → Field`, et **les paliers
//! intermédiaires sont optionnels**. SQLite n'a pas de palier catalogue : il a
//! des *bases attachées*, qui portent un nom et contiennent des tables. Elles
//! occupent donc le palier **espace de noms** :
//!
//! | Palier | SQLite |
//! |---|---|
//! | Catalog | — |
//! | Namespace | `main`, `temp`, et chaque base attachée par `ATTACH` |
//! | Relation | table ou vue de `sqlite_master` |
//!
//! Un chemin vide désigne `main` : c'est la base à laquelle la session est
//! connectée.
//!
//! # Le SQL composé ici cite ses identifiants
//!
//! Une base nommée `"x"; DROP TABLE audit; --` s'attache légalement. Le nom
//! d'une base attachée traverse donc [`quote_identifier`], et les noms de
//! relations passent en **valeurs liées** ou par la citation d'identifiant de
//! `rusqlite` (`Connection::pragma`, qui cite le schéma et échappe la valeur).
//! Aucun nom reçu du moteur n'est concaténé tel quel
//! ([I-10](../../../CLAUDE.md#i-10)).
//!
//! # Ce que SQLite ne sait pas dire, et qu'on n'invente pas
//!
//! * **Aucun commentaire d'objet.** SQLite n'a pas de `COMMENT ON`. Les champs
//!   `comment` restent `None`, et la capacité `COMMENTS` n'est pas déclarée.
//! * **Aucune estimation de volumétrie sans compter.** `estimated_rows` reste
//!   `None` — jamais `Some(0)`, qui affirmerait une table vide. Lancer un
//!   `COUNT(*)` scannerait la table à chaque rafraîchissement d'arborescence.
//! * **Aucun nom de contrainte de clé étrangère.** `PRAGMA foreign_key_list`
//!   n'en rend pas ; le nom construit (`fk_<n>`) est celui de l'ordre de
//!   déclaration, et rien d'autre.
//! * **Les tables internes `sqlite_*` sont listées** comme les autres. Les
//!   cacher demanderait de décider à la place de l'utilisateur ce qui existe.

use async_trait::async_trait;
use oxyn_catalog::model::{
    CatalogRef, Field, ForeignKey, ForeignKeyTarget, Index, LogicalType, NamespaceRef,
    ReferentialAction, Relation, RelationKind, RelationRef, ServerInfo,
};
use oxyn_catalog::path::{CatalogPath, QuoteStyle, quote_identifier};
use oxyn_catalog::provider::CatalogProvider;
use oxyn_core::{CancelToken, Capabilities, OxynError, Result};
use rusqlite::Connection;

use crate::error::{self, Effect};
use crate::worker::WorkerHandle;

/// L'espace de noms par défaut d'une session SQLite.
pub const MAIN: &str = "main";

/// L'introspection d'une session SQLite.
#[derive(Debug)]
pub struct SqliteCatalog {
    worker: WorkerHandle,
    capabilities: Capabilities,
    version: &'static str,
}

impl SqliteCatalog {
    /// Construit le catalogue d'une session.
    pub(crate) fn new(worker: WorkerHandle, capabilities: Capabilities) -> Self {
        Self {
            worker,
            capabilities,
            // La version du moteur **lié**, pas celle du fichier : c'est elle
            // qui décide de ce que la session sait faire.
            version: rusqlite::version(),
        }
    }

    /// L'espace de noms visé par un chemin, `main` à défaut.
    fn database_of(path: &CatalogPath) -> &str {
        path.namespace().unwrap_or(MAIN)
    }

    /// Le nom de relation d'un chemin.
    fn relation_of(path: &CatalogPath) -> Result<&str> {
        path.relation()
            .ok_or_else(|| OxynError::Config("le chemin ne désigne pas une relation".to_owned()))
    }
}

/// Toute introspection est une lecture : une erreur n'y est jamais ambiguë.
fn read(err: rusqlite::Error) -> OxynError {
    error::engine(err, Effect::ReadOnly)
}

#[async_trait]
impl CatalogProvider for SqliteCatalog {
    async fn server_info(&self, _cancel: &CancelToken) -> Result<ServerInfo> {
        // Aucun aller-retour : la version du moteur est celle de la
        // bibliothèque liée, connue sans interroger quoi que ce soit.
        Ok(ServerInfo::new("SQLite", self.version, self.capabilities))
    }

    /// SQLite n'a pas de palier catalogue : la liste est **vide**, ce qui dit
    /// « ce palier n'existe pas ici » et non « aucun catalogue accessible ».
    async fn list_catalogs(&self, _cancel: &CancelToken) -> Result<Vec<CatalogRef>> {
        Ok(Vec::new())
    }

    async fn list_namespaces(
        &self,
        catalog: Option<&str>,
        cancel: &CancelToken,
    ) -> Result<Vec<NamespaceRef>> {
        if catalog.is_some() {
            // Rien ne peut se trouver sous un palier qui n'existe pas.
            return Ok(Vec::new());
        }
        let names: Vec<String> = self
            .worker
            .call(cancel, |connection: &Connection| {
                let mut names = Vec::new();
                connection
                    .pragma_query(None, "database_list", |row| {
                        names.push(row.get::<_, String>("name")?);
                        Ok(())
                    })
                    .map_err(read)?;
                Ok(names)
            })
            .await?;

        names
            .into_iter()
            .map(|name| {
                // `temp` porte les objets temporaires de la session : réel, mais
                // replié par défaut dans l'arborescence.
                let is_system = name == "temp";
                let reference = NamespaceRef::new(CatalogPath::empty(), name)?;
                Ok(if is_system {
                    reference.as_system()
                } else {
                    reference
                })
            })
            .collect()
    }

    async fn list_relations(
        &self,
        namespace: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<RelationRef>> {
        let database = Self::database_of(namespace).to_owned();
        let parent = CatalogPath::for_namespace(None, database.clone())?;

        let rows: Vec<(String, String)> = self
            .worker
            .call(cancel, move |connection: &Connection| {
                let sql = format!(
                    "SELECT name, type FROM {}.sqlite_master \
                     WHERE type IN ('table', 'view') ORDER BY type, name",
                    // I-10 : le nom d'une base attachée vient de l'utilisateur.
                    quote_identifier(&database, QuoteStyle::Double)
                );
                let mut statement = connection.prepare(&sql).map_err(read)?;
                let mut rows = statement.query([]).map_err(read)?;
                let mut collected = Vec::new();
                while let Some(row) = rows.next().map_err(read)? {
                    collected.push((
                        row.get("name").map_err(read)?,
                        row.get("type").map_err(read)?,
                    ));
                }
                Ok(collected)
            })
            .await?;

        rows.into_iter()
            .map(|(name, kind)| {
                let kind = match kind.as_str() {
                    "view" => RelationKind::View,
                    _ => RelationKind::Table,
                };
                Ok(RelationRef::new(parent.clone(), name, kind)?)
            })
            .collect()
    }

    async fn describe_relation(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Relation> {
        let database = Self::database_of(relation).to_owned();
        let name = Self::relation_of(relation)?.to_owned();
        let visible = name.clone();

        let described: Option<RawRelation> = self
            .worker
            .call(cancel, move |connection: &Connection| {
                let Some(kind) = relation_kind(connection, &database, &name)? else {
                    return Ok(None);
                };
                Ok(Some(RawRelation {
                    kind,
                    columns: table_info(connection, &database, &name)?,
                }))
            })
            .await?;

        let Some(described) = described else {
            return Err(OxynError::Query(format!(
                "la relation `{visible}` n'existe pas"
            )));
        };

        let fields = described
            .columns
            .into_iter()
            .map(RawColumn::into_field)
            .collect();
        // `estimated_rows` et `comment` restent absents : voir la note de module.
        Ok(Relation::new(visible, described.kind).with_fields(fields))
    }

    async fn list_indexes(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<Index>> {
        let database = Self::database_of(relation).to_owned();
        let name = Self::relation_of(relation)?.to_owned();

        let raw: Vec<RawIndex> = self
            .worker
            .call(cancel, move |connection: &Connection| {
                index_list(connection, &database, &name)
            })
            .await?;

        Ok(raw.into_iter().map(RawIndex::into_index).collect())
    }

    async fn list_foreign_keys(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<ForeignKey>> {
        let database = Self::database_of(relation).to_owned();
        let name = Self::relation_of(relation)?.to_owned();
        let namespace = database.clone();

        let raw: Vec<RawForeignKey> = self
            .worker
            .call(cancel, move |connection: &Connection| {
                foreign_key_list(connection, &database, &name)
            })
            .await?;

        raw.into_iter()
            .map(|key| key.into_foreign_key(&namespace))
            .collect()
    }
}

/// Une relation telle que le moteur la décrit, avant traduction.
struct RawRelation {
    kind: RelationKind,
    columns: Vec<RawColumn>,
}

/// Une ligne de `PRAGMA table_info`.
struct RawColumn {
    cid: i64,
    name: String,
    declared: String,
    not_null: bool,
    default: Option<String>,
    primary_key: bool,
}

impl RawColumn {
    fn into_field(self) -> Field {
        // Le type logique est calculé avant, pour que l'emprunt de `declared` se
        // termine avant qu'il ne soit déplacé dans le champ `raw_type`.
        let logical = logical_type(&self.declared);
        let position = u32::try_from(self.cid).unwrap_or(u32::MAX);
        let mut field = Field::new(self.name, position, logical, self.declared);
        // Les champs sont renseignés directement plutôt que par
        // `Field::primary_key()`, qui forcerait `nullable = false` : SQLite
        // accepte un `NULL` dans une clé primaire de table `rowid`, et le modèle
        // reprend ce que le serveur dit, il ne le recalcule pas.
        field.nullable = !self.not_null;
        field.is_primary_key = self.primary_key;
        field.default = self.default;
        field
    }
}

/// Une ligne de `PRAGMA index_list`, avec ses colonnes.
struct RawIndex {
    name: String,
    unique: bool,
    columns: Vec<String>,
    predicate: Option<String>,
}

impl RawIndex {
    fn into_index(self) -> Index {
        let mut index = Index::new(self.name, self.columns);
        index.unique = self.unique;
        // SQLite n'a qu'une méthode d'accès : le b-tree.
        index.method = Some("btree".to_owned());
        index.predicate = self.predicate;
        index
    }
}

/// Les lignes de `PRAGMA foreign_key_list` d'une même contrainte.
struct RawForeignKey {
    id: i64,
    table: String,
    from: Vec<String>,
    to: Vec<String>,
    on_delete: String,
}

impl RawForeignKey {
    fn into_foreign_key(self, namespace: &str) -> Result<ForeignKey> {
        let target = ForeignKeyTarget {
            relation: CatalogPath::for_relation(None, Some(namespace), self.table)?,
            fields: self.to,
        };
        // `PRAGMA foreign_key_list` ne rend pas le nom de la contrainte : SQLite
        // ne le conserve pas de façon interrogeable. Le rang de déclaration est
        // ce qui reste, et il est stable pour une table donnée.
        let mut key = ForeignKey::new(format!("fk_{}", self.id), self.from, target);
        key.on_delete = referential_action(&self.on_delete);
        Ok(key)
    }
}

/// La nature d'une relation, d'après `sqlite_master`.
fn relation_kind(
    connection: &Connection,
    database: &str,
    name: &str,
) -> Result<Option<RelationKind>> {
    let sql = format!(
        "SELECT type FROM {}.sqlite_master WHERE name = ?1",
        quote_identifier(database, QuoteStyle::Double)
    );
    let mut statement = connection.prepare(&sql).map_err(read)?;
    // Le nom est **lié**, jamais concaténé.
    let mut rows = statement.query([name]).map_err(read)?;
    let Some(row) = rows.next().map_err(read)? else {
        return Ok(None);
    };
    let kind: String = row.get(0).map_err(read)?;
    Ok(Some(match kind.as_str() {
        "view" => RelationKind::View,
        _ => RelationKind::Table,
    }))
}

/// Les colonnes d'une relation.
fn table_info(connection: &Connection, database: &str, name: &str) -> Result<Vec<RawColumn>> {
    let mut columns = Vec::new();
    connection
        .pragma(Some(database), "table_info", name, |row| {
            columns.push(RawColumn {
                cid: row.get("cid")?,
                name: row.get("name")?,
                declared: row.get::<_, Option<String>>("type")?.unwrap_or_default(),
                not_null: row.get::<_, i64>("notnull")? != 0,
                default: row.get("dflt_value")?,
                primary_key: row.get::<_, i64>("pk")? != 0,
            });
            Ok(())
        })
        .map_err(read)?;
    Ok(columns)
}

/// Les index d'une relation, colonnes et prédicat compris.
fn index_list(connection: &Connection, database: &str, name: &str) -> Result<Vec<RawIndex>> {
    struct Entry {
        name: String,
        unique: bool,
        partial: bool,
    }

    let mut entries = Vec::new();
    connection
        .pragma(Some(database), "index_list", name, |row| {
            entries.push(Entry {
                name: row.get("name")?,
                unique: row.get::<_, i64>("unique")? != 0,
                partial: row.get::<_, i64>("partial")? != 0,
            });
            Ok(())
        })
        .map_err(read)?;

    let mut indexes = Vec::with_capacity(entries.len());
    for entry in entries {
        let mut columns = Vec::new();
        connection
            .pragma(Some(database), "index_info", entry.name.as_str(), |row| {
                // `name` est NULL pour un index d'expression : la colonne n'a
                // pas de nom, et en inventer un serait mentir.
                if let Some(column) = row.get::<_, Option<String>>("name")? {
                    columns.push(column);
                }
                Ok(())
            })
            .map_err(read)?;

        let predicate = if entry.partial {
            index_sql(connection, database, &entry.name)?
                .as_deref()
                .and_then(partial_predicate)
        } else {
            None
        };

        indexes.push(RawIndex {
            name: entry.name,
            unique: entry.unique,
            columns,
            predicate,
        });
    }
    Ok(indexes)
}

/// La DDL d'un index, quand `sqlite_master` en garde une.
///
/// Elle est `NULL` pour les index créés implicitement par une contrainte
/// (`sqlite_autoindex_*`), qui ne sont jamais partiels.
fn index_sql(connection: &Connection, database: &str, name: &str) -> Result<Option<String>> {
    let sql = format!(
        "SELECT sql FROM {}.sqlite_master WHERE type = 'index' AND name = ?1",
        quote_identifier(database, QuoteStyle::Double)
    );
    let mut statement = connection.prepare(&sql).map_err(read)?;
    let mut rows = statement.query([name]).map_err(read)?;
    let Some(row) = rows.next().map_err(read)? else {
        return Ok(None);
    };
    row.get::<_, Option<String>>(0).map_err(read)
}

/// Les clés étrangères d'une relation, regroupées par contrainte.
fn foreign_key_list(
    connection: &Connection,
    database: &str,
    name: &str,
) -> Result<Vec<RawForeignKey>> {
    struct Entry {
        id: i64,
        table: String,
        from: String,
        to: Option<String>,
        on_delete: String,
    }

    let mut entries: Vec<Entry> = Vec::new();
    connection
        .pragma(Some(database), "foreign_key_list", name, |row| {
            entries.push(Entry {
                id: row.get("id")?,
                table: row.get("table")?,
                from: row.get("from")?,
                to: row.get("to")?,
                on_delete: row.get("on_delete")?,
            });
            Ok(())
        })
        .map_err(read)?;

    let mut keys: Vec<RawForeignKey> = Vec::new();
    for entry in entries {
        // `to` est NULL quand la clé vise implicitement la clé primaire de la
        // table cible. La résoudre est ce qui empêche de rendre une clé
        // désaccordée, sur laquelle `ForeignKey::is_well_formed` répondrait faux.
        let target_field = match entry.to {
            Some(column) => Some(column),
            None => primary_key_column(
                connection,
                database,
                &entry.table,
                field_rank(&keys, entry.id),
            )?,
        };
        let Some(target_field) = target_field else {
            continue;
        };
        match keys.iter_mut().find(|key| key.id == entry.id) {
            Some(key) => {
                key.from.push(entry.from);
                key.to.push(target_field);
            }
            None => keys.push(RawForeignKey {
                id: entry.id,
                table: entry.table,
                from: vec![entry.from],
                to: vec![target_field],
                on_delete: entry.on_delete,
            }),
        }
    }
    Ok(keys)
}

/// Le rang de la colonne courante dans la contrainte en cours de construction.
fn field_rank(keys: &[RawForeignKey], id: i64) -> usize {
    keys.iter()
        .find(|key| key.id == id)
        .map_or(0, |key| key.to.len())
}

/// La `position`-ième colonne de la clé primaire d'une table.
fn primary_key_column(
    connection: &Connection,
    database: &str,
    table: &str,
    position: usize,
) -> Result<Option<String>> {
    let mut columns: Vec<(i64, String)> = Vec::new();
    connection
        .pragma(Some(database), "table_info", table, |row| {
            let rank: i64 = row.get("pk")?;
            if rank > 0 {
                columns.push((rank, row.get("name")?));
            }
            Ok(())
        })
        .map_err(read)?;
    columns.sort_by_key(|(rank, _)| *rank);
    Ok(columns.into_iter().nth(position).map(|(_, name)| name))
}

/// L'action référentielle d'un `ON DELETE`, telle que SQLite la nomme.
fn referential_action(action: &str) -> ReferentialAction {
    match action.trim().to_ascii_uppercase().as_str() {
        "CASCADE" => ReferentialAction::Cascade,
        "SET NULL" => ReferentialAction::SetNull,
        "SET DEFAULT" => ReferentialAction::SetDefault,
        "RESTRICT" => ReferentialAction::Restrict,
        // « NO ACTION », et tout ce que SQLite pourrait nommer autrement : le
        // défaut de la norme, qui ne propage rien.
        _ => ReferentialAction::NoAction,
    }
}

/// Le type logique d'un type déclaré SQLite.
///
/// Deux étages, dans cet ordre :
///
/// 1. **les noms conventionnels** que SQLite ne connaît pas mais que tout le
///    monde écrit — `BOOLEAN`, `DATE`, `DATETIME`, `DECIMAL(p,s)`, `JSON` ;
/// 2. **les règles d'affinité** de SQLite, y compris leurs surprises.
///
/// `DATETIME` devient un horodatage **sans fuseau** : SQLite n'en range aucun,
/// et en inventer un décalerait la donnée de façon invisible et permanente
/// ([`DRIVER-CONTRACT` §7](../../../docs/DRIVER-CONTRACT.md)).
///
/// Une déclaration vide — le cas d'une colonne sans type, parfaitement légal —
/// devient [`LogicalType::Unknown`], jamais un voisin plausible.
#[must_use]
pub fn logical_type(declared: &str) -> LogicalType {
    let (base, args) = split_declared(declared);
    if base.is_empty() {
        return LogicalType::Unknown;
    }
    let upper = base.to_ascii_uppercase();
    match upper.as_str() {
        "BOOLEAN" | "BOOL" => return LogicalType::Boolean,
        "DATE" => return LogicalType::Date,
        "TIME" => return LogicalType::Time,
        "DATETIME" | "TIMESTAMP" => return LogicalType::Timestamp { tz: false },
        "JSON" | "JSONB" => return LogicalType::Json,
        "DECIMAL" | "NUMERIC" => {
            let (precision, scale) = decimal_arguments(args);
            return LogicalType::Decimal { precision, scale };
        }
        _ => {}
    }
    if upper.contains("INT") {
        return LogicalType::INT64;
    }
    if upper.contains("CHAR") || upper.contains("CLOB") || upper.contains("TEXT") {
        return LogicalType::Text;
    }
    if upper.contains("BLOB") {
        return LogicalType::Bytes;
    }
    if upper.contains("REAL") || upper.contains("FLOA") || upper.contains("DOUB") {
        return LogicalType::FLOAT64;
    }
    // Affinité NUMERIC : ni entier ni flottant ne la couvrent.
    LogicalType::Decimal {
        precision: None,
        scale: None,
    }
}

/// Sépare `DECIMAL(10,2)` en `("DECIMAL", Some("10,2"))`.
fn split_declared(declared: &str) -> (&str, Option<&str>) {
    match declared.split_once('(') {
        Some((base, rest)) => (base.trim(), rest.strip_suffix(')').map(str::trim)),
        None => (declared.trim(), None),
    }
}

/// Précision et échelle d'un type décimal, quand elles sont écrites.
fn decimal_arguments(args: Option<&str>) -> (Option<u16>, Option<i16>) {
    let Some(args) = args else {
        return (None, None);
    };
    let mut parts = args.split(',');
    let precision = parts.next().and_then(|part| part.trim().parse().ok());
    let scale = parts.next().and_then(|part| part.trim().parse().ok());
    (precision, scale)
}

/// Le prédicat d'un index partiel, extrait de sa DDL.
///
/// SQLite n'expose pas le prédicat autrement que dans le texte de
/// `CREATE INDEX`. L'analyse cherche le premier mot-clé `WHERE` **hors
/// citation** — un nom de colonne peut s'appeler `where`, et une chaîne
/// littérale peut en contenir le mot.
///
/// Rend `None` si rien n'est trouvé, auquel cas l'index est décrit sans
/// prédicat. Le résiduel est assumé : un prédicat introuvable vaut mieux qu'un
/// prédicat inventé.
#[must_use]
pub fn partial_predicate(sql: &str) -> Option<String> {
    let mut quote: Option<char> = None;
    let mut word: Option<usize> = None;

    for (index, character) in sql.char_indices() {
        if let Some(opening) = quote {
            let closing = if opening == '[' { ']' } else { opening };
            if character == closing {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' | '`' | '[' => {
                quote = Some(character);
                word = None;
            }
            c if c.is_alphanumeric() || c == '_' => {
                if word.is_none() {
                    word = Some(index);
                }
            }
            _ => {
                let Some(start) = word.take() else {
                    continue;
                };
                let mot_cle = sql
                    .get(start..index)
                    .is_some_and(|found| found.eq_ignore_ascii_case("where"));
                if mot_cle {
                    return sql
                        .get(index..)
                        .map(|rest| rest.trim().to_owned())
                        .filter(|predicate| !predicate.is_empty());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_noms_conventionnels_priment_sur_l_affinite() {
        assert_eq!(logical_type("BOOLEAN"), LogicalType::Boolean);
        assert_eq!(logical_type("DATE"), LogicalType::Date);
        assert_eq!(logical_type("JSON"), LogicalType::Json);
        assert_eq!(
            logical_type("DECIMAL(10,2)"),
            LogicalType::Decimal {
                precision: Some(10),
                scale: Some(2)
            }
        );
        assert_eq!(
            logical_type("NUMERIC"),
            LogicalType::Decimal {
                precision: None,
                scale: None
            }
        );
    }

    #[test]
    fn un_datetime_sqlite_ne_recoit_pas_de_fuseau() {
        // DRIVER-CONTRACT §7 : SQLite ne range aucun fuseau. En inventer un
        // décalerait la donnée de façon invisible et permanente.
        assert_eq!(
            logical_type("DATETIME"),
            LogicalType::Timestamp { tz: false }
        );
        assert_ne!(logical_type("TIMESTAMP"), LogicalType::TIMESTAMPTZ);
    }

    #[test]
    fn les_regles_d_affinite_prennent_le_relais() {
        assert_eq!(logical_type("INTEGER"), LogicalType::INT64);
        assert_eq!(logical_type("VARCHAR(255)"), LogicalType::Text);
        assert_eq!(logical_type("BLOB"), LogicalType::Bytes);
        assert_eq!(logical_type("DOUBLE"), LogicalType::FLOAT64);
    }

    #[test]
    fn une_colonne_sans_type_declare_reste_inconnue() {
        // Légal en SQLite : `CREATE TABLE t(x)`. Lui attribuer un voisin
        // plausible afficherait une valeur fausse sans le dire.
        assert_eq!(logical_type(""), LogicalType::Unknown);
        assert_eq!(logical_type("   "), LogicalType::Unknown);
    }

    #[test]
    fn le_predicat_d_un_index_partiel_se_retrouve() {
        assert_eq!(
            partial_predicate("CREATE INDEX i ON t(a) WHERE a > 0"),
            Some("a > 0".to_owned())
        );
        assert_eq!(
            partial_predicate("CREATE INDEX i ON t(a) where (a > 0)"),
            Some("(a > 0)".to_owned())
        );
    }

    #[test]
    fn un_where_dans_une_chaine_ou_un_identifiant_ne_trompe_pas_l_analyse() {
        // Un nom de colonne peut s'appeler `where`, une valeur par défaut peut
        // contenir le mot.
        assert_eq!(
            partial_predicate(r#"CREATE INDEX i ON t("where") WHERE a > 0"#),
            Some("a > 0".to_owned())
        );
        assert_eq!(
            partial_predicate("CREATE INDEX i ON t(a) WHERE a <> 'where'"),
            Some("a <> 'where'".to_owned())
        );
        assert_eq!(partial_predicate("CREATE INDEX i ON t(a)"), None);
    }

    #[test]
    fn les_actions_referentielles_sont_reconnues_et_le_defaut_ne_propage_rien() {
        assert_eq!(referential_action("CASCADE"), ReferentialAction::Cascade);
        assert_eq!(referential_action("SET NULL"), ReferentialAction::SetNull);
        assert_eq!(referential_action("NO ACTION"), ReferentialAction::NoAction);
        assert_eq!(
            referential_action("quelque chose d'inattendu"),
            ReferentialAction::NoAction,
            "l'inconnu ne doit jamais propager une suppression"
        );
        assert!(!referential_action("RESTRICT").propagates_delete());
    }

    #[test]
    fn un_chemin_vide_designe_la_base_principale() {
        assert_eq!(SqliteCatalog::database_of(&CatalogPath::empty()), MAIN);
        let attachee = CatalogPath::for_namespace(None, "archives").expect("chemin valide");
        assert_eq!(SqliteCatalog::database_of(&attachee), "archives");
    }

    #[test]
    fn un_nom_de_base_hostile_est_cite_avant_de_rejoindre_une_requete() {
        // I-10 : une base s'attache légalement sous ce nom.
        let cite = quote_identifier(r#"x"; DROP TABLE audit; --"#, QuoteStyle::Double);
        assert_eq!(cite, r#""x""; DROP TABLE audit; --""#);
        assert!(
            !cite.starts_with('x'),
            "le nom ne doit jamais sortir nu : {cite}"
        );
    }
}
