//! Décodage d'un flux `text/event-stream`.
//!
//! Les trois familles de fournisseurs diffusent en SSE ; seul le contenu des
//! trames diffère. Ce module ne connaît donc **aucun** fournisseur : il rend des
//! trames, et c'est l'appelant qui les interprète.
//!
//! # Ce que le décodage doit tenir
//!
//! Un flux HTTP arrive en morceaux qui n'ont aucun rapport avec les lignes :
//! une trame peut être coupée en son milieu, un caractère multi-octet aussi.
//! Le décodeur accumule donc jusqu'à la fin de ligne avant d'interpréter quoi
//! que ce soit — c'est ce qui évite un `from_utf8_lossy` qui remplacerait un `é`
//! coupé en deux par un caractère de remplacement.
//!
//! # La borne
//!
//! Le tampon est **borné**. Un serveur défaillant, ou hostile, qui émettrait des
//! octets sans jamais envoyer de fin de ligne ferait sinon gonfler la mémoire
//! sans limite. Dépasser la borne est une erreur de décodage, pas une panique.

use bytes::BytesMut;

/// Borne du tampon d'accumulation, en octets.
///
/// Une trame de flux de complétion pèse quelques centaines d'octets ; huit
/// mébioctets laissent une marge considérable tout en gardant la mémoire
/// bornée.
pub(crate) const DEFAULT_BUFFER_LIMIT: usize = 8 * 1024 * 1024;

/// Une trame SSE complète.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SseFrame {
    /// Contenu du champ `event`, quand le serveur en envoie un. Les protocoles
    /// compatibles OpenAI n'en envoient pas ; Anthropic si.
    pub(crate) event: Option<String>,
    /// Champs `data` concaténés, séparés par des sauts de ligne, comme le veut
    /// la spécification.
    pub(crate) data: String,
}

/// Le tampon a dépassé sa borne sans qu'une trame se termine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("trame SSE de plus de {limit} octets sans fin de ligne")]
pub(crate) struct SseOverflow {
    /// Borne dépassée, en octets.
    pub(crate) limit: usize,
}

/// Accumulateur de flux SSE.
///
/// Usage : [`push`](Self::push) à chaque morceau reçu, puis
/// [`next_frame`](Self::next_frame) en boucle jusqu'à `None`. À la fermeture du
/// flux, [`finish`](Self::finish) puis une dernière boucle : certains serveurs
/// ferment sans envoyer la ligne vide finale.
#[derive(Debug)]
pub(crate) struct SseDecoder {
    buffer: BytesMut,
    event: Option<String>,
    data: String,
    has_data: bool,
    limit: usize,
}

impl SseDecoder {
    /// Décodeur avec la borne par défaut.
    pub(crate) fn new() -> Self {
        Self::with_limit(DEFAULT_BUFFER_LIMIT)
    }

    /// Décodeur avec une borne choisie. Réservé aux tests.
    pub(crate) fn with_limit(limit: usize) -> Self {
        Self {
            buffer: BytesMut::new(),
            event: None,
            data: String::new(),
            has_data: false,
            limit,
        }
    }

    /// Ajoute un morceau reçu du réseau.
    ///
    /// # Erreurs
    /// [`SseOverflow`] si l'accumulation — tampon de lignes incomplètes plus
    /// données déjà rassemblées — dépasse la borne.
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Result<(), SseOverflow> {
        self.buffer.extend_from_slice(chunk);
        let accumule = self.buffer.len().saturating_add(self.data.len());
        if accumule > self.limit {
            return Err(SseOverflow { limit: self.limit });
        }
        Ok(())
    }

    /// Signale la fermeture du flux.
    ///
    /// Injecte la fin de trame que le serveur n'a peut-être pas envoyée, pour
    /// qu'un dernier `next_frame` rende ce qui était en cours. Ne passe pas par
    /// la vérification de borne : deux octets ne mettent rien en danger.
    pub(crate) fn finish(&mut self) {
        self.buffer.extend_from_slice(b"\n\n");
    }

    /// Extrait une ligne complète, sans son `\n` ni un éventuel `\r`.
    fn take_line(&mut self) -> Option<Vec<u8>> {
        let position = self.buffer.iter().position(|octet| *octet == b'\n')?;
        let mut ligne = self.buffer.split_to(position + 1);
        // Retire le `\n` terminal.
        let _ = ligne.split_off(position);
        let mut ligne = ligne.to_vec();
        if ligne.last() == Some(&b'\r') {
            ligne.pop();
        }
        Some(ligne)
    }

    /// Rend la trame en cours et réinitialise l'accumulation.
    fn take_frame(&mut self) -> SseFrame {
        self.has_data = false;
        SseFrame {
            event: self.event.take(),
            data: std::mem::take(&mut self.data),
        }
    }

    /// Rend la prochaine trame complète, ou `None` s'il faut plus d'octets.
    pub(crate) fn next_frame(&mut self) -> Option<SseFrame> {
        while let Some(ligne) = self.take_line() {
            // Ligne vide : fin de trame. Une trame sans champ `data` est un
            // battement de cœur, pas un événement.
            if ligne.is_empty() {
                if self.has_data || self.event.is_some() {
                    return Some(self.take_frame());
                }
                continue;
            }
            // Ligne commençant par `:` : commentaire, souvent un maintien de
            // connexion.
            if ligne.first() == Some(&b':') {
                continue;
            }

            // À ce stade la ligne est complète : l'UTF-8 ne peut plus être coupé
            // par une frontière de morceau, et `from_utf8_lossy` ne remplace que
            // ce qui est réellement invalide.
            let texte = String::from_utf8_lossy(&ligne);
            let texte: &str = &texte;
            let (champ, valeur) = match texte.find(':') {
                Some(i) => {
                    let (champ, reste) = texte.split_at(i);
                    // Un unique espace après le deux-points appartient à la
                    // syntaxe, pas à la valeur ; les suivants en font partie.
                    let brut = reste.get(1..).unwrap_or_default();
                    (champ, brut.strip_prefix(' ').unwrap_or(brut))
                }
                None => (texte, ""),
            };

            match champ {
                "data" => {
                    if self.has_data {
                        self.data.push('\n');
                    }
                    self.data.push_str(valeur);
                    self.has_data = true;
                }
                "event" => self.event = Some(valeur.to_owned()),
                // `id` et `retry` ne servent qu'à la reprise de connexion, que
                // cette crate ne fait pas : une génération ne se reprend pas au
                // milieu, elle se relance.
                _ => {}
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pousse un morceau et rassemble toutes les trames disponibles.
    fn trames(decodeur: &mut SseDecoder, morceau: &[u8]) -> Vec<SseFrame> {
        decodeur.push(morceau).expect("pas de dépassement attendu");
        let mut sorties = Vec::new();
        while let Some(trame) = decodeur.next_frame() {
            sorties.push(trame);
        }
        sorties
    }

    #[test]
    fn une_trame_simple_se_lit() {
        let mut d = SseDecoder::new();
        let sorties = trames(&mut d, b"data: bonjour\n\n");
        assert_eq!(sorties.len(), 1);
        assert_eq!(sorties[0].data, "bonjour");
        assert_eq!(sorties[0].event, None);
    }

    #[test]
    fn une_trame_coupee_par_le_reseau_se_recolle() {
        // Le cas nominal : les morceaux HTTP n'ont aucun rapport avec les lignes.
        let mut d = SseDecoder::new();
        assert!(trames(&mut d, b"data: {\"cho").is_empty());
        assert!(trames(&mut d, b"ices\":[]}").is_empty());
        let sorties = trames(&mut d, b"\n\n");
        assert_eq!(sorties.len(), 1);
        assert_eq!(sorties[0].data, r#"{"choices":[]}"#);
    }

    #[test]
    fn un_caractere_multioctet_coupe_ne_se_corrompt_pas() {
        // « é » est 0xC3 0xA9. Coupé entre les deux, un décodage immédiat
        // produirait un caractère de remplacement.
        let mut d = SseDecoder::new();
        assert!(trames(&mut d, b"data: caf\xc3").is_empty());
        let sorties = trames(&mut d, b"\xa9\n\n");
        assert_eq!(sorties.len(), 1);
        assert_eq!(sorties[0].data, "café");
    }

    #[test]
    fn les_fins_de_ligne_windows_sont_acceptees() {
        let mut d = SseDecoder::new();
        let sorties = trames(&mut d, b"data: a\r\n\r\n");
        assert_eq!(sorties.len(), 1);
        assert_eq!(sorties[0].data, "a");
    }

    #[test]
    fn les_commentaires_de_maintien_sont_ignores() {
        let mut d = SseDecoder::new();
        let sorties = trames(&mut d, b": ping\n\ndata: utile\n\n");
        assert_eq!(sorties.len(), 1, "{sorties:?}");
        assert_eq!(sorties[0].data, "utile");
    }

    #[test]
    fn plusieurs_champs_data_se_concatenent_avec_un_saut_de_ligne() {
        let mut d = SseDecoder::new();
        let sorties = trames(&mut d, b"data: une\ndata: deux\n\n");
        assert_eq!(sorties.len(), 1);
        assert_eq!(sorties[0].data, "une\ndeux");
    }

    #[test]
    fn le_champ_event_est_conserve() {
        // Anthropic nomme ses trames ; le décodage doit le rendre.
        let mut d = SseDecoder::new();
        let sorties = trames(&mut d, b"event: content_block_delta\ndata: {}\n\n");
        assert_eq!(sorties.len(), 1);
        assert_eq!(sorties[0].event.as_deref(), Some("content_block_delta"));
        assert_eq!(sorties[0].data, "{}");
    }

    #[test]
    fn la_sentinelle_de_fin_est_une_donnee_comme_une_autre() {
        // C'est à l'appelant de reconnaître `[DONE]`, pas au décodeur SSE.
        let mut d = SseDecoder::new();
        let sorties = trames(&mut d, b"data: [DONE]\n\n");
        assert_eq!(sorties[0].data, "[DONE]");
    }

    #[test]
    fn plusieurs_trames_dans_un_seul_morceau() {
        let mut d = SseDecoder::new();
        let sorties = trames(&mut d, b"data: a\n\ndata: b\n\ndata: c\n\n");
        let contenus: Vec<&str> = sorties.iter().map(|t| t.data.as_str()).collect();
        assert_eq!(contenus, ["a", "b", "c"]);
    }

    #[test]
    fn un_champ_sans_espace_apres_les_deux_points_se_lit_aussi() {
        let mut d = SseDecoder::new();
        let sorties = trames(&mut d, b"data:{\"a\":1}\n\n");
        assert_eq!(sorties[0].data, r#"{"a":1}"#);
    }

    #[test]
    fn un_seul_espace_est_retire_les_suivants_non() {
        let mut d = SseDecoder::new();
        let sorties = trames(&mut d, b"data:   trois espaces\n\n");
        assert_eq!(sorties[0].data, "  trois espaces");
    }

    #[test]
    fn une_fermeture_sans_ligne_vide_finale_rend_la_derniere_trame() {
        // Plusieurs serveurs locaux ferment ainsi.
        let mut d = SseDecoder::new();
        assert!(trames(&mut d, b"data: dernier\n").is_empty());
        d.finish();
        let trame = d.next_frame().expect("la trame en cours doit être rendue");
        assert_eq!(trame.data, "dernier");
        assert!(d.next_frame().is_none());
    }

    #[test]
    fn une_fermeture_sur_un_flux_propre_ne_fabrique_pas_de_trame() {
        let mut d = SseDecoder::new();
        let _ = trames(&mut d, b"data: a\n\n");
        d.finish();
        assert_eq!(d.next_frame(), None);
    }

    #[test]
    fn un_flux_sans_fin_de_ligne_est_borne() {
        // Sans borne, un serveur défaillant fait gonfler la mémoire sans limite.
        let mut d = SseDecoder::with_limit(64);
        let erreur = d.push(&vec![b'x'; 128]).expect_err("dépassement attendu");
        assert_eq!(erreur.limit, 64);
    }

    #[test]
    fn la_borne_compte_aussi_les_donnees_deja_rassemblees() {
        let mut d = SseDecoder::with_limit(64);
        d.push(b"data: ").expect("sous la borne");
        d.push(&[b'y'; 40]).expect("sous la borne");
        d.push(b"\n").expect("sous la borne");
        // Les 40 octets sont passés du tampon vers `data`.
        assert!(d.next_frame().is_none());
        let erreur = d.push(&[b'z'; 40]).expect_err("dépassement attendu");
        assert_eq!(erreur.limit, 64);
    }
}
