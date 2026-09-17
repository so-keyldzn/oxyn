//! La taille d'un lot Arrow — en lignes **et** en octets.
//!
//! Un `batch_size` compté seulement en nombre de lignes marche sur les tables de
//! démonstration et déclenche l'OOM sur les vraies : mille lignes portant chacune
//! un BLOB d'un mégaoctet font un gigaoctet
//! ([`DRIVER-CONTRACT` §3](../../../docs/DRIVER-CONTRACT.md)). Les deux bornes
//! sont donc portées ensemble, et la première atteinte ferme le lot.

use std::fmt;

/// Bornes d'un lot Arrow produit par [`SqliteCursor`](crate::SqliteCursor).
///
/// Les deux bornes sont des **plafonds** : un lot est fermé dès que l'une est
/// atteinte, et un lot peut être plus petit si la source s'épuise avant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchLimits {
    max_rows: usize,
    max_bytes: usize,
}

impl BatchLimits {
    /// Lignes par lot, par défaut.
    ///
    /// 8 192 est un compromis usuel : assez grand pour que le coût par lot
    /// (allocation des tampons Arrow, aller-retour de canal) soit négligeable,
    /// assez petit pour que le premier lot parte vite.
    pub const DEFAULT_MAX_ROWS: usize = 8_192;

    /// Octets de données par lot, par défaut.
    ///
    /// C'est **cette** borne qui protège la mémoire : elle est comptée sur la
    /// taille des valeurs lues, pas sur le nombre de lignes.
    pub const DEFAULT_MAX_BYTES: usize = 8 * 1024 * 1024;

    /// Les bornes par défaut.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_rows: Self::DEFAULT_MAX_ROWS,
            max_bytes: Self::DEFAULT_MAX_BYTES,
        }
    }

    /// Remplace le plafond de lignes.
    ///
    /// La valeur est ramenée à 1 au minimum : un lot de zéro ligne ferait
    /// tourner la boucle de lecture sans jamais avancer.
    #[must_use]
    pub const fn with_max_rows(mut self, rows: usize) -> Self {
        self.max_rows = if rows == 0 { 1 } else { rows };
        self
    }

    /// Remplace le plafond d'octets.
    ///
    /// Ramené à 1 au minimum, pour la même raison que
    /// [`with_max_rows`](Self::with_max_rows).
    #[must_use]
    pub const fn with_max_bytes(mut self, bytes: usize) -> Self {
        self.max_bytes = if bytes == 0 { 1 } else { bytes };
        self
    }

    /// Plafond de lignes.
    #[must_use]
    pub const fn max_rows(&self) -> usize {
        self.max_rows
    }

    /// Plafond d'octets.
    #[must_use]
    pub const fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    /// Le lot en cours doit-il être fermé ?
    ///
    /// Appelé **après** avoir ajouté une ligne : un lot contient donc toujours
    /// au moins une ligne, même si elle dépasse à elle seule le plafond
    /// d'octets. Refuser une ligne trop grosse reviendrait à ne jamais rendre
    /// la valeur que l'utilisateur veut voir.
    #[must_use]
    pub const fn reached(&self, rows: usize, bytes: usize) -> bool {
        rows >= self.max_rows || bytes >= self.max_bytes
    }
}

impl Default for BatchLimits {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for BatchLimits {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} rows / {} bytes", self.max_rows, self.max_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_defauts_bornent_les_deux_dimensions() {
        let limites = BatchLimits::default();
        assert_eq!(limites.max_rows(), 8_192);
        assert!(limites.max_bytes() > 0);
    }

    #[test]
    fn un_lot_se_ferme_sur_la_premiere_borne_atteinte() {
        let limites = BatchLimits::new().with_max_rows(10).with_max_bytes(100);
        assert!(!limites.reached(9, 99));
        assert!(limites.reached(10, 0), "la borne de lignes suffit");
        assert!(
            limites.reached(1, 100),
            "une seule ligne d'un mégaoctet ferme le lot"
        );
    }

    #[test]
    fn une_borne_nulle_est_ramenee_a_une_unite() {
        // Un lot de zéro ligne ferait tourner la boucle de lecture sans avancer.
        let limites = BatchLimits::new().with_max_rows(0).with_max_bytes(0);
        assert_eq!(limites.max_rows(), 1);
        assert_eq!(limites.max_bytes(), 1);
        assert!(limites.reached(1, 0));
    }
}
