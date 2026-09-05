//! Le décodage exact d'un `NUMERIC` PostgreSQL.
//!
//! `sqlx` 0.9 ne sait décoder `NUMERIC` qu'à travers `bigdecimal` ou
//! `rust_decimal`, dont aucun n'est au contrat de dépendances de cette crate.
//! Plutôt que d'ajouter une dépendance pour reconstruire une chaîne, ce module
//! lit le format binaire du serveur — cinquante lignes, aucune perte.
//!
//! **La conversion en `f64` n'existe pas ici, et c'est le sujet du module.** Un
//! `NUMERIC(38, 10)` ne tient dans aucun flottant sur 64 bits : convertir change
//! des montants d'une fraction de centime, ce que personne ne voit avant un
//! rapprochement comptable.
//!
//! # Le format
//!
//! Tel qu'il est écrit par `numeric_send` :
//!
//! | Champ | Taille | Sens |
//! |---|---|---|
//! | `ndigits` | `i16` | nombre de groupes base 10 000 |
//! | `weight` | `i16` | rang du premier groupe, en puissances de 10 000 |
//! | `sign` | `u16` | `0x0000` positif, `0x4000` négatif, `0xC000` NaN, `0xD000` +∞, `0xF000` −∞ |
//! | `dscale` | `u16` | chiffres à afficher après la virgule |
//! | `digits` | `i16` × `ndigits` | groupes, chacun dans `0..=9999` |

use std::fmt::Write as _;

/// Au-delà, la valeur ne vient pas d'un `NUMERIC` : PostgreSQL en borne la
/// précision à 1 000 chiffres décimaux, soit 250 groupes.
///
/// La borne n'est pas de l'optimisation : `weight` est un `i16`, et une valeur
/// hostile de 32 767 ferait allouer 130 Ko de zéros par cellule.
const MAX_GROUPES: usize = 4_096;

/// Borne de l'échelle d'affichage. PostgreSQL la limite à 16 383.
const MAX_DSCALE: usize = 16_384;

/// Groupes de quatre chiffres décimaux.
const BASE: i16 = 10_000;

/// Décode un `NUMERIC` au format binaire vers sa représentation décimale exacte.
///
/// Rend `None` — jamais une panique, jamais une valeur approchée — si les octets
/// ne forment pas un `NUMERIC` lisible. L'appelant en fait alors un repli
/// opaque : les octets viennent du réseau et ne sont pas dignes de confiance
/// ([I-09](../../../CLAUDE.md#i-09)).
///
/// Les valeurs spéciales sont rendues comme PostgreSQL les écrit : `NaN`,
/// `Infinity`, `-Infinity`.
#[must_use]
pub(crate) fn render_binary(bytes: &[u8]) -> Option<String> {
    let mut lecteur = Lecteur::new(bytes);
    let ndigits = lecteur.i16()?;
    let weight = lecteur.i16()?;
    let sign = lecteur.u16()?;
    let dscale = lecteur.u16()?;

    match sign {
        0xC000 => return Some("NaN".to_owned()),
        0xD000 => return Some("Infinity".to_owned()),
        0xF000 => return Some("-Infinity".to_owned()),
        0x0000 | 0x4000 => {}
        _ => return None,
    }

    let compte = usize::try_from(ndigits).ok()?;
    if compte > MAX_GROUPES {
        return None;
    }
    let dscale = usize::from(dscale);
    if dscale > MAX_DSCALE {
        return None;
    }

    let mut groupes = Vec::with_capacity(compte);
    for _ in 0..compte {
        let groupe = lecteur.i16()?;
        if !(0..BASE).contains(&groupe) {
            return None;
        }
        groupes.push(groupe);
    }

    let poids = i32::from(weight);
    if poids > i32::try_from(MAX_GROUPES).ok()? {
        return None;
    }

    let mut sortie = String::with_capacity(dscale + 8);
    if sign == 0x4000 {
        sortie.push('-');
    }

    // Partie entière : le premier groupe s'écrit sans remplissage, les suivants
    // sur quatre chiffres — sinon 1 puis 0002 se liraient 12.
    if poids < 0 {
        sortie.push('0');
    } else {
        for rang in 0..=poids {
            let groupe = groupe_at(&groupes, rang);
            if rang == 0 {
                let _ = write!(sortie, "{groupe}");
            } else {
                let _ = write!(sortie, "{groupe:04}");
            }
        }
    }

    if dscale > 0 {
        let mut fraction = String::with_capacity(dscale + 4);
        let mut rang = poids.checked_add(1)?;
        while fraction.len() < dscale {
            let _ = write!(fraction, "{:04}", groupe_at(&groupes, rang));
            rang = rang.checked_add(1)?;
        }
        fraction.truncate(dscale);
        sortie.push('.');
        sortie.push_str(&fraction);
    }

    Some(sortie)
}

/// Le groupe de rang `rang`, ou zéro : les groupes absents avant et après ceux
/// que le serveur envoie valent zéro, c'est ce qui permet de n'en transmettre
/// que les significatifs.
fn groupe_at(groupes: &[i16], rang: i32) -> i16 {
    if rang < 0 {
        return 0;
    }
    usize::try_from(rang)
        .ok()
        .and_then(|index| groupes.get(index).copied())
        .unwrap_or(0)
}

/// Lecteur gros-boutiste sur une tranche, sans indexation par plage.
///
/// Écrit à la main pour une raison précise : `bytes::Buf::get_i16` **panique**
/// sur un tampon trop court, et ce tampon vient du réseau
/// ([I-09](../../../CLAUDE.md#i-09)).
pub(crate) struct Lecteur<'a> {
    reste: &'a [u8],
}

impl<'a> Lecteur<'a> {
    /// Un lecteur positionné au début de la tranche.
    pub(crate) const fn new(bytes: &'a [u8]) -> Self {
        Self { reste: bytes }
    }

    /// Les octets non encore lus.
    pub(crate) const fn reste(&self) -> &'a [u8] {
        self.reste
    }

    /// La tranche est-elle épuisée ?
    pub(crate) const fn est_vide(&self) -> bool {
        self.reste.is_empty()
    }

    /// `n` octets, ou `None` s'il n'y en a pas assez.
    pub(crate) fn prendre(&mut self, n: usize) -> Option<&'a [u8]> {
        let (debut, suite) = self.reste.split_at_checked(n)?;
        self.reste = suite;
        Some(debut)
    }

    /// Un entier signé sur 16 bits, gros-boutiste.
    pub(crate) fn i16(&mut self) -> Option<i16> {
        let octets: [u8; 2] = self.prendre(2)?.try_into().ok()?;
        Some(i16::from_be_bytes(octets))
    }

    /// Un entier non signé sur 16 bits, gros-boutiste.
    pub(crate) fn u16(&mut self) -> Option<u16> {
        let octets: [u8; 2] = self.prendre(2)?.try_into().ok()?;
        Some(u16::from_be_bytes(octets))
    }

    /// Un entier signé sur 32 bits, gros-boutiste.
    pub(crate) fn i32(&mut self) -> Option<i32> {
        let octets: [u8; 4] = self.prendre(4)?.try_into().ok()?;
        Some(i32::from_be_bytes(octets))
    }

    /// Un entier non signé sur 32 bits, gros-boutiste.
    pub(crate) fn u32(&mut self) -> Option<u32> {
        let octets: [u8; 4] = self.prendre(4)?.try_into().ok()?;
        Some(u32::from_be_bytes(octets))
    }

    /// Un entier signé sur 64 bits, gros-boutiste.
    pub(crate) fn i64(&mut self) -> Option<i64> {
        let octets: [u8; 8] = self.prendre(8)?.try_into().ok()?;
        Some(i64::from_be_bytes(octets))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Construit l'encodage binaire d'un `NUMERIC`, comme le ferait le serveur.
    fn encoder(weight: i16, sign: u16, dscale: u16, groupes: &[i16]) -> Vec<u8> {
        let mut octets = Vec::new();
        let ndigits = i16::try_from(groupes.len()).expect("les cas de test sont courts");
        octets.extend_from_slice(&ndigits.to_be_bytes());
        octets.extend_from_slice(&weight.to_be_bytes());
        octets.extend_from_slice(&sign.to_be_bytes());
        octets.extend_from_slice(&dscale.to_be_bytes());
        for groupe in groupes {
            octets.extend_from_slice(&groupe.to_be_bytes());
        }
        octets
    }

    #[test]
    fn un_entier_simple_se_relit() {
        // 1234
        let octets = encoder(0, 0x0000, 0, &[1234]);
        assert_eq!(render_binary(&octets).as_deref(), Some("1234"));
    }

    #[test]
    fn le_premier_groupe_ne_se_remplit_pas_et_les_suivants_si() {
        // 1 0002 = 10002, pas 12.
        let octets = encoder(1, 0x0000, 0, &[1, 2]);
        assert_eq!(render_binary(&octets).as_deref(), Some("10002"));
    }

    #[test]
    fn un_montant_a_deux_decimales_reste_exact() {
        // 1234.56 : weight 0, groupes [1234, 5600], dscale 2.
        let octets = encoder(0, 0x0000, 2, &[1234, 5600]);
        assert_eq!(render_binary(&octets).as_deref(), Some("1234.56"));
    }

    #[test]
    fn le_signe_negatif_se_reporte() {
        let octets = encoder(0, 0x4000, 2, &[1234, 5600]);
        assert_eq!(render_binary(&octets).as_deref(), Some("-1234.56"));
    }

    #[test]
    fn une_valeur_purement_fractionnaire_recoit_son_zero_de_tete() {
        // 0.1234 : weight -1.
        let octets = encoder(-1, 0x0000, 4, &[1234]);
        assert_eq!(render_binary(&octets).as_deref(), Some("0.1234"));
    }

    #[test]
    fn les_groupes_absents_avant_les_significatifs_valent_zero() {
        // 0.00001234 : weight -2, un seul groupe transmis.
        let octets = encoder(-2, 0x0000, 8, &[1234]);
        assert_eq!(render_binary(&octets).as_deref(), Some("0.00001234"));
    }

    #[test]
    fn les_decimales_manquantes_sont_completees_par_des_zeros() {
        // 12.5 déclaré avec quatre décimales.
        let octets = encoder(0, 0x0000, 4, &[12, 5000]);
        assert_eq!(render_binary(&octets).as_deref(), Some("12.5000"));
    }

    #[test]
    fn zero_se_rend_zero() {
        let octets = encoder(0, 0x0000, 0, &[]);
        assert_eq!(render_binary(&octets).as_deref(), Some("0"));
    }

    #[test]
    fn une_precision_qui_ne_tient_pas_dans_un_f64_reste_exacte() {
        // 12 345 678 901 234 567 890,12345678 : 26 chiffres significatifs, soit
        // bien au-delà des 15 à 17 d'un f64. C'est le cas qui motive ce module.
        let octets = encoder(4, 0x0000, 8, &[1234, 5678, 9012, 3456, 7890, 1234, 5678]);
        assert_eq!(
            render_binary(&octets).as_deref(),
            Some("12345678901234567890.12345678")
        );
    }

    #[test]
    fn les_valeurs_speciales_se_nomment() {
        assert_eq!(
            render_binary(&encoder(0, 0xC000, 0, &[])).as_deref(),
            Some("NaN")
        );
        assert_eq!(
            render_binary(&encoder(0, 0xD000, 0, &[])).as_deref(),
            Some("Infinity")
        );
        assert_eq!(
            render_binary(&encoder(0, 0xF000, 0, &[])).as_deref(),
            Some("-Infinity")
        );
    }

    #[test]
    fn une_entree_tronquee_rend_none_au_lieu_de_paniquer() {
        // Les octets viennent du réseau : un serveur peut en envoyer moins que
        // son en-tête n'en annonce.
        assert_eq!(render_binary(&[]), None);
        assert_eq!(render_binary(&[0, 1, 0, 0]), None);
        let mut tronque = encoder(0, 0x0000, 0, &[1234]);
        tronque.pop();
        assert_eq!(render_binary(&tronque), None);
    }

    #[test]
    fn un_signe_inconnu_est_refuse() {
        assert_eq!(render_binary(&encoder(0, 0x1234, 0, &[1])), None);
    }

    #[test]
    fn un_groupe_hors_bornes_est_refuse() {
        // 10 000 n'est pas un groupe base 10 000 valide.
        assert_eq!(render_binary(&encoder(0, 0x0000, 0, &[10_000])), None);
        assert_eq!(render_binary(&encoder(0, 0x0000, 0, &[-1])), None);
    }

    #[test]
    fn un_poids_absurde_n_alloue_pas_130_ko() {
        assert_eq!(render_binary(&encoder(i16::MAX, 0x0000, 0, &[1])), None);
    }

    #[test]
    fn le_lecteur_ne_deborde_jamais() {
        let mut lecteur = Lecteur::new(&[0x00, 0x01, 0x02]);
        assert_eq!(lecteur.i16(), Some(1));
        assert_eq!(lecteur.i16(), None, "un octet restant ne fait pas un i16");
        assert_eq!(lecteur.reste(), &[0x02]);
        assert!(!lecteur.est_vide());
        assert_eq!(lecteur.prendre(1), Some(&[0x02][..]));
        assert!(lecteur.est_vide());
    }
}
