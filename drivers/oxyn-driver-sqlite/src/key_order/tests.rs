use super::*;

fn column(name: &str, descending: bool, collation: &str) -> KeyColumn {
    KeyColumn {
        name: Some(name.to_owned()),
        descending,
        collation: collation.to_owned(),
    }
}

fn binary(_: &str) -> Option<String> {
    Some("BINARY".to_owned())
}

#[test]
fn an_index_in_one_direction_is_walkable_either_way_and_a_mixed_one_is_not() {
    let ascending = [column("a", false, "BINARY"), column("b", false, "BINARY")];
    let descending = [column("a", true, "BINARY"), column("b", true, "BINARY")];
    let mixed = [column("a", false, "BINARY"), column("b", true, "BINARY")];
    assert!(KeyIndex::of(&ascending, binary).walkable);
    assert!(KeyIndex::of(&descending, binary).walkable);
    assert!(!KeyIndex::of(&mixed, binary).walkable);
}

#[test]
fn only_a_collation_that_differs_from_the_column_default_is_named() {
    let columns = [column("a", false, "NOCASE"), column("b", false, "BINARY")];
    let key = KeyIndex::of(&columns, binary);
    assert_eq!(key.collations, [("a".to_owned(), "NOCASE".to_owned())]);
    // The spelling of the collation's name is not a difference.
    let same = KeyIndex::of(&[column("a", false, "nocase")], |_| {
        Some("NOCASE".to_owned())
    });
    assert!(same.collations.is_empty());
}

#[test]
fn a_rowid_or_expression_key_column_overrides_nothing() {
    let rowid = KeyColumn {
        name: None,
        descending: false,
        collation: "BINARY".to_owned(),
    };
    assert!(KeyIndex::of(&[rowid], binary).collations.is_empty());
}
