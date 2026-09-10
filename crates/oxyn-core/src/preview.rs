//! Ce qu'on demande d'un aperçu de relation : un ordre, un filtre, une page.
//!
//! Ces types ne portent **aucun texte SQL**, et c'est leur raison d'être. Un
//! fragment de `WHERE` composé dans l'interface exécuterait ce qu'il contient au
//! premier nom de colonne bien choisi ([I-10](../../CLAUDE.md#i-10)) ; ici la
//! colonne est un identifiant que le driver cite, et la valeur est une
//! [`ScalarValue`] qu'il lie ([ADR-0020](../../docs/adr/0020-apercu-trie-filtre-parcouru.md)).
//!
//! # Ce que le typage empêche d'écrire
//!
//! [`PreviewCondition`] porte sa valeur dans la variante qui en a besoin. Il n'y
//! a donc pas d'état incohérent à valider ensuite : on ne peut pas construire un
//! « est nul » avec une valeur à comparer, ni un « contient » avec un entier.
//! C'est le genre de contrôle qu'un couple `(opérateur, valeur)` obligerait à
//! écrire à la main dans chaque driver — et qu'un driver oublierait.

use serde::{Deserialize, Serialize};

use crate::value::ScalarValue;

/// Une colonne de tri, et son sens.
///
/// `column` est le **nom exact** d'une colonne de la relation, jamais une
/// expression : accepter une expression ici rouvrirait la composition de SQL du
/// mauvais côté de la frontière.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewSort {
    /// Nom de colonne, tel que le catalogue le donne.
    pub column: String,
    /// Décroissant plutôt que croissant.
    pub descending: bool,
}

impl PreviewSort {
    /// Trie cette colonne en ordre croissant.
    #[must_use]
    pub fn ascending(column: impl Into<String>) -> Self {
        Self {
            column: column.into(),
            descending: false,
        }
    }

    /// Trie cette colonne en ordre décroissant.
    #[must_use]
    pub fn descending(column: impl Into<String>) -> Self {
        Self {
            column: column.into(),
            descending: true,
        }
    }
}

/// Ce qu'une colonne doit vérifier pour qu'une ligne soit retenue.
///
/// Énumération **fermée** : chaque variante doit être traduite par chaque
/// driver, et en ajouter une doit faire échouer leur compilation. Une variante
/// qu'un driver ne saurait pas traduire silencieusement rendrait un filtre qui
/// ne filtre pas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "condition")]
pub enum PreviewCondition {
    /// Égal à cette valeur.
    ///
    /// [`ScalarValue::Null`] y est **refusé** par les drivers : en SQL
    /// `= NULL` n'est jamais vrai, et une comparaison qui ne remonte jamais
    /// rien ressemble trop à une table vide. La nullité se demande par
    /// [`IsNull`](Self::IsNull).
    Equals {
        /// La valeur comparée, liée par le driver.
        value: ScalarValue,
    },
    /// Différent de cette valeur.
    NotEquals {
        /// La valeur comparée, liée par le driver.
        value: ScalarValue,
    },
    /// Strictement inférieur.
    LessThan {
        /// La borne, liée par le driver.
        value: ScalarValue,
    },
    /// Inférieur ou égal.
    AtMost {
        /// La borne, liée par le driver.
        value: ScalarValue,
    },
    /// Strictement supérieur.
    GreaterThan {
        /// La borne, liée par le driver.
        value: ScalarValue,
    },
    /// Supérieur ou égal.
    AtLeast {
        /// La borne, liée par le driver.
        value: ScalarValue,
    },
    /// Contient ce texte.
    ///
    /// Le driver **échappe** les métacaractères de son moteur avant de composer
    /// sa recherche : sans cela, chercher `100%` remonterait tout, et personne
    /// ne verrait que le filtre ne fait pas ce qu'il annonce.
    Contains {
        /// Le texte cherché, littéral et non un motif.
        text: String,
    },
    /// Commence par ce texte, mêmes règles d'échappement.
    StartsWith {
        /// Le texte cherché, littéral et non un motif.
        text: String,
    },
    /// Finit par ce texte, mêmes règles d'échappement.
    EndsWith {
        /// Le texte cherché, littéral et non un motif.
        text: String,
    },
    /// La colonne est nulle.
    IsNull,
    /// La colonne n'est pas nulle.
    IsNotNull,
}

impl PreviewCondition {
    /// La valeur à lier, quand la condition en compare une.
    ///
    /// `None` pour les conditions qui n'en ont pas — la nullité — et pour celles
    /// dont le driver compose lui-même le motif à partir d'un texte échappé.
    #[must_use]
    pub const fn bound_value(&self) -> Option<&ScalarValue> {
        match self {
            Self::Equals { value }
            | Self::NotEquals { value }
            | Self::LessThan { value }
            | Self::AtMost { value }
            | Self::GreaterThan { value }
            | Self::AtLeast { value } => Some(value),
            Self::Contains { .. }
            | Self::StartsWith { .. }
            | Self::EndsWith { .. }
            | Self::IsNull
            | Self::IsNotNull => None,
        }
    }
}

/// Un filtre sur une colonne.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreviewFilter {
    /// Nom de colonne, tel que le catalogue le donne.
    pub column: String,
    /// Ce que cette colonne doit vérifier.
    pub condition: PreviewCondition,
}

/// La forme demandée d'un aperçu : son ordre, ses filtres, sa page.
///
/// Groupée plutôt qu'éclatée en trois champs de commande : ces trois-là ne se
/// comprennent qu'ensemble — un `offset` sans ordre déterministe ne veut rien
/// dire — et les séparer inviterait à en oublier un au prochain appelant.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PreviewShape {
    /// L'ordre demandé, vide quand l'utilisateur n'a rien choisi.
    ///
    /// Le driver le **complète** par une clé unique pour lever les ex æquo :
    /// sans ordre total, deux pages consécutives peuvent montrer deux fois la
    /// même ligne et en omettre une autre.
    pub sort: Vec<PreviewSort>,
    /// Les filtres, tous devant être vérifiés ensemble.
    pub filter: Vec<PreviewFilter>,
    /// Combien de lignes sauter avant la page demandée.
    ///
    /// `0` est la première page. Une valeur non nulle n'a de sens que si l'ordre
    /// est déterministe, ce que le driver vérifie : autrement il refuse, plutôt
    /// que de rendre une page dont personne ne peut dire ce qu'elle contient.
    pub offset: u64,
}

impl PreviewShape {
    /// Ne demande rien de particulier : la première page, sans ordre ni filtre.
    #[must_use]
    pub fn unordered() -> Self {
        Self::default()
    }

    /// Rien n'a été demandé : ni ordre, ni filtre, ni page suivante.
    ///
    /// C'est ce qui distingue l'aperçu automatique d'une table qu'on vient de
    /// sélectionner d'une lecture que l'utilisateur a composée.
    #[must_use]
    pub fn is_plain(&self) -> bool {
        self.sort.is_empty() && self.filter.is_empty() && self.offset == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seules_les_conditions_comparatives_portent_une_valeur_a_lier() {
        assert_eq!(
            PreviewCondition::Equals {
                value: ScalarValue::Int64(1)
            }
            .bound_value(),
            Some(&ScalarValue::Int64(1))
        );
        assert!(PreviewCondition::IsNull.bound_value().is_none());
        // Le texte d'un `Contains` n'est pas lié tel quel : le driver en compose
        // un motif après échappement, et c'est ce motif qu'il lie.
        assert!(
            PreviewCondition::Contains {
                text: "100%".into()
            }
            .bound_value()
            .is_none()
        );
    }

    #[test]
    fn un_apercu_sans_demande_se_distingue_d_un_apercu_compose() {
        assert!(PreviewShape::unordered().is_plain());
        let trie = PreviewShape {
            sort: vec![PreviewSort::ascending("id")],
            ..PreviewShape::default()
        };
        assert!(!trie.is_plain());
        let page = PreviewShape {
            offset: 200,
            ..PreviewShape::default()
        };
        assert!(!page.is_plain());
    }
}
