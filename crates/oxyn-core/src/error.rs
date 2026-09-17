//! Le type d'erreur du domaine.
//!
//! La classification vient de [`DRIVER-CONTRACT` §4](../../../docs/DRIVER-CONTRACT.md) :
//! une erreur est **transitoire** (on peut retenter), **permanente** (on n'a
//! aucune raison de retenter) ou **ambiguë** (on ne sait pas si l'effet a eu
//! lieu — et dans ce cas on ne retente surtout pas).
//!
//! [`OxynError::is_retryable`] encode cette classification. Le cas qui coûte le
//! plus cher est celui de l'expiration côté client pendant une écriture : le
//! serveur peut l'avoir appliquée. Rejouer crée un doublon silencieux dans les
//! données de l'utilisateur. C'est pourquoi [`OxynError::Timeout`] n'est **pas**
//! retentable.

use std::time::Duration;

use crate::ids::DriverId;

/// Alias de résultat pour tout le workspace.
pub type Result<T> = std::result::Result<T, OxynError>;

/// Famille d'erreur au sens de
/// [`DRIVER-CONTRACT` §4](../../../docs/DRIVER-CONTRACT.md).
///
/// La classe est une **donnée** portée par l'erreur, jamais une déduction faite
/// par l'appelant à partir du message : un message change, un appelant qui
/// l'analysait casse en silence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorClass {
    /// Coupure réseau, `too many connections`, verrou expiré. L'appelant peut
    /// retenter, avec recul exponentiel.
    Transient,
    /// Erreur de syntaxe, table absente, droits insuffisants. On affiche, on ne
    /// retente **jamais**.
    Permanent,
    /// L'effet côté serveur est inconnu — typiquement une expiration côté
    /// client pendant une écriture. On ne retente **jamais**, et on signale
    /// l'incertitude : rejouer un `INSERT` expiré crée un doublon silencieux
    /// dans les données de l'utilisateur.
    Ambiguous,
}

impl ErrorClass {
    /// Cette famille autorise-t-elle une reprise ?
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(self, Self::Transient)
    }

    /// Nom stable, écrit dans l'état local et lu par un humain sans Oxyn
    /// ([I-11](../../../CLAUDE.md#i-11)).
    ///
    /// Les versions antérieures au 2026-09-16 écrivaient ce code en français ;
    /// `oxyn-store` relit encore ces trois valeurs, mais plus personne ne les
    /// écrit.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Transient => "transient",
            Self::Permanent => "permanent",
            Self::Ambiguous => "ambiguous",
        }
    }
}

/// Un seul nom pour une famille : celui qu'on affiche est celui qu'on écrit.
/// Deux rendus différant par la seule langue laissaient un lecteur incapable
/// de dire lequel des deux était le code d'audit.
impl std::fmt::Display for ErrorClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Erreur du domaine Oxyn.
///
/// Les messages sont destinés à un professionnel : ils reprennent ce que dit le
/// serveur plutôt qu'une paraphrase rassurante. En revanche ils ne contiennent
/// jamais de secret, d'identifiant de connexion ni de valeur liée (I-03) — la
/// responsabilité en revient à qui construit la variante.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OxynError {
    /// Configuration invalide ou incomplète : workspace illisible, paramètre de
    /// connexion manquant, valeur hors domaine.
    #[error("invalid configuration: {0}")]
    Config(String),

    /// La connexion au serveur n'a pas pu être établie ou a été perdue.
    ///
    /// C'est la famille transitoire : coupure réseau, `too many connections`,
    /// serveur en redémarrage.
    #[error("connection failed: {0}")]
    Connection(String),

    /// Le serveur a refusé les identifiants, ou le compte n'a pas les droits
    /// nécessaires pour ouvrir la session.
    #[error("authentication failed: {0}")]
    Authentication(String),

    /// Erreur remontée par un driver, avec la famille à laquelle il la
    /// rattache.
    ///
    /// Le driver **classe** son erreur : c'est lui, et lui seul, qui sait si
    /// `08006` est une coupure ou un refus définitif. L'appelant lit
    /// [`ErrorClass`], il ne relit pas le message.
    #[error("driver `{driver}` ({class} error): {source}")]
    Driver {
        /// Le driver d'où vient l'erreur.
        driver: DriverId,
        /// Famille de l'erreur, telle que le driver la classe.
        class: ErrorClass,
        /// L'erreur d'origine, telle que le driver l'a produite.
        ///
        /// L'effacement de type est ici inévitable — `oxyn-core` ne connaît
        /// aucune implémentation de driver — mais il n'efface **aucune
        /// information de décision** : celle-ci est dans `class`.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Le serveur a rejeté l'instruction : syntaxe, objet absent, contrainte
    /// violée, droits insuffisants. Famille permanente : on affiche, on ne
    /// retente pas.
    #[error("query rejected: {0}")]
    Query(String),

    /// Le délai imparti est écoulé **côté client**.
    ///
    /// Famille ambiguë : rien ne dit que le serveur n'a pas appliqué l'écriture.
    /// Voir [`OxynError::is_retryable`].
    #[error("timed out after {after:?}")]
    Timeout {
        /// Durée au bout de laquelle l'attente a été abandonnée.
        after: Duration,
    },

    /// La requête est partie et son effet côté serveur est inconnu.
    ///
    /// Famille ambiguë, comme [`Timeout`](Self::Timeout), pour le cas où aucun
    /// délai n'a expiré : la connexion a lâché après l'envoi. Un fournisseur de
    /// modèles a peut-être produit — et facturé — la réponse ; un serveur a
    /// peut-être appliqué l'écriture.
    ///
    /// Le texte décrit le fait. Il ne porte ni URL, ni hôte, ni en-tête, ni le
    /// message brut de la pile réseau, qui peut contenir les trois (I-03).
    #[error("outcome unknown: {0}")]
    OutcomeUnknown(String),

    /// L'opération a été annulée, à la demande de l'utilisateur ou par
    /// propagation d'un [`CancelToken`](crate::cancel::CancelToken) parent.
    #[error("operation cancelled")]
    Cancelled,

    /// Le `PolicyGate` a refusé la commande. Ce n'est pas une panne : c'est le
    /// produit qui fait son travail.
    #[error("denied by policy: {reason}")]
    PolicyDenied {
        /// Motif du refus, montrable tel quel à l'utilisateur.
        reason: String,
    },

    /// La commande exige une approbation explicite qui n'a pas été donnée.
    #[error("approval required: {reason}")]
    ApprovalRequired {
        /// Ce sur quoi l'utilisateur doit se prononcer.
        reason: String,
    },

    /// La capacité demandée n'existe pas sur cette session.
    ///
    /// « Ne pas savoir faire est une réponse acceptable ; laisser croire ne
    /// l'est pas » ([`DRIVER-CONTRACT` §5](../../../docs/DRIVER-CONTRACT.md)).
    #[error("capability not supported: {capability}")]
    NotSupported {
        /// Nom du ou des drapeaux manquants.
        capability: String,
    },

    /// Échec d'entrée-sortie locale : débordement disque, fichier de workspace,
    /// export.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Une donnée n'a pas pu être encodée ou décodée : workspace écrit par une
    /// version future, JSON malformé, identifiant illisible.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// Le catalogue n'est pas disponible : introspection en cours, cache vide,
    /// droits insuffisants pour lire les métadonnées.
    #[error("catalog unavailable: {0}")]
    CatalogUnavailable(String),

    /// Invariant interne rompu. C'est un bug d'Oxyn, pas une erreur d'usage.
    #[error("internal error: {0}")]
    Internal(String),
}

impl OxynError {
    /// Construit une erreur de driver en nommant sa famille.
    pub fn driver<E>(driver: DriverId, class: ErrorClass, source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::Driver {
            driver,
            class,
            source: Box::new(source),
        }
    }

    /// Famille de l'erreur, quand elle est connue.
    ///
    /// Seules les erreurs de driver la portent explicitement ; pour les autres
    /// variantes elle se déduit de la variante elle-même, qui est une donnée
    /// tout aussi typée.
    #[must_use]
    pub const fn class(&self) -> ErrorClass {
        match self {
            Self::Driver { class, .. } => *class,
            Self::Connection(_) | Self::CatalogUnavailable(_) => ErrorClass::Transient,
            Self::Timeout { .. } | Self::OutcomeUnknown(_) => ErrorClass::Ambiguous,
            _ => ErrorClass::Permanent,
        }
    }

    /// L'opération peut-elle être rejouée telle quelle, avec recul exponentiel ?
    ///
    /// Seule la famille **transitoire** répond `true`. En particulier :
    ///
    /// * [`Timeout`](Self::Timeout) répond `false` : l'effet côté serveur est
    ///   inconnu, et rejouer un `INSERT` expiré crée un doublon ;
    /// * [`OutcomeUnknown`](Self::OutcomeUnknown) répond `false` pour la même
    ///   raison, sans qu'un délai ait expiré : la requête est partie, la réponse
    ///   n'est pas arrivée ;
    /// * [`Driver`](Self::Driver) répond selon la classe que le driver a
    ///   déclarée, et elle seule.
    ///
    /// La politique de reprise elle-même appartient à l'appelant : un driver ne
    /// retente jamais tout seul, parce que lui seul ne sait pas si l'opération
    /// est rejouable.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        self.class().is_retryable()
    }

    /// L'erreur vient-elle de ce qui a été demandé, plutôt que d'un défaut
    /// d'Oxyn ?
    ///
    /// Sert à décider du registre d'affichage : une erreur d'usage se montre
    /// telle quelle avec l'action suivante ; le reste mérite d'être signalé
    /// comme un incident. [`Cancelled`](Self::Cancelled) n'est ni l'un ni
    /// l'autre — c'est une action délibérée — et répond `false`.
    #[must_use]
    pub const fn is_user_error(&self) -> bool {
        matches!(
            self,
            Self::Config(_)
                | Self::Authentication(_)
                | Self::Query(_)
                | Self::PolicyDenied { .. }
                | Self::ApprovalRequired { .. }
                | Self::NotSupported { .. }
        )
    }

    /// L'opération a-t-elle été interrompue à la demande ?
    #[must_use]
    pub const fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}

impl From<crate::ids::IdParseError> for OxynError {
    fn from(err: crate::ids::IdParseError) -> Self {
        Self::Serialization(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, thiserror::Error)]
    #[error("le socket a été fermé par le pair")]
    struct ErreurDriverFactice;

    #[test]
    fn seules_les_erreurs_transitoires_se_retentent() {
        assert!(OxynError::Connection("réseau coupé".into()).is_retryable());
        assert!(OxynError::CatalogUnavailable("cache vide".into()).is_retryable());

        assert!(!OxynError::Query("syntax error at or near \"slect\"".into()).is_retryable());
        assert!(!OxynError::Authentication("mot de passe refusé".into()).is_retryable());
        assert!(!OxynError::Cancelled.is_retryable());
        assert!(!OxynError::Internal("invariant rompu".into()).is_retryable());
    }

    #[test]
    fn une_expiration_ne_se_retente_jamais() {
        // DRIVER-CONTRACT §4 : l'ambiguïté ne se retente pas. Un INSERT expiré
        // côté client peut avoir été appliqué côté serveur ; le rejouer crée un
        // doublon silencieux.
        let expiration = OxynError::Timeout {
            after: Duration::from_secs(30),
        };
        assert!(!expiration.is_retryable());
    }

    #[test]
    fn une_erreur_de_driver_suit_la_classe_declaree_par_le_driver() {
        let transitoire = OxynError::driver(
            DriverId::postgres(),
            ErrorClass::Transient,
            ErreurDriverFactice,
        );
        assert!(transitoire.is_retryable());
        assert!(transitoire.to_string().contains("postgres"));
        assert!(transitoire.to_string().contains("fermé par le pair"));

        let permanente = OxynError::driver(
            DriverId::postgres(),
            ErrorClass::Permanent,
            ErreurDriverFactice,
        );
        assert!(!permanente.is_retryable());

        let ambigue = OxynError::driver(
            DriverId::sqlite(),
            ErrorClass::Ambiguous,
            ErreurDriverFactice,
        );
        assert!(
            !ambigue.is_retryable(),
            "l'ambiguïté ne se retente pas : le serveur a peut-être appliqué l'écriture"
        );
    }

    #[test]
    fn la_classe_ne_se_deduit_pas_du_message() {
        // Deux erreurs au message identique, deux classes différentes : c'est
        // exactement ce qu'un appelant qui analyserait le texte raterait.
        let a = OxynError::driver(
            DriverId::postgres(),
            ErrorClass::Transient,
            ErreurDriverFactice,
        );
        let b = OxynError::driver(
            DriverId::postgres(),
            ErrorClass::Permanent,
            ErreurDriverFactice,
        );
        assert_ne!(a.is_retryable(), b.is_retryable());
    }

    #[test]
    fn classement_par_famille() {
        assert_eq!(
            OxynError::Connection("coupure".into()).class(),
            ErrorClass::Transient
        );
        assert_eq!(
            OxynError::Timeout {
                after: Duration::from_secs(1)
            }
            .class(),
            ErrorClass::Ambiguous
        );
        let inconnu = OxynError::OutcomeUnknown("connexion perdue après l'envoi".into());
        assert_eq!(inconnu.class(), ErrorClass::Ambiguous);
        assert!(!inconnu.is_retryable());
        assert_eq!(
            OxynError::Query("syntaxe".into()).class(),
            ErrorClass::Permanent
        );
    }

    #[test]
    fn classement_des_erreurs_d_usage() {
        assert!(OxynError::Query("relation \"users\" n'existe pas".into()).is_user_error());
        assert!(
            OxynError::PolicyDenied {
                reason: "connexion en lecture seule".into()
            }
            .is_user_error()
        );
        assert!(
            OxynError::NotSupported {
                capability: "TRANSACTIONS".into()
            }
            .is_user_error()
        );

        assert!(!OxynError::Internal("invariant rompu".into()).is_user_error());
        assert!(!OxynError::Cancelled.is_user_error());
        assert!(!OxynError::Connection("réseau coupé".into()).is_user_error());
    }

    #[test]
    fn une_erreur_est_soit_d_usage_soit_retentable_jamais_les_deux() {
        let cas = [
            OxynError::Config("champ manquant".into()),
            OxynError::Connection("coupure".into()),
            OxynError::Authentication("refusé".into()),
            OxynError::Query("syntaxe".into()),
            OxynError::Timeout {
                after: Duration::from_millis(1),
            },
            OxynError::OutcomeUnknown("inconnu".into()),
            OxynError::Cancelled,
            OxynError::PolicyDenied { reason: "r".into() },
            OxynError::ApprovalRequired { reason: "r".into() },
            OxynError::NotSupported {
                capability: "c".into(),
            },
            OxynError::Serialization("json".into()),
            OxynError::CatalogUnavailable("vide".into()),
            OxynError::Internal("bug".into()),
        ];
        for erreur in &cas {
            assert!(
                !(erreur.is_retryable() && erreur.is_user_error()),
                "{erreur} ne peut pas être à la fois retentable et une erreur d'usage"
            );
        }
    }

    #[test]
    fn un_identifiant_illisible_devient_une_erreur_de_serialisation() {
        let err: OxynError = "pas-un-uuid"
            .parse::<crate::ids::SessionId>()
            .expect_err("invalide")
            .into();
        assert!(matches!(err, OxynError::Serialization(_)));
    }
}
