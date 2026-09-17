use agent_client_protocol::schema::v1::{
    SessionConfigBoolean, SessionConfigSelect, SessionConfigSelectGroup, SessionConfigSelectOption,
    SessionMode,
};

use super::*;

fn select(
    id: &str,
    category: Option<SessionConfigOptionCategory>,
    current: &str,
) -> SessionConfigOption {
    let options = vec![
        SessionConfigSelectOption::new("low", "Low"),
        SessionConfigSelectOption::new("high", "High"),
    ];
    let option = SessionConfigOption::new(
        id.to_owned(),
        "Reasoning",
        SessionConfigKind::Select(SessionConfigSelect::new(current.to_owned(), options)),
    );
    match category {
        Some(category) => option.category(category),
        None => option,
    }
}

#[test]
fn modes_and_options_are_read_in_the_agents_order() {
    let modes = SessionModeState::new(
        "ask",
        vec![
            SessionMode::new("ask", "Ask").description("Asks before acting".to_owned()),
            SessionMode::new("plan", "Plan"),
        ],
    );
    let options = vec![
        select(
            "effort",
            Some(SessionConfigOptionCategory::ThoughtLevel),
            "high",
        ),
        SessionConfigOption::new(
            "fast",
            "Fast mode",
            SessionConfigKind::Boolean(SessionConfigBoolean::new(true)),
        ),
    ];
    let settings = AgentSettings::declared(Some(&modes), Some(&options));

    assert_eq!(settings.current_mode.as_deref(), Some("ask"));
    assert_eq!(
        settings
            .modes
            .iter()
            .map(|mode| mode.id.as_str())
            .collect::<Vec<_>>(),
        vec!["ask", "plan"]
    );
    assert_eq!(settings.options[0].category, OptionCategory::ThoughtLevel);
    assert_eq!(
        settings.options[0].value,
        OptionValue::Select {
            current: "high".to_owned(),
            choices: vec![
                AgentChoice {
                    id: "low".to_owned(),
                    name: "Low".to_owned(),
                    description: None
                },
                AgentChoice {
                    id: "high".to_owned(),
                    name: "High".to_owned(),
                    description: None
                },
            ],
        }
    );
    assert_eq!(settings.options[1].value, OptionValue::Boolean(true));
}

#[test]
fn an_unknown_or_missing_category_is_never_read_as_a_known_one() {
    // The effort selector reads `ThoughtLevel` only: an option guessed into it
    // would show a control for a setting the agent never offered.
    let settings = AgentSettings::declared(
        None,
        Some(&[
            select("a", None, "low"),
            select(
                "b",
                Some(SessionConfigOptionCategory::Other("vibes".to_owned())),
                "low",
            ),
        ]),
    );
    assert!(
        settings
            .options
            .iter()
            .all(|option| option.category == OptionCategory::Other)
    );
}

#[test]
fn groups_are_flattened_without_losing_a_choice() {
    let grouped = SessionConfigOption::new(
        "model",
        "Model",
        SessionConfigKind::Select(SessionConfigSelect::new(
            "b",
            vec![
                SessionConfigSelectGroup::new(
                    "g1",
                    "First",
                    vec![SessionConfigSelectOption::new("a", "A")],
                ),
                SessionConfigSelectGroup::new(
                    "g2",
                    "Second",
                    vec![SessionConfigSelectOption::new("b", "B")],
                ),
            ],
        )),
    );
    let settings = AgentSettings::declared(None, Some(&[grouped]));
    let OptionValue::Select { choices, .. } = &settings.options[0].value else {
        panic!("a select");
    };
    assert_eq!(
        choices.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        vec!["a", "b"]
    );
}

#[test]
fn options_are_replaced_whole_and_the_mode_follows_the_agent() {
    let mut settings = AgentSettings::declared(
        None,
        Some(&[select(
            "effort",
            Some(SessionConfigOptionCategory::ThoughtLevel),
            "low",
        )]),
    );
    settings.set_options(&[select("other", None, "high")]);
    assert_eq!(settings.options.len(), 1, "replaced, not merged");
    assert_eq!(settings.options[0].id, "other");

    settings.set_current_mode("plan");
    assert_eq!(settings.current_mode.as_deref(), Some("plan"));
}

#[test]
fn a_hostile_agent_is_bounded() {
    // Every byte comes from another process (I-09): huge lists and names are
    // cut, never grown into the panel.
    let name = "é".repeat(4096);
    let modes = SessionModeState::new(
        "m0",
        (0..1000)
            .map(|index| SessionMode::new(format!("m{index}"), name.clone()))
            .collect::<Vec<_>>(),
    );
    let options: Vec<_> = (0..1000)
        .map(|index| select(&format!("o{index}"), None, "low"))
        .collect();
    let settings = AgentSettings::declared(Some(&modes), Some(&options));

    assert_eq!(settings.modes.len(), MAX_MODES);
    assert_eq!(settings.options.len(), MAX_OPTIONS);
    assert!(settings.modes[0].name.len() <= MAX_TEXT_BYTES);
    assert!(
        settings.modes[0]
            .name
            .is_char_boundary(settings.modes[0].name.len())
    );
}

#[test]
fn an_identifier_past_the_bound_is_left_out_never_cut() {
    // Two models sharing their first 512 bytes: a cut identifier would send
    // the agent a prefix it never offered, or the other model.
    let shared = "m".repeat(MAX_ID_BYTES);
    let long = format!("{shared}-b");
    let modes = SessionModeState::new(
        long.clone(),
        vec![
            SessionMode::new("ask", "Ask"),
            SessionMode::new(long.clone(), "Long"),
        ],
    );
    let model = SessionConfigOption::new(
        "model",
        "Model",
        SessionConfigKind::Select(SessionConfigSelect::new(
            "a",
            vec![
                SessionConfigSelectOption::new("a", "A"),
                SessionConfigSelectOption::new(long.clone(), "B"),
            ],
        )),
    );
    let settings = AgentSettings::declared(
        Some(&modes),
        Some(&[
            model,
            select(&long, None, "low"),
            select("current-too-long", None, &long),
        ]),
    );

    assert_eq!(
        settings
            .modes
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        vec!["ask"]
    );
    assert_eq!(settings.current_mode, None);
    assert_eq!(
        settings
            .options
            .iter()
            .map(|o| o.id.as_str())
            .collect::<Vec<_>>(),
        vec!["model"]
    );
    let OptionValue::Select { choices, .. } = &settings.options[0].value else {
        panic!("a select");
    };
    assert_eq!(
        choices.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        vec!["a"]
    );
    // Nothing the user can pick is a prefix of what the agent declared.
    assert_eq!(
        settings.check(&SettingChange::Option {
            id: "model".to_owned(),
            value: SettingValue::Choice(shared.clone()),
        }),
        Err(SettingRefused::UnknownValue)
    );
    let mut followed = settings.clone();
    followed.set_current_mode(&long);
    assert_eq!(followed.current_mode, None);
    // A name is still cut, not dropped.
    let named = AgentSettings::declared(
        Some(&SessionModeState::new(
            "ask",
            vec![SessionMode::new("ask", "é".repeat(4096))],
        )),
        None,
    );
    assert_eq!(named.modes.len(), 1);
}

#[test]
fn a_change_names_only_what_the_agent_declared() {
    let modes = SessionModeState::new("ask", vec![SessionMode::new("ask", "Ask")]);
    let settings = AgentSettings::declared(
        Some(&modes),
        Some(&[
            select(
                "effort",
                Some(SessionConfigOptionCategory::ThoughtLevel),
                "high",
            ),
            SessionConfigOption::new(
                "fast".to_owned(),
                "Fast",
                SessionConfigKind::Boolean(SessionConfigBoolean::new(false)),
            ),
        ]),
    );
    let option = |id: &str, value| SettingChange::Option {
        id: id.to_owned(),
        value,
    };

    assert_eq!(
        settings.check(&SettingChange::Mode {
            id: "ask".to_owned()
        }),
        Ok(())
    );
    assert_eq!(
        settings.check(&SettingChange::Mode {
            id: "yolo".to_owned()
        }),
        Err(SettingRefused::UnknownMode)
    );
    assert_eq!(
        settings.check(&option("effort", SettingValue::Choice("low".to_owned()))),
        Ok(())
    );
    assert_eq!(
        settings.check(&option("effort", SettingValue::Choice("max".to_owned()))),
        Err(SettingRefused::UnknownValue),
        "a level the agent did not declare is not sent"
    );
    assert_eq!(
        settings.check(&option("effort", SettingValue::Boolean(true))),
        Err(SettingRefused::UnknownValue)
    );
    assert_eq!(
        settings.check(&option("fast", SettingValue::Boolean(true))),
        Ok(())
    );
    assert_eq!(
        settings.check(&option("fast", SettingValue::Choice("true".to_owned()))),
        Err(SettingRefused::UnknownValue)
    );
    assert_eq!(
        settings.check(&option("model", SettingValue::Choice("a".to_owned()))),
        Err(SettingRefused::UnknownOption)
    );
    assert_eq!(
        AgentSettings::default().check(&SettingChange::Mode {
            id: "ask".to_owned()
        }),
        Err(SettingRefused::UnknownMode),
        "nothing declared, nothing sent"
    );
}

#[test]
fn a_confirmed_mode_is_taken_only_if_still_declared() {
    let modes = SessionModeState::new(
        "ask",
        vec![
            SessionMode::new("ask", "Ask"),
            SessionMode::new("plan", "Plan"),
        ],
    );
    let mut settings = AgentSettings::declared(Some(&modes), None);

    assert!(settings.confirm_mode("plan"));
    assert_eq!(settings.current_mode.as_deref(), Some("plan"));

    assert!(!settings.confirm_mode("gone"));
    assert_eq!(settings.current_mode.as_deref(), Some("plan"), "kept");
}

#[test]
fn a_mode_option_supersedes_the_modes_and_nothing_else_does() {
    let modes = SessionModeState::new(
        "ask",
        vec![
            SessionMode::new("ask", "Ask"),
            SessionMode::new("plan", "Plan"),
        ],
    );
    let as_option = select("mode", Some(SessionConfigOptionCategory::Mode), "ask");
    let effort = select(
        "effort",
        Some(SessionConfigOptionCategory::ThoughtLevel),
        "high",
    );

    // Both declared: one mode selector, the option.
    let mut both =
        AgentSettings::declared(Some(&modes), Some(&[as_option.clone(), effort.clone()]));
    assert!(
        both.modes.is_empty() && both.current_mode.is_none(),
        "{both:?}"
    );
    assert_eq!(
        both.check(&SettingChange::Mode {
            id: "plan".to_owned()
        }),
        Err(SettingRefused::UnknownMode),
        "the superseded mechanism is not used either"
    );
    // A later mode notification does not bring the second selector back.
    both.set_current_mode("plan");
    assert!(both.current_mode.is_none());
    assert!(!both.confirm_mode("plan"));

    // Modes alone, or with options of other categories: kept.
    let alone = AgentSettings::declared(Some(&modes), Some(&[effort]));
    assert_eq!(alone.modes.len(), 2);
    assert_eq!(alone.current_mode.as_deref(), Some("ask"));

    // An option list that gains a mode option later supersedes then.
    let mut later = alone;
    later.set_options(&[as_option]);
    assert!(later.modes.is_empty() && later.current_mode.is_none());
}
