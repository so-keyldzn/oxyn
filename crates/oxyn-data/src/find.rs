//! `Find in loaded results…` : où sont les correspondances, et ce qui n'a pas
//! été regardé.
//!
//! # Trouver n'est pas filtrer
//!
//! La maquette `191:1521` écrit **`Find`**, et sa grille montre les dix lignes
//! du résultat sous le champ de recherche — aucune n'est cachée, aucun compteur
//! de correspondances ne s'affiche. Ce module rend donc **où regarder**, pas
//! quoi montrer : la grille révèle et souligne, elle ne retranche rien.
//!
//! La distinction n'est pas cosmétique. Un filtre d'affichage rendrait fausse
//! la règle « [ce qui est exporté est ce qui est
//! affiché](../../../docs/UX-SPEC.md) », puisque l'export porte le tampon et non
//! la vue. Une recherche qui révèle laisse cette règle intacte.
//!
//! # Ce qui est parcouru, et ce qui ne l'est pas
//!
//! **Les lots résidents seulement.** Un [`ResultBuffer`] déborde sur disque
//! au-delà de son budget, et `batch` « peut lire le disque » : l'appeler ici
//! ferait une entrée-sortie sur le fil d'interface
//! ([I-05](../../../CLAUDE.md#i-05)). Le libellé de la maquette dit lui-même
//! « in **loaded** results ».
//!
//! Le compte des lots sautés voyage donc avec le résultat. Une recherche qui
//! tait ce qu'elle n'a pas lu ment par omission : « aucune correspondance »
//! voudrait alors dire « aucune correspondance dans ce que j'ai bien voulu
//! regarder », et l'utilisateur conclurait que la valeur n'est pas là.

use crate::buffer::{BatchIndex, ResultBuffer};
use crate::cell::{CellValue, FormatOptions, format_cell};

/// Combien de correspondances sont conservées au plus.
///
/// Sans plafond, `rows` croît avec les données du serveur : sur un résultat
/// d'une colonne étroite, le budget résident tient des dizaines de millions de
/// lignes, et une aiguille peu sélective — un chiffre, une lettre — les fait
/// presque toutes correspondre. Le `Vec` d'indices dépasserait alors le budget
/// que tout le reste du code respecte ([I-06](../../../CLAUDE.md#i-06)).
///
/// 50 000 est très au-delà de ce qu'une navigation « correspondance suivante »
/// peut servir, et borne l'allocation à quelques centaines de kilooctets.
pub const MATCH_LIMIT: usize = 50_000;

/// Ce qu'une recherche a trouvé, et ce qu'elle n'a pas pu lire.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct FindOutcome {
    /// Indices de ligne, absolus dans le tampon, en ordre croissant.
    ///
    /// Croissant parce que « correspondance suivante » n'a de sens que sur une
    /// suite ordonnée, et parce que la recherche parcourt les lots dans l'ordre.
    pub rows: Vec<usize>,
    /// Lots non parcourus parce qu'ils avaient débordé sur disque.
    ///
    /// Non nul, il doit être **dit à l'écran** : voir le module.
    pub skipped_batches: usize,
    /// Le parcours s'est arrêté à [`MATCH_LIMIT`] correspondances.
    ///
    /// À dire à l'écran pour la même raison que `skipped_batches` : sans cela,
    /// « 50 000 correspondances » se lit comme un compte exact.
    pub capped: bool,
}

impl FindOutcome {
    /// La première correspondance au niveau ou après `row`.
    ///
    /// Rend la première de toutes quand il n'y en a plus après : une recherche
    /// qui s'arrête au bas du résultat oblige à remonter à la main, ce qu'aucun
    /// éditeur ne fait.
    #[must_use]
    pub fn next_from(&self, row: usize) -> Option<usize> {
        self.rows
            .iter()
            .copied()
            .find(|candidate| *candidate >= row)
            .or_else(|| self.rows.first().copied())
    }

    /// La dernière correspondance strictement avant `row`, en rebouclant.
    #[must_use]
    pub fn previous_from(&self, row: usize) -> Option<usize> {
        self.rows
            .iter()
            .copied()
            .rev()
            .find(|candidate| *candidate < row)
            .or_else(|| self.rows.last().copied())
    }
}

/// Les lignes des lots **résidents** qui contiennent `needle`.
///
/// La comparaison est insensible à la casse et porte sur la valeur **entière**,
/// pas sur son affichage : la grille coupe à 512 caractères, et une recherche
/// qui s'arrêterait là rendrait « aucune correspondance » pour un identifiant
/// présent plus loin dans une valeur `jsonb`. Un `NULL` ne correspond jamais —
/// chercher « null » trouverait sinon toutes les absences de valeur, ce que
/// personne ne demande en tapant ce mot.
///
/// Au-delà de [`MATCH_LIMIT`] correspondances le parcours cesse d'en retenir,
/// et `capped` le dit. Les lots restants sont tout de même visités : c'est ce
/// qui garde `skipped_batches` exact, et ce compte-là ne doit pas dépendre du
/// moment où le plafond est atteint.
///
/// Une aiguille vide ne rend aucune correspondance : elle en rendrait toutes,
/// ce qui revient à ne pas chercher.
///
/// **Ne lit jamais le disque** ([I-05](../../../CLAUDE.md#i-05)) : à appeler
/// malgré tout hors du fil d'interface, parce que le coût croît avec le nombre
/// de cellules résidentes.
#[must_use]
pub fn find_rows(buffer: &ResultBuffer, needle: &str, options: &FormatOptions) -> FindOutcome {
    let aiguille = needle.trim();
    if aiguille.is_empty() {
        return FindOutcome::default();
    }
    let aiguille = aiguille.to_lowercase();

    // La recherche porte sur la valeur **entière**, pas sur son affichage coupé
    // à 512 caractères : une valeur `jsonb` dont l'identifiant recherché tombe
    // au-delà de la coupe donnerait « aucune correspondance », et l'utilisateur
    // en conclurait que la valeur n'est pas dans son résultat.
    let entier = options.clone().with_max_len(0);

    let mut trouvees = Vec::new();
    let mut sautes = 0usize;
    let mut plafonne = false;
    for position in 0..buffer.batch_count() {
        let index = BatchIndex::new(position);
        if !buffer.is_resident(index) {
            sautes += 1;
            continue;
        }
        let (Some(lot), Some(depart)) = (buffer.cached_batch(index), buffer.batch_start(index))
        else {
            // Résident à l'instant du test, absent à celui de la lecture : le
            // tampon a pu déborder entre les deux. Compté comme sauté plutôt
            // qu'ignoré, sinon le total mentirait sur ce qui a été lu.
            sautes += 1;
            continue;
        };
        for ligne in 0..lot.num_rows() {
            if trouvees.len() >= MATCH_LIMIT {
                plafonne = true;
                break;
            }
            let correspond = (0..lot.num_columns()).any(|colonne| {
                match format_cell(&lot, ligne, colonne, &entier) {
                    CellValue::Text(texte) => texte.to_lowercase().contains(&aiguille),
                    CellValue::Truncated { text, .. } => text.to_lowercase().contains(&aiguille),
                    // `Null` et `Unrenderable` ne correspondent à rien : voir
                    // le `///`. Une variante inconnue non plus — inventer une
                    // correspondance serait pire que d'en manquer une.
                    _ => false,
                }
            });
            if correspond {
                trouvees.push(depart + ligne);
            }
        }
    }
    FindOutcome {
        rows: trouvees,
        skipped_batches: sautes,
        capped: plafonne,
    }
}

#[cfg(test)]
mod tests;
