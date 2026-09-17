//! La table `external_agents` : les agents externes déclarés.
//!
//! **Par machine, pas par workspace**, pour la raison d'`ai_providers` : un
//! agent installé sur la machine sert tous les workspaces
//! ([ADR-0026](../../../docs/adr/0026-agents-externes-acp.md)).
//!
//! **Aucun secret n'est écrit ici, et il n'y a pas de colonne pour en écrire
//! un.** Un agent externe porte sa propre authentification ; Oxyn n'en détient
//! aucune. C'est la différence qui fonde ce mode : la seule façon certaine de ne
//! pas divulguer une clé est de ne pas l'avoir ([I-03](../../../CLAUDE.md#i-03)).
//!
//! `env` n'est pas un endroit où ranger un jeton non plus — ce qui est là part
//! dans l'environnement d'un processus, visible de la table des processus sur
//! certains systèmes. Le domaine le dit dans la documentation du champ ; ce
//! module n'a aucun moyen de le vérifier, et n'en invente pas.
//!
//! **Aucune portée n'est persistée**, et cette fois ce n'est pas parce qu'elle
//! serait périmée comme pour un fournisseur : la portée d'un agent externe est
//! **inconnaissable**. Il n'y a rien à écrire.
//!
//! Toutes les méthodes peuvent bloquer : elles ne s'appellent jamais depuis le
//! thread d'interface ([I-05](../../../CLAUDE.md#i-05)).

use chrono::Utc;
use oxyn_core::{ExternalAgentConfig, ProviderId};
use rusqlite::{Row, params};

use crate::error::{Result, StoreError};
use crate::store::Store;

/// Accès typé à la table `external_agents`.
#[derive(Debug)]
pub struct ExternalAgents<'a> {
    store: &'a Store,
}

impl<'a> ExternalAgents<'a> {
    /// Rattache l'accesseur à son `Store`.
    pub(crate) fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Les agents déclarés, par nom.
    ///
    /// L'ordre est celui du nom donné par l'utilisateur, départagé par
    /// l'identifiant : une liste de réglages qui se réordonne d'une ouverture à
    /// l'autre se lit comme un défaut.
    ///
    /// Une ligne que ce binaire ne sait pas relire est **écartée** avec un
    /// `warn`, pas propagée en erreur — même parti que pour les fournisseurs :
    /// un agent qu'on ne saurait pas lancer ne doit pas être proposé, et une
    /// seule ligne étrange ne doit pas rendre l'écran inutilisable.
    ///
    /// # Erreurs
    /// [`StoreError::Sqlite`] si la lecture échoue.
    pub fn list(&self) -> Result<Vec<ExternalAgentConfig>> {
        self.store.with_connection(|conn| {
            let mut requete = conn.prepare(
                "SELECT id, label, command, args, env, created_at, updated_at
                 FROM external_agents ORDER BY label, id",
            )?;
            let lignes = requete.query_map([], depuis_ligne)?;
            let mut agents = Vec::new();
            for ligne in lignes {
                match ligne? {
                    Ok(Some(agent)) => agents.push(agent),
                    Ok(None) => {}
                    Err(erreur) => return Err(erreur),
                }
            }
            Ok(agents)
        })
    }

    /// Écrit ou remplace une déclaration.
    ///
    /// Valide **avant** le disque : c'est le dernier point où une commande
    /// porteuse d'un caractère de contrôle peut être refusée avec un message.
    ///
    /// # Erreurs
    /// [`StoreError::Corrupted`] si la déclaration est invalide — le message
    /// nomme la raison, jamais la valeur ; [`StoreError::Sqlite`] si l'écriture
    /// échoue.
    pub fn save(&self, agent: &ExternalAgentConfig) -> Result<()> {
        agent.validate().map_err(|erreur| StoreError::Corrupted {
            field: "external_agents",
            detail: erreur.to_string(),
        })?;
        let args = serde_json::to_string(&agent.args).map_err(|erreur| StoreError::Corrupted {
            field: "external_agents.args",
            detail: erreur.to_string(),
        })?;
        let env = serde_json::to_string(&agent.env).map_err(|erreur| StoreError::Corrupted {
            field: "external_agents.env",
            detail: erreur.to_string(),
        })?;
        let maintenant = Utc::now();
        self.store.with_connection(|conn| {
            conn.execute(
                "INSERT INTO external_agents
                     (id, label, command, args, env, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(id) DO UPDATE SET
                     label      = excluded.label,
                     command    = excluded.command,
                     args       = excluded.args,
                     env        = excluded.env,
                     updated_at = excluded.updated_at",
                params![
                    agent.id.as_str(),
                    agent.label,
                    agent.command,
                    args,
                    env,
                    agent.created_at,
                    maintenant,
                ],
            )?;
            Ok(())
        })
    }

    /// Retire une déclaration. Rend `true` si une ligne a disparu.
    ///
    /// N'efface rien d'autre. Il n'y a pas de secret à révoquer ailleurs : c'est
    /// le propre de ce mode.
    ///
    /// # Erreurs
    /// [`StoreError::Sqlite`] si la suppression échoue.
    pub fn remove(&self, id: &ProviderId) -> Result<bool> {
        self.store.with_connection(|conn| {
            let effacees = conn.execute(
                "DELETE FROM external_agents WHERE id = ?1",
                params![id.as_str()],
            )?;
            Ok(effacees > 0)
        })
    }
}

/// Reconstruit une déclaration à partir d'une ligne.
///
/// Rend `Ok(None)` pour une ligne que le domaine ne sait pas relire : la
/// distinction avec `Err` propage une vraie panne SQLite tout en écartant une
/// ligne écrite par une version ultérieure. Aucun message ne recopie la valeur
/// fautive (I-03).
fn depuis_ligne(row: &Row<'_>) -> rusqlite::Result<Result<Option<ExternalAgentConfig>>> {
    let brut: String = row.get(0)?;
    let label: String = row.get(1)?;
    let command: String = row.get(2)?;
    let args: String = row.get(3)?;
    let env: String = row.get(4)?;
    let created_at = row.get(5)?;
    let updated_at = row.get(6)?;

    let Ok(id) = ProviderId::new(brut) else {
        tracing::warn!("external agent row skipped: unreadable identifier");
        return Ok(Ok(None));
    };
    let (Ok(args), Ok(env)) = (
        serde_json::from_str::<Vec<String>>(&args),
        serde_json::from_str::<Vec<(String, String)>>(&env),
    ) else {
        tracing::warn!(
            agent = %id.as_str(),
            "external agent row skipped: arguments or environment are unreadable"
        );
        return Ok(Ok(None));
    };

    let agent = ExternalAgentConfig {
        id,
        label,
        command,
        args,
        env,
        created_at,
        updated_at,
    };
    // Une ligne qui ne passerait plus la validation du domaine — écrite par une
    // version dont les bornes différaient — est écartée plutôt que rendue : la
    // proposer ferait échouer son lancement plus tard, loin d'ici.
    if let Err(erreur) = agent.validate() {
        tracing::warn!(
            agent = %agent.id.as_str(),
            reason = %erreur,
            "external agent row skipped: declaration is no longer valid"
        );
        return Ok(Ok(None));
    }
    Ok(Ok(Some(agent)))
}

#[cfg(test)]
mod tests;
