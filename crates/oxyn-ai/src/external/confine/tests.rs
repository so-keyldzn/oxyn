use super::*;
use agent_client_protocol::schema::v1::CurrentModeUpdate;

const CLAUDE: &str = "@agentclientprotocol/claude-agent-acp@0.78.0";
const CODEX: &str = "@agentclientprotocol/codex-acp@1.12.0";
const URL: &str = "http://127.0.0.1:4711/mcp";

fn declared(args: &[&str]) -> ExternalAgentConfig {
    let mut agent = ExternalAgentConfig::new(
        oxyn_core::ProviderId::new("agent").expect("a valid identifier"),
        "Agent",
        "npx",
    );
    agent.args = args.iter().map(|&arg| arg.to_owned()).collect();
    agent
}

fn none() -> Result<Vec<Layer>, Unconfinable> {
    Ok(Vec::new())
}

fn confined(
    agent: &ExternalAgentConfig,
    tool_url: Option<&str>,
    user: Option<&str>,
) -> Result<Confinement, Unconfinable> {
    confinement_for(agent, tool_url, || {
        Ok(user.map(Layer::user).into_iter().collect())
    })
    .map(|found| found.expect("a known adapter is confined"))
}

fn codex_config(confinement: &Confinement) -> Value {
    confinement
        .env
        .iter()
        .find(|(name, _)| *name == "CODEX_CONFIG")
        .map(|(_, value)| serde_json::from_str::<Value>(value).expect("valid JSON"))
        .expect("the configuration is set")
}

#[test]
fn a_mode_left_after_arming_is_seen_once_and_kept() {
    let watch = ModeWatch::default();
    let to = |id: &str| SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new(id.to_owned()));
    // Before arming, the starting mode is announced: not a departure.
    assert!(!watch.observe("default", &to("auto")));
    assert!(!watch.left());
    watch.arm();
    assert!(!watch.observe("default", &to("default")));
    assert!(
        watch.observe("default", &to("bypassPermissions")),
        "the departure"
    );
    assert!(watch.left());
    // Said once, and coming back does not undo it.
    assert!(!watch.observe("default", &to("default")));
    assert!(watch.left());
}

#[test]
fn claude_agent_keeps_no_tool_of_its_own() {
    let confinement = confined(&declared(&["-y", CLAUDE]), Some(URL), None).expect("confined");
    let options = &confinement
        .session_meta
        .clone()
        .expect("options are passed")["claudeCode"]["options"];
    assert_eq!(options["tools"], json!([]));
    assert_eq!(options["allowedTools"], json!(["mcp__oxyn"]));
    assert_eq!(options["strictMcpConfig"], json!(true));
    assert_eq!(options["settingSources"], json!([]));
    assert_eq!(options["allowDangerouslySkipPermissions"], json!(false));
    assert_eq!(confinement.mode, "default");
    assert!(!confinement.tools_in_env, "declared over ACP");
}

/// Every tool of the registry is reachable by both confined agents, and
/// neither confinement names a tool: a tool added to the registry is
/// covered without touching this file. Named one by one, the forgotten
/// tool would be refused in silence — the agent would just not use it.
#[test]
fn every_registry_tool_passes_both_confinements_without_being_named() {
    let tools = crate::tools::ToolRegistry::builtin().names();
    assert!(tools.contains(&crate::tools::DESCRIBE_SCHEMA));

    let claude = confined(&declared(&["-y", CLAUDE]), Some(URL), None).expect("confined");
    let rules: Vec<String> = claude.session_meta.clone().expect("options are passed")["claudeCode"]
        ["options"]["allowedTools"]
        .as_array()
        .expect("a list of rules")
        .iter()
        .filter_map(|rule| rule.as_str().map(str::to_owned))
        .collect();
    for tool in &tools {
        let name = mcp::claude_tool_name(tool);
        assert!(
            rules
                .iter()
                .any(|rule| *rule == name || name.starts_with(&format!("{rule}__"))),
            "{name} is not allowed by {rules:?}"
        );
        assert!(
            rules.iter().all(|rule| !rule.ends_with(tool)),
            "the rule names the tool {tool}: {rules:?}"
        );
    }

    let codex = confined(&declared(&["-y", CODEX]), Some(URL), None).expect("confined");
    let server = &codex_config(&codex)["mcp_servers"][mcp::SERVER_NAME];
    assert_eq!(server["default_tools_approval_mode"], json!("approve"));
    for per_tool in ["enabled_tools", "disabled_tools", "tools"] {
        assert_eq!(
            server[per_tool],
            Value::Null,
            "Codex approves the server as a whole, not a list of tools"
        );
    }
}

#[test]
fn codex_runs_without_shell_nor_web_and_reaches_oxyns_tools_only_by_its_config() {
    let agent = declared(&["-y", CODEX]);
    let confinement = confined(&agent, Some(URL), None).expect("confined");
    let config = codex_config(&confinement);
    assert_eq!(config["features"]["shell_tool"], json!(false));
    assert_eq!(config["features"]["unified_exec"], json!(false));
    assert_eq!(config["features"]["hooks"], json!(false));
    assert_eq!(config["web_search"], json!("disabled"));
    assert!(
        confinement
            .env
            .contains(&("INITIAL_AGENT_MODE", "read-only".to_owned()))
    );
    assert_eq!(confinement.mode, "read-only");
    let server = &config["mcp_servers"]["oxyn"];
    assert_eq!(server["url"], json!(URL));
    assert_eq!(server["bearer_token_env_var"], json!(TOOL_TOKEN_VAR));
    assert!(confinement.tools_in_env);

    let without = confined(&agent, None, None).expect("confined without tools too");
    assert_eq!(codex_config(&without)["mcp_servers"], Value::Null);
    assert!(!without.tools_in_env);
}

#[test]
fn the_users_own_codex_servers_and_plugins_are_turned_off_by_name_only() {
    let agent = declared(&["-y", CODEX]);
    let user = r#"
            model = "whatever"
            [mcp_servers.supabase]
            command = "npx"
            env = { SUPABASE_ACCESS_TOKEN = "sbp_SECRET-TOKEN" }
            [plugins."computer-use@openai-bundled"]
            enabled = true
        "#;
    let confinement = confined(&agent, Some(URL), Some(user)).expect("confined");
    let config = codex_config(&confinement);
    assert_eq!(
        config["mcp_servers"]["supabase"],
        json!({ "enabled": false })
    );
    assert_eq!(
        config["plugins"]["computer-use@openai-bundled"],
        json!({ "enabled": false })
    );
    // Names only: nothing of the user's values travels (I-03).
    assert!(
        !confinement
            .env
            .iter()
            .any(|(_, value)| value.contains("SECRET-TOKEN"))
    );
    // And the `Debug` shows no value at all.
    assert!(!format!("{confinement:?}").contains("features"));
}

#[test]
fn a_codex_configuration_that_cannot_be_read_whole_stops_the_launch() {
    let agent = declared(&["-y", CODEX]);
    assert_eq!(
        confined(&agent, None, Some("[[not toml")),
        Err(Unconfinable::UnreadableConfig)
    );
    let crowded: String = (0..=codex_layers::MAX_ENTRIES)
        .map(|index| format!("[mcp_servers.s{index}]\ncommand = \"x\"\n"))
        .collect();
    assert_eq!(
        confined(&agent, None, Some(&crowded)),
        Err(Unconfinable::TooManyEntries)
    );
    assert_eq!(
        confinement_for(&agent, None, || Err(Unconfinable::UnreadableConfig)),
        Err(Unconfinable::UnreadableConfig)
    );
}

#[test]
fn a_codex_configuration_nested_past_reason_is_refused_not_a_stack_overflow() {
    // `toml` 0.9 stops at 80 levels unless its `unbounded` feature is on;
    // this fails, by aborting, the day a dependency turns it on.
    let deep = format!("a = {}", "[".repeat(200_000));
    assert_eq!(
        confined(&declared(&["-y", CODEX]), None, Some(&deep)),
        Err(Unconfinable::UnreadableConfig)
    );
}

#[cfg(unix)]
#[test]
fn a_special_file_in_place_of_the_codex_configuration_is_refused_before_opening() {
    // A socket, because the standard library makes one: a FIFO — whose
    // opening would block forever — takes a spawn or `mknodat`, which
    // rustix lacks on macOS. Both are refused by the same `stat`.
    let home = std::env::temp_dir().join(format!("oxyn-codex-sock-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("scratch CODEX_HOME");
    let _socket = std::os::unix::net::UnixListener::bind(home.join("config.toml"))
        .expect("a socket where the configuration goes");
    let mut agent = declared(&["-y", CODEX]);
    agent.env = vec![("CODEX_HOME".to_owned(), home.display().to_string())];
    assert_eq!(
        codex_config_layers(&agent).map(|layers| layers.len()),
        Err(Unconfinable::UnreadableConfig)
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_user_server_named_like_oxyns_is_refused_with_tools_and_turned_off_without() {
    let agent = declared(&["-y", CODEX]);
    let user = "[mcp_servers.oxyn]\ncommand = \"not-ours\"\n";
    assert_eq!(
        confined(&agent, Some(URL), Some(user)),
        Err(Unconfinable::NameTaken)
    );
    let without = confined(&agent, None, Some(user)).expect("confined");
    assert_eq!(
        codex_config(&without)["mcp_servers"]["oxyn"],
        json!({ "enabled": false })
    );
}

#[test]
fn only_the_measured_version_with_its_own_arguments_is_confined() {
    for args in [
        vec!["my-agent", "--acp"],
        // Another version was not measured.
        vec!["-y", "@agentclientprotocol/codex-acp@2.0.0"],
        vec!["-y", "@agentclientprotocol/codex-acp"],
        // An argument more may undo the confinement.
        vec!["-y", CODEX, "-c", "sandbox_mode=danger-full-access"],
        // A name that only starts like a known one.
        vec!["-y", "@agentclientprotocol/codex-acp-fork@1.12.0"],
    ] {
        assert_eq!(
            confinement_for(&declared(&args), Some(URL), none),
            Ok(None),
            "{args:?}"
        );
    }
}
