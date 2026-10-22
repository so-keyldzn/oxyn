use oxyn_catalog::{CatalogRef, ServerInfo};
use oxyn_core::Capabilities;

use super::*;
use crate::completion::tests::{app, describe, list, namespace_path, postgres};

const HOSTILE: &str = "users\"; DROP TABLE audit; --";

fn bare(text: &str) -> CompletionNamePart {
    CompletionNamePart {
        text: text.to_owned(),
        quoted: false,
    }
}

fn quoted(text: &str) -> CompletionNamePart {
    CompletionNamePart {
        text: text.to_owned(),
        quoted: true,
    }
}

fn session() -> ServerInfo {
    ServerInfo::new("PostgreSQL", "17.0", Capabilities::SCHEMAS)
}

/// A PostgreSQL connection whose server level and resolution path were read:
/// `public` and `analytics` are the user's, `pg_catalog` the server's.
fn read_server(path: &[&str]) -> CatalogCache {
    let mut cache = postgres(false);
    cache.set_server_info(session());
    cache.set_resolution_path(path.iter().map(|entry| (*entry).to_owned()).collect());
    let parent = CatalogPath::for_catalog("app").expect("legal name");
    let mut spaces: Vec<_> = cache.namespaces(Some("app")).cloned().collect();
    spaces.push(oxyn_catalog::NamespaceRef::new(parent, "analytics").expect("legal name"));
    cache.set_namespaces(Some("app"), spaces);
    cache
}

/// The plan under the rule most dialects share. The tests of PostgreSQL's own
/// call [`super::next_read`] with `Lower`.
fn next_read(
    cache: &CatalogCache,
    names: &[Vec<CompletionNamePart>],
    tried: &HashSet<Read>,
    spent: Spent,
) -> Option<Read> {
    super::next_read(cache, CompletionFolding::Insensitive, names, tried, spent)
}

fn next(cache: &CatalogCache, names: &[Vec<CompletionNamePart>]) -> Option<Read> {
    next_read(cache, names, &HashSet::new(), Spent::default())
}

fn next_in_postgres(cache: &CatalogCache, names: &[Vec<CompletionNamePart>]) -> Option<Read> {
    super::next_read(
        cache,
        CompletionFolding::Lower,
        names,
        &HashSet::new(),
        Spent::default(),
    )
}

fn namespace(name: &str) -> Read {
    Read::Scope(CatalogScope::Namespace(app(name)))
}

fn relation(namespace: &str, name: &str) -> Read {
    Read::Scope(CatalogScope::Relation(
        app(namespace).with_relation(name).expect("legal name"),
    ))
}

#[test]
fn a_cache_that_knows_nothing_reads_the_server_first_then_the_path() {
    let mut cache = CatalogCache::new();
    assert_eq!(next(&cache, &[]), Some(Read::Scope(CatalogScope::Server)));

    // The server level read — its identity and its first level — the path is next.
    cache.set_server_info(session());
    cache.set_catalogs(vec![CatalogRef::new("app").expect("legal name")]);
    assert_eq!(next(&cache, &[]), Some(Read::ResolutionPath));

    cache.set_resolution_path(vec!["public".into()]);
    assert_ne!(next(&cache, &[]), Some(Read::ResolutionPath));
}

#[test]
fn a_read_that_failed_is_not_asked_twice_in_a_pass() {
    let cache = CatalogCache::new();
    let mut tried = HashSet::new();
    let server = next(&cache, &[]).expect("the server is missing");
    tried.insert(server);
    assert_eq!(
        next_read(&cache, &[], &tried, Spent::default()),
        Some(Read::ResolutionPath),
        "the path does not wait for a server that refused"
    );
    tried.insert(Read::ResolutionPath);
    assert_eq!(next_read(&cache, &[], &tried, Spent::default()), None);
}

#[test]
fn only_the_namespaces_of_the_path_are_listed() {
    let mut cache = read_server(&["public"]);
    // The database level was read; `analytics` is the user's, and off the path.
    assert_eq!(next(&cache, &[]), Some(namespace("public")));

    list(&mut cache, &app("public"), &["users"]);
    assert_eq!(
        next(&cache, &[]),
        None,
        "off the path, not named: nothing to read for it"
    );
}

#[test]
fn a_namespace_the_statement_qualifies_with_is_listed_whatever_the_path() {
    let mut cache = read_server(&["public"]);
    list(&mut cache, &app("public"), &["users"]);

    assert_eq!(
        next(&cache, &[vec![bare("analytics"), bare("ev")]]),
        Some(namespace("analytics"))
    );
    // The tree folds the system namespaces; typing their name asks for them.
    assert_eq!(
        next(&cache, &[vec![bare("PG_CATALOG"), bare("")]]),
        Some(namespace("pg_catalog")),
        "unquoted, the case is the server's to fold"
    );
    assert_eq!(
        next(&cache, &[vec![quoted("PG_CATALOG"), bare("")]]),
        None,
        "quoted, it is exactly that name, which does not exist"
    );
}

#[test]
fn a_name_of_one_part_qualifies_nothing() {
    let mut cache = read_server(&["public"]);
    list(&mut cache, &app("public"), &["users"]);
    assert_eq!(
        next(&cache, &[vec![bare("analytics")]]),
        None,
        "a relation called like a schema, not the schema"
    );
}

#[test]
fn the_relations_the_statement_names_are_described_once_their_namespace_is_listed() {
    let mut cache = read_server(&["public"]);
    let names = [vec![bare("users")], vec![bare("orders")]];
    assert_eq!(next(&cache, &names), Some(namespace("public")));

    list(&mut cache, &app("public"), &["users", "orders", "audit"]);
    assert_eq!(next(&cache, &names), Some(relation("public", "users")));

    describe(&mut cache, &app("public"), "users", &[("id", "int8")]);
    assert_eq!(next(&cache, &names), Some(relation("public", "orders")));

    describe(&mut cache, &app("public"), "orders", &[("id", "int8")]);
    assert_eq!(next(&cache, &names), None, "`audit` was not named");
}

#[test]
fn a_name_the_cache_does_not_hold_costs_no_read() {
    let mut cache = read_server(&["public"]);
    list(&mut cache, &app("public"), &["users"]);
    for chain in [
        vec![bare("nothing")],
        vec![bare("public"), bare("nothing")],
        vec![bare("a"), bare("b"), bare("c")],
        vec![],
        vec![bare(HOSTILE)],
    ] {
        assert_eq!(
            next(&cache, std::slice::from_ref(&chain)),
            None,
            "{chain:?}"
        );
    }
}

#[test]
fn a_hostile_name_is_compared_never_composed_into_a_read() {
    let mut cache = read_server(&["public"]);
    list(&mut cache, &app("public"), &["users", HOSTILE]);

    // The name the cache holds is described through the cache's own path…
    assert_eq!(
        next(&cache, &[vec![quoted(HOSTILE)]]),
        Some(relation("public", HOSTILE))
    );
    // …and a name that only resembles it reaches nothing.
    assert_eq!(
        next(&cache, &[vec![quoted("users\"; DROP TABLE audit")]]),
        None
    );
}

#[test]
fn a_quoted_name_is_exact_and_a_bare_one_is_not_case_sensitive() {
    let mut cache = read_server(&["public"]);
    list(&mut cache, &app("public"), &["Users"]);

    assert_eq!(
        next(&cache, &[vec![bare("USERS")]]),
        Some(relation("public", "Users"))
    );
    assert_eq!(next(&cache, &[vec![quoted("users")]]), None);
    assert_eq!(
        next(&cache, &[vec![quoted("Users")]]),
        Some(relation("public", "Users"))
    );
}

#[test]
fn an_unqualified_name_resolves_by_the_path_first_match_first() {
    let mut cache = read_server(&["analytics", "public"]);
    let names = [vec![bare("events")]];
    // Both namespaces first, in the tree's order: a name found in one could be
    // shadowed by the other, which has not been read.
    assert_eq!(next(&cache, &names), Some(namespace("public")));
    list(&mut cache, &app("public"), &["events"]);
    assert_eq!(
        next(&cache, &names),
        Some(namespace("analytics")),
        "`public` holds `events`, but comes second on the path"
    );
    list(&mut cache, &app("analytics"), &["events"]);

    assert_eq!(
        next(&cache, &names),
        Some(relation("analytics", "events")),
        "the first of the path hides the second"
    );
}

#[test]
fn a_qualified_name_ignores_the_path() {
    let mut cache = read_server(&["analytics"]);
    list(&mut cache, &app("analytics"), &["events"]);
    list(&mut cache, &app("public"), &["events"]);
    assert_eq!(
        next(&cache, &[vec![bare("public"), bare("events")]]),
        Some(relation("public", "events"))
    );
}

#[test]
fn an_earlier_namespace_that_cannot_be_read_leaves_the_name_unresolved() {
    let mut cache = read_server(&["analytics", "public"]);
    list(&mut cache, &app("public"), &["events"]);
    let names = [vec![bare("events")]];
    let mut tried = HashSet::new();
    tried.insert(namespace("analytics"));

    // `analytics` refused to be listed: `events` may be there, and shadow.
    assert_eq!(
        next_read(&cache, &names, &tried, Spent::default()),
        None,
        "a guess would describe the wrong relation"
    );
}

#[test]
fn an_unknown_path_resolves_no_bare_name() {
    let mut cache = postgres(false);
    cache.set_server_info(session());
    list(&mut cache, &app("public"), &["users"]);
    let mut tried = HashSet::new();
    tried.insert(Read::ResolutionPath);

    assert_eq!(
        next_read(&cache, &[vec![bare("users")]], &tried, Spent::default()),
        None,
        "`public` is never assumed"
    );
    assert_eq!(
        next_read(
            &cache,
            &[vec![bare("public"), bare("users")]],
            &tried,
            Spent::default()
        ),
        Some(relation("public", "users")),
        "a qualified name needs no path"
    );
}

#[test]
fn a_source_without_namespaces_describes_its_relations_by_name() {
    let mut cache = CatalogCache::new();
    cache.set_server_info(ServerInfo::new("SQLite", "3.50", Capabilities::empty()));
    cache.set_resolution_path(Vec::new());
    // Before the relations: a fresh list of catalogs is authoritative on its
    // level, and the root is one.
    cache.set_catalogs(Vec::new());
    list(&mut cache, &CatalogPath::empty(), &["logs"]);

    assert_eq!(
        next(&cache, &[vec![bare("logs")]]),
        Some(Read::Scope(CatalogScope::Relation(
            CatalogPath::empty()
                .with_relation("logs")
                .expect("legal name")
        )))
    );
}

#[test]
fn what_is_fresh_is_not_read_again_and_what_a_ddl_invalidated_is() {
    let mut cache = read_server(&["public"]);
    list(&mut cache, &app("public"), &["users"]);
    describe(&mut cache, &app("public"), "users", &[("id", "int8")]);
    let names = [vec![bare("users")]];
    assert_eq!(next(&cache, &names), None);

    cache.invalidate_all();
    assert_eq!(
        next(&cache, &names),
        Some(Read::Scope(CatalogScope::Server)),
        "a DDL: from the server again, level by level"
    );
}

#[test]
fn the_bounds_stop_listings_and_descriptions_separately() {
    let mut cache = read_server(&["public", "analytics"]);
    list(&mut cache, &app("public"), &["users"]);
    let names = [vec![bare("users")]];

    let listings_spent = Spent {
        listings: MAX_LISTINGS,
        descriptions: 0,
    };
    assert_eq!(
        next_read(&cache, &names, &HashSet::new(), listings_spent),
        Some(relation("public", "users")),
        "no more listings; the description goes on"
    );

    let descriptions_spent = Spent {
        listings: 0,
        descriptions: MAX_DESCRIPTIONS,
    };
    assert_eq!(
        next_read(&cache, &names, &HashSet::new(), descriptions_spent),
        Some(namespace("analytics")),
        "no more descriptions; the listings go on"
    );

    let both = Spent {
        listings: MAX_LISTINGS,
        descriptions: MAX_DESCRIPTIONS,
    };
    assert_eq!(next_read(&cache, &names, &HashSet::new(), both), None);
}

#[test]
fn a_pass_over_a_schema_per_tenant_stops_at_the_listing_bound() {
    let mut cache = postgres(false);
    cache.set_server_info(session());
    cache.set_resolution_path(vec!["public".into()]);
    let parent = CatalogPath::for_catalog("app").expect("legal name");
    let tenants: Vec<_> = (0..500)
        .map(|n| {
            oxyn_catalog::NamespaceRef::new(parent.clone(), format!("tenant_{n}"))
                .expect("legal name")
        })
        .collect();
    cache.set_namespaces(Some("app"), tenants);
    // Qualified with every tenant at once: the worst a statement can ask.
    let names: Vec<_> = (0..500)
        .map(|n| vec![bare(&format!("tenant_{n}")), bare("t")])
        .collect();

    let mut tried = HashSet::new();
    let mut spent = Spent::default();
    let mut reads = 0;
    while let Some(read) = next_read(&cache, &names, &tried, spent) {
        if matches!(read, Read::Scope(CatalogScope::Namespace(_))) {
            spent.listings += 1;
        }
        tried.insert(read);
        reads += 1;
        assert!(reads < 1_000, "the pass does not end");
    }
    let listed = tried
        .iter()
        .filter(|read| matches!(read, Read::Scope(CatalogScope::Namespace(_))))
        .count();
    assert_eq!(listed, MAX_LISTINGS);
}

#[test]
fn a_namespace_path_is_built_from_the_cache_not_from_the_name() {
    // The path of the read is the cache's, so a part the cache would refuse
    // never reaches `CatalogPath`.
    let mut cache = read_server(&["public"]);
    list(&mut cache, &app("public"), &["users"]);
    let read = next(&cache, &[vec![bare("public"), bare("users")]]);
    assert_eq!(
        read,
        Some(Read::Scope(CatalogScope::Relation(
            namespace_path(Some("app"), "public")
                .with_relation("users")
                .expect("legal name")
        )))
    );
}

#[test]
fn postgres_folds_a_bare_name_to_lowercase_and_reaches_only_that_one() {
    let mut cache = read_server(&["public"]);
    list(&mut cache, &app("public"), &["Users", "users"]);

    // Both spellings fold to `users`; `"Users"` is reached by quoting only.
    for typed in [bare("users"), bare("USERS"), bare("Users")] {
        assert_eq!(
            next_in_postgres(&cache, &[vec![typed.clone()]]),
            Some(relation("public", "users")),
            "{typed:?}"
        );
    }
    assert_eq!(
        next_in_postgres(&cache, &[vec![quoted("Users")]]),
        Some(relation("public", "Users"))
    );

    // With only `"Users"`, a bare `users` names nothing the server would find.
    let mut only = read_server(&["public"]);
    list(&mut only, &app("public"), &["Users"]);
    assert_eq!(next_in_postgres(&only, &[vec![bare("users")]]), None);
}

#[test]
fn a_dialect_that_ignores_case_prefers_the_name_as_typed() {
    let mut cache = read_server(&["public"]);
    list(&mut cache, &app("public"), &["users", "Users"]);

    assert_eq!(
        next(&cache, &[vec![bare("Users")]]),
        Some(relation("public", "Users")),
        "written as typed"
    );
    assert_eq!(
        next(&cache, &[vec![bare("users")]]),
        Some(relation("public", "users"))
    );
    // Neither is exact: the first of the cache.
    assert_eq!(
        next(&cache, &[vec![bare("USERS")]]),
        Some(relation("public", "users"))
    );
}

#[test]
fn the_system_namespace_the_server_searches_first_is_listed_before_a_name_is_resolved() {
    // PostgreSQL looks in `pg_catalog` before `public`: a `pg_class` of `public`
    // is not what an unqualified `pg_class` reaches.
    let mut cache = read_server(&["pg_catalog", "public"]);
    list(&mut cache, &app("public"), &["pg_class", "users"]);
    let names = [vec![bare("pg_class")]];

    assert_eq!(
        next_in_postgres(&cache, &names),
        Some(namespace("pg_catalog")),
        "a system namespace on the path is listed all the same"
    );
    list(&mut cache, &app("pg_catalog"), &["pg_class", "pg_type"]);
    assert_eq!(
        next_in_postgres(&cache, &names),
        Some(relation("pg_catalog", "pg_class")),
        "the system one shadows the user's"
    );
    assert_eq!(
        next_in_postgres(&cache, &[vec![bare("users")]]),
        Some(relation("public", "users")),
        "a name `pg_catalog` does not hold resolves in the next"
    );
}
