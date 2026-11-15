use super::*;
use crate::{
    Actor, AgentId, AgentSessionId, Command, ConnectionConfig, DefaultPolicy, Environment,
    PolicyGate,
};
#[test]
fn restore_is_mutating_readonly_refused_and_always_reviewed_even_locally() {
    let policy = DefaultPolicy::new();
    let mut config = ConnectionConfig::new("named destination", DriverId::postgres());
    config.environment = Environment::Local;
    policy.register(&config);
    let restore = Command::RestoreDatabase {
        connection: config.id,
        owner: WindowId::new(),
        transfer: NativeTransferId::new(),
    };
    assert!(restore.is_mutating());
    assert!(restore.touches_database());
    assert!(
        policy
            .authorize(&Actor::Human, &restore, Environment::Local)
            .requires_approval()
    );
    config.read_only = true;
    policy.register(&config);
    assert!(
        policy
            .authorize(&Actor::Human, &restore, Environment::Local)
            .is_denied()
    );
}
#[test]
fn an_agent_cannot_inspect_prepare_run_release_or_reconcile_native_transfers() {
    let actor = Actor::agent(AgentId::new(), AgentSessionId::new());
    let policy = DefaultPolicy::new();
    let config = ConnectionConfig::new("fixture", DriverId::postgres());
    policy.register(&config);
    let owner = WindowId::new();
    let grant = NativeGrantId::new();
    let transfer = NativeTransferId::new();
    for command in [
        Command::ReadNativeTransferSupport {
            connection: config.id,
            session: SessionId::new(),
        },
        Command::PrepareNativeTransfer {
            request: Box::new(NativeTransferRequest {
                connection: config.id,
                session: SessionId::new(),
                owner,
                operation: NativeTransferOperation::Export,
                format: NativeTransferFormat::CustomArchive,
                tool: grant,
                file: NativeGrantId::new(),
                no_owner: false,
                no_privileges: false,
            }),
        },
        Command::DiscardNativeTransfer {
            connection: config.id,
            owner,
            transfer,
        },
        Command::ReadNativeTransfers {
            connection: config.id,
        },
        Command::InspectNativeTool { owner, grant },
        Command::RevokeNativeSelection { owner, grant },
        Command::ExportDatabase {
            connection: config.id,
            owner,
            transfer,
        },
        Command::RestoreDatabase {
            connection: config.id,
            owner,
            transfer,
        },
        Command::ReconcileNativeTransfer {
            connection: config.id,
            transfer,
        },
    ] {
        assert!(
            policy
                .authorize(&actor, &command, Environment::Local)
                .is_denied()
        );
    }
}
