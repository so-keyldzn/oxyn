//! What an external agent must never get from Oxyn.

use super::*;
use crate::privacy::PrivacyTier;
use oxyn_core::{ExternalAgentConfig, ProviderId};

fn agent(command: &str, args: &[&str]) -> ExternalAgentConfig {
    ExternalAgentConfig::new(
        ProviderId::new("agent").expect("a valid identifier"),
        "Agent",
        command,
    )
    .with_args(args.iter().copied())
}

/// The command and its arguments are never reassembled into a string.
///
/// It is what puts the launch out of reach of shell-style splitting: a program
/// name containing a space, a quote or a semicolon reaches the operating
/// system as is, and takes on no extra meaning there. The protocol's example,
/// on the other hand, starts from a single string that it splits — it is that
/// grammar we do not borrow.
#[test]
fn the_command_and_its_arguments_are_never_glued_back() {
    let hostile = agent(
        "/opt/mes agents/claude code",
        &["--acp", "; rm -rf /", "--flag=a b c"],
    );
    check_launchable(&hostile).expect("the declaration is valid");

    // The property holds because the declaration carries two distinct fields
    // down to the system call: there is no point on the path where a single
    // string would be split again.
    assert_eq!(
        hostile.command, "/opt/mes agents/claude code",
        "the space in the path does not cut the command"
    );
    assert_eq!(
        hostile.args,
        ["--acp", "; rm -rf /", "--flag=a b c"],
        "each argument stays an argument, semicolon included"
    );
}

/// An invalid declaration produces no launch.
///
/// Validation is redone here, and not only at input time: a declaration can
/// come from a state file written by a third party or by a future version.
#[test]
fn an_invalid_declaration_launches_nothing() {
    assert!(check_launchable(&agent("", &[])).is_err(), "empty command");
    assert!(
        check_launchable(&agent("claude\n--evil", &[])).is_err(),
        "a line feed in the command has no legitimate use"
    );
    assert!(
        check_launchable(&agent("claude", &["--ok\u{7}"])).is_err(),
        "neither has a control character in an argument"
    );
}

/// Nothing that touches the machine is granted.
///
/// It is the module's central guarantee. It is written kind by kind rather
/// than as a block: adding a variant to the protocol must not loosen it
/// silently, and a test `match` that enumerates forces coming back here.
#[test]
fn no_system_access_is_granted() {
    for kind in [
        ToolKind::Read,
        ToolKind::Search,
        ToolKind::Edit,
        ToolKind::Delete,
        ToolKind::Move,
        ToolKind::Execute,
        ToolKind::Fetch,
        ToolKind::Other,
    ] {
        assert!(
            permission_for(kind).is_refused(),
            "{kind:?} must never be granted: Oxyn is a database workbench"
        );
    }
}

/// What does not leave the agent is granted, otherwise nothing works.
#[test]
fn what_stays_in_the_agent_is_granted() {
    assert_eq!(permission_for(ToolKind::Think), PermissionVerdict::Granted);
    assert_eq!(
        permission_for(ToolKind::SwitchMode),
        PermissionVerdict::Granted
    );
}

/// Every refusal carries a reason, and it quotes no value.
///
/// The reason goes **to the agent**: without it, the agent rephrases its
/// request indefinitely. And it is displayed, so it must not copy a file path
/// or an identifier coming from the request
/// ([I-03](../../../../CLAUDE.md#i-03)).
#[test]
fn every_refusal_says_why_without_quoting_the_request() {
    for kind in [
        ToolKind::Read,
        ToolKind::Edit,
        ToolKind::Delete,
        ToolKind::Move,
        ToolKind::Execute,
        ToolKind::Fetch,
        ToolKind::Search,
        ToolKind::Other,
    ] {
        let PermissionVerdict::Refused(reason) = permission_for(kind) else {
            panic!("{kind:?} should be refused");
        };
        assert!(
            reason.len() > 20 && reason.ends_with('.'),
            "{kind:?}: a reason reads as a sentence, {reason:?}"
        );
        // The reason is a constant: it cannot carry a value coming from the
        // request. This test holds that by forbidding the formatting marks
        // someone would one day be tempted to slip in.
        assert!(
            !reason.contains('{') && !reason.contains('}'),
            "{kind:?}: a reason is not composed, {reason:?}"
        );
    }
}

/// One option of each kind, as an agent offers them.
fn options() -> Vec<PermissionOption> {
    [
        (PermissionOptionKind::AllowOnce, "allow-once"),
        (PermissionOptionKind::AllowAlways, "allow-always"),
        (PermissionOptionKind::RejectOnce, "reject-once"),
        (PermissionOptionKind::RejectAlways, "reject-always"),
    ]
    .into_iter()
    .map(|(kind, id)| {
        PermissionOption::new(
            agent_client_protocol::schema::v1::PermissionOptionId::new(id),
            id,
            kind,
        )
    })
    .collect()
}

/// Oxyn never allows "always", and rejects "always" when it can.
///
/// The asymmetry is the point: remembering a broad grant is a decision the user
/// did not make; remembering a refusal is not one, since what Oxyn refuses it
/// will refuse every time.
#[test]
fn oxyn_is_cautious_when_allowing_and_decisive_when_refusing() {
    let all_options = options();

    let granted = option_for(&PermissionVerdict::Granted, &all_options).expect("an allow option");
    assert_eq!(
        granted.kind,
        PermissionOptionKind::AllowOnce,
        "never \"allow always\": the user did not decide it"
    );

    let rejected =
        option_for(&PermissionVerdict::Refused("non"), &all_options).expect("a reject option");
    assert_eq!(
        rejected.kind,
        PermissionOptionKind::RejectAlways,
        "what Oxyn refuses, it will always refuse: asking again makes the conversation go round in circles"
    );
}

/// Without the right option, we do not take another one.
///
/// The failure avoided: selecting an allow option because no reject option is
/// offered. `None` makes the answer `Cancelled`, which interrupts — it is the
/// only honest outcome.
#[test]
fn no_suitable_option_is_replaced_by_its_opposite() {
    let grant_only: Vec<PermissionOption> = options()
        .into_iter()
        .filter(|option| {
            matches!(
                option.kind,
                PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways
            )
        })
        .collect();
    assert!(
        option_for(&PermissionVerdict::Refused("non"), &grant_only).is_none(),
        "a refusal is never satisfied by an allow option"
    );

    let refusal_only: Vec<PermissionOption> = options()
        .into_iter()
        .filter(|option| {
            matches!(
                option.kind,
                PermissionOptionKind::RejectOnce | PermissionOptionKind::RejectAlways
            )
        })
        .collect();
    assert!(
        option_for(&PermissionVerdict::Granted, &refusal_only).is_none(),
        "a grant is never satisfied by a reject option"
    );

    assert!(option_for(&PermissionVerdict::Granted, &[]).is_none());
}

/// Failing "reject always", "reject once" will do.
#[test]
fn a_refusal_settles_for_a_one_time_reject_if_that_is_all_there_is() {
    let one_shot: Vec<PermissionOption> = options()
        .into_iter()
        .filter(|option| option.kind != PermissionOptionKind::RejectAlways)
        .collect();
    let chosen = option_for(&PermissionVerdict::Refused("non"), &one_shot).expect("a refusal");
    assert_eq!(chosen.kind, PermissionOptionKind::RejectOnce);
}

/// The tier is checked **before** the process is launched.
///
/// The order is the guarantee, not just the refusal. Starting the agent and
/// then refusing to talk to it would already be too late: merely launching it
/// can be enough to make it contact its service.
///
/// The command deliberately points to a **nonexistent** program. Without the
/// guard, execution reaches `connect_with`, attempts the launch and returns
/// "the external agent did not complete its turn" — checked by sabotage. That
/// the error bears on the **tier** therefore proves no process started.
///
/// Beware of the false sabotage: swapping the guard and `check_launchable`
/// changes nothing, because `check_launchable` launches nothing — it only
/// validates a configuration. The only launcher is `connect_with`, and it is
/// relative to it that the order matters.
#[test]
fn the_local_tier_refuses_before_even_launching_the_process() {
    let missing = agent("/oxyn/this-program-does-not-exist", &["--acp"]);

    // The prompt is composed under a tier that admits it, then presented to
    // `run_turn` under `Local`: it is **defense in depth** being tested here.
    // Since ADR-0027, `AgentPrompt::from_user` would already refuse under
    // `Local` — but `run_turn` must not rely on its caller.
    let launch_request =
        super::prompt::AgentPrompt::from_user(PrivacyTier::Metadata, "which tables exist?")
            .expect("this tier admits an external agent");

    let refusal = futures::executor::block_on(super::turn::run_turn(
        &missing,
        PrivacyTier::Local,
        &launch_request,
        &oxyn_core::CancelToken::new(),
        std::sync::Arc::new(()),
        None,
    ))
    .expect_err("a local connection cannot talk to an external agent");

    let message = refusal.to_string();
    assert!(
        message.contains("local-only"),
        "the refusal must bear on the tier, not on the launch: {message}"
    );
    assert!(
        !message.contains("does-not-exist"),
        "a tier refusal does not quote the command: {message}"
    );
}

/// An already cancelled turn launches no process.
///
/// The token was not passed at all: `start_agent_turn` created one, returned it
/// to the panel, and never passed it to `run_turn`. The stop button therefore
/// changed the display while the subprocess kept talking to a service Oxyn does
/// not know the location of — the mode where cancellation matters most,
/// precisely because the destination is not verifiable
/// ([ADR-0026](../../../../docs/adr/0026-agents-externes-acp.md)).
///
/// This test bears on the case observable without launching anything: an
/// already armed token. It is deliberately backed by a **nonexistent** command
/// — if the cancellation guard disappeared, execution would reach
/// `connect_with` and return an error, not `Cancelled`.
#[test]
fn an_already_cancelled_turn_launches_nothing() {
    let missing = agent("/oxyn/this-program-does-not-exist", &["--acp"]);
    let token = oxyn_core::CancelToken::new();
    token.cancel();

    let launch_request =
        super::prompt::AgentPrompt::from_user(PrivacyTier::Metadata, "which tables exist?")
            .expect("valid prompt");

    let header_end = futures::executor::block_on(super::turn::run_turn(
        &missing,
        PrivacyTier::Metadata,
        &launch_request,
        &token,
        std::sync::Arc::new(()),
        None,
    ))
    .expect("a cancellation is an end of turn, not an error");

    assert_eq!(
        header_end,
        super::turn::TurnEnd::Cancelled,
        "a cancelled turn says it is cancelled, and above all does not launch the program"
    );
}

/// An invalid declaration is refused before the launch too.
#[test]
fn an_invalid_declaration_is_refused_before_the_launch() {
    let empty = agent("", &[]);
    let launch_request = super::prompt::AgentPrompt::from_user(PrivacyTier::Metadata, "hello")
        .expect("valid prompt");

    let error = futures::executor::block_on(super::turn::run_turn(
        &empty,
        PrivacyTier::Metadata,
        &launch_request,
        &oxyn_core::CancelToken::new(),
        std::sync::Arc::new(()),
        None,
    ))
    .expect_err("an empty command does not launch");
    assert!(error.to_string().contains("command"), "{error}");
}
