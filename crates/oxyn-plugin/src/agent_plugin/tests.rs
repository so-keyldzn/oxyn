use crate::fixtures::{TempDir, agent_toml};

use super::*;

fn manifest_toml(section_agent: &str) -> String {
    format!(
        "id          = \"revue-schema\"\n\
             name        = \"Schema review\"\n\
             version     = \"1.0.0\"\n\
             api_version = \"0.1.0\"\n\
             kind        = \"agent\"\n\
             \n\
             [permissions]\n\
             connections = \"read_only\"\n\
             \n\
             [agent]\n\
             {section_agent}"
    )
}

#[test]
fn an_agent_is_read_directly_from_a_directory() {
    let root = TempDir::new("agent-file");
    root.plugin("revue", &agent_toml("revue", "\"refresh_catalog\""));

    let agent = load_from_dir(&root.path().join("revue")).expect("agent read from disk");
    assert_eq!(agent.plugin().as_str(), "revue");
    assert_eq!(agent.spec().name, "Schema");
}

#[test]
fn a_directory_without_manifest_gives_a_file_error() {
    let root = TempDir::new("agent-absent");
    let err = load_from_dir(&root.path().join("nulle-part")).expect_err("refusal expected");
    assert!(matches!(err, PluginError::Directory { .. }), "{err}");
}

#[test]
fn a_declarative_agent_loads_without_wasm_host() {
    let toml = manifest_toml(
        "name          = \"Schema\"\n\
             description   = \"Reviews a schema.\"\n\
             system_prompt = \"You review database schemas.\"\n\
             allowed_tools = [\"refresh_catalog\"]\n\
             max_turns     = 4\n",
    );
    let manifest = PluginManifest::from_toml(&toml).expect("valid manifest");
    assert!(!manifest.requires_wasm());

    let agent = DeclarativeAgent::from_manifest(&manifest).expect("declarative agent");
    assert_eq!(agent.plugin().as_str(), "revue-schema");
    assert_eq!(agent.plugin_name(), "Schema review");
    assert_eq!(agent.connections(), ConnectionAccess::ReadOnly);
    assert_eq!(agent.spec().name, "Schema");
    assert_eq!(agent.spec().max_turns, Some(4));
    assert!(agent.spec().allows("refresh_catalog"));
}

#[test]
fn a_declaration_cannot_choose_its_privacy_tier() {
    // I-04: the tier is attached to the connection and to nothing else. A
    // key that was ignored would suggest it had been honored.
    let toml = manifest_toml(
        "name          = \"Schema\"\n\
             system_prompt = \"You review database schemas.\"\n\
             privacy       = \"sampled\"\n",
    );
    let err = PluginManifest::from_toml(&toml).expect_err("refusal expected");
    assert!(
        matches!(err, PluginError::UnreadableManifest { .. }),
        "{err}"
    );
}

#[test]
fn a_declaration_cannot_carry_a_key_or_an_endpoint() {
    // I-03: a plugin does not get a network channel by way of an agent, and
    // above all not a secret written in clear in a file.
    for key in [
        "api_key       = \"sk-abc\"",
        "endpoint      = \"https://exfiltration.example\"",
        "connection    = \"prod-eu\"",
        "id            = \"018f0000-0000-7000-8000-000000000000\"",
    ] {
        let toml = manifest_toml(&format!(
            "name          = \"Schema\"\n\
                 system_prompt = \"You review database schemas.\"\n\
                 {key}\n"
        ));
        assert!(
            PluginManifest::from_toml(&toml).is_err(),
            "{key} should be refused"
        );
    }
}

#[test]
fn an_agent_without_tools_is_legitimate() {
    let toml = manifest_toml(
        "name          = \"Doc\"\n\
             system_prompt = \"You describe schemas.\"\n",
    );
    let manifest = PluginManifest::from_toml(&toml).expect("valid manifest");
    let agent = DeclarativeAgent::from_manifest(&manifest).expect("declarative agent");
    assert!(agent.spec().allowed_tools.is_empty());
    assert!(!agent.spec().allows("execute_query"));
}

#[test]
fn an_empty_prompt_or_name_is_refused() {
    let spec = PluginAgentSpec::new("   ", "You review schemas.");
    assert!(spec.validate("x").is_err());

    let spec = PluginAgentSpec::new("Schema", "\n\t ");
    assert!(spec.validate("x").is_err());
}

#[test]
fn a_tool_name_is_a_technical_key() {
    for ident in [
        "",
        "Execute",
        "execute-query",
        "execute query",
        "2query",
        "execute/query",
        "execute_query\u{0}",
    ] {
        let spec = PluginAgentSpec::new("Schema", "prompt").with_tools([ident]);
        assert!(spec.validate("x").is_err(), "{ident:?} should be refused");
    }

    let spec =
        PluginAgentSpec::new("Schema", "prompt").with_tools(["execute_query", "refresh_catalog"]);
    spec.validate("x").expect("valid tool names");
}

#[test]
fn a_tool_declared_twice_is_refused() {
    let spec =
        PluginAgentSpec::new("Schema", "prompt").with_tools(["execute_query", "execute_query"]);
    let err = spec.validate("x").expect_err("refusal expected");
    assert!(err.to_string().contains("twice"), "{err}");
}

#[test]
fn zero_turns_is_refused() {
    let mut spec = PluginAgentSpec::new("Schema", "prompt");
    spec.max_turns = Some(0);
    assert!(spec.validate("x").is_err());
}

#[test]
fn a_plugin_that_is_not_an_agent_provides_no_agent() {
    let toml = "id          = \"duckdb\"\n\
                    name        = \"DuckDB\"\n\
                    version     = \"1.0.0\"\n\
                    api_version = \"0.1.0\"\n\
                    kind        = \"export\"\n\
                    entrypoint  = \"duckdb.wasm\"\n";
    let manifest = PluginManifest::from_toml(toml).expect("valid manifest");
    let err = DeclarativeAgent::from_manifest(&manifest).expect_err("refusal expected");
    assert!(err.to_string().contains("export"), "{err}");
}

#[test]
fn the_output_schema_goes_through_opaque() {
    let toml = manifest_toml(
        "name          = \"Schema\"\n\
             system_prompt = \"You review database schemas.\"\n\
             output_schema = \"{\\\"type\\\":\\\"object\\\"}\"\n",
    );
    let manifest = PluginManifest::from_toml(&toml).expect("valid manifest");
    let agent = DeclarativeAgent::from_manifest(&manifest).expect("declarative agent");
    assert_eq!(
        agent.spec().output_schema.as_deref(),
        Some("{\"type\":\"object\"}"),
        "the schema is carried as is, without being parsed here"
    );
}
