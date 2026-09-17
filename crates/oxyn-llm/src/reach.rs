//! Où part une requête : sur cette machine, ou dehors.
//!
//! [`AI-PROVIDERS`](../../../docs/AI-PROVIDERS.md) pose la règle et le piège :
//!
//! > Un point d'accès « compatible OpenAI » pointé sur `localhost` peut être un
//! > proxy qui réémet vers le nuage. Le classement local/distant se fait sur
//! > l'hôte réel **après résolution**, jamais sur la présence de `localhost`
//! > dans l'URL, et il se re-vérifie à chaque changement de configuration.
//!
//! D'où deux fonctions et non une : [`literal_reach`] tranche sans réseau les
//! cas décidables (une adresse IP littérale), [`resolve_reach`] fait la
//! résolution DNS pour les autres.
//!
//! # Ce que `Local` promet, et ce qu'il ne promet pas
//!
//! [`Reach::Local`] dit que **la connexion TCP se termine sur cette machine**.
//! Il ne dit pas que la donnée y reste : un proxy en écoute sur `127.0.0.1` peut
//! réémettre vers n'importe où, et aucune inspection du point d'accès ne le
//! détectera. Ce que le classement apporte, c'est l'élimination du cas
//! inverse — un point d'accès nommé `localhost.mon-nuage.example` qui n'a de
//! local que le nom.

use std::fmt;
use std::net::{IpAddr, ToSocketAddrs};

use reqwest::Url;

/// Classement d'un point d'accès.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reach {
    /// L'hôte est une adresse de bouclage : la connexion ne quitte pas la
    /// machine. Voir la réserve du module.
    Local,
    /// L'hôte est joignable ailleurs que sur la boucle locale. Les données
    /// **quittent la machine**.
    Remote,
    /// L'hôte n'a pas pu être résolu, ou l'URL n'a pas d'hôte du tout.
    ///
    /// À traiter comme [`Remote`](Self::Remote) partout où une décision doit
    /// être prise : dans le doute, on protège.
    Unresolved,
}

impl Reach {
    /// Les données quittent-elles la machine, en l'état de ce qu'on sait ?
    ///
    /// [`Unresolved`](Self::Unresolved) répond `true` : un point d'accès qu'on
    /// n'a pas su classer n'obtient pas le bénéfice du doute.
    #[must_use]
    pub const fn leaves_machine(&self) -> bool {
        !matches!(self, Self::Local)
    }

    /// Nom stable, pour l'affichage et l'audit.
    ///
    /// **En anglais**, comme tout ce qui traverse la frontière du code source
    /// (CLAUDE.md) : cette valeur est montrée telle quelle par l'écran de
    /// configuration des fournisseurs, et le reste de l'interface d'Oxyn est en
    /// anglais. Un libellé de domaine dans une autre langue que l'écran qui
    /// l'affiche oblige chaque appelant à le retraduire — donc à réinventer une
    /// correspondance par variante, qui divergera.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
            Self::Unresolved => "unresolved",
        }
    }
}

impl fmt::Display for Reach {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Extrait l'adresse IP littérale d'une URL, quand son hôte en est une.
///
/// `host_str` rend un IPv6 entre crochets (`[::1]`) : ils sont retirés avant
/// analyse.
fn literal_ip(url: &Url) -> Option<IpAddr> {
    let hote = url.host_str()?;
    let nu = hote
        .strip_prefix('[')
        .and_then(|reste| reste.strip_suffix(']'))
        .unwrap_or(hote);
    nu.parse::<IpAddr>().ok()
}

/// Classe un point d'accès **sans réseau**, quand c'est possible.
///
/// Rend `Some` uniquement pour une adresse IP littérale — le seul cas où la
/// question se tranche sans résolution. Un nom de domaine, `localhost` compris,
/// rend `None` : c'est exactement le piège que la règle vise.
#[must_use]
pub fn literal_reach(url: &Url) -> Option<Reach> {
    literal_ip(url).map(|ip| {
        if ip.is_loopback() {
            Reach::Local
        } else {
            Reach::Remote
        }
    })
}

/// Classe un point d'accès, en résolvant son nom si nécessaire.
///
/// Un hôte n'est [`Reach::Local`] que si **toutes** ses adresses résolues sont
/// de bouclage : un nom qui résout à la fois vers `127.0.0.1` et vers une
/// adresse publique est distant, parce que c'est la seconde qui sera peut-être
/// utilisée.
///
/// # Bloquant
///
/// La résolution DNS de la bibliothèque standard est **bloquante**. Cette
/// fonction ne doit jamais être appelée depuis le fil d'interface (I-05), ni
/// depuis une tâche asynchrone sans passer par un pool bloquant. Elle est faite
/// pour être appelée à l'inscription d'un fournisseur et à chaque changement de
/// sa configuration, pas à chaque requête.
#[must_use]
pub fn resolve_reach(url: &Url) -> Reach {
    if let Some(immediat) = literal_reach(url) {
        return immediat;
    }
    let Some(hote) = url.host_str() else {
        return Reach::Unresolved;
    };
    // Le port est indifférent à la résolution ; `0` évite d'imposer un défaut
    // arbitraire quand l'URL n'en porte pas et que le schéma n'en impose pas.
    let port = url.port_or_known_default().unwrap_or(0);
    let Ok(adresses) = (hote, port).to_socket_addrs() else {
        return Reach::Unresolved;
    };
    let mut vu = false;
    for adresse in adresses {
        vu = true;
        if !adresse.ip().is_loopback() {
            return Reach::Remote;
        }
    }
    if vu { Reach::Local } else { Reach::Unresolved }
}

/// Classe un point d'accès donné sous forme de chaîne.
///
/// C'est la porte d'entrée pour un appelant qui tient une
/// [`AiProviderConfig::base_url`](oxyn_core::AiProviderConfig) — une `String` —
/// et n'a aucune raison de dépendre du client HTTP pour analyser une URL.
///
/// Une URL **illisible rend [`Reach::Unresolved`]**, jamais une erreur : un
/// point d'accès qu'on ne sait pas classer compte comme distant partout où une
/// décision se prend ([`Reach::leaves_machine`] répond déjà `true` dessus).
/// Rendre un `Result` obligerait chaque appelant à choisir un défaut, et le
/// mauvais défaut — « local » — est silencieux.
///
/// # Bloquant
///
/// La résolution DNS de la bibliothèque standard est **bloquante**. Comme
/// [`resolve_reach`], cette fonction ne doit jamais être appelée depuis le fil
/// d'interface (I-05), ni depuis une tâche asynchrone sans passer par un pool
/// bloquant. Elle est faite pour être appelée à l'enregistrement d'un
/// fournisseur et à chaque ouverture de runtime, pas à chaque requête.
#[must_use]
pub fn endpoint_reach(base_url: &str) -> Reach {
    match Url::parse(base_url) {
        Ok(url) => resolve_reach(&url),
        Err(_) => Reach::Unresolved,
    }
}

/// Rend une URL montrable, débarrassée de ses identifiants.
///
/// Une URL peut porter un couple `utilisateur:motdepasse` dans sa partie
/// autorité. Le recopier dans un message d'erreur ou dans un `Debug` est une
/// fuite (I-03) — et c'est exactement ce que fait l'affichage naturel d'une
/// [`Url`].
#[must_use]
pub fn redacted(url: &Url) -> String {
    let mut propre = url.clone();
    // `set_username` et `set_password` échouent sur les URL sans autorité
    // (`data:`, `mailto:`) : il n'y a alors pas d'identifiant à retirer.
    let _ = propre.set_username("");
    let _ = propre.set_password(None);
    propre.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(brut: &str) -> Url {
        Url::parse(brut).expect("URL de test valide")
    }

    #[test]
    fn une_adresse_de_bouclage_litterale_se_tranche_sans_dns() {
        assert_eq!(
            literal_reach(&url("http://127.0.0.1:11434/v1")),
            Some(Reach::Local)
        );
        assert_eq!(
            literal_reach(&url("http://127.0.0.53:8080")),
            Some(Reach::Local)
        );
        assert_eq!(
            literal_reach(&url("http://[::1]:1234/v1")),
            Some(Reach::Local)
        );
    }

    #[test]
    fn une_adresse_publique_litterale_est_distante() {
        assert_eq!(
            literal_reach(&url("https://93.184.216.34/v1")),
            Some(Reach::Remote)
        );
        assert_eq!(
            literal_reach(&url("http://192.168.1.10:11434/v1")),
            Some(Reach::Remote),
            "le réseau local n'est pas la machine locale : les données sortent"
        );
    }

    #[test]
    fn un_nom_de_domaine_ne_se_tranche_pas_sur_sa_forme() {
        // Le piège d'AI-PROVIDERS, sous ses deux faces.
        assert_eq!(literal_reach(&url("http://localhost:11434/v1")), None);
        assert_eq!(
            literal_reach(&url("https://localhost.mon-nuage.example/v1")),
            None,
            "un nom qui contient `localhost` ne prouve rien"
        );
    }

    #[test]
    fn un_point_d_acces_non_resolu_ne_beneficie_pas_du_doute() {
        assert!(Reach::Unresolved.leaves_machine());
        assert!(Reach::Remote.leaves_machine());
        assert!(!Reach::Local.leaves_machine());
    }

    #[test]
    fn une_adresse_litterale_se_resout_sans_reseau() {
        // `resolve_reach` court-circuite : ce test ne dépend d'aucun résolveur.
        assert_eq!(
            resolve_reach(&url("http://127.0.0.1:11434/v1")),
            Reach::Local
        );
        assert_eq!(resolve_reach(&url("https://8.8.8.8/")), Reach::Remote);
    }

    #[test]
    fn une_chaine_illisible_ne_beneficie_pas_du_doute() {
        // Le mauvais défaut serait « local », et il serait silencieux.
        for brut in ["", "pas une url", "://", "mailto:quelquun@example.com"] {
            assert_eq!(endpoint_reach(brut), Reach::Unresolved, "{brut}");
            assert!(endpoint_reach(brut).leaves_machine(), "{brut}");
        }
    }

    #[test]
    fn une_chaine_litterale_se_classe_sans_reseau() {
        assert_eq!(endpoint_reach("http://127.0.0.1:11434"), Reach::Local);
        assert_eq!(endpoint_reach("http://[::1]:1234/v1"), Reach::Local);
        assert_eq!(endpoint_reach("https://93.184.216.34/v1"), Reach::Remote);
    }

    #[test]
    fn les_identifiants_d_une_url_ne_s_affichent_pas() {
        let avec = url("https://alice:motdepasse@api.example.com/v1/");
        let rendu = redacted(&avec);
        assert!(!rendu.contains("motdepasse"), "{rendu}");
        assert!(!rendu.contains("alice"), "{rendu}");
        assert!(rendu.contains("api.example.com"), "{rendu}");
    }

    #[test]
    fn une_url_sans_identifiants_est_rendue_telle_quelle() {
        let simple = url("http://localhost:11434/v1/");
        assert_eq!(redacted(&simple), "http://localhost:11434/v1/");
    }
}
