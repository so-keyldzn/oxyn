use super::*;
use crate::workspace::tests::{connected_workspace, preview_fixture, wait_for_preview};
use gpui::TestAppContext;
use gpui::px;
use oxyn_catalog::model::{Field, LogicalType, RelationKind};

/// A relation as the catalog would hold it.
fn relation(columns: &[(&str, bool)]) -> Relation {
    let fields = columns
        .iter()
        .enumerate()
        .map(|(rank, (name, key))| {
            let position = u32::try_from(rank).expect("fewer than 2^32 test columns");
            let mut field = Field::new(*name, position, LogicalType::Text, "TEXT");
            field.is_primary_key = *key;
            field
        })
        .collect::<Vec<_>>();
    Relation::new("t", RelationKind::Table).with_fields(fields)
}

#[test]
fn an_undescribed_relation_offers_no_column_to_sort_on() {
    let choices = sort_choices_from(None);
    assert_eq!(choices.len(), 1, "only the entry that sorts nothing");
    assert_eq!(sort_labels(&choices), vec!["Unsorted"]);
}

#[test]
fn every_column_offers_both_directions_and_each_rank_is_one_read() {
    let relation = relation(&[("id", true), ("name", false)]);
    let choices = sort_choices_from(Some(&relation));
    assert_eq!(choices.len(), 5);
    assert_eq!(
        sort_labels(&choices),
        vec![
            "Unsorted",
            "id · Ascending",
            "id · Descending",
            "name · Ascending",
            "name · Descending",
        ]
    );
    let descending_name = PreviewShape {
        sort: vec![PreviewSort::descending("name")],
        ..PreviewShape::default()
    };
    assert_eq!(sort_rank(&choices, &descending_name), 4);
    // A sort on a column the menu no longer offers falls back to rank 0
    // rather than pointing at whatever sits at that rank now.
    let gone = PreviewShape {
        sort: vec![PreviewSort::ascending("removed")],
        ..PreviewShape::default()
    };
    assert_eq!(sort_rank(&choices, &gone), 0);
    assert_eq!(sort_rank(&choices, &PreviewShape::unordered()), 0);
}

#[test]
fn a_page_is_offered_only_where_the_order_is_total() {
    let plain = PreviewShape::unordered();
    assert_eq!(
        pagination_from(&plain, Some(true), 200),
        Pagination::NeedsOrder,
        "the first plain preview has no order, so it has no second page"
    );
    let sorted = PreviewShape {
        sort: vec![PreviewSort::ascending("name")],
        ..PreviewShape::default()
    };
    assert_eq!(
        pagination_from(&sorted, Some(false), 200),
        Pagination::NoUniqueKey
    );
    assert_eq!(
        pagination_from(&sorted, None, 200),
        Pagination::NoUniqueKey,
        "an undescribed relation is not a relation with a key"
    );
    assert_eq!(
        pagination_from(&sorted, Some(true), 200),
        Pagination::Ready {
            previous: false,
            next: true
        }
    );
    assert_eq!(
        pagination_from(&sorted, Some(true), 199),
        Pagination::Ready {
            previous: false,
            next: false
        },
        "a short page is the last one"
    );
    let second = PreviewShape {
        offset: 200,
        ..sorted
    };
    assert_eq!(
        pagination_from(&second, Some(true), 0),
        Pagination::Ready {
            previous: true,
            next: false
        },
        "an empty page still leads back to the one before it"
    );
}

/// A complete result that carries no row: the shape of an empty answer.
fn empty_buffer() -> std::sync::Arc<oxyn_data::ResultBuffer> {
    let schema = std::sync::Arc::new(arrow::datatypes::Schema::new(vec![
        arrow::datatypes::Field::new("id", arrow::datatypes::DataType::Int64, false),
    ]));
    let buffer = std::sync::Arc::new(oxyn_data::ResultBuffer::new(schema, 1024));
    buffer.mark_complete(oxyn_core::ExecStats::default());
    buffer
}

#[test]
fn an_empty_answer_says_whether_a_filter_caused_it() {
    let buffer = empty_buffer();
    assert_eq!(
        preview_state_from(false, &GridState::Streaming(buffer.clone()), true),
        PreviewState::Empty { filtered: true }
    );
    assert_eq!(
        preview_state_from(false, &GridState::Streaming(buffer.clone()), false),
        PreviewState::Empty { filtered: false },
        "an empty table is not a filter that matched nothing"
    );
    // A read under way outranks the rows still on screen: that is the state
    // that has to carry a way out.
    assert_eq!(
        preview_state_from(true, &GridState::Streaming(buffer), true),
        PreviewState::Loading
    );
    assert_eq!(
        preview_state_from(
            false,
            &GridState::Failed {
                message: "near \"WERE\": syntax error".into(),
                retryable: false
            },
            true
        ),
        PreviewState::Failed {
            message: "near \"WERE\": syntax error".to_owned()
        }
    );
    assert_eq!(
        preview_state_from(false, &GridState::Idle, false),
        PreviewState::Initial
    );
}

#[test]
fn a_draft_that_is_not_in_force_is_told_apart_from_one_that_is() {
    assert!(
        !draft_differs("  ", None),
        "an erased field filters nothing"
    );
    assert!(draft_differs("id > 10", None));
    assert!(!draft_differs(" id > 10 ", Some("id > 10")));
    assert!(draft_differs("id > 11", Some("id > 10")));
}

/// The most that can be proven without an engine that lacks the capability:
/// SQLite declares both, so the view is made to see neither.
#[gpui::test]
fn a_session_without_the_capabilities_has_no_bar_and_dispatches_nothing(cx: &mut TestAppContext) {
    let (backend, open) = preview_fixture();
    let mut events = backend.subscribe();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.simulate_resize(gpui::size(px(1600.), px(1060.)));
    view.update(cx, |view, cx| {
        view.capabilities
            .remove(Capabilities::PREVIEW_SORT | Capabilities::PREVIEW_FILTER);
        cx.notify();
    });
    let path = CatalogPath::for_relation(None, Some("main"), "preview_rows").expect("table path");
    let tree = view.read_with(cx, |view, _| view.catalog.clone().expect("catalog"));
    tree.update(cx, |tree, cx| tree.select(path, cx));
    wait_for_preview(&view, cx);
    while events.try_recv().is_ok() {}

    assert!(
        cx.debug_bounds("preview-filter-bar").is_none(),
        "no capability, no bar — not a greyed one"
    );
    for id in [
        "preview-filter-input",
        "preview-filter-apply",
        "preview-sort",
        "preview-page-next",
        "preview-page-previous",
    ] {
        assert!(cx.debug_bounds(id).is_none(), "{id} must not exist");
    }
    // The commands themselves refuse too: the absence of a control is not
    // the only thing standing between a session and a shape it cannot honour.
    view.update(cx, |view, cx| {
        view.apply_preview_predicate(cx);
        view.choose_preview_sort(1, cx);
        view.page_preview(true, cx);
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(view.preview_active.is_none());
        assert!(view.preview_controls.applied.is_plain());
    });
    assert!(
        events.try_recv().is_err(),
        "nothing may reach the executor from a surface that does not exist"
    );
}

/// A real predicate over a real SQLite table, and the console left alone.
#[gpui::test]
fn a_predicate_filters_the_real_rows_and_leaves_the_sql_draft_alone(cx: &mut TestAppContext) {
    let (backend, open) = preview_fixture();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.simulate_resize(gpui::size(px(1600.), px(1060.)));
    cx.simulate_input("SELECT 'keep my draft';");
    let path = CatalogPath::for_relation(None, Some("main"), "preview_rows").expect("table path");
    let tree = view.read_with(cx, |view, _| view.catalog.clone().expect("catalog"));
    tree.update(cx, |tree, cx| tree.select(path, cx));
    wait_for_preview(&view, cx);

    let field = view.read_with(cx, |view, _| view.preview_controls.predicate.clone());
    cx.update(|window, cx| window.focus(&field.read(cx).focus_handle(cx)));
    cx.simulate_input("id <= 3");
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(
            view.preview_active.is_none(),
            "typing reads nothing: only Apply does"
        );
        assert!(view.preview_controls.applied.is_plain());
    });

    let apply = cx
        .debug_bounds("preview-filter-apply")
        .expect("the filter carries its own way to run");
    cx.simulate_click(apply.center(), Default::default());
    wait_for_preview(&view, cx);
    view.read_with(cx, |view, cx| {
        assert_eq!(
            view.preview_controls.applied.predicate(),
            Some("id <= 3"),
            "what the rows came from is what the field held when Apply ran"
        );
        let buffer = view
            .preview_grid
            .read(cx)
            .state()
            .buffer()
            .expect("filtered rows")
            .clone();
        assert_eq!(buffer.row_count(), 3, "1000 rows, three of them under four");
        assert_eq!(
            view.draft_text(cx),
            "SELECT 'keep my draft';",
            "filtering a preview never touches the console draft"
        );
        assert!(view.console.read(cx).active.is_none());
    });
}

/// A filter that matches nothing is a success with no rows, and it says so
/// in words that name the filter rather than the table.
#[gpui::test]
fn a_predicate_that_matches_nothing_is_empty_not_failed(cx: &mut TestAppContext) {
    let (backend, open) = preview_fixture();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.simulate_resize(gpui::size(px(1600.), px(1060.)));
    let path = CatalogPath::for_relation(None, Some("main"), "preview_rows").expect("table path");
    let tree = view.read_with(cx, |view, _| view.catalog.clone().expect("catalog"));
    tree.update(cx, |tree, cx| tree.select(path, cx));
    wait_for_preview(&view, cx);
    view.update(cx, |view, cx| {
        view.preview_controls
            .predicate
            .update(cx, |field, cx| field.set_text("id < 0".into(), cx));
        view.apply_preview_predicate(cx);
    });
    wait_for_preview(&view, cx);
    view.read_with(cx, |view, cx| {
        assert_eq!(
            view.preview_state(cx),
            PreviewState::Empty { filtered: true },
            "an empty answer is not an error, and it names the filter"
        );
        let buffer = view
            .preview_grid
            .read(cx)
            .state()
            .buffer()
            .expect("an empty success still has a schema");
        assert_eq!(buffer.row_count(), 0);
        assert!(buffer.is_complete());
    });
    assert!(
        cx.debug_bounds("preview-shape-notice").is_some(),
        "the empty state is readable, not merely a grid with no rows"
    );
}

/// The audience reads server messages; this one arrives with its code and
/// replaces nothing on screen with rows that are not the server's.
#[gpui::test]
fn an_invalid_predicate_shows_the_server_message_and_keeps_no_false_rows(cx: &mut TestAppContext) {
    let (backend, open) = preview_fixture();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.simulate_resize(gpui::size(px(1600.), px(1060.)));
    let path = CatalogPath::for_relation(None, Some("main"), "preview_rows").expect("table path");
    let tree = view.read_with(cx, |view, _| view.catalog.clone().expect("catalog"));
    tree.update(cx, |tree, cx| tree.select(path, cx));
    wait_for_preview(&view, cx);
    view.read_with(cx, |view, cx| {
        assert!(matches!(
            view.preview_state(cx),
            PreviewState::Loaded { rows: 200 }
        ));
    });

    view.update(cx, |view, cx| {
        view.preview_controls.predicate.update(cx, |field, cx| {
            field.set_text("no_such_column = 1".into(), cx)
        });
        view.apply_preview_predicate(cx);
    });
    wait_for_preview(&view, cx);
    view.read_with(cx, |view, cx| {
        let PreviewState::Failed { message } = view.preview_state(cx) else {
            panic!("a broken predicate fails, and the preview can now fail on syntax")
        };
        assert!(
            message.contains("no_such_column"),
            "the server's own words, naming what it could not resolve: {message}"
        );
        assert!(
            view.preview_grid.read(cx).state().buffer().is_none(),
            "the rows of the previous shape are not kept under the new one"
        );
        assert!(
            view.preview_controls.applied.predicate().is_none(),
            "a refused read applies nothing"
        );
        assert_eq!(
            view.preview_controls.predicate.read(cx).text(),
            "no_such_column = 1",
            "the text stays where the user can fix it"
        );
    });
}

/// The running state keeps its way out for whoever is standing in the field.
///
/// The field swallows Escape, so the workspace shortcut never sees it: without
/// the subscription, focusing the filter would remove the only way to stop a
/// read ([UX-SPEC](../../../../docs/UX-SPEC.md#annulation)).
#[gpui::test]
fn escape_in_the_filter_field_still_stops_the_read_at_the_server(cx: &mut TestAppContext) {
    let (backend, open) = preview_fixture();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.simulate_resize(gpui::size(px(1600.), px(1060.)));
    let path = CatalogPath::for_relation(None, Some("main"), "preview_rows").expect("table path");
    let tree = view.read_with(cx, |view, _| view.catalog.clone().expect("catalog"));
    tree.update(cx, |tree, cx| tree.select(path, cx));
    wait_for_preview(&view, cx);

    let token = CancelToken::new();
    view.update(cx, |view, cx| {
        view.preview_active = Some((CommandId::new(), token.clone()));
        cx.notify();
    });
    cx.run_until_parked();
    view.read_with(cx, |view, cx| {
        assert_eq!(view.preview_state(cx), PreviewState::Loading);
    });
    let field = view.read_with(cx, |view, _| view.preview_controls.predicate.clone());
    cx.update(|window, cx| window.focus(&field.read(cx).focus_handle(cx)));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(
        token.is_cancelled(),
        "the token the executor propagates is the one this keystroke reaches"
    );
}

/// The order really changes, and it is the server that changed it.
#[gpui::test]
fn a_sort_changes_the_order_the_server_returns(cx: &mut TestAppContext) {
    let (backend, open) = preview_fixture();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.simulate_resize(gpui::size(px(1600.), px(1060.)));
    let path = CatalogPath::for_relation(None, Some("main"), "preview_keyed").expect("path");
    let tree = view.read_with(cx, |view, _| view.catalog.clone().expect("catalog"));
    tree.update(cx, |tree, cx| tree.select(path.clone(), cx));
    wait_for_preview(&view, cx);
    describe_relation(&view, cx, &path);

    // Unfolded by pointer, chosen by keyboard: the menu has to be usable
    // without a mouse ([ADR-0001](../../../../docs/adr/0001-ui-toolkit.md)).
    let sort = cx
        .debug_bounds("preview-sort")
        .expect("a session that can order a preview shows the control");
    cx.simulate_click(sort.center(), Default::default());
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(
            view.preview_controls.sort_open,
            "the click reached the control rather than landing past the edge"
        );
        assert_eq!(rank_of(view, "id", false), 1, "the first column, ascending");
    });
    assert!(
        cx.debug_bounds("preview-sort-field").is_some(),
        "a described relation offers its columns instead of asking for them"
    );
    let field = view.read_with(cx, |view, _| view.preview_controls.sort.clone());
    cx.update(|window, cx| window.focus(&field.read(cx).focus_handle(cx)));
    assert!(
        cx.update(|window, cx| field.read(cx).focus_handle(cx).is_focused(window)),
        "the menu is reachable and holds the keyboard"
    );
    // The first `down` opens the closed menu; the second moves onto rank 1.
    cx.simulate_keystrokes("down");
    cx.simulate_keystrokes("down");
    cx.simulate_keystrokes("enter");
    wait_for_preview(&view, cx);
    let ascending = view.read_with(cx, ids);
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.preview_controls.applied.sort,
            vec![PreviewSort::ascending("id")],
            "the menu drives the read; nothing else does"
        );
    });

    view.update(cx, |view, cx| {
        let rank = rank_of(view, "id", true);
        view.choose_preview_sort(rank, cx);
    });
    wait_for_preview(&view, cx);
    view.read_with(cx, |view, cx| {
        assert_eq!(
            view.preview_controls.applied.sort,
            vec![PreviewSort::descending("id")],
            "what the menu shows is what produced the rows"
        );
        let descending = ids(view, cx);
        assert_eq!(ascending.first().map(String::as_str), Some("1"));
        assert_eq!(descending.first().map(String::as_str), Some("1000"));
        assert_eq!(
            descending.len(),
            200,
            "the bound still applies to a sorted read"
        );
        let mut reversed = descending.clone();
        reversed.reverse();
        assert_ne!(
            reversed, ascending,
            "two windows of 200 rows over 1000, taken from opposite ends"
        );
        assert!(
            descending.windows(2).all(|pair| {
                pair.first().and_then(|text| text.parse::<i64>().ok())
                    > pair.last().and_then(|text| text.parse::<i64>().ok())
            }),
            "the rows themselves descend: the server ordered them, not the grid"
        );
    });
}

/// The rank of one column and direction in the menu the catalog filled.
fn rank_of(view: &Workspace, column: &str, descending: bool) -> usize {
    view.preview_controls
        .sort_choices
        .iter()
        .position(|choice| {
            choice
                .as_ref()
                .is_some_and(|sort| sort.column == column && sort.descending == descending)
        })
        .expect("a described relation offers both directions of each column")
}

/// Two pages of the same total order share no row.
#[gpui::test]
fn two_consecutive_pages_do_not_overlap(cx: &mut TestAppContext) {
    let (backend, open) = preview_fixture();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.simulate_resize(gpui::size(px(1600.), px(1060.)));
    let path = CatalogPath::for_relation(None, Some("main"), "preview_keyed").expect("path");
    let tree = view.read_with(cx, |view, _| view.catalog.clone().expect("catalog"));
    tree.update(cx, |tree, cx| tree.select(path.clone(), cx));
    wait_for_preview(&view, cx);
    describe_relation(&view, cx, &path);
    view.read_with(cx, |view, cx| {
        assert_eq!(
            view.preview_pagination(cx),
            Pagination::NeedsOrder,
            "a plain preview has no order, so it is not browsed page by page"
        );
    });
    assert!(
        cx.debug_bounds("preview-page-next").is_none(),
        "the control appears with the order that makes it correct"
    );

    view.update(cx, |view, cx| {
        let rank = rank_of(view, "id", false);
        view.choose_preview_sort(rank, cx);
    });
    wait_for_preview(&view, cx);
    let first_page = view.read_with(cx, ids);
    view.read_with(cx, |view, cx| {
        assert_eq!(
            view.preview_pagination(cx),
            Pagination::Ready {
                previous: false,
                next: true
            }
        );
    });

    let next = cx
        .debug_bounds("preview-page-next")
        .expect("a total order carries its pages");
    cx.simulate_click(next.center(), Default::default());
    wait_for_preview(&view, cx);
    let second_page = view.read_with(cx, ids);
    view.read_with(cx, |view, cx| {
        assert_eq!(view.preview_controls.applied.offset, 200);
        assert_eq!(
            view.preview_pagination(cx),
            Pagination::Ready {
                previous: true,
                next: true
            }
        );
    });
    assert_eq!(first_page.len(), 200);
    assert_eq!(second_page.len(), 200);
    assert!(
        first_page.iter().all(|id| !second_page.contains(id)),
        "an OFFSET over a total order repeats nothing"
    );
    assert_eq!(first_page.first().map(String::as_str), Some("1"));
    assert_eq!(
        second_page.first().map(String::as_str),
        Some("201"),
        "the second page starts exactly where the first stopped"
    );
}

/// Typing is not executing, and the bar says the draft is not in force.
#[gpui::test]
fn typing_dispatches_nothing_and_the_bar_says_the_draft_is_not_in_force(cx: &mut TestAppContext) {
    let (backend, open) = preview_fixture();
    let mut events = backend.subscribe();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.simulate_resize(gpui::size(px(1600.), px(1060.)));
    let path = CatalogPath::for_relation(None, Some("main"), "preview_rows").expect("table path");
    let tree = view.read_with(cx, |view, _| view.catalog.clone().expect("catalog"));
    tree.update(cx, |tree, cx| tree.select(path, cx));
    wait_for_preview(&view, cx);
    while events.try_recv().is_ok() {}

    let field = view.read_with(cx, |view, _| view.preview_controls.predicate.clone());
    cx.update(|window, cx| window.focus(&field.read(cx).focus_handle(cx)));
    assert!(
        cx.update(|window, cx| field.read(cx).focus_handle(cx).is_focused(window)),
        "the field is reachable and holds the keyboard"
    );
    cx.simulate_input("id = 1");
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(view.preview_active.is_none(), "no read left for the server");
        assert!(view.preview_controls.applied.is_plain());
    });
    assert!(
        events.try_recv().is_err(),
        "a keystroke that executes is what this bar refuses"
    );
    assert!(
        cx.debug_bounds("preview-shape-notice").is_some(),
        "the difference between the draft and what is in force is said, not hidden"
    );

    // Enter is Apply, and it is the only other way in.
    cx.simulate_keystrokes("enter");
    wait_for_preview(&view, cx);
    view.read_with(cx, |view, _| {
        assert_eq!(view.preview_controls.applied.predicate(), Some("id = 1"));
    });
}

/// A late answer never replaces the rows of a newer filter.
#[gpui::test]
fn a_stale_answer_never_overwrites_a_newer_filter(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.run_until_parked();
    let stale = CommandId::new();
    let current = CommandId::new();
    view.update(cx, |view, cx| {
        view.preview_controls.requested = PreviewShape {
            predicate: Some("id > 10".into()),
            ..PreviewShape::default()
        };
        view.preview_active = Some((current, CancelToken::new()));
        view.complete_preview(
            stale,
            Err(OxynError::Query("near \"WERE\": syntax error".into())),
            cx,
        );
    });
    view.read_with(cx, |view, cx| {
        assert!(
            view.preview_active.is_some(),
            "the newer read is still the one being waited for"
        );
        assert_eq!(
            view.preview_state(cx),
            PreviewState::Loading,
            "an outdated failure applies to nothing"
        );
    });
}

/// Reads the first column of the preview rows, in the order they arrived.
///
/// Reads the Arrow batches directly: converting the buffer to rows would
/// prove something about the conversion rather than about the rows the
/// server returned ([ADR-0002](../../../../docs/adr/0002-arrow-result-model.md)).
fn ids(view: &Workspace, cx: &gpui::App) -> Vec<String> {
    let Some(buffer) = view.preview_grid.read(cx).state().buffer() else {
        return Vec::new();
    };
    (0..buffer.row_count())
        .filter_map(|row| {
            let (batch, offset) = buffer.locate(row)?;
            let batch = buffer.batch(batch).ok()??;
            oxyn_data::format_cell(&batch, offset, 0, &oxyn_data::FormatOptions::default())
                .text()
                .map(str::to_owned)
        })
        .collect()
}

/// Loads the columns of one relation, as the Structure tab would.
#[expect(
    clippy::disallowed_methods,
    reason = "test harness polling a wall-clock executor, not the UI thread"
)]
fn describe_relation(
    view: &Entity<Workspace>,
    cx: &mut gpui::VisualTestContext,
    path: &CatalogPath,
) {
    view.update(cx, |view, cx| {
        view.refresh_catalog(CatalogScope::Relation(path.clone()), cx);
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        cx.run_until_parked();
        if view.read_with(cx, |view, _| {
            view.catalog_active.is_none() && view.preview_controls.sort_choices.len() > 1
        }) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the relation was never described"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}
