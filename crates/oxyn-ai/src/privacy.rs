//! Les trois niveaux de confidentialité, choisis **par connexion**.
//!
//! Autorité : [ADR-0006](../../../docs/adr/0006-ai-privacy-tiers.md). Le tableau
//! des niveaux y vit et n'est pas recopié ici ; ce module porte ce que l'ADR ne
//! dit pas : les conséquences dans le code.
//!
//! # Pourquoi par connexion, et jamais globalement
//!
//! La panne visée par [AI-PROVIDERS](../../../docs/AI-PROVIDERS.md) : l'utilisateur
//! règle le niveau sur `Sampled` pour sa base de bac à sable, l'oublie, puis
//! ouvre trois jours plus tard la base client de son employeur. Si le niveau
//! était global, des lignes réelles partiraient chez un fournisseur tiers.
//! Techniquement rien n'a échoué ; contractuellement, c'est irréversible.
//!
//! Conséquence de conception : ce type ne porte **aucun** constructeur qui le
//! dérive d'un fournisseur, d'une session ou d'un réglage d'application. Il
//! vient de la [`ConnectionConfig`](oxyn_core::ConnectionConfig) et de rien
//! d'autre.
//!
//! # `Local` est une garantie, pas une préférence
//!
//! [`PrivacyTier::allows_remote_provider`] rend `false` pour `Local`, et
//! [`AgentRuntime::run`](crate::runtime::AgentRuntime::run) refuse avant
//! d'assembler quoi que ce soit : il n'existe pas de chemin qui envoie une
//! invite hors de la machine sous ce niveau. La vérification a lieu sur la
//! **session**, parce que c'est le seul endroit où le niveau de la connexion
//! est connu. Le classement local/distant se fait sur l'hôte réel
//! **après résolution** ([`Reach`]), jamais sur la présence de `localhost` dans
//! une URL — un point d'accès compatible OpenAI en écoute sur la boucle locale
//! peut être un proxy vers le nuage.
//!
//! # `Metadata` par défaut n'est pas « rien ne sort »
//!
//! Le DDL, les noms, les types, les index et les cardinalités **sortent** dès
//! qu'un fournisseur distant est configuré. Une table `patients` avec une
//! colonne `hiv_status` révèle l'essentiel sans qu'une seule ligne ne sorte.
//! C'est un compromis délibéré, et l'interface doit le montrer en permanence.

use std::fmt;

use oxyn_llm::Reach;
use serde::{Deserialize, Serialize};

/// Ce qui a le droit de quitter la machine pour une connexion donnée.
///
/// L'énumération est **fermée**, contrairement à la convention du dépôt sur les
/// énumérations publiques : la triade d'ADR-0006 est un contrat, et un
/// quatrième niveau serait une décision d'ADR, pas une variante ajoutée au fil
/// de l'eau. Un `_ =>` dans l'interface qui avalerait un niveau inconnu
/// choisirait silencieusement le mauvais comportement.
///
/// L'ordre est celui de la **divulgation croissante** : `Local < Metadata <
/// Sampled`. C'est ce qui rend [`most_restrictive`](Self::most_restrictive)
/// écrivable, et donc composable quand deux niveaux s'appliquent au même envoi.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyTier {
    /// Rien ne quitte la machine. Modèle local uniquement.
    Local,
    /// DDL, noms, types, index, cardinalités, plans d'exécution.
    /// **Aucune valeur de ligne.** C'est le défaut.
    #[default]
    Metadata,
    /// Idem, plus un échantillon de lignes explicitement approuvé, colonne par
    /// colonne.
    Sampled,
}

impl PrivacyTier {
    /// Des valeurs de lignes peuvent-elles rejoindre une invite ?
    ///
    /// Seul [`Sampled`](Self::Sampled) répond `true`, et même alors les valeurs
    /// doivent avoir été approuvées colonne par colonne en amont : ce prédicat
    /// est une condition nécessaire, pas suffisante.
    #[must_use]
    pub const fn allows_row_values(&self) -> bool {
        matches!(self, Self::Sampled)
    }

    /// Un fournisseur dont les données quittent la machine est-il utilisable ?
    ///
    /// `false` pour [`Local`](Self::Local). Ce n'est pas une valeur par défaut
    /// qu'un réglage renverse : c'est la promesse du niveau.
    #[must_use]
    pub const fn allows_remote_provider(&self) -> bool {
        !matches!(self, Self::Local)
    }

    /// Ce point d'accès est-il utilisable sous ce niveau ?
    ///
    /// [`Reach::Unresolved`] est traité comme distant : un point d'accès qu'on
    /// n'a pas su classer n'obtient pas le bénéfice du doute
    /// ([`Reach::leaves_machine`]).
    #[must_use]
    pub const fn allows_endpoint(&self, reach: Reach) -> bool {
        !reach.leaves_machine() || self.allows_remote_provider()
    }

    /// Le plus contraignant des deux niveaux.
    ///
    /// Sert partout où deux niveaux se rencontrent — une conversation qui
    /// touche deux connexions, un contexte assemblé avant que l'utilisateur ne
    /// change de connexion. Le résultat ne divulgue jamais plus que le plus
    /// prudent des deux.
    #[must_use]
    pub fn most_restrictive(self, other: Self) -> Self {
        self.min(other)
    }

    /// Nom stable, pour l'affichage, la persistance et l'audit.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Metadata => "metadata",
            Self::Sampled => "sampled",
        }
    }

    /// Ce qui sort de la machine sous ce niveau, en une phrase montrable.
    ///
    /// L'interface doit afficher le niveau effectif **en permanence** et non
    /// dans un panneau de réglages : un utilisateur qui ne peut pas dire d'un
    /// coup d'œil où part sa requête ne donne pas un consentement éclairé
    /// (AI-PROVIDERS).
    #[must_use]
    pub const fn describe(&self) -> &'static str {
        match self {
            Self::Local => "nothing leaves this machine; local model only",
            Self::Metadata => "schema only: names, types, indexes, cardinalities — no row values",
            Self::Sampled => "schema, plus row samples you approved column by column",
        }
    }
}

impl fmt::Display for PrivacyTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_defaut_est_metadata() {
        // ADR-0006 : le défaut est sûr, et envoyer des valeurs est un acte
        // délibéré.
        assert_eq!(PrivacyTier::default(), PrivacyTier::Metadata);
        assert!(!PrivacyTier::default().allows_row_values());
    }

    #[test]
    fn seul_sampled_laisse_sortir_des_valeurs() {
        assert!(!PrivacyTier::Local.allows_row_values());
        assert!(!PrivacyTier::Metadata.allows_row_values());
        assert!(PrivacyTier::Sampled.allows_row_values());
    }

    #[test]
    fn local_interdit_tout_fournisseur_distant() {
        assert!(!PrivacyTier::Local.allows_remote_provider());
        assert!(PrivacyTier::Metadata.allows_remote_provider());
        assert!(PrivacyTier::Sampled.allows_remote_provider());
    }

    #[test]
    fn un_point_d_acces_non_resolu_est_traite_comme_distant() {
        // Le piège d'AI-PROVIDERS : un proxy en écoute sur localhost. Le
        // classement vient de `Reach`, jamais de la forme de l'URL.
        assert!(PrivacyTier::Local.allows_endpoint(Reach::Local));
        assert!(!PrivacyTier::Local.allows_endpoint(Reach::Remote));
        assert!(
            !PrivacyTier::Local.allows_endpoint(Reach::Unresolved),
            "dans le doute, on protège"
        );
        assert!(PrivacyTier::Metadata.allows_endpoint(Reach::Unresolved));
    }

    #[test]
    fn le_plus_contraignant_gagne() {
        assert_eq!(
            PrivacyTier::Sampled.most_restrictive(PrivacyTier::Metadata),
            PrivacyTier::Metadata
        );
        assert_eq!(
            PrivacyTier::Metadata.most_restrictive(PrivacyTier::Local),
            PrivacyTier::Local
        );
        assert_eq!(
            PrivacyTier::Local.most_restrictive(PrivacyTier::Local),
            PrivacyTier::Local
        );
    }

    #[test]
    fn un_niveau_se_relit_apres_serialisation() {
        // Le niveau est persisté avec la connexion : l'aller-retour doit être
        // exact, sinon un workspace relu dégraderait la protection.
        for niveau in [
            PrivacyTier::Local,
            PrivacyTier::Metadata,
            PrivacyTier::Sampled,
        ] {
            let json = serde_json::to_string(&niveau).expect("sérialisation");
            let relu: PrivacyTier = serde_json::from_str(&json).expect("désérialisation");
            assert_eq!(relu, niveau, "{json}");
        }
    }
}
