//! Les outils offerts aux agents — c'est-à-dire les `Command` du noyau.
//!
//! **Point d'architecture non négociable** (ADR-0004, I-01) : il n'existe pas de
//! seconde API « pour l'IA ». Un outil est une traduction d'un appel du modèle
//! vers une [`Command`] existante, et rien d'autre. Un outil qui ne s'exprime
//! pas en `Command` n'est pas un outil manquant : c'est une commande manquante,
//! et cela se règle dans `oxyn-core`, pas ici.
//!
//! # Ce que le modèle ne peut pas dire
//!
//! Les arguments d'un outil ne portent **que** ce que le modèle a le droit de
//! choisir. Tout le reste vient du [`ToolScope`], que l'appelant construit :
//!
//! | Ce que le modèle écrit | Ce que le scope impose |
//! |---|---|
//! | le texte de l'instruction | la connexion, la session, le langage |
//! | | l'intention et le risque, **classés par `oxyn-query`** |
//! | | les [`ExecLimits`], dérivées de cette classification |
//!
//! Conséquences directes : un agent ne peut pas viser une autre connexion que
//! celle sur laquelle l'utilisateur l'a ouvert — donc pas d'exfiltration vers
//! une base tierce — et il ne peut pas se déclarer en lecture seule pour
//! contourner le `PolicyGate`, puisqu'il n'écrit jamais ce champ. Les schémas
//! JSON portent `additionalProperties: false` (`deny_unknown_fields`) : un
//! argument inventé fait échouer la traduction au lieu d'être ignoré en
//! silence.
//!
//! # Ce qui n'est délibérément pas exposé
//!
//! * [`Command::CreateConnection`], [`Command::UpdateConnection`],
//!   [`Command::DeleteConnection`] — un agent qui pourrait créer une connexion
//!   vers l'hôte de son choix disposerait d'un canal d'exfiltration. Le
//!   `PolicyGate` les soumettrait à approbation ; ici elles n'existent pas du
//!   tout, ce qui est plus fort qu'un refus.
//! * [`Command::Export`] — l'agent choisirait un chemin de fichier, donc
//!   écrirait où il veut sur le disque de l'utilisateur.
//! * [`Command::Cancel`] — annuler est un geste de l'utilisateur ; et la
//!   poignée d'exécution n'est jamais dans la conversation.
//! * [`Command::PreviewRelation`] **en direct** — un agent qui lirait des
//!   lignes les verrait arriver dans la grille, jamais dans sa conversation ;
//!   il n'y gagnerait rien. Il ne la déclenche que par [`REQUEST_SAMPLE`], qui
//!   la fait précéder de l'approbation de l'utilisateur, colonne par colonne
//!   ([ADR-0034](../../../docs/adr/0034-echantillon-pour-toute-destination.md)).
//! * [`Command::OpenDocument`] et [`Command::WriteDocument`] —
//!   `// TODO(phase 4)` : elles demandent un `DocumentId` que le scope devrait
//!   porter, ce qui n'a de sens qu'une fois les documents de workspace écrits.
//!
//! # La classification faite ici n'est pas celle qui protège
//!
//! `oxyn-exec` **reclassifie** systématiquement le texte avant de soumettre au
//! `PolicyGate` (ARCHITECTURE §8) : l'intention portée par une `Command` vient
//! de l'appelant, et un agent est un appelant. Ce que ce module classe sert à
//! poser des limites honnêtes et à alimenter la prévisualisation, pas à décider.

use std::fmt;

use oxyn_core::{
    Command, ConnectionId, ExecLimits, ExecRequest, MAX_CATALOG_FOCUS_BYTES, PreviewShape,
    QueryLanguage, SessionId,
};
use oxyn_llm::{ToolCall, ToolSpec};
use schemars::{JsonSchema, SchemaGenerator, generate::SchemaSettings};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::error::AiError;

/// Nom de l'outil qui exécute une instruction.
pub const EXECUTE_QUERY: &str = "execute_query";

/// Nom de l'outil qui relit le catalogue depuis le serveur.
pub const REFRESH_CATALOG: &str = "refresh_catalog";

/// Nom de l'outil qui décrit la structure de la base, depuis le catalogue local.
pub const DESCRIBE_SCHEMA: &str = "describe_schema";

/// La consigne qui dit au modèle comment faire dessiner un schéma
/// entité-relation par Oxyn.
///
/// Une macro et non une constante : `concat!` ne prend que des littéraux, et
/// les descriptions d'outils comme les invites des agents intégrés sont des
/// littéraux. C'est ce qui garde **une** phrase pour les invites système, la
/// description de [`DESCRIBE_SCHEMA`] et l'invite d'un agent externe ; deux
/// copies finiraient par dire deux formats, et le panneau n'en dessine qu'un.
macro_rules! erd_hint {
    () => {
        "To show an entity-relationship diagram, write a fenced code block whose language \
         is `erd` and that lists one table name per line, nothing else: Oxyn draws the \
         diagram from its catalog. Do not draw one in ASCII or in another diagram language."
    };
}
pub(crate) use erd_hint;

/// La consigne de `erd_hint!`, pour qui compose une invite à l'exécution : la
/// même phrase que celle des invites système et de [`DESCRIBE_SCHEMA`].
pub const ERD_HINT: &str = erd_hint!();

/// Nom de l'outil par lequel un agent **demande** un échantillon de lignes.
///
/// Demander n'est pas lire : rien n'est lu ni envoyé avant que l'utilisateur
/// ait coché, dans le panneau, les colonnes qui partent
/// ([ADR-0034](../../../docs/adr/0034-echantillon-pour-toute-destination.md)).
pub const REQUEST_SAMPLE: &str = "request_sample";

/// Lignes demandées quand l'agent n'en dit rien : celles d'un échantillon que
/// l'utilisateur épingle lui-même. De quoi illustrer une forme de données.
pub const DEFAULT_SAMPLE_ROWS: u32 = 5;

/// Le plus de lignes qu'un échantillon demandé porte, quoi que l'agent écrive.
///
/// Borne de produit : un échantillon illustre des valeurs, il ne sert pas à
/// extraire une table. Chaque ligne est une ligne qui sort de la machine.
pub const MAX_SAMPLE_ROWS: u32 = 20;

/// Le plus de colonnes qu'une demande peut nommer. Au-delà, l'agent demande
/// toute la relation en omettant `columns`, et l'utilisateur coche.
pub const MAX_SAMPLE_COLUMNS: usize = 64;

/// La plus longue désignation acceptée — un nom de relation, d'espace de noms
/// ou de colonne —, en octets. Des noms, jamais du texte de requête.
pub const MAX_SAMPLE_NAME_BYTES: usize = 256;

/// Ce que l'appelant impose, et que le modèle ne choisit pas.
///
/// Construit par l'appelant à l'ouverture d'une conversation, à partir de la
/// connexion et de la session que l'utilisateur a lui-même ouvertes. C'est le
/// **principe de moindre autorité** appliqué à la lettre : un agent n'atteint
/// que ce que ce type nomme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolScope {
    /// La connexion sur laquelle l'agent travaille.
    pub connection: ConnectionId,
    /// La session ouverte sur cette connexion.
    pub session: SessionId,
    /// Le langage de requête de cette session. Détermine aussi quel analyseur
    /// classe le texte : un langage non SQL n'est pas analysé, donc `Unknown`,
    /// donc mutant.
    pub language: QueryLanguage,
}

impl ToolScope {
    /// Construit un périmètre d'outils.
    #[must_use]
    pub const fn new(
        connection: ConnectionId,
        session: SessionId,
        language: QueryLanguage,
    ) -> Self {
        Self {
            connection,
            session,
            language,
        }
    }
}

/// Arguments d'[`EXECUTE_QUERY`].
///
/// Un seul champ, et c'est le point : tout ce qui pourrait affaiblir la
/// politique — connexion, lecture seule, intention déclarée — est absent du
/// schéma, donc inaccessible au modèle.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecuteQueryArgs {
    /// La description part vers le modèle : elle est en anglais, comme tout
    /// texte de code (CLAUDE.md). L'attribut prime sur ce commentaire, qui
    /// s'adresse aux relecteurs.
    #[schemars(
        description = "The statement to run, exactly as it should reach the database. \
                       Write one statement. Reads run immediately; writes and DDL are \
                       held for the user's approval before anything happens."
    )]
    pub statement: String,
}

/// Arguments de [`DESCRIBE_SCHEMA`].
///
/// Un seul champ, facultatif : des mots de recherche. La connexion vient du
/// [`ToolScope`], et rien ici ne compose de requête — les mots servent à classer
/// des noms du catalogue local.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DescribeSchemaArgs {
    /// La description part vers le modèle : elle est en anglais. Elle écrit la
    /// borne en octets, parce que `maxLength` compte des caractères et ne dirait
    /// pas la même chose que le refus de la traduction et de l'exécuteur
    /// ([`MAX_CATALOG_FOCUS_BYTES`]) ; un test tient le chiffre aligné.
    #[schemars(
        description = "Optional search words — a table, collection or column name, or a \
                       topic — to describe the most relevant objects first. Omit it to \
                       describe the database from the start. At most 256 bytes of UTF-8."
    )]
    #[serde(default)]
    pub search: Option<String>,
}

/// Arguments de [`REQUEST_SAMPLE`].
///
/// Des **noms**, jamais du texte de requête : la relation et les colonnes sont
/// cherchées dans le catalogue local, et la lecture est composée par le driver,
/// qui cite chaque identifiant ([I-10](../../../CLAUDE.md#i-10)). La connexion
/// vient du [`ToolScope`].
///
/// Le schéma annonce les bornes que la traduction applique : un agent qui ne
/// les voit pas les dépasse, se fait refuser, et ne sait pas de combien
/// corriger. Les chiffres des attributs sont des littéraux — `schemars` n'accepte
/// pas une constante dans une description —, et un test les tient alignés sur
/// [`MAX_SAMPLE_ROWS`], [`MAX_SAMPLE_COLUMNS`] et [`MAX_SAMPLE_NAME_BYTES`]. La
/// borne des noms s'écrit en octets dans la description : `maxLength` compte
/// des caractères, et ne dirait pas la même chose que le refus.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestSampleArgs {
    #[schemars(
        description = "The table, view or collection to sample, exactly as describe_schema \
                       names it, without quotes. At most 256 bytes of UTF-8."
    )]
    pub relation: String,
    #[schemars(
        description = "Its schema or namespace, without quotes, when several objects share \
                       the name. Omit it otherwise. At most 256 bytes of UTF-8."
    )]
    #[serde(default)]
    pub namespace: Option<String>,
    #[schemars(
        description = "The columns you need, without quotes. Omit it to let the user choose \
                       among all of them. At most 64 columns, each name at most 256 bytes \
                       of UTF-8.",
        length(max = 64)
    )]
    #[serde(default)]
    pub columns: Option<Vec<String>>,
    #[schemars(
        description = "How many rows: 5 when omitted, from 1 to 20.",
        range(min = 1, max = 20)
    )]
    #[serde(default)]
    pub rows: Option<u32>,
}

/// Arguments de [`REFRESH_CATALOG`] : aucun.
///
/// La connexion vient du [`ToolScope`]. Un objet vide plutôt qu'une absence de
/// schéma : plusieurs fournisseurs refusent un outil sans objet `parameters`.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RefreshCatalogArgs {}

/// Traduit des arguments validés en demande.
type Translate = fn(&serde_json::Value, &ToolScope) -> Result<ToolRequest, AiError>;

/// Ce qu'un appel d'outil demande au puits, une fois traduit.
///
/// Deux formes, et **une seule commande** au bout de chacune : il n'existe pas
/// de demande qui n'en porte pas ([I-01](../../../CLAUDE.md#i-01)).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ToolRequest {
    /// Une commande à soumettre telle quelle.
    Dispatch(Command),
    /// Un échantillon à faire approuver, puis lire.
    Sample(SampleAsk),
}

impl ToolRequest {
    /// La commande que la demande porte — pour un échantillon, la lecture qui
    /// ne partira qu'après l'approbation.
    #[must_use]
    pub const fn command(&self) -> &Command {
        match self {
            Self::Dispatch(command) => command,
            Self::Sample(ask) => &ask.command,
        }
    }
}

/// Un échantillon demandé par un agent : la lecture, et les colonnes voulues.
///
/// La lecture est une [`Command::PreviewRelation`] bornée — la commande même
/// qu'emprunte l'échantillon épinglé par l'utilisateur. Elle ne s'exécute
/// **qu'après** l'approbation, que le puits obtient de l'utilisateur ; les
/// colonnes nommées ici ne sont qu'une demande, que l'approbation restreint.
///
/// Les noms sont écrits par le modèle : ils n'ont pas encore été confrontés au
/// catalogue. C'est le puits qui le fait, avant de rien montrer
/// ([ADR-0034](../../../docs/adr/0034-echantillon-pour-toute-destination.md)).
#[derive(Clone, PartialEq)]
pub struct SampleAsk {
    /// La lecture : `PreviewRelation`, bornée à [`MAX_SAMPLE_ROWS`].
    pub command: Command,
    /// Les colonnes voulues, dans l'ordre de l'agent ; vide pour « toutes,
    /// au choix de l'utilisateur ».
    pub columns: Vec<String>,
}

// Des noms de colonnes : des métadonnées, mais tenues hors des journaux quand
// même — un nom de colonne peut être la donnée (`hiv_status`).
impl fmt::Debug for SampleAsk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SampleAsk")
            .field("command", &self.command.name())
            .field("columns", &self.columns.len())
            .finish()
    }
}

impl SampleAsk {
    /// Le nombre de lignes demandé, déjà borné par la traduction.
    #[must_use]
    pub const fn rows(&self) -> u32 {
        match &self.command {
            Command::PreviewRelation { limit, .. } => *limit,
            _ => 0,
        }
    }
}

/// Produit le schéma JSON des arguments d'un outil.
type Schema = fn() -> serde_json::Value;

/// Un outil, c'est-à-dire une `Command` rendue appelable par un modèle.
#[derive(Clone)]
pub struct ToolDefinition {
    name: &'static str,
    description: &'static str,
    /// Nom de la variante de [`Command`] produite. Sert au journal d'audit et à
    /// la relecture : la correspondance outil → commande doit être lisible sans
    /// dérouler la fonction de traduction.
    command: &'static str,
    schema: Schema,
    translate: Translate,
}

impl ToolDefinition {
    /// Nom sous lequel le modèle appelle cet outil.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Ce que fait l'outil, en une phrase destinée au modèle.
    #[must_use]
    pub const fn description(&self) -> &'static str {
        self.description
    }

    /// La variante de [`Command`] que cet outil produit.
    #[must_use]
    pub const fn command(&self) -> &'static str {
        self.command
    }

    /// La déclaration à transmettre au fournisseur.
    #[must_use]
    pub fn spec(&self) -> ToolSpec {
        ToolSpec::new(self.name, self.description, (self.schema)())
    }
}

impl fmt::Debug for ToolDefinition {
    /// Écrit à la main : un pointeur de fonction dans un `Debug` dérivé est une
    /// adresse, qui n'apprend rien. Le nom de la commande produite, si.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolDefinition")
            .field("name", &self.name)
            .field("command", &self.command)
            .finish_non_exhaustive()
    }
}

/// Les outils qu'Oxyn sait traduire.
///
/// Le registre est **fermé au contenu, ouvert à la déclaration** : les
/// traductions sont du code Rust — elles construisent des `Command`, donc elles
/// ne peuvent pas venir d'un plugin —, tandis que le choix des outils accordés à
/// un agent est déclaratif ([`AgentSpec::allowed_tools`](crate::spec::AgentSpec)).
/// C'est ce qui permet à un agent de venir d'un plugin sans ouvrir une seconde
/// voie vers les drivers.
#[derive(Debug, Clone)]
pub struct ToolRegistry {
    tools: Vec<ToolDefinition>,
}

impl ToolRegistry {
    /// Le registre livré avec Oxyn.
    #[must_use]
    pub fn builtin() -> Self {
        Self {
            tools: vec![
                ToolDefinition {
                    name: EXECUTE_QUERY,
                    // « Reads return rows » disait le contraire de ce que
                    // rend l'outil : un agent qui attendait des lignes de
                    // `sqlite_master` a conclu qu'il ne pouvait pas lire la
                    // base. Le schéma est dans le contexte, pas ici.
                    description: "Run one statement against the database the user opened. \
                                  Write it against the structure described to you. \
                                  Reads run immediately: the rows go to the user's result \
                                  grid, and you receive only the shape of the result (row \
                                  and batch counts), never the values — so do not query \
                                  system tables to learn the schema: call describe_schema. \
                                  Writes, DDL and anything the analyzer \
                                  cannot classify are held for the user's explicit approval, \
                                  so never assume a statement ran until the tool result says \
                                  so. You cannot choose the connection.",
                    command: "Execute",
                    schema: schema_of::<ExecuteQueryArgs>,
                    translate: translate_execute_query,
                },
                ToolDefinition {
                    name: DESCRIBE_SCHEMA,
                    description: concat!(
                        "Describe the structure of the database the user opened: \
                         its objects (tables, views, collections, indexes, key \
                         patterns, labels…), their fields and types as the server \
                         names them, keys and indexes, and the query language to \
                         write in. Read from Oxyn's catalog of this connection; \
                         what it has not loaded yet — the objects of a schema, the \
                         fields of the objects that match — Oxyn first reads from \
                         the server, as metadata only and within a few seconds. It \
                         never returns row values. The answer is bounded and says \
                         what is not loaded yet; when it says objects were left \
                         out or not loaded, call it again with search words. ",
                        erd_hint!()
                    ),
                    command: "DescribeCatalog",
                    schema: schema_of::<DescribeSchemaArgs>,
                    translate: translate_describe_schema,
                },
                ToolDefinition {
                    name: REQUEST_SAMPLE,
                    description: "Ask the user to share real rows of one table, view or \
                                  collection with you — when the structure is not enough, \
                                  for instance to see how values are written. Nothing is read \
                                  until the user approves, column by column, in Oxyn; you \
                                  receive only the columns they approve, or `the user \
                                  declined`. Allowed only when the connection's privacy tier \
                                  is `sampled`: under any other tier it is refused, and asking \
                                  again changes nothing. Ask once per answer, for the columns \
                                  you need.",
                    command: "PreviewRelation",
                    schema: schema_of::<RequestSampleArgs>,
                    translate: translate_request_sample,
                },
                ToolDefinition {
                    name: REFRESH_CATALOG,
                    description: "Re-read the structure of the database from the server: \
                                  its identity, then the objects of each schema, within a \
                                  bound of schemas and seconds. Use it after a schema change, \
                                  or when the schema shown to you looks out of date. It is \
                                  slow on large schemas; describe_schema already reads what \
                                  was never loaded.",
                    command: "RefreshCatalog",
                    schema: schema_of::<RefreshCatalogArgs>,
                    translate: translate_refresh_catalog,
                },
            ],
        }
    }

    /// Les noms connus, dans l'ordre de déclaration.
    #[must_use]
    pub fn names(&self) -> Vec<&'static str> {
        self.tools.iter().map(ToolDefinition::name).collect()
    }

    /// La définition portant ce nom.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ToolDefinition> {
        self.tools.iter().find(|tool| tool.name == name)
    }

    /// Cet outil existe-t-il ?
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Les déclarations à transmettre au fournisseur pour cet agent.
    ///
    /// L'ordre suit la liste blanche de l'agent : ce qu'il déclare en premier
    /// est présenté en premier.
    ///
    /// # Erreurs
    /// [`AiError::UnknownTool`] si la liste blanche nomme un outil que le
    /// registre ne connaît pas. Échouer ici plutôt qu'ignorer l'entrée est
    /// délibéré : un agent qui croit disposer d'un outil absent produit des
    /// tours de conversation perdus, sans que rien ne le signale.
    pub fn specs_for(&self, allowed: &[String]) -> Result<Vec<ToolSpec>, AiError> {
        allowed
            .iter()
            .map(|name| {
                self.get(name)
                    .map(ToolDefinition::spec)
                    .ok_or_else(|| AiError::UnknownTool { name: name.clone() })
            })
            .collect()
    }

    /// Traduit un appel du modèle en commande du noyau.
    ///
    /// Trois refus possibles, dans cet ordre : l'outil n'existe pas, il n'est
    /// pas accordé à cet agent, ses arguments ne correspondent pas au schéma.
    /// L'ordre compte pour le message rendu au modèle — « inconnu » et « non
    /// accordé » ne demandent pas la même correction.
    ///
    /// La commande rendue n'a **rien exécuté** : elle doit encore traverser le
    /// `PolicyGate` en portant `Actor::Agent` (I-07).
    ///
    /// **[`REQUEST_SAMPLE`] est refusé ici.** Sa lecture n'existe qu'après
    /// l'approbation de l'utilisateur, que seul
    /// [`CommandSink::request_sample`](crate::runtime::CommandSink::request_sample)
    /// obtient : la rendre nue ferait de cette fonction publique un second
    /// chemin vers une lecture de valeurs sans consentement. Un appelant qui
    /// doit servir cet outil passe par [`ToolRegistry::request`].
    ///
    /// # Erreurs
    /// [`AiError::UnknownTool`], [`AiError::ToolNotAllowed`] — y compris pour
    /// [`REQUEST_SAMPLE`] — ou [`AiError::InvalidArguments`].
    pub fn translate(
        &self,
        call: &ToolCall,
        allowed: &[String],
        scope: &ToolScope,
    ) -> Result<Command, AiError> {
        match self.request(call, allowed, scope)? {
            ToolRequest::Dispatch(command) => Ok(command),
            ToolRequest::Sample(_) => Err(AiError::ToolNotAllowed {
                name: call.name.clone(),
            }),
        }
    }

    /// Traduit un appel du modèle en demande au puits : une commande, ou un
    /// échantillon à faire approuver.
    ///
    /// Les mêmes refus, dans le même ordre, que [`ToolRegistry::translate`].
    ///
    /// # Erreurs
    /// [`AiError::UnknownTool`], [`AiError::ToolNotAllowed`] ou
    /// [`AiError::InvalidArguments`].
    pub fn request(
        &self,
        call: &ToolCall,
        allowed: &[String],
        scope: &ToolScope,
    ) -> Result<ToolRequest, AiError> {
        let Some(tool) = self.get(&call.name) else {
            return Err(AiError::UnknownTool {
                name: call.name.clone(),
            });
        };
        if !allowed.iter().any(|name| name == tool.name) {
            return Err(AiError::ToolNotAllowed {
                name: call.name.clone(),
            });
        }
        (tool.translate)(&call.arguments, scope)
    }
}

impl Default for ToolRegistry {
    /// Le registre livré avec Oxyn. Un registre vide n'aurait aucun usage.
    fn default() -> Self {
        Self::builtin()
    }
}

/// Produit le schéma JSON des arguments d'un type.
///
/// Quatre réglages, chacun pour une raison :
///
/// * `meta_schema: None` — le champ `$schema` fait échouer la validation stricte
///   de plusieurs fournisseurs ;
/// * `inline_subschemas: true` — pas de `$ref` ni de `$defs`, que tous les
///   fournisseurs ne savent pas suivre ;
/// * `title` retiré — c'est le nom du type Rust, qui n'apprend rien au modèle et
///   fait fuiter un détail d'implémentation dans l'invite ;
/// * `description` racine retirée — `schemars` la tire du `///` du type, qui est
///   en français et s'adresse aux relecteurs. Ce que le modèle doit lire est
///   dans [`ToolDefinition::description`] et dans les attributs `schemars` des
///   champs, en anglais.
fn schema_of<T: JsonSchema>() -> serde_json::Value {
    let mut settings = SchemaSettings::draft2020_12();
    settings.meta_schema = None;
    settings.inline_subschemas = true;
    let mut schema = SchemaGenerator::new(settings).into_root_schema_for::<T>();
    let _ = schema.remove("title");
    let _ = schema.remove("description");
    schema.to_value()
}

/// Décode des arguments, en tolérant l'absence d'objet.
///
/// Plusieurs fournisseurs transmettent `null` plutôt qu'un objet vide pour un
/// outil sans argument. Refuser cela ferait échouer un appel correct.
fn parse_args<T: DeserializeOwned>(
    name: &'static str,
    raw: &serde_json::Value,
) -> Result<T, AiError> {
    let value = if raw.is_null() {
        serde_json::Value::Object(serde_json::Map::new())
    } else {
        raw.clone()
    };
    serde_json::from_value(value).map_err(|err| AiError::InvalidArguments {
        name: name.to_owned(),
        detail: err.to_string(),
    })
}

/// [`EXECUTE_QUERY`] → [`Command::Execute`].
fn translate_execute_query(
    raw: &serde_json::Value,
    scope: &ToolScope,
) -> Result<ToolRequest, AiError> {
    let args: ExecuteQueryArgs = parse_args(EXECUTE_QUERY, raw)?;
    let statement = args.statement.trim();
    if statement.is_empty() {
        return Err(AiError::InvalidArguments {
            name: EXECUTE_QUERY.to_owned(),
            detail: "`statement` is empty".to_owned(),
        });
    }

    // L'intention et le risque viennent de l'analyse du texte, jamais d'un
    // champ que le modèle aurait rempli. `oxyn-exec` refera ce travail avant le
    // `PolicyGate` : ici, il sert à ne pas mentir dans la prévisualisation et à
    // poser des limites cohérentes.
    let analysis = oxyn_query::classify_language(scope.language, statement);

    // Les limites par défaut interdisent l'écriture. On ne les desserre que
    // lorsque l'analyse dit que le texte écrit — et une écriture d'agent est de
    // toute façon soumise à approbation.
    let limits = if analysis.is_mutating() {
        ExecLimits::default().writable()
    } else {
        ExecLimits::default()
    };

    let request = ExecRequest::new(scope.language, statement)
        .with_intent(analysis.intent)
        .with_risk(analysis.risk)
        .with_limits(limits);

    Ok(ToolRequest::Dispatch(Command::Execute {
        connection: scope.connection,
        session: scope.session,
        request: Box::new(request),
    }))
}

/// [`DESCRIBE_SCHEMA`] → [`Command::DescribeCatalog`].
///
/// Les mots de recherche sont bornés ici, avant la commande : un agent qui en
/// écrirait un mégaoctet se voit répondre de raccourcir, et l'exécuteur refait
/// la vérification.
fn translate_describe_schema(
    raw: &serde_json::Value,
    scope: &ToolScope,
) -> Result<ToolRequest, AiError> {
    let args: DescribeSchemaArgs = parse_args(DESCRIBE_SCHEMA, raw)?;
    let focus = args
        .search
        .as_deref()
        .map(str::trim)
        .filter(|search| !search.is_empty())
        .map(str::to_owned);
    if focus
        .as_deref()
        .is_some_and(|focus| focus.len() > MAX_CATALOG_FOCUS_BYTES)
    {
        return Err(AiError::InvalidArguments {
            name: DESCRIBE_SCHEMA.to_owned(),
            detail: format!("`search` is limited to {MAX_CATALOG_FOCUS_BYTES} bytes"),
        });
    }
    Ok(ToolRequest::Dispatch(Command::DescribeCatalog {
        connection: scope.connection,
        focus,
    }))
}

/// [`REFRESH_CATALOG`] → [`Command::RefreshCatalog`].
fn translate_refresh_catalog(
    raw: &serde_json::Value,
    scope: &ToolScope,
) -> Result<ToolRequest, AiError> {
    let _args: RefreshCatalogArgs = parse_args(REFRESH_CATALOG, raw)?;
    Ok(ToolRequest::Dispatch(Command::RefreshCatalog {
        connection: scope.connection,
    }))
}

/// Un nom écrit par le modèle, borné et non vide. Aucun caractère n'est
/// refusé : un nom légal pour le serveur doit rester demandable, et c'est la
/// citation par le driver qui le rend inoffensif, pas un filtre ici.
fn sample_name(field: &str, raw: &str) -> Result<String, AiError> {
    let invalid = |detail: String| AiError::InvalidArguments {
        name: REQUEST_SAMPLE.to_owned(),
        detail,
    };
    if raw.trim().is_empty() {
        return Err(invalid(format!("`{field}` is empty")));
    }
    if raw.len() > MAX_SAMPLE_NAME_BYTES {
        return Err(invalid(format!(
            "`{field}` is limited to {MAX_SAMPLE_NAME_BYTES} bytes"
        )));
    }
    Ok(raw.to_owned())
}

/// [`REQUEST_SAMPLE`] → une [`SampleAsk`] portant [`Command::PreviewRelation`].
///
/// Les noms restent des **données** : la relation voyage en champ, jamais dans
/// un texte d'instruction, et le driver la cite quand il compose la lecture
/// ([I-10](../../../CLAUDE.md#i-10)). Les colonnes ne rejoignent aucune
/// instruction : elles désignent ce qu'on recopie du résultat.
fn translate_request_sample(
    raw: &serde_json::Value,
    scope: &ToolScope,
) -> Result<ToolRequest, AiError> {
    let args: RequestSampleArgs = parse_args(REQUEST_SAMPLE, raw)?;
    let invalid = |detail: String| AiError::InvalidArguments {
        name: REQUEST_SAMPLE.to_owned(),
        detail,
    };
    let relation = sample_name("relation", &args.relation)?;
    let namespace = args
        .namespace
        .as_deref()
        .filter(|namespace| !namespace.trim().is_empty())
        .map(|namespace| sample_name("namespace", namespace))
        .transpose()?;
    let requested = args.columns.unwrap_or_default();
    if requested.len() > MAX_SAMPLE_COLUMNS {
        return Err(invalid(format!(
            "`columns` names at most {MAX_SAMPLE_COLUMNS} columns; omit it to offer all of them"
        )));
    }
    let mut columns: Vec<String> = Vec::with_capacity(requested.len());
    for column in &requested {
        let column = sample_name("columns", column)?;
        if !columns.contains(&column) {
            columns.push(column);
        }
    }
    let rows = match args.rows {
        None => DEFAULT_SAMPLE_ROWS,
        Some(rows) if (1..=MAX_SAMPLE_ROWS).contains(&rows) => rows,
        Some(_) => {
            return Err(invalid(format!(
                "`rows` must be between 1 and {MAX_SAMPLE_ROWS}"
            )));
        }
    };
    Ok(ToolRequest::Sample(SampleAsk {
        command: Command::PreviewRelation {
            connection: scope.connection,
            session: scope.session,
            catalog: None,
            namespace,
            relation,
            limit: rows,
            shape: PreviewShape::unordered(),
        },
        columns,
    }))
}

#[cfg(test)]
mod tests {
    use oxyn_core::{MutationRisk, StatementIntent};
    use serde_json::json;

    use super::*;

    fn scope() -> ToolScope {
        ToolScope::new(ConnectionId::new(), SessionId::new(), QueryLanguage::SQL)
    }

    fn appel(nom: &str, args: serde_json::Value) -> ToolCall {
        ToolCall::new("call_1", nom, args)
    }

    fn tous() -> Vec<String> {
        ToolRegistry::builtin()
            .names()
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn chaque_outil_produit_une_commande_du_noyau() {
        // I-01 : il n'existe pas de seconde API pour l'IA. Chaque outil nomme
        // la variante de `Command` qu'il produit, et cette variante existe.
        let noms_de_commandes = [
            "Connect",
            "Disconnect",
            "Execute",
            "PreviewRelation",
            "Cancel",
            "RefreshCatalog",
            "DescribeCatalog",
            "Export",
            "OpenDocument",
            "WriteDocument",
            "CreateConnection",
            "UpdateConnection",
            "DeleteConnection",
        ];
        for outil in &ToolRegistry::builtin().tools {
            assert!(
                noms_de_commandes.contains(&outil.command()),
                "l'outil « {} » prétend produire « {} », qui n'est pas une Command",
                outil.name(),
                outil.command()
            );
        }
    }

    #[test]
    fn la_gestion_des_connexions_n_est_pas_un_outil() {
        // Un agent qui pourrait créer une connexion vers l'hôte de son choix
        // disposerait d'un canal d'exfiltration. Ces commandes n'ont pas
        // d'outil : c'est plus fort qu'un refus du PolicyGate.
        let registre = ToolRegistry::builtin();
        for interdit in [
            "create_connection",
            "update_connection",
            "delete_connection",
            "export",
            "cancel",
        ] {
            assert!(!registre.contains(interdit), "{interdit}");
        }
        for outil in &registre.tools {
            assert!(
                !outil.command().contains("Connection"),
                "{} produit {}",
                outil.name(),
                outil.command()
            );
            assert_ne!(outil.command(), "Export");
        }
    }

    /// Le changement de schéma n'est pas un outil, et ne doit pas le devenir.
    ///
    /// [ADR-0025](../../../docs/adr/0025-proposition-de-changement-de-schema.md)
    /// écrit que `Propose change…` est **indisponible** pour un `Actor::Agent`,
    /// et non « confirmable » — c'est [I-02](../../../CLAUDE.md#i-02) au mot,
    /// qui nomme la confirmation renforcée comme insuffisante.
    ///
    /// Cette garantie tenait par **absence de chemin** : aucun outil n'atteint
    /// le geste. C'est plus fort qu'un refus, mais rien ne l'aurait maintenue
    /// vraie — un relevé de divergences l'a signalé le 2026-09-14. Ce test la
    /// tient : le jour où quelqu'un expose un outil de structure, il échoue et
    /// oblige à rouvrir l'ADR plutôt qu'à le contredire en silence.
    #[test]
    fn le_changement_de_schema_n_est_pas_un_outil() {
        let registre = ToolRegistry::builtin();
        for interdit in [
            "propose_change",
            "alter_table",
            "create_table",
            "drop_table",
            "apply_ddl",
        ] {
            assert!(!registre.contains(interdit), "{interdit}");
        }
        // Et par la commande produite, pour que renommer l'outil ne suffise pas
        // à passer au travers.
        for outil in &registre.tools {
            for marque in ["Ddl", "Alter", "Schema", "Propose"] {
                assert!(
                    !outil.command().contains(marque),
                    "{} produit {}, qui touche au schéma",
                    outil.name(),
                    outil.command()
                );
            }
        }
    }

    /// ADR-0042: `Drop…`, `Truncate…` and `Rename…` from the catalog are human
    /// actions. An agent that wants a table gone writes SQL like anyone, and
    /// the gate refuses it on production; no tool reaches the review.
    #[test]
    fn object_operations_are_not_tools() {
        let registre = ToolRegistry::builtin();
        for interdit in [
            "review_object_operation",
            "run_object_operation",
            "drop_object",
            "truncate_table",
            "rename_object",
            "rename_table",
        ] {
            assert!(!registre.contains(interdit), "{interdit}");
        }
        for outil in &registre.tools {
            for marque in ["Drop", "Truncate", "Rename", "ObjectOperation"] {
                assert!(
                    !outil.name().to_lowercase().contains(&marque.to_lowercase())
                        && !outil.command().contains(marque),
                    "{} reaches {}, an object operation",
                    outil.name(),
                    outil.command()
                );
            }
        }
    }

    #[test]
    fn un_outil_hors_liste_blanche_est_refuse() {
        let registre = ToolRegistry::builtin();
        let call = appel(REFRESH_CATALOG, json!({}));
        let refus = registre
            .translate(&call, &[EXECUTE_QUERY.to_owned()], &scope())
            .expect_err("l'outil n'est pas accordé");
        assert!(matches!(refus, AiError::ToolNotAllowed { .. }), "{refus:?}");
    }

    #[test]
    fn un_outil_inconnu_est_refuse_avant_la_liste_blanche() {
        let registre = ToolRegistry::builtin();
        let call = appel("drop_everything", json!({"target": "*"}));
        let refus = registre
            .translate(&call, &tous(), &scope())
            .expect_err("l'outil n'existe pas");
        assert!(matches!(refus, AiError::UnknownTool { .. }), "{refus:?}");
    }

    #[test]
    fn le_modele_ne_choisit_ni_la_connexion_ni_la_session() {
        // La panne visée : un modèle qui vise une autre connexion que celle
        // ouverte par l'utilisateur, et exfiltre d'une base vers une autre.
        let registre = ToolRegistry::builtin();
        let perimetre = scope();
        let call = appel(
            EXECUTE_QUERY,
            json!({
                "statement": "SELECT 1",
                "connection": "00000000-0000-0000-0000-000000000000",
            }),
        );
        let refus = registre
            .translate(&call, &tous(), &perimetre)
            .expect_err("`connection` n'est pas dans le schéma");
        assert!(
            matches!(refus, AiError::InvalidArguments { .. }),
            "{refus:?}"
        );

        // Et l'appel légitime vise bien le périmètre imposé.
        let call = appel(EXECUTE_QUERY, json!({"statement": "SELECT 1"}));
        let commande = registre
            .translate(&call, &tous(), &perimetre)
            .expect("appel valide");
        assert_eq!(commande.target_connection(), Some(perimetre.connection));
    }

    #[test]
    fn le_modele_ne_peut_pas_se_declarer_en_lecture_seule() {
        // La panne visée : un modèle qui joint `read_only: true` à un DELETE
        // pour se faire passer pour une lecture devant le PolicyGate.
        let registre = ToolRegistry::builtin();
        let call = appel(
            EXECUTE_QUERY,
            json!({"statement": "DELETE FROM commandes", "read_only": true}),
        );
        assert!(registre.translate(&call, &tous(), &scope()).is_err());

        // Sans le champ, la classification tranche seule — et elle voit un
        // DELETE sans WHERE.
        let call = appel(EXECUTE_QUERY, json!({"statement": "DELETE FROM commandes"}));
        let commande = registre
            .translate(&call, &tous(), &scope())
            .expect("appel valide");
        assert_eq!(commande.intent(), StatementIntent::Write);
        assert_eq!(commande.mutation_risk(), MutationRisk::UnboundedDelete);
        assert!(commande.is_mutating());
    }

    #[test]
    fn une_lecture_reste_bornee_en_lecture_seule() {
        let registre = ToolRegistry::builtin();
        let call = appel(EXECUTE_QUERY, json!({"statement": "SELECT * FROM clients"}));
        let commande = registre
            .translate(&call, &tous(), &scope())
            .expect("appel valide");
        let Command::Execute { request, .. } = commande else {
            panic!("execute_query doit produire Command::Execute");
        };
        assert!(request.limits.read_only);
        assert_eq!(request.intent, StatementIntent::Read);
        assert!(request.params.is_empty(), "le modèle ne lie pas de valeurs");
    }

    #[test]
    fn un_texte_illisible_est_traite_comme_mutant() {
        // « Dans le doute, on protège » : ce qui ne s'analyse pas est `Unknown`,
        // qui compte pour mutant, donc soumis à approbation.
        let registre = ToolRegistry::builtin();
        let call = appel(EXECUTE_QUERY, json!({"statement": "SELEKT * FORM t"}));
        let commande = registre
            .translate(&call, &tous(), &scope())
            .expect("appel valide");
        assert_eq!(commande.intent(), StatementIntent::Unknown);
        assert!(commande.is_mutating());
    }

    #[test]
    fn une_instruction_vide_est_refusee() {
        let registre = ToolRegistry::builtin();
        let call = appel(EXECUTE_QUERY, json!({"statement": "   \n  "}));
        let refus = registre
            .translate(&call, &tous(), &scope())
            .expect_err("instruction vide");
        assert!(
            matches!(refus, AiError::InvalidArguments { .. }),
            "{refus:?}"
        );
    }

    #[test]
    fn un_outil_sans_argument_accepte_null() {
        // Plusieurs fournisseurs transmettent `null` au lieu de `{}`.
        let registre = ToolRegistry::builtin();
        let call = appel(REFRESH_CATALOG, serde_json::Value::Null);
        let commande = registre
            .translate(&call, &tous(), &scope())
            .expect("appel valide");
        assert_eq!(commande.name(), "RefreshCatalog");
    }

    #[test]
    fn les_schemas_sont_transmissibles_a_un_fournisseur() {
        let registre = ToolRegistry::builtin();
        let specs = registre.specs_for(&tous()).expect("outils connus");
        assert_eq!(specs.len(), 4);
        for spec in &specs {
            let params = &spec.parameters;
            assert_eq!(params.get("type").and_then(|v| v.as_str()), Some("object"));
            assert!(
                params.get("$schema").is_none(),
                "`$schema` fait échouer la validation stricte de certains fournisseurs"
            );
            assert!(
                params.get("$defs").is_none(),
                "les sous-schémas doivent être inlinés"
            );
            assert!(
                params.get("title").is_none(),
                "le nom du type Rust n'a rien à faire dans une invite"
            );
            assert!(
                params.get("description").is_none(),
                "la description racine vient du `///` français : elle n'a rien à faire \
                 dans une invite"
            );
            assert_eq!(
                params.get("additionalProperties"),
                Some(&serde_json::Value::Bool(false)),
                "un argument inventé doit faire échouer la traduction, pas être ignoré"
            );
            assert!(!spec.description.is_empty());
        }
    }

    #[test]
    fn la_borne_de_la_recherche_est_annoncee_au_modele() {
        // La panne visée : un agent qui ne voit pas la borne la dépasse, se
        // fait refuser, et ne sait pas de combien raccourcir.
        let registre = ToolRegistry::builtin();
        let specs = registre
            .specs_for(&[DESCRIBE_SCHEMA.to_owned()])
            .expect("outil connu");
        let description = specs
            .first()
            .and_then(|spec| spec.parameters.pointer("/properties/search/description"))
            .and_then(serde_json::Value::as_str)
            .expect("`search` est décrit");
        assert!(
            description.contains(&format!("At most {MAX_CATALOG_FOCUS_BYTES} bytes")),
            "{description}"
        );

        // Et la borne annoncée est celle qui refuse.
        let juste = "a".repeat(MAX_CATALOG_FOCUS_BYTES);
        let call = appel(DESCRIBE_SCHEMA, json!({ "search": juste }));
        assert!(registre.translate(&call, &tous(), &scope()).is_ok());
        let trop = "é".repeat(MAX_CATALOG_FOCUS_BYTES / 2 + 1);
        let call = appel(DESCRIBE_SCHEMA, json!({ "search": trop }));
        assert!(registre.translate(&call, &tous(), &scope()).is_err());
    }

    #[test]
    fn les_bornes_d_un_echantillon_sont_annoncees_au_modele() {
        // Même panne que pour la recherche : une borne que le schéma tait est
        // une borne que l'agent dépasse. Les chiffres des attributs sont des
        // littéraux ; ce test les tient alignés sur les constantes qui refusent.
        let specs = ToolRegistry::builtin()
            .specs_for(&[REQUEST_SAMPLE.to_owned()])
            .expect("outil connu");
        let schema = &specs.first().expect("un outil").parameters;
        let pointe = |pointer: &str| schema.pointer(pointer).cloned();
        assert_eq!(
            pointe("/properties/rows/minimum"),
            Some(json!(1)),
            "{schema}"
        );
        assert_eq!(
            pointe("/properties/rows/maximum"),
            Some(json!(MAX_SAMPLE_ROWS)),
            "{schema}"
        );
        assert_eq!(
            pointe("/properties/columns/maxItems"),
            Some(json!(MAX_SAMPLE_COLUMNS)),
            "{schema}"
        );
        let decrit = |champ: &str| {
            schema
                .pointer(&format!("/properties/{champ}/description"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        for champ in ["relation", "namespace", "columns"] {
            assert!(
                decrit(champ).contains(&format!("at most {MAX_SAMPLE_NAME_BYTES} bytes of UTF-8"))
                    || decrit(champ)
                        .contains(&format!("At most {MAX_SAMPLE_NAME_BYTES} bytes of UTF-8")),
                "{champ} : {}",
                decrit(champ)
            );
        }
        assert!(
            decrit("columns").contains(&format!("At most {MAX_SAMPLE_COLUMNS} columns")),
            "{}",
            decrit("columns")
        );
        assert!(
            decrit("rows").contains(&format!(
                "{DEFAULT_SAMPLE_ROWS} when omitted, from 1 to {MAX_SAMPLE_ROWS}"
            )),
            "{}",
            decrit("rows")
        );

        // Et les bornes annoncées sont celles qui refusent.
        assert!(demande(json!({ "relation": "t", "rows": MAX_SAMPLE_ROWS })).is_ok());
        assert!(demande(json!({ "relation": "t", "rows": MAX_SAMPLE_ROWS + 1 })).is_err());
        assert!(demande(json!({ "relation": "t", "rows": 0 })).is_err());
        let colonnes = |n: usize| (0..n).map(|i| format!("c{i}")).collect::<Vec<_>>();
        assert!(
            demande(json!({ "relation": "t", "columns": colonnes(MAX_SAMPLE_COLUMNS) })).is_ok()
        );
        assert!(
            demande(json!({ "relation": "t", "columns": colonnes(MAX_SAMPLE_COLUMNS + 1) }))
                .is_err()
        );
        assert!(demande(json!({ "relation": "a".repeat(MAX_SAMPLE_NAME_BYTES) })).is_ok());
        assert!(demande(json!({ "relation": "é".repeat(MAX_SAMPLE_NAME_BYTES / 2 + 1) })).is_err());
    }

    #[test]
    fn translate_ne_rend_jamais_la_lecture_d_un_echantillon() {
        // L'API publique qui rend une `Command` nue ne doit pas rendre la
        // lecture d'un échantillon : ce serait une lecture de valeurs sans
        // l'écran d'approbation, à portée de tout appelant de la crate.
        let call = appel(REQUEST_SAMPLE, json!({ "relation": "customers" }));
        let refus = ToolRegistry::builtin()
            .translate(&call, &tous(), &scope())
            .expect_err("aucune commande pour un échantillon");
        assert!(
            matches!(&refus, AiError::ToolNotAllowed { name } if name == REQUEST_SAMPLE),
            "{refus:?}"
        );
        // `request`, lui, rend la demande à faire approuver.
        assert!(demande(json!({ "relation": "customers" })).is_ok());
    }

    fn demande(args: serde_json::Value) -> Result<SampleAsk, AiError> {
        let call = appel(REQUEST_SAMPLE, args);
        match ToolRegistry::builtin().request(&call, &tous(), &scope())? {
            ToolRequest::Sample(ask) => Ok(ask),
            ToolRequest::Dispatch(command) => {
                panic!("request_sample doit demander une approbation, pas {command:?}")
            }
        }
    }

    #[test]
    fn un_echantillon_demande_est_une_lecture_bornee_a_faire_approuver() {
        let ask = demande(json!({"relation": "clients"})).expect("appel valide");
        assert_eq!(ask.rows(), DEFAULT_SAMPLE_ROWS, "la valeur par défaut");
        assert!(ask.columns.is_empty(), "toutes, au choix de l'utilisateur");
        assert!(!ask.command.is_mutating());
        let Command::PreviewRelation { shape, .. } = &ask.command else {
            panic!("la lecture est un aperçu : {:?}", ask.command);
        };
        assert_eq!(
            *shape,
            PreviewShape::unordered(),
            "ni filtre ni ordre : l'agent n'écrit aucun prédicat"
        );

        let ask = demande(json!({"relation": "clients", "rows": MAX_SAMPLE_ROWS}))
            .expect("le plafond est permis");
        assert_eq!(ask.rows(), MAX_SAMPLE_ROWS);
        for trop in [0, MAX_SAMPLE_ROWS + 1] {
            let refus =
                demande(json!({"relation": "clients", "rows": trop})).expect_err("hors bornes");
            assert!(matches!(refus, AiError::InvalidArguments { .. }), "{trop}");
        }
    }

    #[test]
    fn le_modele_ne_choisit_ni_la_connexion_ni_un_filtre_pour_un_echantillon() {
        for invente in [
            json!({"relation": "clients", "connection": "00000000-0000-0000-0000-000000000000"}),
            json!({"relation": "clients", "predicate": "1=1"}),
            json!({"relation": "clients", "approved": true}),
        ] {
            let refus = demande(invente.clone()).expect_err("champ hors schéma");
            assert!(
                matches!(refus, AiError::InvalidArguments { .. }),
                "{invente}"
            );
        }
        let perimetre = scope();
        let call = appel(REQUEST_SAMPLE, json!({"relation": "clients"}));
        let Ok(ToolRequest::Sample(ask)) =
            ToolRegistry::builtin().request(&call, &tous(), &perimetre)
        else {
            panic!("appel valide");
        };
        assert_eq!(ask.command.target_connection(), Some(perimetre.connection));
    }

    /// I-10 : un nom hostile reste une **donnée**. Il voyage dans le champ
    /// `relation` de la commande, tel quel, et aucun texte d'instruction n'est
    /// composé ici — c'est le driver qui le citera.
    #[test]
    fn un_nom_hostile_reste_un_champ_et_n_entre_dans_aucune_instruction() {
        let hostile = r#"users"; DROP TABLE audit; --"#;
        let ask = demande(json!({
            "relation": hostile,
            "namespace": hostile,
            "columns": [hostile, "email", "email"],
        }))
        .expect("un nom légal pour le serveur reste demandable");
        assert_eq!(
            ask.columns,
            [hostile, "email"],
            "dédoublonnées, dans l'ordre"
        );
        assert!(
            ask.command.statement_text().is_none(),
            "aucune instruction n'est composée à la traduction"
        );
        let Command::PreviewRelation {
            relation,
            namespace,
            ..
        } = &ask.command
        else {
            panic!("la lecture est un aperçu");
        };
        assert_eq!(relation, hostile);
        assert_eq!(namespace.as_deref(), Some(hostile));
    }

    #[test]
    fn les_noms_d_un_echantillon_sont_bornes() {
        let long = "n".repeat(MAX_SAMPLE_NAME_BYTES + 1);
        for args in [
            json!({"relation": ""}),
            json!({"relation": "   "}),
            json!({"relation": long}),
            json!({"relation": "clients", "columns": [long]}),
            json!({"relation": "clients", "columns": [""]}),
            json!({"relation": "clients", "columns": vec!["c"; MAX_SAMPLE_COLUMNS + 1]}),
        ] {
            assert!(demande(args.clone()).is_err(), "{args}");
        }
    }

    #[test]
    fn un_outil_inconnu_de_la_liste_blanche_echoue_a_la_declaration() {
        let registre = ToolRegistry::builtin();
        let refus = registre
            .specs_for(&["drop_everything".to_owned()])
            .expect_err("outil absent du registre");
        assert!(matches!(refus, AiError::UnknownTool { .. }), "{refus:?}");
    }

    #[test]
    fn un_langage_non_analyse_est_mutant() {
        // Mongo n'est pas analysé par oxyn-query : la classification rend
        // `Unknown`, donc mutant, donc approbation. C'est le comportement voulu
        // tant qu'aucun analyseur n'existe pour ce langage.
        let registre = ToolRegistry::builtin();
        let perimetre = ToolScope::new(
            ConnectionId::new(),
            SessionId::new(),
            QueryLanguage::MongoQuery,
        );
        let call = appel(EXECUTE_QUERY, json!({"statement": "db.clients.find({})"}));
        let commande = registre
            .translate(&call, &tous(), &perimetre)
            .expect("appel valide");
        assert!(commande.is_mutating());
    }
}
