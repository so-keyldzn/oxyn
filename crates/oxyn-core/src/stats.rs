//! Volumétrie et temps d'une exécution.
//!
//! Ce type vit dans `oxyn-core` parce que `oxyn-data` le remplit au fil des lots
//! et que `oxyn-driver` le renseigne côté serveur : le mettre dans l'une des
//! deux ferait dépendre l'autre d'elle.
//!
//! [`ExecStats::bytes`] compte les octets **de données**, pas les lignes. La
//! distinction n'est pas académique : mille lignes portant chacune un BLOB d'un
//! mégaoctet font un gigaoctet, et c'est cette mesure — pas le compte de lignes
//! — qui doit borner la taille d'un lot.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Ce qu'a coûté une exécution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ExecStats {
    /// Lignes produites, ou affectées pour une écriture.
    pub rows: u64,
    /// Octets de données traversés.
    pub bytes: u64,
    /// Temps mesuré **par le serveur**, quand il le rend. Distinct de
    /// [`total_time`](Self::total_time) : leur écart est le coût du réseau et du
    /// décodage, c'est-à-dire ce sur quoi Oxyn peut agir.
    pub server_time: Option<Duration>,
    /// Temps total, du départ de la requête à la fin du flux.
    pub total_time: Duration,
    /// Nombre de lots produits.
    pub batches: u64,
    /// Le résultat a-t-il été tronqué par
    /// [`ExecLimits`](crate::query::ExecLimits) ?
    ///
    /// Doit remonter jusqu'à l'écran : un résultat tronqué qui a l'air complet
    /// conduit à des conclusions fausses sur des données réelles.
    pub truncated: bool,
}

impl ExecStats {
    /// Enregistre un lot.
    pub fn record_batch(&mut self, rows: u64, bytes: u64) {
        self.rows = self.rows.saturating_add(rows);
        self.bytes = self.bytes.saturating_add(bytes);
        self.batches = self.batches.saturating_add(1);
    }

    /// Marque le résultat comme tronqué.
    pub fn mark_truncated(&mut self) {
        self.truncated = true;
    }

    /// Temps passé hors du serveur : réseau, décodage, conversion.
    ///
    /// `None` si le serveur n'a pas rendu son propre temps. Une soustraction
    /// qui passerait en négatif — horloges différentes — rend `None` plutôt
    /// qu'une valeur absurde.
    #[must_use]
    pub fn client_time(&self) -> Option<Duration> {
        self.server_time
            .and_then(|serveur| self.total_time.checked_sub(serveur))
    }

    /// Aucune ligne produite.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.rows == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_lots_s_accumulent() {
        let mut stats = ExecStats::default();
        assert!(stats.is_empty());

        stats.record_batch(1_000, 4_096);
        stats.record_batch(500, 2_048);

        assert_eq!(stats.rows, 1_500);
        assert_eq!(stats.bytes, 6_144);
        assert_eq!(stats.batches, 2);
        assert!(!stats.is_empty());
        assert!(!stats.truncated);
    }

    #[test]
    fn l_accumulation_ne_deborde_pas() {
        // Un compteur qui déborde vaut mieux qu'une panique dans le chemin de
        // décodage : l'entrée vient du serveur.
        let mut stats = ExecStats {
            rows: u64::MAX,
            bytes: u64::MAX,
            ..ExecStats::default()
        };
        stats.record_batch(10, 10);
        assert_eq!(stats.rows, u64::MAX);
        assert_eq!(stats.bytes, u64::MAX);
    }

    #[test]
    fn le_temps_client_est_l_ecart_avec_le_serveur() {
        let stats = ExecStats {
            server_time: Some(Duration::from_millis(30)),
            total_time: Duration::from_millis(200),
            ..ExecStats::default()
        };
        assert_eq!(stats.client_time(), Some(Duration::from_millis(170)));
    }

    #[test]
    fn un_temps_serveur_incoherent_ne_produit_pas_de_valeur_absurde() {
        let stats = ExecStats {
            server_time: Some(Duration::from_secs(10)),
            total_time: Duration::from_millis(5),
            ..ExecStats::default()
        };
        assert_eq!(stats.client_time(), None);

        let sans_serveur = ExecStats {
            total_time: Duration::from_millis(5),
            ..ExecStats::default()
        };
        assert_eq!(sans_serveur.client_time(), None);
    }

    #[test]
    fn la_troncature_se_declare() {
        let mut stats = ExecStats::default();
        stats.record_batch(10_000, 1);
        stats.mark_truncated();
        assert!(stats.truncated, "un résultat tronqué doit se savoir");
    }
}
