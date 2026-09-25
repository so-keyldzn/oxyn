//! The restored object tab: where browsing stopped in a window, kept on the
//! window's line of the workspace file
//! ([ADR-0043](../../../../../docs/adr/0043-multi-fenetre.md), « Persistance »).
//! It lived in the preferences as `object_location`
//! ([ADR-0013](../../../../../docs/adr/0013-preferences-workspace.md)): the
//! first window of the first launch after migration 18 takes that value, and
//! the preferences field is no longer written.
//!
//! Reading it touches no session and no catalog: a restored tab comes back
//! as a place, and nothing is read from a server until the user asks
//! (docs/UX-SPEC.md, « Restauration après un arrêt brutal »). Neither does
//! it check that the object still exists — an object that vanished is the
//! object view's to explain, and its location is never erased for it.

use oxyn_core::ConnectionId;

use crate::backend::{Backend, WindowKey};
use crate::ipc::IpcError;
use crate::ipc::location::{ObjectPlace, SavedObjectPlace};

impl Backend {
    /// The object tab this window showed last, with its connection.
    #[must_use]
    pub fn read_object_location(&self, window: WindowKey) -> Option<SavedObjectPlace> {
        self.inner
            .layouts
            .object_location(window)
            .as_ref()
            .and_then(SavedObjectPlace::of)
    }

    /// Saves the object tab this window shows on `connection`, or forgets it
    /// when the user closed it (`None`).
    ///
    /// Forgetting clears only this connection's place: closing a tab here
    /// says nothing about the object another connection left open.
    ///
    /// # Errors
    /// An address that is not a relation's, or a failed save.
    pub async fn write_object_location(
        &self,
        window: WindowKey,
        connection: ConnectionId,
        place: Option<ObjectPlace>,
    ) -> Result<(), IpcError> {
        let location = place
            .map(|place| place.to_location(connection))
            .transpose()?;
        // A place too long to store forgets like a closed tab: this
        // connection's place only, never another's.
        let changed =
            self.inner
                .layouts
                .set_object_location(window, |stored| match location.flatten() {
                    Some(location) => *stored = Some(location),
                    None => {
                        if stored
                            .as_ref()
                            .is_some_and(|stored| stored.connection == connection)
                        {
                            *stored = None;
                        }
                    }
                });
        if !changed {
            return Ok(());
        }
        self.save_layout(window).await
    }
}
