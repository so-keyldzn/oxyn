use oxyn_core::{Command, CommandId, DriverId, ServerProfile, ServerProfileId};

use crate::backend::Backend;
use crate::backend::recovery::is_local_write;

#[tokio::test]
async fn profile_ipc_round_trip_preserves_revision_and_never_serializes_credentials() {
    let backend = Backend::open_temporary().expect("temporary backend");
    let profile = ServerProfile {
        id: ServerProfileId::new(),
        name: "Local development".into(),
        driver: DriverId::postgres(),
        params: [("host".into(), "localhost".into())].into_iter().collect(),
    };
    let saved = backend
        .save_server_profile(CommandId::new(), profile.clone(), None)
        .await
        .expect("saved");
    let json = serde_json::to_value(&saved).expect("serialize boundary");
    assert_eq!(json["revision"], 1);
    assert_eq!(json["profile"]["id"], profile.id.to_string());
    assert!(json["profile"].get("secret_ref").is_none());
    assert!(json["profile"].get("environment").is_none());
    assert!(json["profile"].get("privacy_tier").is_none());
    assert!(json["profile"].get("read_only").is_none());
    let mut edited = profile.clone();
    edited.name = "Edited".into();
    backend
        .save_server_profile(CommandId::new(), edited, Some(1))
        .await
        .expect("update");
    assert!(
        backend
            .save_server_profile(CommandId::new(), profile.clone(), Some(1))
            .await
            .is_err()
    );
    assert_eq!(
        backend
            .list_server_profiles(CommandId::new())
            .await
            .expect("list")[0]
            .revision,
        2
    );
    backend
        .remove_server_profile(CommandId::new(), profile.id, 2)
        .await
        .expect("remove");
    assert!(
        backend
            .list_server_profiles(CommandId::new())
            .await
            .expect("list")
            .is_empty()
    );
}

#[test]
fn shutdown_waits_for_profile_writes_but_not_for_listing() {
    let profile = ServerProfile {
        id: ServerProfileId::new(),
        name: "Group".into(),
        driver: DriverId::postgres(),
        params: Default::default(),
    };
    assert!(is_local_write(&Command::SaveServerProfile {
        profile: Box::new(profile.clone()),
        expected_revision: None
    }));
    assert!(is_local_write(&Command::RemoveServerProfile {
        profile: profile.id,
        expected_revision: 1
    }));
    assert!(is_local_write(&Command::SetServerProfileMembership {
        connection: Default::default(),
        profile: Some(profile.id),
        expected_profile: None
    }));
    assert!(!is_local_write(&Command::ListServerProfiles));
}
