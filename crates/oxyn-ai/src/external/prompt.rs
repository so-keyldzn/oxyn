//! Ce qu'un agent externe reçoit, et la seule porte qui sait le fabriquer.
//!
//! Autorité : [ADR-0027](../../../../docs/adr/0027-porte-unique-pour-les-deux-destinations.md).
//!
//! # Le défaut que ce module ferme
//!
//! [`run_turn`](super::turn::run_turn) prenait son invite en `&str`. Le niveau
//! de la connexion y gouvernait le **lancement** de l'agent — refus avant que le
//! processus ne démarre — mais pas **l'assemblage de ce qui lui est envoyé**,
//! puisqu'il n'y avait rien à assembler.
//!
//! Rien ne fuyait : l'unique appelant ne transmettait que la question tapée par
//! l'utilisateur. Ce qui manquait n'était pas une protection, c'était la
//! **garantie** — pour un fournisseur, « qu'est-ce qui est sorti ? » se répond en
//! relisant une fonction ; pour un agent, il aurait fallu relire tous les
//! appelants, présents et à venir. C'est exactement la propriété
//! qu'[I-04](../../../../CLAUDE.md#i-04) existe pour supprimer.
//!
//! `.claude/rules/ia.md` nomme le raccourci qui la détruit : « juste pour le
//! schéma, c'est du `Metadata` de toute façon ». Il se serait écrit ici en un
//! `format!`, sans qu'aucun type ni aucun test ne rougisse.
//!
//! # Pourquoi un type mince, et non l'`AgentContext` du chemin fournisseur
//!
//! Un agent externe reçoit un texte, et c'est tout : il n'a l'usage ni d'un
//! message système, ni d'une session de conversation à outils. Lui imposer
//! l'`AgentSession` du chemin fournisseur serait l'abstraction pour un seul
//! appelant que [CLAUDE.md](../../../../CLAUDE.md#organisation-du-code)
//! déconseille. Ce type porte donc le texte — et, quand un schéma l'accompagne,
//! l'[`AgentContext`] qui l'a rendu, pour que l'appelant dise ce qui est parti.
//!
//! Ce module prend l'option **B** d'ADR-0027 : un type qui ne porte que ce qui
//! part, dont les constructeurs exigent le niveau de la connexion.
//!
//! # Le schéma, et par quelle porte il entre
//!
//! Un agent externe qui ne reçoit que la question ne connaît pas la base : il
//! lance `SELECT name FROM sqlite_master`, reçoit « 11 rows » — l'outil rend la
//! forme, jamais les valeurs (ADR-0030 § 4) — et finit par proposer
//! `SELECT * FROM your_table`. C'est ce qu'a constaté l'utilisateur le
//! 2026-09-23.
//!
//! [`AgentPrompt::with_schema`] y répond **sans écrire de seconde porte** : le
//! schéma est rendu par [`ContextBuilder::build`], le point de passage d'I-04,
//! le même code, sous le même niveau, avec le même budget que pour l'assistant
//! interne. Ce module ne rend rien lui-même ; il place un bloc déjà rendu.
//!
//! # L'échantillon approuvé, et par la même porte
//!
//! Un échantillon que l'utilisateur a approuvé colonne par colonne entre dans
//! l'invite par le **même** appel : [`AgentPrompt::with_schema`] le passe à
//! [`ContextBuilder::with_samples`], qui l'écarte sous tout niveau autre que
//! `Sampled`. Ce module ne rend aucune valeur lui-même
//! ([ADR-0034](../../../../docs/adr/0034-echantillon-pour-toute-destination.md)).
//!
//! La mémoire de l'agent n'est pas l'affaire de ce type, et c'est pourquoi
//! [`AgentPrompt::from_user`] ne prend pas d'échantillon : une invite qui
//! **continue** une session parle à un processus qui se souvient, et une valeur
//! montrée là y resterait. L'appelant qui joint un échantillon ouvre une session
//! neuve — `with_schema` est le constructeur d'ouverture — et la relâche après
//! l'échange, qui ne laisse aucune mémoire.

use std::fmt;

use oxyn_catalog::CatalogCache;
use oxyn_core::{OxynError, QueryLanguage};

use crate::context::{AgentContext, ContextBuilder, RowSample};
use crate::privacy::PrivacyTier;
use crate::untrusted;

/// Ce qui précède le schéma : d'où il vient, et ce qu'on peut en attendre.
///
/// En anglais, parce qu'il part vers un modèle. Il dit aussi ce que l'outil
/// **ne** rend pas : un agent qui attend des lignes de `execute_query` conclut
/// qu'il ne peut pas lire la base, et c'est la panne constatée. Composé à
/// l'appel : les noms d'outils et du serveur viennent de leurs constantes, pas
/// d'une copie.
fn schema_intro() -> String {
    format!(
        "You are working inside Oxyn, a database workspace. The structure of the database the \
         user has open is described below, as far as it fits. Write statements only against \
         the objects and fields it names; for anything it leaves out, call the \
         `{describe}` tool of the `{server}` MCP server with search words rather than guessing \
         a name. Run a statement with its `{execute}` tool: the rows appear in the user's \
         result grid, and the tool returns you the shape of the result — row and batch \
         counts — never the values.",
        describe = crate::tools::DESCRIBE_SCHEMA,
        execute = crate::tools::EXECUTE_QUERY,
        server = super::mcp::SERVER_NAME,
    )
}

/// Ce qui annonce la question, pour qu'elle ne se confonde pas avec le schéma.
const QUESTION_HEADER: &str = "The user's question:";

/// Ce qui part vers un agent externe, une fois le niveau appliqué.
///
/// **Aucun constructeur public naïf.** Les seules façons d'en obtenir un sont
/// [`AgentPrompt::from_user`] et [`AgentPrompt::with_schema`], qui exigent le
/// niveau de la connexion. Un `From<String>` ou un `new(&str)` rouvrirait
/// exactement le trou que ce type ferme — c'est la discipline à maintenir, et la
/// seule.
///
/// Le `Debug` est écrit à la main : le texte porte des noms de tables et de
/// colonnes de la base de l'utilisateur, qu'un `tracing::debug!("{prompt:?}")`
/// écrirait dans un journal.
#[derive(Clone, PartialEq)]
pub struct AgentPrompt {
    text: String,
    context: Option<AgentContext>,
}

impl fmt::Debug for AgentPrompt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentPrompt")
            .field(
                "text",
                &format_args!("<redacted, {} bytes>", self.text.len()),
            )
            .field("context", &self.context)
            .finish()
    }
}

impl AgentPrompt {
    /// Compose l'invite à partir de ce que **l'utilisateur a tapé**, et de rien
    /// d'autre.
    ///
    /// # Ce que cette porte garantit, et ce qu'elle ne garantit pas
    ///
    /// Elle garantit qu'une invite d'agent externe ne peut naître que d'une
    /// saisie utilisateur, sous un niveau qui admet cette destination. Elle ne
    /// prétend pas filtrer le contenu de la saisie : ce que l'utilisateur écrit
    /// lui appartient, et [ADR-0006](../../../../docs/adr/0006-ai-privacy-tiers.md)
    /// n'a jamais eu pour objet de censurer sa propre question.
    ///
    /// Du contexte ne rejoint cette invite que par [`AgentPrompt::with_schema`],
    /// qui le fait rendre par [`ContextBuilder::build`].
    ///
    /// # Erreurs
    ///
    /// [`OxynError::Config`] si le niveau ferme les agents externes, ou si la
    /// question est vide. Le message ne recopie jamais la saisie.
    pub fn from_user(tier: PrivacyTier, question: &str) -> Result<Self, OxynError> {
        // Le même refus que `run_turn`, mais **avant** qu'une invite n'existe :
        // sous un niveau qui ferme les agents, il n'y a rien à composer.
        if !tier.allows_remote_provider() {
            return Err(OxynError::Config(
                "this connection is marked local-only, and Oxyn cannot see where an external \
                 agent sends its prompts; declare a local model provider instead"
                    .into(),
            ));
        }
        let question = question.trim();
        if question.is_empty() {
            return Err(OxynError::Config("an empty question is not sent".into()));
        }
        Ok(Self {
            text: question.to_owned(),
            context: None,
        })
    }

    /// Compose l'invite qui ouvre une session d'agent : la structure de la base,
    /// puis la question.
    ///
    /// Le schéma est rendu par [`ContextBuilder::build`] sous `tier` — le point
    /// de passage d'[I-04](../../../../CLAUDE.md#i-04), avec son budget, sa
    /// sélection orientée par la question et son encadré `untrusted`. Le
    /// préambule qui dit ce qu'est un encadré vient **avant** l'encadré, comme
    /// dans le message système de l'assistant interne : un modèle qui lit la
    /// consigne après les données a déjà lu les données.
    ///
    /// `samples` sont les échantillons que l'utilisateur a approuvés pour
    /// **cette** question. Ils entrent par [`ContextBuilder::with_samples`], et
    /// n'en sortent que sous `Sampled` : sous tout autre niveau ils sont
    /// écartés, et [`AgentContext::dropped_samples`] le dit. L'appelant qui en
    /// joint ouvre une session neuve et la relâche après l'échange (voir
    /// l'en-tête du module).
    ///
    /// # Erreurs
    ///
    /// Celles de [`AgentPrompt::from_user`], vérifiées **avant** que le moindre
    /// schéma ne soit rendu.
    pub fn with_schema(
        tier: PrivacyTier,
        question: &str,
        cache: &CatalogCache,
        language: QueryLanguage,
        samples: Vec<RowSample>,
    ) -> Result<Self, OxynError> {
        let asked = Self::from_user(tier, question)?;
        let context = ContextBuilder::new(cache, tier)
            .with_language(language)
            .focused_on(asked.text.clone())
            .with_samples(samples)
            .build();
        let text = format!(
            "{}\n\n{}\n\n{}\n\n{QUESTION_HEADER}\n{}",
            schema_intro(),
            untrusted::PREAMBLE,
            context.prompt_block(),
            asked.text
        );
        Ok(Self {
            text,
            context: Some(context),
        })
    }

    /// Le texte qui part sur le protocole.
    ///
    /// Emprunté et non rendu : ce type n'existe que pour être consommé par
    /// [`run_turn`](super::turn::run_turn).
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Le contexte joint, s'il y en a un — pour dire à l'utilisateur ce qui
    /// est parti : combien de relations, combien écartées, quel coût.
    #[must_use]
    pub const fn context(&self) -> Option<&AgentContext> {
        self.context.as_ref()
    }
}

#[cfg(test)]
mod tests;
