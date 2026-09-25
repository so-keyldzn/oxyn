//! The restored object tab: where browsing stopped, kept in the preferences
//! as `object_location` ([ADR-0013](../../../../../docs/adr/0013-preferences-workspace.md)).
//!
//! Reading it touches no session and no catalog: a restored tab comes back
//! as a place, and nothing is read from a server until the user asks
//! (docs/UX-SPEC.md, « Restauration après un arrêt brutal »). Neither does
//! it check that the object still exists — an object that vanished is the
//! object view's to explain, and its location is never erased for it.

use oxyn_core::ConnectionId;

use crate::backend::Backend;
use crate::ipc::IpcError;
use crate::ipc::location::{ObjectPlace, SavedObjectPlace};

impl Backend {
    /// The object tab saved last, with its connection.
    ///
    /// # Errors
    /// If the stored preferences cannot be read.
    pub async fn read_object_location(&self) -> Result<Option<SavedObjectPlace>, IpcError> {
        let preferences = self.applied_preferences().await?;
        Ok(preferences
            .object_location
            .as_ref()
            .and_then(SavedObjectPlace::of))
    }

    /// Saves the object tab shown on `connection`, or forgets it when the
    /// user closed it (`None`).
    ///
    /// Forgetting clears only this connection's place: closing a tab here
    /// says nothing about the object another connection left open.
    ///
    /// # Errors
    /// An address that is not a relation's, or a failed save.
    pub async fn write_object_location(
        &self,
        connection: ConnectionId,
        place: Option<ObjectPlace>,
    ) -> Result<(), IpcError> {
        let location = place
            .map(|place| place.to_location(connection))
            .transpose()?;
        // A place too long to store forgets like a closed tab: this
        // connection's place only, never another's.
        self.save_preferences(|preferences| match location.flatten() {
            Some(location) => preferences.object_location = Some(location),
            None => {
                if preferences
                    .object_location
                    .as_ref()
                    .is_some_and(|stored| stored.connection == connection)
                {
                    preferences.object_location = None;
                }
            }
        })
        .await
        .map(|_| ())
    }
}
