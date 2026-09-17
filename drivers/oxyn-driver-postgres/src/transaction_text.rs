//! Reconnaître, sans l'analyser, un texte qui ouvre une transaction.
//!
//! # Pourquoi le driver regarde le texte ici
//!
//! Une session Oxyn s'appuie sur un bassin : chaque exécution emprunte une
//! connexion et la rend. Un `BEGIN` tapé dans une console laisse sa connexion
//! **dans une transaction** ; rendue au bassin, elle serait héritée par
//! l'emprunteur suivant — l'introspection, ou la requête suivante d'une autre
//! console, qui écrirait alors dans une transaction que personne ne validera.
//! La défaire par un `ROLLBACK` implicite annulerait en silence ce que
//! l'utilisateur voulait garder. Le driver ferme donc la connexion.
//!
//! Il faut pour cela savoir que l'exécution a ouvert une transaction, et
//! sqlx-postgres 0.9.0 ne l'expose pas : l'état porté par `ReadyForQuery` reste
//! `pub(crate)` (`PgConnection::in_transaction`). Le texte, lui, suffit :
//!
//! * le protocole étendu prépare **une** instruction par exécution, donc le
//!   premier mot du texte est celui de l'instruction ;
//! * en PostgreSQL, seules `BEGIN` et `START TRANSACTION` ouvrent un bloc de
//!   transaction au niveau d'une instruction. Un `BEGIN` dans le corps d'un `DO`
//!   ou d'une fonction est entre guillemets dollar, jamais en tête ; une
//!   procédure qui valide ne laisse pas de bloc ouvert après elle.
//!
//! Ce n'est **pas** un classifieur : `oxyn-query` décide si une instruction
//! écrit. Ce module ne répond qu'à une question, et se trompe du côté prudent —
//! un faux positif ferme une connexion de trop.

/// Le texte est-il une instruction de contrôle de transaction ?
///
/// `BEGIN`, `START TRANSACTION`, `COMMIT`, `END`, `ROLLBACK`, `ABORT`,
/// `SAVEPOINT`, `RELEASE`, `PREPARE TRANSACTION` — lus sur le premier mot, comme
/// [`opens_transaction`]. Une session sans
/// [`Capabilities::TRANSACTIONS`](oxyn_core::Capabilities::TRANSACTIONS) les
/// refuse **avant l'envoi** : sur un bassin, chacune partirait sur la connexion
/// d'emprunt du moment, et un `ROLLBACK` sans transaction « réussit » côté
/// serveur — un simple `WARNING` — pendant que l'écriture qu'il devait annuler
/// reste validée. Ne pas savoir faire est acceptable ; laisser croire ne l'est
/// pas ([DRIVER-CONTRACT §5](../../../docs/DRIVER-CONTRACT.md)).
///
/// `PREPARE nom AS …` n'est **pas** concerné : seul `PREPARE TRANSACTION` l'est.
#[must_use]
pub(crate) fn controls_transaction(text: &str) -> bool {
    let mut mots = Words::new(text);
    let Some(premier) = mots.next() else {
        return false;
    };
    let est = |attendu: &str| premier.eq_ignore_ascii_case(attendu);
    if [
        "begin",
        "commit",
        "end",
        "rollback",
        "abort",
        "savepoint",
        "release",
    ]
    .into_iter()
    .any(est)
    {
        return true;
    }
    if est("start") || est("prepare") {
        return mots
            .next()
            .is_some_and(|suivant| suivant.eq_ignore_ascii_case("transaction"));
    }
    false
}

/// Le texte ouvre-t-il un bloc de transaction ?
///
/// Saute les blancs et les commentaires de tête — `--` jusqu'à la fin de ligne,
/// `/* */` imbriqués comme le serveur les imbrique —, puis lit le premier mot.
/// Ne panique sur aucune entrée ([I-09](../../../CLAUDE.md#i-09)).
#[must_use]
pub(crate) fn opens_transaction(text: &str) -> bool {
    let mut mots = Words::new(text);
    match mots.next() {
        Some(mot) if mot.eq_ignore_ascii_case("begin") => true,
        Some(mot) if mot.eq_ignore_ascii_case("start") => mots
            .next()
            .is_some_and(|suivant| suivant.eq_ignore_ascii_case("transaction")),
        _ => false,
    }
}

/// Les mots de tête d'un texte SQL, commentaires et blancs sautés.
///
/// S'arrête au premier caractère qui ne commence ni un mot, ni un blanc, ni un
/// commentaire : au-delà, la structure ne sert plus à la question posée.
struct Words<'a> {
    rest: &'a str,
}

impl<'a> Words<'a> {
    const fn new(text: &'a str) -> Self {
        Self { rest: text }
    }

    /// Avance au-delà des blancs et des commentaires. Rend `false` si un
    /// commentaire n'est pas terminé.
    fn skip_trivia(&mut self) -> bool {
        loop {
            let trimmed = self
                .rest
                .trim_start_matches([' ', '\t', '\n', '\r', '\u{000B}', '\u{000C}']);
            if let Some(apres) = trimmed.strip_prefix("--") {
                self.rest = apres
                    .find(['\n', '\r'])
                    .map_or("", |fin| apres.get(fin..).unwrap_or(""));
            } else if trimmed.starts_with("/*") {
                match block_comment_end(trimmed) {
                    Some(apres) => self.rest = apres,
                    None => {
                        self.rest = "";
                        return false;
                    }
                }
            } else {
                self.rest = trimmed;
                return true;
            }
        }
    }
}

impl<'a> Iterator for Words<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        if !self.skip_trivia() {
            return None;
        }
        let mut caracteres = self.rest.char_indices();
        let (_, premier) = caracteres.next()?;
        if !(premier.is_alphabetic() || premier == '_') {
            return None;
        }
        let fin = caracteres
            .find(|(_, c)| !(c.is_alphanumeric() || *c == '_' || *c == '$'))
            .map_or(self.rest.len(), |(indice, _)| indice);
        let (mot, reste) = self.rest.split_at_checked(fin)?;
        self.rest = reste;
        Some(mot)
    }
}

/// Ce qui suit un commentaire `/* … */` en tête de `text`, imbrication comprise.
fn block_comment_end(text: &str) -> Option<&str> {
    let mut profondeur = 0_usize;
    let mut reste = text;
    loop {
        let ouverture = reste.find("/*");
        let fermeture = reste.find("*/");
        match (ouverture, fermeture) {
            (Some(o), Some(f)) if o < f => {
                profondeur = profondeur.saturating_add(1);
                reste = reste.get(o.saturating_add(2)..)?;
            }
            (Some(o), None) => {
                profondeur = profondeur.saturating_add(1);
                reste = reste.get(o.saturating_add(2)..)?;
            }
            (_, Some(f)) => {
                profondeur = profondeur.checked_sub(1)?;
                reste = reste.get(f.saturating_add(2)..)?;
                if profondeur == 0 {
                    return Some(reste);
                }
            }
            (None, None) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{controls_transaction, opens_transaction};

    #[test]
    fn le_controle_de_transaction_est_reconnu() {
        for texte in [
            "BEGIN",
            "start transaction",
            "COMMIT",
            "commit and chain",
            "END",
            "end transaction",
            "ROLLBACK",
            "rollback to savepoint s",
            "ABORT",
            "SAVEPOINT s",
            "RELEASE SAVEPOINT s",
            "release s",
            "PREPARE TRANSACTION 'x'",
            "COMMIT PREPARED 'x'",
            "ROLLBACK PREPARED 'x'",
            "/* annuler */ ROLLBACK",
            "-- valider\r\ncommit;",
        ] {
            assert!(controls_transaction(texte), "{texte:?}");
        }
    }

    #[test]
    fn les_faux_amis_du_controle_de_transaction_passent() {
        for texte in [
            "SELECT 'BEGIN'",
            "SELECT 'ROLLBACK'",
            "DO $$ BEGIN PERFORM 1; END $$",
            "PREPARE lecture AS SELECT 1",
            "START",
            "ENDING",
            "commit_log",
            "SELECT 1 -- COMMIT",
            "/* ROLLBACK */ SELECT 1",
            "\"rollback\"",
            "",
        ] {
            assert!(!controls_transaction(texte), "{texte:?}");
        }
    }

    #[test]
    fn les_ouvertures_de_transaction_sont_reconnues() {
        for texte in [
            "BEGIN",
            "begin;",
            "Begin Work",
            "BEGIN TRANSACTION ISOLATION LEVEL SERIALIZABLE",
            "START TRANSACTION",
            "start   transaction read write",
            "  \n\t BEGIN",
            "-- ouvrir\nBEGIN",
            "-- ouvrir\r\nSTART TRANSACTION",
            "/* commentaire */ begin",
            "/* a /* imbriqué */ toujours dedans */ BEGIN",
            "START /* entre */ TRANSACTION",
            "START\n-- ligne\nTRANSACTION",
        ] {
            assert!(opens_transaction(texte), "{texte:?}");
        }
    }

    #[test]
    fn le_reste_n_est_pas_une_ouverture() {
        for texte in [
            "",
            "   ",
            "SELECT 'BEGIN'",
            "SELECT 1 -- BEGIN",
            "COMMIT",
            "ROLLBACK",
            "END",
            "BEGINNING",
            "START",
            "START TRANSACTIONS",
            "DO $$ BEGIN PERFORM 1; END $$",
            "/* BEGIN */ SELECT 1",
            "-- BEGIN",
            "\"begin\"",
            "/* jamais fermé BEGIN",
            "/* a /* b */ BEGIN",
            "*/ BEGIN",
            "(BEGIN)",
            "déjà BEGIN",
        ] {
            assert!(!opens_transaction(texte), "{texte:?}");
        }
    }

    #[test]
    fn aucune_entree_ne_fait_paniquer() {
        // I-09 : le texte vient de l'utilisateur ou d'un agent. Les coupures en
        // plein caractère multi-octets et les marqueurs orphelins sont le
        // corpus minimal.
        for texte in [
            "/",
            "-",
            "*",
            "/*",
            "*/",
            "--",
            "é",
            "/*é*/é",
            "--é\né",
            "\u{0}",
            "𝔅EGIN",
            "/**/",
            "/*/",
            "/*/*/",
            "BEGIN\u{0}",
        ] {
            let _ = opens_transaction(texte);
        }
    }
}
