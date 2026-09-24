//! Le `PolicyGate` : le point de passage unique du command bus (ADR-0004).
//!
//! Donner à des agents IA l'accès à des bases de production est le risque n° 1
//! du produit. Toute commande — de l'interface, d'un agent, d'un plugin —
//! traverse ce gate et en ressort avec une [`Decision`] : `Allow`,
//! `RequireApproval` ou `Deny`. Il n'y a pas de second chemin, et il ne doit
//! jamais y avoir de `if` de vérification dupliqué dans un appelant : deux
//! endroits qui décident, c'est un endroit qui oubliera.
//!
//! # La politique par défaut
//!
//! [`DefaultPolicy`] applique, dans cet ordre :
//!
//! | Règle | Effet |
//! |---|---|
//! | agent + `GRANT`/`REVOKE` | `Deny` |
//! | agent + `SetSessionContext` | `Deny` — l'effet porte sur les instructions suivantes |
//! | commande mutante sur une connexion marquée lecture seule | `Deny`, humain compris |
//! | commande mutante sur une connexion **inconnue** du gate | `Deny` |
//! | agent + commande mutante + production | `Deny` — lecture seule stricte |
//! | risque de mutation non borné (`UPDATE`/`DELETE` sans `WHERE`, `TRUNCATE`, `DROP`) | `RequireApproval`, quel que soit l'acteur |
//! | agent + commande mutante | `RequireApproval` avec prévisualisation |
//! | humain + commande mutante + production | `RequireApproval` nommant la connexion |
//! | le reste | `Allow` |
//!
//! Deux points qui ne sont pas des détails :
//!
//! * pour un agent, une connexion de production est en **lecture seule
//!   stricte** — un refus, pas une confirmation renforcée. La différence
//!   compte : une confirmation finit par être cliquée ;
//! * la règle « humain + production » vient de
//!   [`SECURITY`](../../../docs/SECURITY.md) : sur une connexion `production`,
//!   toute écriture et tout DDL exigent une confirmation explicite qui **nomme
//!   la connexion**. Elle s'ajoute à la matrice acteur × intention.
//!
//! # Fermé par défaut
//!
//! Une commande mutante visant une connexion que le gate ne connaît pas est
//! **refusée**. Le gate ne peut pas décider de ce qu'il ne voit pas : ignorer le
//! marquage d'environnement d'une connexion non enregistrée reviendrait à
//! traiter la production comme du local. Enregistrer les connexions avec
//! [`DefaultPolicy::register`] fait donc partie du câblage, pas de la
//! configuration optionnelle.

use std::collections::HashMap;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::command::{Actor, Command};
use crate::connection::{ConnectionConfig, Environment};
use crate::ids::ConnectionId;
use crate::query::StatementIntent;

/// Ce que le gate répond.
///
/// Énumération **fermée** : la triade d'ADR-0004 est le contrat du bus. Une
/// quatrième réponse serait une décision d'ADR.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "decision")]
pub enum Decision {
    /// La commande peut s'exécuter.
    Allow,
    /// La commande ne s'exécute qu'après un accord explicite de l'utilisateur.
    RequireApproval {
        /// Ce sur quoi l'utilisateur doit se prononcer, rédigé pour être lu.
        reason: String,
        /// De quoi juger sans relire la requête ailleurs.
        preview: Option<Preview>,
    },
    /// La commande ne s'exécutera pas. Aucune confirmation ne la débloque.
    Deny {
        /// Pourquoi, en des termes montrables à l'utilisateur.
        reason: String,
    },
}

impl Decision {
    /// Construit un refus.
    #[must_use]
    pub fn deny(reason: impl Into<String>) -> Self {
        Self::Deny {
            reason: reason.into(),
        }
    }

    /// Construit une demande d'approbation.
    #[must_use]
    pub fn approval(reason: impl Into<String>, preview: Option<Preview>) -> Self {
        Self::RequireApproval {
            reason: reason.into(),
            preview,
        }
    }

    /// La commande peut-elle s'exécuter sans autre formalité ?
    #[must_use]
    pub const fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow)
    }

    /// La commande est-elle refusée ?
    #[must_use]
    pub const fn is_denied(&self) -> bool {
        matches!(self, Self::Deny { .. })
    }

    /// La commande attend-elle un accord ?
    #[must_use]
    pub const fn requires_approval(&self) -> bool {
        matches!(self, Self::RequireApproval { .. })
    }

    /// Degré de restriction, croissant : `Allow` < `RequireApproval` < `Deny`.
    ///
    /// Sert à comparer deux décisions — notamment à vérifier qu'une décision
    /// rendue pour un agent est au moins aussi restrictive que celle rendue
    /// pour un humain sur la même commande.
    #[must_use]
    pub const fn restrictiveness(&self) -> u8 {
        match self {
            Self::Allow => 0,
            Self::RequireApproval { .. } => 1,
            Self::Deny { .. } => 2,
        }
    }
}

/// Ce qu'on montre à l'utilisateur avant qu'il tranche.
///
/// Une confirmation qui se clique par réflexe ne protège personne : la
/// prévisualisation nomme la connexion et montre l'instruction exacte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preview {
    /// L'instruction exacte, telle que l'utilisateur ou l'agent l'a écrite.
    /// Sans les valeurs liées (I-03).
    pub statement: String,
    /// Le **nom** de la connexion, jamais son identifiant.
    pub connection: String,
    /// Estimation du nombre de lignes touchées, quand elle est connue.
    // TODO(2026-12-31) : l'estimation demande un `EXPLAIN` avant revue, dont le
    // coût et la forme diffèrent par driver — c'est cela qui la débloque, et
    // c'est un lot identifié dans IMPLEMENTATION-PLAN.
    //
    // La rédaction précédente disait « tant que `oxyn-catalog` n'existe pas ».
    // Cette crate existe et sert partout depuis longtemps : la marque portait
    // une condition déjà satisfaite, donc un déblocage qui ne viendrait jamais.
    // `script/verifier-todo` contrôle l'échéance, pas la véracité du motif.
    //
    // Un chiffre inventé serait pire que pas de chiffre : la revue de
    // production le lirait comme une mesure.
    pub estimated_rows: Option<u64>,
}

impl Preview {
    /// Construit une prévisualisation sans estimation.
    #[must_use]
    pub fn new(statement: impl Into<String>, connection: impl Into<String>) -> Self {
        Self {
            statement: statement.into(),
            connection: connection.into(),
            estimated_rows: None,
        }
    }
}

/// Le point de passage unique de toute commande.
///
/// C'est une **frontière**, pas une abstraction spéculative : les politiques
/// d'entreprise et les politiques de test sont d'autres implémentations.
pub trait PolicyGate: Send + Sync {
    /// Rend la décision applicable à cette commande, pour cet acteur, dans cet
    /// environnement.
    ///
    /// `env` est l'environnement que l'appelant attribue à la commande. Une
    /// implémentation qui connaît le marquage réel de la connexion visée doit
    /// retenir **le plus contraignant des deux** : un appelant qui se trompe ne
    /// doit pas pouvoir dégrader la protection.
    fn authorize(&self, actor: &Actor, cmd: &Command, env: Environment) -> Decision;

    /// Nom de la politique, pour le journal d'audit.
    fn name(&self) -> &'static str {
        "policy"
    }
}

/// Ce que le gate doit savoir d'une connexion pour décider.
///
/// Volontairement réduit : ni paramètres, ni référence de secret. Le gate n'a
/// pas besoin de savoir se connecter, seulement de savoir ce qu'il protège.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionFacts {
    /// Le nom montré à l'utilisateur.
    pub name: String,
    /// Le marquage d'environnement.
    pub environment: Environment,
    /// La connexion est-elle déclarée en lecture seule ?
    pub read_only: bool,
}

impl From<&ConnectionConfig> for ConnectionFacts {
    fn from(cfg: &ConnectionConfig) -> Self {
        Self {
            name: cfg.name.clone(),
            environment: cfg.environment,
            read_only: cfg.read_only,
        }
    }
}

/// La politique par défaut d'Oxyn.
///
/// Le registre de connexions est interne et protégé par un verrou en lecture
/// écriture : le gate est partagé entre le thread d'interface, l'exécuteur et
/// les agents, et les connexions apparaissent et disparaissent pendant la vie
/// du programme.
#[derive(Debug, Default)]
pub struct DefaultPolicy {
    connections: RwLock<HashMap<ConnectionId, ConnectionFacts>>,
}

impl DefaultPolicy {
    /// Crée une politique sans aucune connexion enregistrée.
    ///
    /// Dans cet état, toute commande **mutante** visant un serveur est refusée :
    /// voir la note « fermé par défaut » du module.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Enregistre ou met à jour ce que le gate sait d'une connexion.
    ///
    /// À appeler à la création d'une connexion et à chaque modification : un
    /// registre en retard sur la configuration ferait décider le gate sur un
    /// marquage périmé.
    pub fn register(&self, config: &ConnectionConfig) {
        self.connections
            .write()
            .insert(config.id, ConnectionFacts::from(config));
    }

    /// Enregistre des faits construits à la main.
    pub fn register_facts(&self, id: ConnectionId, facts: ConnectionFacts) {
        self.connections.write().insert(id, facts);
    }

    /// Oublie une connexion supprimée.
    pub fn forget(&self, id: ConnectionId) {
        self.connections.write().remove(&id);
    }

    /// Ce que le gate sait d'une connexion.
    #[must_use]
    pub fn facts(&self, id: ConnectionId) -> Option<ConnectionFacts> {
        self.connections.read().get(&id).cloned()
    }

    /// Nom montrable d'une connexion, ou une mention neutre si elle est
    /// inconnue. Ne rend **jamais** l'identifiant (I-03).
    fn display_name(facts: Option<&ConnectionFacts>) -> String {
        facts.map_or_else(|| "unknown connection".to_owned(), |f| f.name.clone())
    }

    /// Construit la prévisualisation d'une commande.
    fn preview(cmd: &Command, facts: Option<&ConnectionFacts>) -> Option<Preview> {
        let statement = cmd
            .statement_text()
            .map_or_else(|| cmd.name().to_owned(), |texte| texte.to_owned());
        Some(Preview::new(statement, Self::display_name(facts)))
    }
}

impl PolicyGate for DefaultPolicy {
    fn authorize(&self, actor: &Actor, cmd: &Command, env: Environment) -> Decision {
        if actor.is_agent() && matches!(cmd, Command::WriteWorkspacePreferences { .. }) {
            return Decision::Deny {
                reason: "only the human may change workspace display preferences".into(),
            };
        }
        // Un agent ne déclare pas le point d'accès par lequel il parle. Une
        // déclaration porte une URL de base : un agent qui pourrait l'écrire
        // ferait sortir de la machine tout ce qu'on lui confie ensuite, sans
        // qu'aucune exécution ne figure au journal. C'est un refus et non une
        // approbation renforcée — une confirmation finit par être cliquée
        // (I-02, ADR-0023).
        if actor.is_agent()
            && matches!(
                cmd,
                Command::SaveAiProvider { .. }
                    | Command::RemoveAiProvider { .. }
                    | Command::SaveExternalAgent { .. }
                    | Command::RemoveExternalAgent { .. }
            )
        {
            return Decision::Deny {
                reason: "only the human may declare or remove an AI provider".into(),
            };
        }
        // Trying a configuration is choosing the host a session opens on: for
        // an agent, the exfiltration channel `CreateConnection` is guarded
        // against, without even a saved connection to show for it.
        if actor.is_agent() && matches!(cmd, Command::TestConnection { .. }) {
            return Decision::deny("only the human may test a connection configuration");
        }
        // Réconcilier, c'est déclarer avoir inspecté le serveur. Un agent n'a
        // rien inspecté : l'accepter ferait taire l'avertissement d'une écriture
        // au résultat inconnu que personne n'a regardée (I-13). Un refus, pas
        // une confirmation : une confirmation finit par être cliquée (I-02).
        if actor.is_agent() && matches!(cmd, Command::ReconcileHistoryEntry { .. }) {
            return Decision::Deny {
                reason: "only the human may declare an interrupted write reconciled".into(),
            };
        }
        let intent = cmd.intent();
        let mutating = cmd.is_mutating();
        let facts = cmd.target_connection().and_then(|id| self.facts(id));

        // L'environnement retenu est le plus contraignant entre celui que
        // l'appelant annonce et celui dont la connexion est marquée. Un
        // appelant qui se trompe ne doit pas pouvoir dégrader la protection.
        let env = facts.as_ref().map_or(env, |f| env.max(f.environment));

        // ── Refus ───────────────────────────────────────────────────────────

        // Un agent ne gère jamais les droits. Ce n'est pas une question de
        // confiance dans le modèle : c'est la seule catégorie d'action dont un
        // agent n'a aucun usage légitime et dont l'effet survit à la session.
        if actor.is_agent() && intent == StatementIntent::Grant {
            return Decision::deny("an agent may not change privileges (GRANT / REVOKE)");
        }

        // Un agent ne déplace pas le contexte de session. La commande ne lit ni
        // n'écrit de donnée — mais son effet survit à la commande, et il porte
        // sur le sens des instructions **suivantes** : l'utilisateur qui écrit
        // ensuite `DELETE FROM users` frapperait un autre schéma que celui qu'il
        // croit viser, sans qu'aucune confirmation ne parle de ce déplacement.
        if actor.is_agent() && matches!(cmd, Command::SetSessionContext { .. }) {
            return Decision::deny(
                "an agent may not change the session context: \
                 it changes the meaning of the statements that follow",
            );
        }

        if mutating && cmd.touches_database() {
            match (cmd.target_connection(), facts.as_ref()) {
                (Some(_), None) => {
                    return Decision::deny(
                        "connection unknown to the policy: \
                         its markings cannot be checked",
                    );
                }
                (Some(_), Some(f)) if f.read_only => {
                    return Decision::deny(format!(
                        "connection \"{}\" is marked read-only",
                        f.name
                    ));
                }
                _ => {}
            }
        }

        // Pour un agent, une connexion de production est en lecture seule
        // stricte. Un refus, pas une confirmation renforcée.
        if actor.is_agent() && mutating && env.is_production() {
            return Decision::deny(format!(
                "an agent is strictly read-only on \"{}\" (production)",
                Self::display_name(facts.as_ref())
            ));
        }

        // ── Approbations ────────────────────────────────────────────────────

        // Le risque prime, parce que son motif est le plus informatif : mieux
        // vaut lire « DELETE sans WHERE » que « écriture par un agent ».
        if let Some(motif) = cmd.mutation_risk().reason() {
            return Decision::approval(motif, Self::preview(cmd, facts.as_ref()));
        }

        if actor.is_agent() && mutating {
            return Decision::approval(
                format!(
                    "an agent is requesting a {intent} operation on \"{}\"",
                    Self::display_name(facts.as_ref())
                ),
                Self::preview(cmd, facts.as_ref()),
            );
        }

        // SECURITY : sur une connexion de production, toute écriture et tout
        // DDL exigent une confirmation qui nomme la connexion.
        if mutating && env.is_production() {
            return Decision::approval(
                format!(
                    "{intent} operation on \"{}\", marked production",
                    Self::display_name(facts.as_ref())
                ),
                Self::preview(cmd, facts.as_ref()),
            );
        }

        Decision::Allow
    }

    fn name(&self) -> &'static str {
        "DefaultPolicy"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{AgentId, AgentSessionId, DriverId, SessionId, StatementHandle};
    use crate::query::{ExecRequest, MutationRisk, QueryLanguage};

    /// Forme attendue d'une décision, indépendamment du texte du motif.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Forme {
        Allow,
        Approve,
        Deny,
    }

    impl Forme {
        fn de(decision: &Decision) -> Self {
            match decision {
                Decision::Allow => Self::Allow,
                Decision::RequireApproval { .. } => Self::Approve,
                Decision::Deny { .. } => Self::Deny,
            }
        }
    }

    struct Banc {
        politique: DefaultPolicy,
        ouverte: ConnectionId,
        lecture_seule: ConnectionId,
    }

    impl Banc {
        fn new() -> Self {
            let politique = DefaultPolicy::new();

            // Les deux connexions sont marquées `Local` : c'est l'environnement
            // passé à `authorize` qui pilote la matrice. L'escalade par le
            // marquage a son propre test.
            let ouverte = ConnectionConfig::new("atelier", DriverId::postgres())
                .with_environment(Environment::Local);
            let lecture_seule = ConnectionConfig::new("réplica", DriverId::postgres())
                .with_environment(Environment::Local)
                .read_only();

            politique.register(&ouverte);
            politique.register(&lecture_seule);

            Self {
                politique,
                ouverte: ouverte.id,
                lecture_seule: lecture_seule.id,
            }
        }

        fn connexion(&self, read_only: bool) -> ConnectionId {
            if read_only {
                self.lecture_seule
            } else {
                self.ouverte
            }
        }

        fn execute(&self, read_only: bool, intent: StatementIntent) -> Command {
            Command::Execute {
                connection: self.connexion(read_only),
                session: SessionId::new(),
                request: Box::new(
                    ExecRequest::new(QueryLanguage::SQL, "SELECT 1").with_intent(intent),
                ),
            }
        }
    }

    fn humain() -> Actor {
        Actor::Human
    }

    fn agent() -> Actor {
        Actor::agent(AgentId::new(), AgentSessionId::new())
    }

    const ENVS: [Environment; 4] = [
        Environment::Local,
        Environment::Development,
        Environment::Staging,
        Environment::Production,
    ];

    /// La matrice complète : acteur × intention × environnement × lecture seule.
    ///
    /// Chaque cellule est écrite à la main. Une table calculée par une fonction
    /// oracle ne prouverait rien : elle recopierait l'implémentation.
    #[rustfmt::skip]
    const MATRICE: &[(bool, StatementIntent, Environment, bool, Forme)] = &[
        // ── humain, connexion ouverte ───────────────────────────────────────
        (false, StatementIntent::Read,    Environment::Local,       false, Forme::Allow),
        (false, StatementIntent::Read,    Environment::Development, false, Forme::Allow),
        (false, StatementIntent::Read,    Environment::Staging,     false, Forme::Allow),
        (false, StatementIntent::Read,    Environment::Production,  false, Forme::Allow),
        (false, StatementIntent::Write,   Environment::Local,       false, Forme::Allow),
        (false, StatementIntent::Write,   Environment::Development, false, Forme::Allow),
        (false, StatementIntent::Write,   Environment::Staging,     false, Forme::Allow),
        (false, StatementIntent::Write,   Environment::Production,  false, Forme::Approve),
        (false, StatementIntent::Ddl,     Environment::Local,       false, Forme::Allow),
        (false, StatementIntent::Ddl,     Environment::Development, false, Forme::Allow),
        (false, StatementIntent::Ddl,     Environment::Staging,     false, Forme::Allow),
        (false, StatementIntent::Ddl,     Environment::Production,  false, Forme::Approve),
        (false, StatementIntent::Grant,   Environment::Local,       false, Forme::Allow),
        (false, StatementIntent::Grant,   Environment::Development, false, Forme::Allow),
        (false, StatementIntent::Grant,   Environment::Staging,     false, Forme::Allow),
        (false, StatementIntent::Grant,   Environment::Production,  false, Forme::Approve),
        (false, StatementIntent::Unknown, Environment::Local,       false, Forme::Allow),
        (false, StatementIntent::Unknown, Environment::Development, false, Forme::Allow),
        (false, StatementIntent::Unknown, Environment::Staging,     false, Forme::Allow),
        (false, StatementIntent::Unknown, Environment::Production,  false, Forme::Approve),

        // ── humain, connexion en lecture seule ──────────────────────────────
        (false, StatementIntent::Read,    Environment::Local,       true,  Forme::Allow),
        (false, StatementIntent::Read,    Environment::Development, true,  Forme::Allow),
        (false, StatementIntent::Read,    Environment::Staging,     true,  Forme::Allow),
        (false, StatementIntent::Read,    Environment::Production,  true,  Forme::Allow),
        (false, StatementIntent::Write,   Environment::Local,       true,  Forme::Deny),
        (false, StatementIntent::Write,   Environment::Development, true,  Forme::Deny),
        (false, StatementIntent::Write,   Environment::Staging,     true,  Forme::Deny),
        (false, StatementIntent::Write,   Environment::Production,  true,  Forme::Deny),
        (false, StatementIntent::Ddl,     Environment::Local,       true,  Forme::Deny),
        (false, StatementIntent::Ddl,     Environment::Development, true,  Forme::Deny),
        (false, StatementIntent::Ddl,     Environment::Staging,     true,  Forme::Deny),
        (false, StatementIntent::Ddl,     Environment::Production,  true,  Forme::Deny),
        (false, StatementIntent::Grant,   Environment::Local,       true,  Forme::Deny),
        (false, StatementIntent::Grant,   Environment::Development, true,  Forme::Deny),
        (false, StatementIntent::Grant,   Environment::Staging,     true,  Forme::Deny),
        (false, StatementIntent::Grant,   Environment::Production,  true,  Forme::Deny),
        (false, StatementIntent::Unknown, Environment::Local,       true,  Forme::Deny),
        (false, StatementIntent::Unknown, Environment::Development, true,  Forme::Deny),
        (false, StatementIntent::Unknown, Environment::Staging,     true,  Forme::Deny),
        (false, StatementIntent::Unknown, Environment::Production,  true,  Forme::Deny),

        // ── agent, connexion ouverte ────────────────────────────────────────
        (true,  StatementIntent::Read,    Environment::Local,       false, Forme::Allow),
        (true,  StatementIntent::Read,    Environment::Development, false, Forme::Allow),
        (true,  StatementIntent::Read,    Environment::Staging,     false, Forme::Allow),
        (true,  StatementIntent::Read,    Environment::Production,  false, Forme::Allow),
        (true,  StatementIntent::Write,   Environment::Local,       false, Forme::Approve),
        (true,  StatementIntent::Write,   Environment::Development, false, Forme::Approve),
        (true,  StatementIntent::Write,   Environment::Staging,     false, Forme::Approve),
        (true,  StatementIntent::Write,   Environment::Production,  false, Forme::Deny),
        (true,  StatementIntent::Ddl,     Environment::Local,       false, Forme::Approve),
        (true,  StatementIntent::Ddl,     Environment::Development, false, Forme::Approve),
        (true,  StatementIntent::Ddl,     Environment::Staging,     false, Forme::Approve),
        (true,  StatementIntent::Ddl,     Environment::Production,  false, Forme::Deny),
        (true,  StatementIntent::Grant,   Environment::Local,       false, Forme::Deny),
        (true,  StatementIntent::Grant,   Environment::Development, false, Forme::Deny),
        (true,  StatementIntent::Grant,   Environment::Staging,     false, Forme::Deny),
        (true,  StatementIntent::Grant,   Environment::Production,  false, Forme::Deny),
        (true,  StatementIntent::Unknown, Environment::Local,       false, Forme::Approve),
        (true,  StatementIntent::Unknown, Environment::Development, false, Forme::Approve),
        (true,  StatementIntent::Unknown, Environment::Staging,     false, Forme::Approve),
        (true,  StatementIntent::Unknown, Environment::Production,  false, Forme::Deny),

        // ── agent, connexion en lecture seule ───────────────────────────────
        (true,  StatementIntent::Read,    Environment::Local,       true,  Forme::Allow),
        (true,  StatementIntent::Read,    Environment::Development, true,  Forme::Allow),
        (true,  StatementIntent::Read,    Environment::Staging,     true,  Forme::Allow),
        (true,  StatementIntent::Read,    Environment::Production,  true,  Forme::Allow),
        (true,  StatementIntent::Write,   Environment::Local,       true,  Forme::Deny),
        (true,  StatementIntent::Write,   Environment::Development, true,  Forme::Deny),
        (true,  StatementIntent::Write,   Environment::Staging,     true,  Forme::Deny),
        (true,  StatementIntent::Write,   Environment::Production,  true,  Forme::Deny),
        (true,  StatementIntent::Ddl,     Environment::Local,       true,  Forme::Deny),
        (true,  StatementIntent::Ddl,     Environment::Development, true,  Forme::Deny),
        (true,  StatementIntent::Ddl,     Environment::Staging,     true,  Forme::Deny),
        (true,  StatementIntent::Ddl,     Environment::Production,  true,  Forme::Deny),
        (true,  StatementIntent::Grant,   Environment::Local,       true,  Forme::Deny),
        (true,  StatementIntent::Grant,   Environment::Development, true,  Forme::Deny),
        (true,  StatementIntent::Grant,   Environment::Staging,     true,  Forme::Deny),
        (true,  StatementIntent::Grant,   Environment::Production,  true,  Forme::Deny),
        (true,  StatementIntent::Unknown, Environment::Local,       true,  Forme::Deny),
        (true,  StatementIntent::Unknown, Environment::Development, true,  Forme::Deny),
        (true,  StatementIntent::Unknown, Environment::Staging,     true,  Forme::Deny),
        (true,  StatementIntent::Unknown, Environment::Production,  true,  Forme::Deny),
    ];

    #[test]
    fn la_matrice_couvre_toutes_les_cellules() {
        // 2 acteurs × 5 intentions × 4 environnements × 2 marquages.
        assert_eq!(MATRICE.len(), 80, "une cellule manque à la matrice");

        let mut vues = std::collections::HashSet::new();
        for (est_agent, intent, env, ro, _) in MATRICE {
            assert!(
                vues.insert((*est_agent, *intent, *env, *ro)),
                "cellule dupliquée : {est_agent} {intent} {env} {ro}"
            );
        }
    }

    #[test]
    fn matrice_de_la_politique_par_defaut() {
        let banc = Banc::new();

        for (est_agent, intent, env, read_only, attendu) in MATRICE {
            let acteur = if *est_agent { agent() } else { humain() };
            let cmd = banc.execute(*read_only, *intent);
            let decision = banc.politique.authorize(&acteur, &cmd, *env);

            assert_eq!(
                Forme::de(&decision),
                *attendu,
                "acteur={acteur} intention={intent} env={env} lecture_seule={read_only} \
                 → {decision:?}"
            );
        }
    }

    #[test]
    fn un_agent_n_est_jamais_moins_restreint_qu_un_humain() {
        // Le test que demande la commande `/commande` : la même commande émise
        // par un agent donne une décision au moins aussi restrictive.
        let banc = Banc::new();
        let agent = agent();

        for intent in [
            StatementIntent::Read,
            StatementIntent::Write,
            StatementIntent::Ddl,
            StatementIntent::Grant,
            StatementIntent::Unknown,
        ] {
            for env in ENVS {
                for read_only in [false, true] {
                    let cmd = banc.execute(read_only, intent);
                    let pour_humain = banc.politique.authorize(&humain(), &cmd, env);
                    let pour_agent = banc.politique.authorize(&agent, &cmd, env);
                    assert!(
                        pour_agent.restrictiveness() >= pour_humain.restrictiveness(),
                        "intention={intent} env={env} lecture_seule={read_only} : \
                         agent={pour_agent:?} humain={pour_humain:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn un_risque_non_borne_exige_une_approbation_meme_pour_un_humain_en_local() {
        let banc = Banc::new();

        for risque in [
            MutationRisk::UnboundedUpdate,
            MutationRisk::UnboundedDelete,
            MutationRisk::Truncate,
            MutationRisk::DropObject,
        ] {
            let cmd = Command::Execute {
                connection: banc.ouverte,
                session: SessionId::new(),
                request: Box::new(
                    ExecRequest::new(QueryLanguage::SQL, "DELETE FROM events")
                        .with_intent(StatementIntent::Write)
                        .with_risk(risque),
                ),
            };
            let decision = banc
                .politique
                .authorize(&humain(), &cmd, Environment::Local);
            assert!(
                decision.requires_approval(),
                "{risque:?} devrait exiger une approbation, obtenu {decision:?}"
            );

            let Decision::RequireApproval { reason, preview } = decision else {
                unreachable!("vérifié juste au-dessus");
            };
            assert!(!reason.is_empty());
            let preview = preview.expect("une opération destructrice se prévisualise");
            assert_eq!(
                preview.connection, "atelier",
                "la connexion doit être nommée"
            );
            assert_eq!(preview.statement, "DELETE FROM events");
        }
    }

    #[test]
    fn un_risque_non_borne_reste_refuse_sur_une_connexion_en_lecture_seule() {
        let banc = Banc::new();
        // Intention déclarée en lecture, risque destructeur : l'incohérence est
        // tranchée du côté prudent, donc le refus s'applique.
        let cmd = Command::Execute {
            connection: banc.lecture_seule,
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::SQL, "TRUNCATE audit")
                    .with_intent(StatementIntent::Read)
                    .with_risk(MutationRisk::Truncate),
            ),
        };
        let decision = banc
            .politique
            .authorize(&humain(), &cmd, Environment::Local);
        assert!(decision.is_denied(), "{decision:?}");
    }

    #[test]
    fn une_connexion_inconnue_ferme_la_porte() {
        let politique = DefaultPolicy::new();
        let inconnue = ConnectionId::new();

        let ecriture = Command::Execute {
            connection: inconnue,
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::SQL, "INSERT INTO t VALUES (1)")
                    .with_intent(StatementIntent::Write),
            ),
        };
        assert!(
            politique
                .authorize(&humain(), &ecriture, Environment::Local)
                .is_denied(),
            "le gate ne peut pas décider de ce qu'il ne voit pas"
        );

        // Une lecture, elle, n'a pas besoin du marquage.
        let lecture = Command::Execute {
            connection: inconnue,
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::SQL, "SELECT 1").with_intent(StatementIntent::Read),
            ),
        };
        assert!(
            politique
                .authorize(&humain(), &lecture, Environment::Local)
                .is_allowed()
        );
    }

    #[test]
    fn le_marquage_de_la_connexion_l_emporte_sur_un_environnement_annonce_trop_permissif() {
        let politique = DefaultPolicy::new();
        let prod = ConnectionConfig::new("caisse", DriverId::postgres())
            .with_environment(Environment::Production);
        politique.register(&prod);

        let cmd = Command::Execute {
            connection: prod.id,
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::SQL, "UPDATE t SET a = 1 WHERE id = 2")
                    .with_intent(StatementIntent::Write),
            ),
        };

        // L'appelant annonce `Local` — par erreur, ou parce qu'il a mal câblé.
        let decision = politique.authorize(&humain(), &cmd, Environment::Local);
        assert!(
            decision.requires_approval(),
            "le marquage production doit l'emporter : {decision:?}"
        );

        let decision = politique.authorize(&agent(), &cmd, Environment::Local);
        assert!(
            decision.is_denied(),
            "un agent reste en lecture seule stricte sur une connexion production : {decision:?}"
        );
    }

    #[test]
    fn oublier_une_connexion_referme_la_porte() {
        let banc = Banc::new();
        let cmd = banc.execute(false, StatementIntent::Write);
        assert!(
            banc.politique
                .authorize(&humain(), &cmd, Environment::Local)
                .is_allowed()
        );

        banc.politique.forget(banc.ouverte);
        assert!(
            banc.politique
                .authorize(&humain(), &cmd, Environment::Local)
                .is_denied()
        );
    }

    #[test]
    fn annuler_reste_possible_partout() {
        // Refuser une annulation ne protège rien et laisse une requête tourner.
        let banc = Banc::new();
        for read_only in [false, true] {
            for env in ENVS {
                for acteur in [humain(), agent()] {
                    let cmd = Command::Cancel {
                        connection: banc.connexion(read_only),
                        statement: StatementHandle::new(),
                    };
                    assert!(
                        banc.politique.authorize(&acteur, &cmd, env).is_allowed(),
                        "annulation refusée : acteur={acteur} env={env} ro={read_only}"
                    );
                }
            }
        }
    }

    #[test]
    fn lire_le_catalogue_local_est_permis_partout_sans_approbation() {
        // Une lecture du cache local : aucun serveur contacté, aucune valeur de
        // ligne. La refuser en production laisserait l'agent deviner des noms ;
        // la soumettre à approbation apprendrait à cliquer. Ce qui en sort est
        // gouverné par le niveau de confidentialité, dans `oxyn-ai`.
        let banc = Banc::new();
        for read_only in [false, true] {
            for env in ENVS {
                for acteur in [humain(), agent()] {
                    let cmd = Command::DescribeCatalog {
                        connection: banc.connexion(read_only),
                        focus: Some("clients".to_owned()),
                    };
                    assert!(!cmd.is_mutating());
                    assert!(!cmd.touches_database());
                    assert!(
                        banc.politique.authorize(&acteur, &cmd, env).is_allowed(),
                        "lecture du catalogue refusée : acteur={acteur} env={env} ro={read_only}"
                    );
                }
            }
        }
    }

    #[test]
    fn un_agent_ne_peut_pas_creer_de_connexion_sans_approbation() {
        // Le canal d'exfiltration : un agent qui déclarerait une connexion vers
        // l'hôte de son choix.
        let politique = DefaultPolicy::new();
        let cfg = ConnectionConfig::new("ailleurs", DriverId::postgres())
            .with_environment(Environment::Local);
        let cmd = Command::CreateConnection {
            config: Box::new(cfg),
        };

        let decision = politique.authorize(&agent(), &cmd, Environment::Local);
        assert!(
            !decision.is_allowed(),
            "un agent ne crée pas une connexion sans accord : {decision:?}"
        );

        // Un humain, lui, n'est pas gêné : la connexion n'est pas encore
        // enregistrée, et la règle du « fermé par défaut » ne vise que ce qui
        // atteint un serveur.
        assert!(
            politique
                .authorize(&humain(), &cmd, Environment::Local)
                .is_allowed()
        );
    }

    #[test]
    fn ecrire_un_document_local_ne_declenche_aucune_approbation() {
        let politique = DefaultPolicy::new();
        let cmd = Command::WriteDocument {
            workspace: crate::ids::WorkspaceId::new(),
            document: crate::ids::DocumentId::new(),
            text: "SELECT 1".into(),
        };
        for acteur in [humain(), agent()] {
            assert!(
                politique
                    .authorize(&acteur, &cmd, Environment::Production)
                    .is_allowed(),
                "un document local n'est pas une écriture en base"
            );
        }
    }

    #[test]
    fn aucun_motif_ne_laisse_fuir_un_identifiant_de_connexion() {
        let banc = Banc::new();
        let identifiants = [banc.ouverte.to_string(), banc.lecture_seule.to_string()];

        for (est_agent, intent, env, read_only, _) in MATRICE {
            let acteur = if *est_agent { agent() } else { humain() };
            let cmd = banc.execute(*read_only, *intent);
            let decision = banc.politique.authorize(&acteur, &cmd, *env);

            let texte = match &decision {
                Decision::Allow => String::new(),
                Decision::Deny { reason } => reason.clone(),
                Decision::RequireApproval { reason, preview } => {
                    let mut t = reason.clone();
                    if let Some(p) = preview {
                        t.push_str(&p.connection);
                        t.push_str(&p.statement);
                    }
                    t
                }
            };
            for id in &identifiants {
                assert!(
                    !texte.contains(id.as_str()),
                    "un identifiant de connexion a fuité dans un motif : {texte}"
                );
            }
        }
    }

    #[test]
    fn un_refus_dit_pourquoi() {
        let banc = Banc::new();
        for (est_agent, intent, env, read_only, attendu) in MATRICE {
            if *attendu != Forme::Deny {
                continue;
            }
            let acteur = if *est_agent { agent() } else { humain() };
            let cmd = banc.execute(*read_only, *intent);
            let Decision::Deny { reason } = banc.politique.authorize(&acteur, &cmd, *env) else {
                panic!("refus attendu");
            };
            assert!(
                reason.len() > 10,
                "motif trop pauvre pour être montré : {reason}"
            );
        }
    }

    #[test]
    fn le_gate_est_utilisable_derriere_un_objet_de_trait() {
        // Il doit pouvoir être partagé entre le thread d'interface, l'exécuteur
        // et les agents.
        let gate: std::sync::Arc<dyn PolicyGate> = std::sync::Arc::new(DefaultPolicy::new());
        assert_eq!(gate.name(), "DefaultPolicy");

        let cmd = Command::Connect {
            connection: ConnectionId::new(),
        };
        assert!(
            gate.authorize(&humain(), &cmd, Environment::Production)
                .is_allowed()
        );
    }

    #[test]
    fn un_agent_ne_deplace_pas_le_contexte_de_session() {
        // Le geste ne lit ni n'écrit de donnée : classé `Read`, il passe pour un
        // humain, y compris sur une connexion en lecture seule où changer de
        // schéma pour lire ailleurs est justement l'usage. Pour un agent c'est
        // un refus, parce que l'effet porte sur les instructions suivantes.
        let banc = Banc::new();
        for read_only in [false, true] {
            let cmd = Command::SetSessionContext {
                connection: banc.connexion(read_only),
                session: SessionId::new(),
                catalog: None,
                namespace: Some("analytics".to_owned()),
            };
            for env in [
                Environment::Local,
                Environment::Development,
                Environment::Staging,
                Environment::Production,
            ] {
                let humaine = banc.politique.authorize(&humain(), &cmd, env);
                assert!(
                    humaine.is_allowed(),
                    "un humain change de contexte : env={env} lecture_seule={read_only} → {humaine:?}"
                );
                let agentive = banc.politique.authorize(&agent(), &cmd, env);
                assert!(
                    !agentive.is_allowed(),
                    "un agent ne le fait jamais : env={env} lecture_seule={read_only}"
                );
            }
        }
    }

    #[test]
    fn seul_l_humain_declare_une_ecriture_reconciliee() {
        // Local, sans connexion visée : l'humain n'a rien à confirmer. L'agent
        // est refusé partout — pas soumis à approbation — parce qu'il affirmerait
        // une vérification du serveur que personne n'a faite.
        let politique = DefaultPolicy::new();
        let cmd = Command::ReconcileHistoryEntry { entry: 1 };
        for env in ENVS {
            let humaine = politique.authorize(&humain(), &cmd, env);
            assert!(humaine.is_allowed(), "env={env} → {humaine:?}");
            let agentive = politique.authorize(&agent(), &cmd, env);
            assert!(
                matches!(agentive, Decision::Deny { .. }),
                "env={env} → {agentive:?}"
            );
        }
    }
}
