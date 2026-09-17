//! L'emprunt d'une connexion : elle ne revient au bassin que remise en état.
//!
//! # Le défaut que ce module ferme
//!
//! Une exécution pose de l'état **par connexion** : `SET search_path` pour le
//! contexte d'une console ([ADR-0019](../../../docs/adr/0019-contexte-de-session.md)),
//! `BEGIN READ ONLY` pour la lecture seule, et l'utilisateur lui-même peut taper
//! `SET standard_conforming_strings = off`. Le curseur défait cet état à la fin
//! du flux. Mais un **futur abandonné** — Échap pendant l'aller-retour du `SET`,
//! un agent qui coupe sa connexion MCP — ne suit aucun chemin d'erreur :
//! `PoolConnection::drop` rend alors la connexion au bassin telle quelle
//! (sqlx-core 0.9.0 : un `ping` qui resynchronise le protocole, rien de plus).
//! L'emprunteur suivant hérite du `search_path` d'une autre console, et son
//! `DELETE FROM orders` vise un autre schéma que celui que l'utilisateur croit.
//!
//! # La règle
//!
//! [`Lease`] est **marqué sale avant** toute instruction qui peut changer l'état
//! de la connexion, et **redevient propre seulement après** une remise au
//! défaut confirmée par le serveur. Détruit sale — par un `?`, un futur
//! abandonné, une tâche interrompue —, il ferme la connexion au lieu de la
//! rendre.
//!
//! Pourquoi une garde et pas `PoolConnection::close_on_drop` posé d'avance :
//! dans sqlx-core 0.9.0 ce drapeau ne se relève pas (`close_on_drop` ne fait que
//! le mettre à `true`, sans accesseur inverse). Poser d'avance fermerait **toute**
//! connexion ayant porté un contexte, y compris celles que le curseur a remises
//! au défaut. La garde ne le pose qu'au dernier moment, dans son `Drop`.

use std::ops::{Deref, DerefMut};

use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgConnection, Postgres};

/// Une connexion empruntée au bassin, qui sait si elle peut y retourner.
#[derive(Debug)]
pub(crate) struct Lease {
    connection: PoolConnection<Postgres>,
    /// L'état de la connexion n'est plus garanti être celui du bassin.
    dirty: bool,
}

impl Lease {
    /// Un emprunt neuf : la connexion est dans l'état où le bassin l'a rendue.
    pub(crate) const fn new(connection: PoolConnection<Postgres>) -> Self {
        Self {
            connection,
            dirty: false,
        }
    }

    /// À appeler **avant** d'envoyer une instruction qui peut changer l'état de
    /// session : à partir d'ici, un abandon ferme la connexion.
    pub(crate) const fn taint(&mut self) {
        self.dirty = true;
    }

    /// À appeler **après** que le serveur a confirmé la remise au défaut.
    pub(crate) const fn restored(&mut self) {
        self.dirty = false;
    }

    /// La connexion ne reviendra pas au bassin, quoi qu'il arrive ensuite.
    ///
    /// Pour une connexion dont le flux a été abandonné en cours : elle peut
    /// garder des octets non lus, et aucune remise au défaut ne s'y fie.
    pub(crate) fn discard(&mut self) {
        self.connection.close_on_drop();
    }
}

impl Deref for Lease {
    type Target = PgConnection;

    fn deref(&self) -> &PgConnection {
        &self.connection
    }
}

impl DerefMut for Lease {
    fn deref_mut(&mut self) -> &mut PgConnection {
        &mut self.connection
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        if self.dirty {
            // Posé ici, juste avant que `PoolConnection` soit détruit à son
            // tour : sqlx la ferme au lieu de la rendre.
            self.connection.close_on_drop();
        }
    }
}
