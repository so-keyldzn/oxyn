//! Ce qu'on demande d'un aperçu de relation : un ordre, un prédicat, une page.
//!
//! # Deux moitiés qui ne se ressemblent pas
//!
//! L'**ordre** est structuré : une colonne est un identifiant que le driver
//! cite, jamais une expression. Le laisser libre rouvrirait la composition de
//! SQL du mauvais côté de la frontière, sur une chaîne qu'Oxyn insérerait dans
//! une requête qu'il compose lui-même ([I-10](../../CLAUDE.md#i-10)).
//!
//! Le **prédicat**, lui, est du SQL que l'utilisateur écrit. C'est le champ
//! `WHERE` de la maquette (`272:10667`), et [I-10](../../CLAUDE.md#i-10) le dit
//! sans ambiguïté : « le SQL que *l'utilisateur écrit* part tel quel — c'est la
//! fonctionnalité ». Ce qui est interdit, c'est qu'Oxyn concatène un
//! identifiant **reçu du serveur** ; pas qu'il transmette ce qu'un
//! professionnel a tapé.
//!
//! Ce prédicat n'est pour autant pas une porte ouverte. Le texte final est
//! reclassifié par `oxyn-query` et refusé s'il devient mutant, la session est
//! tenue en lecture seule par le serveur, et la borne de lignes s'applique
//! ([ADR-0020](../../docs/adr/0020-apercu-trie-filtre-parcouru.md)).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{OxynError, Result};

/// Une colonne de tri, et son sens.
///
/// `column` est le **nom exact** d'une colonne de la relation, jamais une
/// expression : c'est la moitié structurée de la demande, celle qu'Oxyn compose
/// et cite lui-même.
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

/// La forme demandée d'un aperçu : son ordre, son prédicat, sa page.
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
    /// Le prédicat écrit par l'utilisateur, **sans** le mot-clé `WHERE`.
    ///
    /// Transmis tel quel : ni analysé, ni réécrit, ni complété. Un prédicat vide
    /// ou fait d'espaces vaut « aucun filtre » — insérer un `WHERE` sans
    /// condition produirait une erreur de syntaxe là où l'utilisateur croit
    /// avoir tout effacé.
    pub predicate: Option<String>,
    /// Combien de lignes sauter avant la page demandée.
    ///
    /// `0` est la première page. Une valeur non nulle n'a de sens que si l'ordre
    /// est déterministe, ce que le driver vérifie : autrement il refuse, plutôt
    /// que de rendre une page dont personne ne peut dire ce qu'elle contient.
    pub offset: u64,
    /// Les seules colonnes à lire, dans cet ordre ; `None` les lit toutes.
    ///
    /// C'est ce qui borne une lecture à ce que l'utilisateur a approuvé : un
    /// échantillon dont trois colonnes sont cochées ne rapatrie pas les douze
    /// autres pour les jeter ensuite. Comme l'ordre, c'est une moitié
    /// structurée : des **noms exacts**, que le driver cite et refuse quand la
    /// relation ne les déclare pas, jamais des expressions
    /// ([I-10](../../CLAUDE.md#i-10)).
    ///
    /// Lue par [`Self::projection`], qui la déduplique et la borne. Absente
    /// d'une forme sérialisée avant elle, elle vaut « toutes ».
    #[serde(default)]
    pub columns: Option<Vec<String>>,
}

/// Combien de noms une projection peut porter, doublons compris.
///
/// Une borne d'Oxyn, pas celle d'un moteur : elle tient la taille d'une
/// commande reçue par l'IPC ou d'un agent — comptée avant la déduplication,
/// sans quoi un million de fois le même nom passerait —, et dépasse ce qu'un
/// écran d'approbation propose de cocher.
pub const MAX_PROJECTED_COLUMNS: usize = 1024;

impl PreviewShape {
    /// Ne demande rien de particulier : la première page, sans ordre ni filtre.
    #[must_use]
    pub fn unordered() -> Self {
        Self::default()
    }

    /// Le prédicat, s'il en reste un une fois les espaces retirés.
    ///
    /// C'est la seule normalisation appliquée au texte de l'utilisateur, et elle
    /// ne change pas son sens : un champ où il ne reste qu'une espace est un
    /// champ vide.
    #[must_use]
    pub fn predicate(&self) -> Option<&str> {
        self.predicate
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
    }

    /// Cette demande exige-t-elle un ordre **total** ?
    ///
    /// Vrai dès qu'un tri est demandé ou qu'une page autre que la première
    /// l'est. Dans les deux cas le driver doit compléter l'ordre par une clé
    /// unique, ce qui lui coûte une lecture de métadonnées : c'est la seule
    /// raison pour laquelle un aperçu en paie une, et un aperçu sans demande
    /// n'en paie aucune.
    ///
    /// Le tri seul en a besoin autant que la page : ordonner la première page
    /// par une colonne et la suivante par deux ferait réapparaître une ligne
    /// exactement à la frontière.
    #[must_use]
    pub fn needs_total_order(&self) -> bool {
        !self.sort.is_empty() || self.offset > 0
    }

    /// Rien n'a été demandé : ni ordre, ni prédicat, ni page suivante.
    ///
    /// C'est ce qui distingue l'aperçu automatique d'une table qu'on vient de
    /// sélectionner d'une lecture que l'utilisateur a composée.
    #[must_use]
    pub fn is_plain(&self) -> bool {
        self.sort.is_empty() && self.predicate().is_none() && self.offset == 0
    }

    /// La projection à composer : `None` pour toutes les colonnes, sinon les
    /// noms demandés, chacun une fois, dans l'ordre de leur première mention.
    ///
    /// Un doublon ne se refuse pas — cocher deux fois la même colonne ne dit
    /// rien d'autre que la cocher —, mais il ne se compose pas non plus : deux
    /// colonnes de même nom dans un résultat rendent ambiguë la lecture par nom.
    ///
    /// # Erreurs
    /// [`OxynError::Config`] quand la liste est vide ou qu'elle dépasse
    /// [`MAX_PROJECTED_COLUMNS`]. Une liste vide n'est pas « toutes » : c'est une
    /// demande qui ne lit rien, et la composer en `SELECT *` lirait justement
    /// ce que personne n'a approuvé.
    pub fn projection(&self) -> Result<Option<Vec<&str>>> {
        let Some(columns) = &self.columns else {
            return Ok(None);
        };
        if columns.is_empty() {
            return Err(OxynError::Config(
                "a preview projection must name at least one column".into(),
            ));
        }
        if columns.len() > MAX_PROJECTED_COLUMNS {
            return Err(OxynError::Config(format!(
                "a preview projection names at most {MAX_PROJECTED_COLUMNS} columns"
            )));
        }
        let mut seen = HashSet::with_capacity(columns.len());
        Ok(Some(
            columns
                .iter()
                .map(String::as_str)
                .filter(|name| seen.insert(*name))
                .collect(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_predicat_vide_ou_blanc_ne_filtre_rien() {
        let vide = PreviewShape {
            predicate: Some(String::new()),
            ..PreviewShape::default()
        };
        assert!(vide.predicate().is_none());
        assert!(vide.is_plain(), "un champ effacé ne compose aucun WHERE");

        let blanc = PreviewShape {
            predicate: Some("   \n\t ".into()),
            ..PreviewShape::default()
        };
        assert!(blanc.predicate().is_none());
    }

    #[test]
    fn le_texte_de_l_utilisateur_n_est_pas_reecrit() {
        // Les espaces de bordure tombent, le reste est intact : ni normalisation
        // de casse, ni guillemets ajoutés, ni opérateur traduit.
        let forme = PreviewShape {
            predicate: Some("  status = 'active' AND note LIKE '100%'  ".into()),
            ..PreviewShape::default()
        };
        assert_eq!(
            forme.predicate(),
            Some("status = 'active' AND note LIKE '100%'")
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
        let filtre = PreviewShape {
            predicate: Some("id > 10".into()),
            ..PreviewShape::default()
        };
        assert!(!filtre.is_plain());
        let page = PreviewShape {
            offset: 200,
            ..PreviewShape::default()
        };
        assert!(!page.is_plain());
    }

    #[test]
    fn une_projection_est_dedoublonnee_dans_l_ordre_de_premiere_mention() {
        assert_eq!(PreviewShape::unordered().projection().ok(), Some(None));
        let forme = PreviewShape {
            columns: Some(vec!["email".into(), "id".into(), "email".into()]),
            ..PreviewShape::default()
        };
        assert_eq!(forme.projection().ok().flatten(), Some(vec!["email", "id"]));
    }

    #[test]
    fn une_projection_vide_ou_demesuree_est_refusee() {
        // Vide, elle ne vaut pas « toutes » : la composer en `SELECT *` lirait
        // ce que personne n'a coché.
        let vide = PreviewShape {
            columns: Some(Vec::new()),
            ..PreviewShape::default()
        };
        assert!(matches!(vide.projection(), Err(OxynError::Config(_))));

        let limite: Vec<String> = (0..MAX_PROJECTED_COLUMNS)
            .map(|n| format!("c{n}"))
            .collect();
        let mut trop = limite.clone();
        trop.push("une de plus".into());
        let au_plafond = PreviewShape {
            columns: Some(limite),
            ..PreviewShape::default()
        };
        assert!(au_plafond.projection().is_ok());
        let dessus = PreviewShape {
            columns: Some(trop),
            ..PreviewShape::default()
        };
        assert!(matches!(dessus.projection(), Err(OxynError::Config(_))));
        // La borne compte les noms reçus, doublons compris : un seul nom répété
        // sans fin ne passe pas sous elle.
        let repete = PreviewShape {
            columns: Some(vec!["id".to_owned(); MAX_PROJECTED_COLUMNS + 1]),
            ..PreviewShape::default()
        };
        assert!(matches!(repete.projection(), Err(OxynError::Config(_))));
    }

    #[test]
    fn une_forme_serialisee_sans_projection_lit_toutes_les_colonnes() {
        let ancienne: PreviewShape =
            serde_json::from_str(r#"{"sort":[],"predicate":null,"offset":0}"#)
                .expect("forme d'avant la projection");
        assert_eq!(ancienne.columns, None);
    }

    #[test]
    fn un_ordre_total_est_exige_par_le_tri_autant_que_par_la_page() {
        assert!(!PreviewShape::unordered().needs_total_order());
        // Un prédicat seul ne change pas l'ordre : il n'exige aucune clé, et ne
        // doit donc pas coûter une lecture de métadonnées.
        let filtre = PreviewShape {
            predicate: Some("id > 10".into()),
            ..PreviewShape::default()
        };
        assert!(!filtre.needs_total_order());

        let trie = PreviewShape {
            sort: vec![PreviewSort::ascending("name")],
            ..PreviewShape::default()
        };
        assert!(trie.needs_total_order(), "dès la première page");
        let page = PreviewShape {
            offset: 200,
            ..PreviewShape::default()
        };
        assert!(page.needs_total_order());
    }
}
