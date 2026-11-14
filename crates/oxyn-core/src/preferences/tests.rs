use super::*;

#[test]
fn each_mode_keeps_its_palette_in_the_readable_payload() {
    let preferences = WorkspacePreferences {
        follow_system_appearance: true,
        light_palette: ColorPalette::Ocean,
        dark_palette: ColorPalette::Plum,
        ..Default::default()
    };
    let json = serde_json::to_string(&preferences).expect("preferences serialize");
    assert!(json.contains(r#""light_palette":"ocean""#));
    assert!(json.contains(r#""dark_palette":"plum""#));
    let restored: WorkspacePreferences = serde_json::from_str(&json).expect("preferences restore");
    assert_eq!(restored, preferences);
    let older: WorkspacePreferences =
        serde_json::from_str(r#"{"appearance":"light"}"#).expect("old preferences restore");
    assert_eq!(older.light_palette, ColorPalette::Oxyn);
    assert_eq!(older.dark_palette, ColorPalette::Oxyn);
    let future: WorkspacePreferences = serde_json::from_str(r#"{"dark_palette":"future_palette"}"#)
        .expect("future palettes do not block startup");
    assert_eq!(future.dark_palette, ColorPalette::Oxyn);
}

#[test]
fn old_empty_payload_uses_defaults_and_invalid_ranges_are_refused() {
    let preferences: WorkspacePreferences = serde_json::from_str("{}").expect("defaults");
    assert_eq!(preferences, WorkspacePreferences::default());
    let mut snapshot = PreferencesSnapshot {
        revision: u64::MAX,
        preferences,
    };
    assert!(snapshot.validate().is_err());
    snapshot.revision = 1;
    snapshot.preferences.inspector_width = 0;
    assert!(snapshot.validate().is_err());
    snapshot.preferences.inspector_width = 280;
    snapshot.preferences.version = 99;
    assert!(snapshot.validate().is_err());
    snapshot.preferences.version = 1;
    snapshot.preferences.null_text = "private marker".into();
    assert!(!format!("{snapshot:?}").contains("private marker"));
}

#[test]
fn a_payload_without_a_location_reads_as_no_location() {
    let preferences: WorkspacePreferences =
        serde_json::from_str(r#"{"version":1,"inspector_width":300}"#).expect("older payload");
    assert_eq!(preferences.object_location, None);
    assert_eq!(preferences.inspector_width, 300);
    assert!(preferences.validate().is_ok());
}

#[test]
fn an_oversized_object_name_costs_the_location_and_nothing_else() {
    let connection = crate::ConnectionId::new();
    let long = "t".repeat(ObjectLocation::MAX_PATH_BYTES + 1);
    assert_eq!(
        ObjectLocation::new(connection, long, ObjectSection::Structure),
        None
    );
    assert_eq!(
        ObjectLocation::new(connection, "", ObjectSection::Data),
        None
    );
    let mut preferences = WorkspacePreferences {
        object_location: ObjectLocation::new(
            connection,
            "t".repeat(ObjectLocation::MAX_PATH_BYTES),
            ObjectSection::Indexes,
        ),
        ..Default::default()
    };
    assert!(
        preferences.validate().is_ok(),
        "a location at the budget is storable"
    );
    let payload = serde_json::to_string(&preferences).expect("payload");
    // The store caps the payload at 4096 bytes: the budget has to leave
    // room for every other preference, escaping included.
    assert!(
        payload.len() <= 4096,
        "payload stays inside the storage cap"
    );
    let read: WorkspacePreferences = serde_json::from_str(&payload).expect("round trip");
    assert_eq!(read, preferences);
    // A hand-written payload is still refused rather than trusted.
    if let Some(location) = preferences.object_location.as_mut() {
        location.path.push('t');
    }
    assert!(preferences.validate().is_err());
    assert!(!format!("{preferences:?}").contains("tttt"));
}

#[test]
fn a_section_survives_the_json_round_trip_under_its_readable_name() {
    let payload = serde_json::to_string(&ObjectSection::IncomingRelations).expect("payload");
    assert_eq!(payload, r#""incoming_relations""#);
    assert_eq!(
        serde_json::from_str::<ObjectSection>(&payload).expect("read back"),
        ObjectSection::IncomingRelations
    );
}

#[test]
fn display_settings_added_later_read_as_defaults_and_stay_bounded() {
    // A payload written before these fields existed keeps its meaning.
    let older: WorkspacePreferences =
        serde_json::from_str(r#"{"version":1,"appearance":"light"}"#).expect("older payload");
    assert!(!older.follow_system_appearance);
    assert_eq!(older.binary_display, BinaryPreference::Hex);
    assert_eq!(older.cell_max_chars, 512);
    assert!(
        !older.semantic_ranking,
        "a payload from before ADR-0056 never starts a download"
    );

    // A rendering a newer Oxyn knows, this one does not: read, not refused.
    let newer: WorkspacePreferences =
        serde_json::from_str(r#"{"version":1,"binary_display":"hexdump_v2"}"#)
            .expect("an unknown rendering does not block startup");
    assert_eq!(newer.binary_display, BinaryPreference::Hex);

    let mut unbounded = WorkspacePreferences {
        cell_max_chars: 0,
        ..Default::default()
    };
    assert!(unbounded.validate().is_err(), "an uncut cell is refused");
    unbounded.cell_max_chars = 16_384;
    assert!(unbounded.validate().is_ok());
    let payload = serde_json::to_string(&unbounded).expect("payload");
    assert!(payload.contains(r#""binary_display":"hex""#), "{payload}");
}

/// An older binary must be able to read what a newer binary wrote.
///
/// That is why this type refuses `deny_unknown_fields`: reading the
/// preferences fails at application **startup**, so a field added by a
/// newer version would prevent a rollback from launching Oxyn. What keeps
/// compatibility is `version`, checked explicitly.
#[test]
fn a_field_from_a_newer_version_is_ignored_not_refused() {
    let payload = r#"{
        "version": 1,
        "appearance": "dark",
        "sidebar_collapsed": true,
        "a_future_setting": {"shape": "unknown"}
    }"#;
    let read_back: WorkspacePreferences =
        serde_json::from_str(payload).expect("an unknown field does not block reading");
    assert_eq!(read_back.appearance, Appearance::Dark);
    assert!(read_back.sidebar_collapsed, "known fields are read");
    assert!(read_back.validate().is_ok());

    // A really different format version is refused, though: the number
    // carries the incompatibility, not the presence of a field.
    let future = r#"{"version": 99}"#;
    let from_the_future: WorkspacePreferences = serde_json::from_str(future).expect("read");
    assert!(from_the_future.validate().is_err());
}
