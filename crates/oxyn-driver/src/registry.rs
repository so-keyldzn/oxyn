//! Le registre des drivers disponibles.
//!
//! C'est le seul endroit où l'application apprend qu'un protocole existe.
//! `oxyn-desktop` y enregistre les drivers compilés dans le binaire ; la phase 4 y
//! ajoutera ceux qui viennent d'un plugin WASM
//! ([ADR-0005](../../../docs/adr/0005-wasm-plugins.md)) et ceux qui tournent en
//! sidecar ([ADR-0007](../../../docs/adr/0007-driver-sidecar.md)). Le registre
//! ne fait aucune différence entre les trois : il ne voit que des
//! `Arc<dyn Driver>`.
//!
//! # Pourquoi l'enregistrement refuse plutôt qu'il ne remplace
//!
//! Réenregistrer `postgres` **remplacerait** le driver PostgreSQL — silencieusement,
//! et pour toutes les connexions ouvertes ensuite. Le jour où un plugin peut
//! s'enregistrer, ce remplacement devient une prise de contrôle : le plugin
//! reçoit les identifiants de production que l'utilisateur croit donner au
//! driver d'origine. Le registre refuse donc un identifiant déjà pris, et il le
//! dit.

use std::fmt;
use std::sync::Arc;

use indexmap::IndexMap;
use oxyn_core::{DriverId, OxynError, Result};

use crate::metadata::DriverMetadata;
use crate::traits::Driver;

/// Les drivers connus de cette instance d'Oxyn.
///
/// Un [`IndexMap`] et non une `HashMap` : l'ordre d'enregistrement est
/// reproductible, ce qui rend les tests et les journaux lisibles.
/// [`sorted`](Self::sorted) donne l'ordre d'affichage, qui est un autre sujet.
#[derive(Default)]
pub struct DriverRegistry {
    drivers: IndexMap<DriverId, Arc<dyn Driver>>,
}

impl DriverRegistry {
    /// Un registre vide.
    ///
    /// C'est un état légitime et durable : sans driver, Oxyn affiche sa fenêtre
    /// et son workspace, il ne propose simplement aucune connexion.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Enregistre un driver.
    ///
    /// Trois vérifications, toutes destinées à faire échouer tôt ce qui
    /// échouerait tard :
    ///
    /// 1. les métadonnées sont cohérentes ([`DriverMetadata::check`]) ;
    /// 2. [`Driver::id`] est égal à [`DriverMetadata::id`] — sinon le driver
    ///    serait rangé sous une clé et se présenterait sous une autre, donc
    ///    introuvable ;
    /// 3. l'identifiant n'est pas déjà pris.
    ///
    /// # Erreurs
    /// [`OxynError::Config`] dans les trois cas, en nommant le driver fautif.
    pub fn register(&mut self, driver: Arc<dyn Driver>) -> Result<()> {
        let metadata = driver.metadata();
        metadata.check()?;

        let id = driver.id();
        if id != metadata.id {
            return Err(OxynError::Config(format!(
                "the driver declares itself `{id}` but its metadata says `{}`: \
                 it would be unreachable once registered",
                metadata.id
            )));
        }
        if self.drivers.contains_key(&id) {
            return Err(OxynError::Config(format!(
                "a driver `{id}` is already registered: replacing the original one \
                 would divert the connections that target it"
            )));
        }

        self.drivers.insert(id, driver);
        Ok(())
    }

    /// Le driver portant cet identifiant.
    ///
    /// Rend un [`Arc`] cloné : l'appelant garde le driver vivant le temps
    /// d'ouvrir une session, même si le registre est reconstruit entre-temps.
    #[must_use]
    pub fn get(&self, id: &DriverId) -> Option<Arc<dyn Driver>> {
        self.drivers.get(id).map(Arc::clone)
    }

    /// Le driver portant cet identifiant, ou une erreur montrable.
    ///
    /// C'est la forme utile quand l'identifiant vient d'un fichier de
    /// workspace : ouvrir une connexion dont le driver a disparu doit produire
    /// un message, pas un `None` que l'appelant traduira à sa façon.
    ///
    /// # Erreurs
    /// [`OxynError::Config`] si aucun driver ne porte cet identifiant.
    pub fn require(&self, id: &DriverId) -> Result<Arc<dyn Driver>> {
        self.get(id).ok_or_else(|| {
            OxynError::Config(format!(
                "no driver `{id}` is registered in this build of Oxyn"
            ))
        })
    }

    /// Les métadonnées d'un driver, sans le maintenir vivant.
    #[must_use]
    pub fn metadata(&self, id: &DriverId) -> Option<&DriverMetadata> {
        self.drivers.get(id).map(|driver| driver.metadata())
    }

    /// Ce driver est-il enregistré ?
    #[must_use]
    pub fn contains(&self, id: &DriverId) -> bool {
        self.drivers.contains_key(id)
    }

    /// Nombre de drivers enregistrés.
    #[must_use]
    pub fn len(&self) -> usize {
        self.drivers.len()
    }

    /// Aucun driver n'est enregistré.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.drivers.is_empty()
    }

    /// Les identifiants, dans l'ordre d'enregistrement.
    pub fn ids(&self) -> impl Iterator<Item = &DriverId> {
        self.drivers.keys()
    }

    /// Les drivers, dans l'ordre d'enregistrement.
    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn Driver>> {
        self.drivers.values()
    }

    /// Les drivers dans l'ordre d'affichage : **famille, puis nom affiché**.
    ///
    /// C'est l'ordre du sélecteur de connexion. Il est stable : le tri se fait
    /// sur des valeurs déclarées, pas sur l'ordre d'enregistrement — deux
    /// binaires qui enregistrent les mêmes drivers dans un ordre différent
    /// montrent la même liste.
    ///
    /// Alloue à chaque appel ; c'est un chemin d'ouverture de fenêtre, pas un
    /// chemin par ligne.
    #[must_use]
    pub fn sorted(&self) -> Vec<Arc<dyn Driver>> {
        let mut ordonnes: Vec<Arc<dyn Driver>> = self.drivers.values().map(Arc::clone).collect();
        ordonnes.sort_by(|a, b| {
            let (famille_a, nom_a) = a.metadata().sort_key();
            let (famille_b, nom_b) = b.metadata().sort_key();
            famille_a.cmp(&famille_b).then_with(|| nom_a.cmp(nom_b))
        });
        ordonnes
    }
}

impl fmt::Debug for DriverRegistry {
    /// Écrit à la main : `dyn Driver` n'est pas `Debug`, et il n'a pas à
    /// l'être. Ce qu'un diagnostic veut savoir, c'est quels protocoles sont
    /// disponibles.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DriverRegistry")
            .field("drivers", &self.drivers.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use oxyn_core::{CancelToken, Capabilities, ConnectionConfig};

    use super::*;
    use crate::credentials::Credentials;
    use crate::metadata::{ConnectionField, DriverFamily, FieldKind};
    use crate::traits::Session;

    /// Un driver qui ne sait pas se connecter : le registre n'en demande pas
    /// plus, et `connect` n'est pas ce qu'on éprouve ici.
    #[derive(Debug)]
    struct DriverFactice {
        metadata: DriverMetadata,
        /// Ce que rend `id()`, pour pouvoir le faire diverger des métadonnées.
        id: DriverId,
    }

    impl DriverFactice {
        fn new(id: &str, nom: &str, famille: DriverFamily) -> Self {
            let id = DriverId::new(id).expect("identifiant de test valide");
            Self {
                metadata: DriverMetadata::new(id.clone(), nom, famille),
                id,
            }
        }

        fn arc(self) -> Arc<dyn Driver> {
            Arc::new(self)
        }
    }

    #[async_trait]
    impl Driver for DriverFactice {
        fn id(&self) -> DriverId {
            self.id.clone()
        }

        fn metadata(&self) -> &DriverMetadata {
            &self.metadata
        }

        fn capabilities(&self) -> Capabilities {
            Capabilities::SQL
        }

        async fn connect(
            &self,
            _config: &ConnectionConfig,
            _credentials: &Credentials,
            _cancel: &CancelToken,
        ) -> Result<Box<dyn Session>> {
            Err(OxynError::Connection(
                "driver factice : aucune connexion".to_owned(),
            ))
        }
    }

    fn registre() -> DriverRegistry {
        let mut registre = DriverRegistry::new();
        registre
            .register(DriverFactice::new("postgres", "PostgreSQL", DriverFamily::Relational).arc())
            .expect("enregistrement");
        registre
            .register(DriverFactice::new("redis", "Redis", DriverFamily::KeyValue).arc())
            .expect("enregistrement");
        registre
            .register(
                DriverFactice::new("clickhouse", "ClickHouse", DriverFamily::Analytical).arc(),
            )
            .expect("enregistrement");
        registre
            .register(DriverFactice::new("mysql", "MySQL", DriverFamily::Relational).arc())
            .expect("enregistrement");
        registre
    }

    #[test]
    fn un_registre_vide_est_un_etat_legitime() {
        let registre = DriverRegistry::new();
        assert!(registre.is_empty());
        assert_eq!(registre.len(), 0);
        assert!(registre.sorted().is_empty());
    }

    #[test]
    fn un_driver_enregistre_se_retrouve_par_son_identifiant() {
        let registre = registre();
        let postgres = DriverId::postgres();

        assert!(registre.contains(&postgres));
        assert_eq!(registre.len(), 4);

        let driver = registre.get(&postgres).expect("enregistré");
        assert_eq!(driver.id(), postgres);
        assert_eq!(
            registre
                .metadata(&postgres)
                .map(|m| m.display_name.as_str()),
            Some("PostgreSQL")
        );
    }

    #[test]
    fn un_driver_absent_donne_un_message_pas_un_none_muet() {
        // L'identifiant vient d'un fichier de workspace : l'utilisateur doit
        // apprendre que ce protocole n'existe pas dans cette version.
        let registre = registre();
        let inconnu = DriverId::new("oracle").expect("identifiant valide");

        assert!(registre.get(&inconnu).is_none());
        // `expect_err` exigerait `Debug` sur la variante `Ok`, donc sur
        // `dyn Driver` — et un driver porte des identifiants de connexion, que
        // I-03 interdit d'exposer par `Debug`. Le `match` ne demande rien.
        let err = match registre.require(&inconnu) {
            Ok(_) => panic!("refus attendu : « oracle » n'est pas enregistré"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("oracle"), "{err}");
        assert!(err.is_user_error());
    }

    #[test]
    fn un_identifiant_deja_pris_est_refuse_pas_remplace() {
        // Remplacer le driver PostgreSQL détournerait les connexions qui le
        // visent — et, en phase 4, les identifiants de production avec elles.
        let mut registre = registre();
        let err = registre
            .register(DriverFactice::new("postgres", "Autre chose", DriverFamily::Relational).arc())
            .expect_err("refus attendu");

        assert!(err.to_string().contains("postgres"), "{err}");
        assert_eq!(
            registre
                .metadata(&DriverId::postgres())
                .map(|m| m.display_name.as_str()),
            Some("PostgreSQL"),
            "le driver d'origine reste en place"
        );
    }

    #[test]
    fn un_driver_qui_ment_sur_son_identifiant_est_refuse() {
        // Rangé sous une clé, présenté sous une autre : introuvable dès le
        // premier `require`.
        let mut driver = DriverFactice::new("postgres", "PostgreSQL", DriverFamily::Relational);
        driver.id = DriverId::mysql();

        let mut registre = DriverRegistry::new();
        let err = registre.register(driver.arc()).expect_err("refus attendu");
        assert!(err.to_string().contains("unreachable"), "{err}");
        assert!(registre.is_empty());
    }

    #[test]
    fn des_metadonnees_incoherentes_sont_refusees_a_l_enregistrement() {
        // Découvrir la faute à l'enregistrement plutôt qu'au premier formulaire
        // affiché.
        let mut driver = DriverFactice::new("postgres", "PostgreSQL", DriverFamily::Relational);
        driver.metadata = driver.metadata.clone().with_field(
            ConnectionField::new("password", "Mot de passe", FieldKind::Password)
                .with_default("postgres"),
        );

        let mut registre = DriverRegistry::new();
        assert!(registre.register(driver.arc()).is_err());
        assert!(registre.is_empty());
    }

    #[test]
    fn l_ordre_d_affichage_va_par_famille_puis_par_nom() {
        let registre = registre();
        let ordonnes = registre.sorted();
        let noms: Vec<&str> = ordonnes
            .iter()
            .map(|driver| driver.metadata().display_name.as_str())
            .collect();

        // Relational avant Analytical avant KeyValue, et MySQL avant PostgreSQL.
        assert_eq!(noms, ["MySQL", "PostgreSQL", "ClickHouse", "Redis"]);
    }

    #[test]
    fn l_iteration_brute_garde_l_ordre_d_enregistrement() {
        let registre = registre();
        let ids: Vec<&str> = registre.ids().map(DriverId::as_str).collect();
        assert_eq!(ids, ["postgres", "redis", "clickhouse", "mysql"]);
        assert_eq!(registre.iter().count(), 4);
    }

    #[test]
    fn le_debug_ne_montre_que_les_identifiants() {
        let registre = registre();
        let rendu = format!("{registre:?}");
        assert!(rendu.contains("postgres"), "{rendu}");
        assert!(!rendu.contains("PostgreSQL"), "{rendu}");
    }
}
