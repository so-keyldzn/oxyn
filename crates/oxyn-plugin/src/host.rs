//! L'hôte WebAssembly — **squelette de la phase 4**.
//!
//! Ce module n'existe que derrière la feature `wasm-host`. Les plugins WASM
//! sont explicitement reportés en phase 4
//! ([ADR-0005](../../../docs/adr/0005-wasm-plugins.md),
//! [IMPLEMENTATION-PLAN](../../../docs/IMPLEMENTATION-PLAN.md)), après six
//! implémentations natives ou plus : ouvrir une frontière d'extension sur des
//! traits que trop peu d'implémentations ont éprouvés fige des erreurs qu'il
//! faudra ensuite supporter indéfiniment.
//!
//! # Ce qui est vrai dès aujourd'hui
//!
//! Tout ce qui **refuse** l'est. Un plugin non approuvé, une version
//! d'interface incompatible, un agent déclaratif qui se présenterait à l'hôte,
//! un point d'entrée absent : ce sont des erreurs rendues par
//! [`WasmHost::prepare`], et elles sont testées. Le moteur, ses limites de
//! ressources et son carburant sont configurés réellement — ce sont les
//! réglages qui empêchent un plugin en boucle infinie de figer le workspace
//! ([`PLUGIN-CONTRACT` §2](../../../docs/PLUGIN-CONTRACT.md)).
//!
//! # Ce qui attend la phase 4
//!
//! La liaison des interfaces WIT (`oxyn:driver`, `oxyn:export`,
//! `oxyn:visualization`) et l'instanciation : [`WasmHost::linker`] et
//! [`WasmHost::instantiate`] portent un `todo!` explicite. C'est assumé — un
//! `Linker` vide qui instancierait « pour voir » produirait des composants sans
//! import satisfait et des diagnostics illisibles.
//!
//! # Le tempo, et pourquoi il ne suffit pas de le configurer
//!
//! Le carburant borne le **travail** ; il ne borne pas le **temps**, parce
//! qu'un composant bloqué dans un appel hôte n'en consomme pas. L'échéance
//! d'époque borne le temps, mais seulement si quelqu'un fait avancer l'horloge :
//! [`Engine::increment_epoch`] doit être appelé périodiquement depuis un fil
//! dédié. `// TODO(phase 4)` : ce fil, sa période, et l'arrêt propre qui va
//! avec. Sans lui, l'échéance posée ici ne se déclenche jamais.

use std::fmt;
use std::path::{Path, PathBuf};

use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};

use crate::error::{PluginError, Result};
use crate::manifest::{HOST_API_VERSION, PluginId, PluginKind, PluginPermissions};
use crate::registry::InstalledPlugin;

/// Les bornes imposées à tout composant de plugin.
///
/// **Ces valeurs sont provisoires.** Elles sont choisies pour être largement
/// suffisantes à un driver et largement insuffisantes à une fuite ; aucune n'est
/// mesurée. `// TODO(phase 4)` : les établir par la mesure, comme
/// [PERFORMANCE](../../../docs/PERFORMANCE.md) l'exige pour tout budget chiffré,
/// et les rendre configurables par plugin dans l'écran d'approbation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostLimits {
    /// Mémoire linéaire maximale, en octets.
    pub memory_bytes: usize,
    /// Nombre maximal d'éléments dans les tables du composant.
    pub table_elements: usize,
    /// Nombre maximal d'instances par magasin.
    pub instances: usize,
    /// Nombre maximal de tables par magasin.
    pub tables: usize,
    /// Nombre maximal de mémoires par magasin.
    pub memories: usize,
    /// Pile WebAssembly maximale, en octets. Réglée sur le moteur.
    pub stack_bytes: usize,
    /// Carburant accordé à un appel. Borne le travail, pas le temps.
    pub fuel: u64,
    /// Nombre de tops d'époque avant interruption. Borne le temps — à condition
    /// que quelqu'un fasse avancer l'horloge, cf. la note du module.
    pub epoch_ticks: u64,
}

impl Default for HostLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 64 * 1024 * 1024,
            table_elements: 10_000,
            instances: 1,
            tables: 4,
            memories: 1,
            stack_bytes: 512 * 1024,
            fuel: 1_000_000_000,
            epoch_ticks: 10,
        }
    }
}

/// Ce que l'hôte a vérifié avant d'accepter de faire tourner un composant.
///
/// Obtenir cette valeur est le seul chemin vers l'instanciation : elle est la
/// preuve que l'approbation, la version d'interface et le point d'entrée ont
/// été contrôlés. Un type qui porte une vérification vaut mieux qu'un
/// commentaire qui la rappelle.
#[derive(Debug, Clone)]
pub struct PreparedPlugin {
    id: PluginId,
    kind: PluginKind,
    component: PathBuf,
    permissions: PluginPermissions,
}

impl PreparedPlugin {
    /// Le plugin concerné.
    #[must_use]
    pub const fn id(&self) -> &PluginId {
        &self.id
    }

    /// La surface occupée.
    #[must_use]
    pub const fn kind(&self) -> PluginKind {
        self.kind
    }

    /// Le chemin du composant `.wasm`.
    #[must_use]
    pub fn component_path(&self) -> &Path {
        &self.component
    }

    /// Les permissions approuvées, celles que l'hôte fera respecter.
    #[must_use]
    pub const fn permissions(&self) -> &PluginPermissions {
        &self.permissions
    }
}

/// L'état que porte le magasin d'un composant.
///
/// Il tient deux choses, et rien d'autre : les limites de ressources, que
/// wasmtime interroge, et les permissions approuvées, que les implémentations
/// d'interfaces WIT interrogeront avant d'ouvrir quoi que ce soit. Aucun accès
/// au trousseau, aucune poignée de driver, aucune liste de connexions — c'est
/// [`PLUGIN-CONTRACT` §1](../../../docs/PLUGIN-CONTRACT.md) : « la protection
/// est de ne pas les lui donner ».
#[derive(Debug)]
pub struct HostState {
    plugin: PluginId,
    permissions: PluginPermissions,
    limits: StoreLimits,
}

impl HostState {
    /// Le plugin auquel ce magasin appartient.
    #[must_use]
    pub const fn plugin(&self) -> &PluginId {
        &self.plugin
    }

    /// Les permissions approuvées.
    #[must_use]
    pub const fn permissions(&self) -> &PluginPermissions {
        &self.permissions
    }

    /// Autorise une destination réseau, ou refuse en le disant.
    ///
    /// Le bac à sable rend la tentative inoffensive ; ce refus la rend
    /// **visible**, ce que le bac à sable seul ne fait pas.
    ///
    /// # Erreurs
    /// [`PluginError::PermissionDenied`] si la destination n'a pas été
    /// accordée hôte par hôte, port par port.
    pub fn authorize_host(&self, host: &str, port: u16) -> Result<()> {
        if self.permissions.allows_host(host, port) {
            return Ok(());
        }
        Err(PluginError::PermissionDenied {
            plugin: self.plugin.as_str().to_owned(),
            detail: format!("hôte `{host}:{port}` non accordé au manifeste"),
        })
    }

    /// Autorise un chemin, ou refuse en le disant.
    ///
    /// # Erreurs
    /// [`PluginError::PermissionDenied`] si le chemin n'est sous aucune racine
    /// accordée, ou s'il remonte hors d'une racine accordée.
    pub fn authorize_path(&self, path: &Path) -> Result<()> {
        if self.permissions.allows_path(path) {
            return Ok(());
        }
        Err(PluginError::PermissionDenied {
            plugin: self.plugin.as_str().to_owned(),
            detail: format!("chemin `{}` hors des racines accordées", path.display()),
        })
    }
}

/// Le moteur wasmtime d'Oxyn, et ses limites.
///
/// Un seul moteur pour tous les plugins : il porte le cache de compilation et
/// n'est pas un contexte d'exécution. L'isolation se fait par magasin — un
/// [`Store`] par appel, avec son propre carburant et sa propre échéance.
pub struct WasmHost {
    engine: Engine,
    limits: HostLimits,
}

impl fmt::Debug for WasmHost {
    /// Écrit à la main : `Engine` n'est pas `Debug`, et ce qu'un diagnostic
    /// veut savoir, ce sont les bornes en vigueur.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WasmHost")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl WasmHost {
    /// Construit le moteur.
    ///
    /// Trois réglages qui ne sont pas des détails :
    ///
    /// * le **modèle de composants** est activé — c'est l'objet même
    ///   d'[ADR-0005](../../../docs/adr/0005-wasm-plugins.md) ; un module
    ///   WebAssembly nu ne franchit pas cette frontière ;
    /// * le **carburant** est consommé, sans quoi une boucle infinie tourne
    ///   jusqu'à l'arrêt du processus ;
    /// * l'**interruption par époque** est instrumentée, seule façon de couper
    ///   un composant qui ne consomme pas de carburant parce qu'il attend.
    ///
    /// # Erreurs
    /// [`PluginError::WasmHost`] si wasmtime refuse la configuration — un
    /// réglage incompatible avec la cible de compilation, par exemple.
    pub fn new(limits: HostLimits) -> Result<Self> {
        let mut config = Config::new();
        config
            .wasm_component_model(true)
            .consume_fuel(true)
            .epoch_interruption(true)
            .wasm_backtrace(true)
            .max_wasm_stack(limits.stack_bytes);

        let engine = Engine::new(&config).map_err(|err| PluginError::WasmHost {
            detail: err.to_string(),
        })?;
        Ok(Self { engine, limits })
    }

    /// Le moteur, pour qui doit faire avancer l'horloge d'époque.
    #[must_use]
    pub const fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Les bornes en vigueur.
    #[must_use]
    pub const fn limits(&self) -> HostLimits {
        self.limits
    }

    /// Vérifie qu'un plugin a le droit d'être chargé, et où se trouve son
    /// composant.
    ///
    /// Quatre refus, dans cet ordre — celui qui donne le message le plus utile
    /// en premier :
    ///
    /// 1. **manifeste absent** — le plugin est en échec, il n'y a rien à
    ///    charger ;
    /// 2. **agent déclaratif** — il n'exécute pas de code, et le présenter à
    ///    l'hôte est un défaut d'appelant, pas une situation à gérer ;
    /// 3. **version d'interface incompatible** — refus explicite, jamais un
    ///    chargement « pour voir »
    ///    ([`PLUGIN-CONTRACT` §3](../../../docs/PLUGIN-CONTRACT.md)) ;
    /// 4. **plugin non approuvé** — déposer un répertoire n'accorde rien.
    ///
    /// Le composant est ensuite localisé et son existence vérifiée : découvrir
    /// un `.wasm` absent au moment de l'instanciation donnerait une erreur de
    /// wasmtime là où une phrase suffit.
    ///
    /// # Erreurs
    /// [`PluginError::InvalidManifest`], [`PluginError::IncompatibleInterface`],
    /// [`PluginError::NotApproved`] ou [`PluginError::Directory`] selon le
    /// refus.
    pub fn prepare(&self, plugin: &InstalledPlugin) -> Result<PreparedPlugin> {
        let slug = plugin.slug();
        let manifest = plugin.manifest().ok_or_else(|| {
            PluginError::invalid_manifest(slug, "aucun manifeste lisible : rien à charger")
        })?;

        if !manifest.requires_wasm() {
            return Err(PluginError::invalid_manifest(
                slug,
                format!(
                    "un plugin `{}` est déclaratif : il ne s'exécute pas dans l'hôte \
                     WebAssembly",
                    manifest.kind
                ),
            ));
        }
        if !manifest.is_api_compatible() {
            return Err(PluginError::IncompatibleInterface {
                plugin: slug.to_owned(),
                declared: manifest.api_version,
                host: HOST_API_VERSION,
            });
        }

        // Passe par `effective_permissions`, qui refuse tout ce qui n'est pas
        // approuvé : c'est le même contrôle que celui du registre, et il ne se
        // duplique pas ici.
        let permissions = plugin.effective_permissions()?.clone();

        let entrypoint = manifest
            .entrypoint
            .as_ref()
            .ok_or_else(|| PluginError::invalid_manifest(slug, "aucun `entrypoint` à charger"))?;
        let component = entrypoint.resolve(plugin.directory());
        if !component.is_file() {
            return Err(PluginError::Directory {
                path: component,
                source: std::io::Error::from(std::io::ErrorKind::NotFound),
            });
        }

        Ok(PreparedPlugin {
            id: manifest.id.clone(),
            kind: manifest.kind,
            component,
            permissions,
        })
    }

    /// Compile le composant, et **refuse** ce qui n'en est pas un.
    ///
    /// C'est le contrôle de forme le plus utile qu'on puisse faire avant la
    /// phase 4 : un module WebAssembly nu, un binaire tronqué ou un fichier qui
    /// n'a de `.wasm` que le nom sont rejetés ici, avec le diagnostic de
    /// wasmtime.
    ///
    /// Compiler prend du temps — c'est du code natif engendré. Ne pas appeler
    /// depuis le thread d'interface (I-05).
    ///
    /// # Erreurs
    /// [`PluginError::WasmHost`] si le fichier n'est pas un composant valide
    /// pour ce moteur.
    pub fn compile(&self, prepared: &PreparedPlugin) -> Result<Component> {
        Component::from_file(&self.engine, prepared.component_path()).map_err(|err| {
            PluginError::WasmHost {
                detail: format!(
                    "composant `{}` refusé : {err}",
                    prepared.component_path().display()
                ),
            }
        })
    }

    /// Ouvre un magasin borné pour un appel.
    ///
    /// Un magasin par appel : le carburant et l'échéance d'époque sont des
    /// budgets, et un budget partagé entre deux appels n'en est plus un.
    ///
    /// # Erreurs
    /// [`PluginError::WasmHost`] si l'allocation du magasin échoue, ou si le
    /// moteur refuse le carburant — ce qui signalerait que
    /// [`Config::consume_fuel`] n'a pas été activé.
    pub fn store(&self, prepared: &PreparedPlugin) -> Result<Store<HostState>> {
        let state = HostState {
            plugin: prepared.id.clone(),
            permissions: prepared.permissions.clone(),
            limits: StoreLimitsBuilder::new()
                .memory_size(self.limits.memory_bytes)
                .table_elements(self.limits.table_elements)
                .instances(self.limits.instances)
                .tables(self.limits.tables)
                .memories(self.limits.memories)
                .build(),
        };

        let mut store =
            Store::try_new(&self.engine, state).map_err(|err| PluginError::WasmHost {
                detail: err.to_string(),
            })?;
        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(self.limits.fuel)
            .map_err(|err| PluginError::WasmHost {
                detail: err.to_string(),
            })?;
        // Sans cet appel, l'échéance vaut zéro et tout appel piège
        // immédiatement : wasmtime l'exige dès que l'interruption par époque
        // est instrumentée.
        store.set_epoch_deadline(self.limits.epoch_ticks);
        Ok(store)
    }

    /// Construit le `Linker` et y lie les interfaces WIT d'Oxyn.
    ///
    /// # Erreurs
    /// Reportées en phase 4.
    ///
    /// # Panique
    /// Toujours, pour l'instant : la phase 4 n'est pas écrite.
    pub fn linker(&self) -> Result<Linker<HostState>> {
        todo!(
            "phase 4 : construire un `Linker<HostState>` sur `self.engine()` et y lier \
             `oxyn:driver`, `oxyn:export` et `oxyn:visualization`, en vérifiant la version \
             de chaque interface au chargement (PLUGIN-CONTRACT §3)"
        )
    }

    /// Instancie un composant préparé dans son magasin.
    ///
    /// # Erreurs
    /// Reportées en phase 4.
    ///
    /// # Panique
    /// Toujours, pour l'instant : la phase 4 n'est pas écrite.
    pub fn instantiate(&self, _component: &Component, _store: &mut Store<HostState>) -> Result<()> {
        todo!(
            "phase 4 : instancier le composant via le `Linker`, puis exposer ses exports \
             par-dessus les traits de `oxyn-driver` — les `RecordBatch` traversent en \
             Arrow IPC, jamais champ à champ (PLUGIN-CONTRACT §4)"
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::fixtures::{TempDir, agent_toml, export_toml};
    use crate::registry::PluginRegistry;

    use super::*;

    /// Un composant fictif : `prepare` ne vérifie que l'existence du fichier,
    /// c'est `compile` qui le refuserait.
    const FAUX_COMPOSANT: &[u8] = b"\0asm\x0d\x00\x01\x00";

    fn hote() -> WasmHost {
        WasmHost::new(HostLimits::default()).expect("le moteur wasmtime se configure")
    }

    fn registre(racine: &TempDir) -> PluginRegistry {
        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("découverte");
        registre
    }

    #[test]
    fn le_moteur_se_configure_avec_carburant_et_epoques() {
        let hote = hote();
        assert_eq!(hote.limits(), HostLimits::default());
        // `engine()` est ce dont a besoin le fil qui fera avancer l'horloge.
        hote.engine().increment_epoch();
    }

    #[test]
    fn un_plugin_non_approuve_ne_se_prepare_pas() {
        // ADR-0005 : déposer un répertoire n'accorde rien, et le refus tombe
        // avant que le moindre octet du composant ne soit lu.
        let racine = TempDir::new("host-non-approuve");
        racine.plugin("csv", &export_toml("csv", ""));
        racine.file("csv", "csv.wasm", FAUX_COMPOSANT);

        let registre = registre(&racine);
        let plugin = registre.require("csv").expect("plugin découvert");
        let err = hote().prepare(plugin).expect_err("refus attendu");
        assert!(matches!(err, PluginError::NotApproved { .. }), "{err}");
        assert!(err.needs_user_decision());
    }

    #[test]
    fn un_agent_declaratif_ne_passe_jamais_par_l_hote() {
        let racine = TempDir::new("host-agent");
        racine.plugin("revue", &agent_toml("revue", "\"refresh_catalog\""));

        let mut registre = registre(&racine);
        registre.approve("revue").expect("approbation");

        let plugin = registre.require("revue").expect("plugin découvert");
        let err = hote().prepare(plugin).expect_err("refus attendu");
        assert!(err.to_string().contains("déclaratif"), "{err}");
    }

    #[test]
    fn un_composant_absent_se_dit_en_une_phrase() {
        let racine = TempDir::new("host-sans-composant");
        racine.plugin("csv", &export_toml("csv", ""));

        let mut registre = registre(&racine);
        registre.approve("csv").expect("approbation");

        let plugin = registre.require("csv").expect("plugin découvert");
        let err = hote().prepare(plugin).expect_err("refus attendu");
        assert!(matches!(err, PluginError::Directory { .. }), "{err}");
        assert!(err.to_string().contains("csv.wasm"), "{err}");
    }

    #[test]
    fn un_plugin_approuve_se_prepare_avec_ses_permissions() {
        let racine = TempDir::new("host-approuve");
        racine.plugin("csv", &export_toml("csv", "\"a.example:443\""));
        let attendu = racine.file("csv", "csv.wasm", FAUX_COMPOSANT);

        let mut registre = registre(&racine);
        registre.approve("csv").expect("approbation");

        let plugin = registre.require("csv").expect("plugin découvert");
        let prepare = hote().prepare(plugin).expect("préparation");

        assert_eq!(prepare.id().as_str(), "csv");
        assert_eq!(prepare.kind(), PluginKind::Export);
        assert_eq!(prepare.component_path(), attendu);
        assert!(prepare.permissions().allows_host("a.example", 443));
        assert!(
            !prepare
                .permissions()
                .allows_host("exfiltration.example", 443)
        );
    }

    #[test]
    fn le_magasin_porte_le_carburant_et_les_permissions() {
        let racine = TempDir::new("host-magasin");
        racine.plugin("csv", &export_toml("csv", "\"a.example:443\""));
        racine.file("csv", "csv.wasm", FAUX_COMPOSANT);

        let mut registre = registre(&racine);
        registre.approve("csv").expect("approbation");

        let hote = hote();
        let plugin = registre.require("csv").expect("plugin découvert");
        let prepare = hote.prepare(plugin).expect("préparation");
        let store = hote.store(&prepare).expect("magasin");

        assert_eq!(
            store.get_fuel().expect("le carburant est activé"),
            HostLimits::default().fuel
        );
        let etat = store.data();
        assert_eq!(etat.plugin().as_str(), "csv");
        etat.authorize_host("a.example", 443)
            .expect("hôte accordé au manifeste");
        let err = etat
            .authorize_host("exfiltration.example", 443)
            .expect_err("refus attendu");
        assert!(matches!(err, PluginError::PermissionDenied { .. }), "{err}");
    }

    #[test]
    fn un_fichier_qui_n_est_pas_un_composant_est_refuse() {
        // « refusé avec un message clair, jamais chargé pour voir. »
        let racine = TempDir::new("host-compilation");
        racine.plugin("csv", &export_toml("csv", ""));
        racine.file("csv", "csv.wasm", b"ceci n'est pas du WebAssembly");

        let mut registre = registre(&racine);
        registre.approve("csv").expect("approbation");

        let hote = hote();
        let plugin = registre.require("csv").expect("plugin découvert");
        let prepare = hote.prepare(plugin).expect("préparation");
        let err = hote.compile(&prepare).expect_err("refus attendu");
        assert!(matches!(err, PluginError::WasmHost { .. }), "{err}");
    }

    #[test]
    fn un_chemin_hors_des_racines_accordees_est_refuse() {
        let racine = TempDir::new("host-fichiers");
        let manifeste = export_toml("csv", "").replace(
            "network = []",
            "network = []\nfilesystem = [\"/donnees/exports\"]",
        );
        racine.plugin("csv", &manifeste);
        racine.file("csv", "csv.wasm", FAUX_COMPOSANT);

        let mut registre = registre(&racine);
        registre.approve("csv").expect("approbation");

        let hote = hote();
        let plugin = registre.require("csv").expect("plugin découvert");
        let prepare = hote.prepare(plugin).expect("préparation");
        let store = hote.store(&prepare).expect("magasin");
        let etat = store.data();

        etat.authorize_path(Path::new("/donnees/exports/rapport.csv"))
            .expect("chemin sous une racine accordée");
        for refuse in [
            "/etc/passwd",
            "/donnees/exports/../../etc/passwd",
            "/donnees/exports-voisin/x",
        ] {
            assert!(
                etat.authorize_path(Path::new(refuse)).is_err(),
                "{refuse} devrait être refusé"
            );
        }
    }
}
