use super::*;
use crate::{TransferError, TransferSource};
use oxyn_core::*;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::path::Path;

fn source() -> TransferSource {
    let config = ConnectionConfig::new("disposable", DriverId::postgres())
        .with_param("host", "127.0.0.1")
        .with_param("port", "5432")
        .with_param("database", "native fixture")
        .with_param("user", "fixture")
        .with_param("sslmode", "disable");
    TransferSource::new(
        &config,
        SessionId::new(),
        "PostgreSQL".into(),
        "17.12".into(),
        false,
    )
}
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn request(source: &TransferSource) -> NativeTransferRequest {
    NativeTransferRequest {
        connection: source.connection,
        session: source.session,
        owner: WindowId::new(),
        operation: NativeTransferOperation::Export,
        format: NativeTransferFormat::CustomArchive,
        tool: NativeGrantId::new(),
        file: NativeGrantId::new(),
        no_owner: false,
        no_privileges: false,
    }
}
#[test]
fn old_wrong_role_and_malformed_version_output_never_becomes_a_tool() {
    assert!(matches!(
        ToolVersion::parse(NativeToolRole::Export, b"pg_dump (PostgreSQL) 17.4\n"),
        Err(TransferError::Version)
    ));
    assert!(
        ToolVersion::parse(NativeToolRole::Export, b"pg_restore (PostgreSQL) 17.12\n").is_err()
    );
    assert!(
        ToolVersion::parse(
            NativeToolRole::Export,
            b"pg_dump (PostgreSQL) 999999999999999.12\n"
        )
        .is_err()
    );
    assert_eq!(
        ToolVersion::parse(NativeToolRole::Export, b"pg_dump (PostgreSQL) 17.12\n")
            .expect("corrected client")
            .display(),
        "17.12"
    );
    assert!(
        ToolVersion::parse(NativeToolRole::Restore, b"pg_restore (PostgreSQL) 18.6\n").is_err()
    );
}
#[test]
fn all_nonpostgres_engines_and_wire_compatible_variants_have_explicit_unavailability() {
    for engine in [
        "sqlite",
        "duckdb",
        "mysql",
        "clickhouse",
        "mongodb",
        "elasticsearch",
        "redis",
        "turso",
    ] {
        let mut source = source();
        source.driver = engine.parse().expect("driver id");
        let support = source.support();
        assert!(!support.export && !support.restore);
        assert!(support.reason.is_some());
        assert!(support.export_formats.is_empty());
    }
    let mut variant = source();
    variant.product = "Redshift".into();
    assert!(!variant.support().export);
}
#[test]
fn no_hostname_tls_service_ssh_or_remote_auth_fallback_is_offered() {
    for (key, value) in [
        ("host", "localhost"),
        ("host", "192.0.2.1"),
        ("sslmode", "prefer"),
        ("sslmode", "verify-full"),
        ("service", "ambient"),
        ("ssh_host", "somewhere"),
    ] {
        let mut source = source();
        source.params.insert(key.into(), value.into());
        assert!(!source.support().export, "{key}");
    }
    let mut source = source();
    source.params.remove("user");
    assert!(!source.support().export);
}
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_readonly_source_can_export_but_cannot_restore() {
    let mut source = source();
    source.read_only = true;
    let support = source.support();
    assert!(support.export);
    assert!(!support.restore);
    assert!(support.restore_formats.is_empty());
}
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn arguments_are_fixed_and_conninfo_members_cannot_redirect_the_target() {
    let mut source = source();
    source
        .params
        .insert("database".into(), "odd'\\ db host=other".into());
    let target = Target::from_source(&source).expect("literal target");
    let mut request = request(&source);
    let args = target
        .arguments(&request, Path::new("/private/empty"), "fixture")
        .expect("fixed argv");
    assert!(args.iter().any(|arg| arg == "--format=custom"));
    let conn = args
        .iter()
        .find(|arg| arg.starts_with("--dbname="))
        .expect("one connection argument");
    assert!(conn.contains("dbname='odd\\'\\\\ db host=other'"));
    assert!(conn.contains("require_auth='none'"));
    assert!(conn.contains("gssencmode='disable'"));
    assert!(conn.contains("sslmode='disable'"));
    assert!(!conn.contains("password="));
    request.operation = NativeTransferOperation::Restore;
    let args = target
        .arguments(&request, Path::new("/private/empty"), "fixture")
        .expect("restore argv");
    assert!(args.iter().any(|arg| arg == "--single-transaction"));
    assert!(args.iter().all(|arg| !arg.starts_with("--create")
        && !arg.starts_with("--clean")
        && !arg.starts_with("--role")));
    request.format = NativeTransferFormat::Sql;
    assert!(
        target
            .arguments(&request, Path::new("/private/empty"), "fixture")
            .is_err()
    );
    assert!(
        target
            .check_tool(ToolVersion {
                major: 16,
                patch: 16
            })
            .is_err()
    );
}
