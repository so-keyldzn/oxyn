//! Oxyn — the modern database workspace, in a Tauri window.
//!
//! This binary starts logging, assembles the backend, and opens the window.
//! The interface lives in `apps/desktop`; the domain lives in the crates this
//! one depends on ([ADR-0029](../../../docs/adr/0029-interface-tauri-shadcn.md)).

// Without it, a release build on Windows opens a console next to the window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backend;
mod catalog;
mod commands;
mod credentials;
mod ipc;
mod logging;
#[cfg(test)]
mod sentinel_tests;

use anyhow::{Context as _, Result};

use crate::backend::Backend;

fn main() -> Result<()> {
    logging::start();

    // One runtime for Tauri and the executor. Two would mean a result produced
    // on one and awaited from the other, and a runtime dropped inside the
    // other's async context panics at exit.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("oxyn-exec")
        .build()
        .context("starting the async runtime")?;
    tauri::async_runtime::set(runtime.handle().clone());
    let _context = runtime.enter();

    // Before the window, deliberately: a failure here reaches the user on
    // stderr. A window that opens onto a broken backend is worse.
    let temporary = std::env::args()
        .skip(1)
        .any(|arg| arg == "--temporary-workspace");
    let backend = if temporary {
        Backend::open_temporary()
    } else {
        Backend::open()
    }
    .context("starting Oxyn")?;

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(backend)
        // Grouped by feature. A feature module adds its block here and nowhere
        // else: a command missing from this list fails silently in the front.
        .invoke_handler(tauri::generate_handler![
            // Connections
            commands::list_drivers,
            commands::list_connections,
            commands::connect,
            commands::decide_connection,
            commands::reconnect,
            commands::disconnect,
            // Execution and results
            commands::execute,
            commands::decide,
            commands::cancel,
            commands::subscribe_events,
            // Metadata: catalog, relation facets, shaped preview
            commands::metadata::preview_relation,
            commands::metadata::preview_pagination,
            commands::metadata::refresh_catalog,
            commands::metadata::refresh_relation_facet,
            commands::metadata::catalog_tree,
            commands::metadata::relation_detail,
            commands::metadata::relation_facets,
            commands::metadata::search_catalog,
            commands::metadata::related_rows_template,
            commands::metadata::subscribe_refresh_signals,
            // Results: pages, search, value inspection, export
            commands::results::read_result_page,
            commands::results::result_columns,
            commands::results::forget_result,
            commands::results::export_formats,
            commands::results::export_result,
            commands::results::find_in_result,
            commands::results::find_matches_in_window,
            commands::results::inspect_value,
            // Consoles
            commands::consoles::open_console,
            commands::consoles::close_console,
            commands::consoles::run_console,
            commands::consoles::set_session_context,
            commands::consoles::session_context_choices,
            commands::consoles::validate_parameters,
            // Documents and history
            commands::library::new_query_document,
            commands::library::save_query_document,
            commands::library::close_query_document,
            commands::library::release_query_document,
            commands::library::delete_query_document,
            commands::library::open_query_document,
            commands::library::list_query_documents,
            commands::library::read_history,
            commands::library::read_history_entry,
            commands::library::list_history_connections,
            commands::library::open_retained_result,
            // Recovery and shutdown
            commands::recovery::recovery_status,
            commands::recovery::subscribe_shutdown,
            commands::recovery::shutdown_flushed,
            // Settings: preferences and saved-connection management
            commands::settings::read_preferences,
            commands::settings::write_preferences,
            commands::settings::connection_details,
            commands::settings::update_connection,
            commands::settings::delete_connection,
            commands::settings::decide_connection_change,
            commands::settings::connection_marking,
            // AI: declarations, conversation, schema-change proposal
            commands::ai::ai_providers,
            commands::ai::ai_external_agents,
            commands::ai::ai_save_provider,
            commands::ai::ai_remove_provider,
            commands::ai::ai_provider_models,
            commands::ai::ai_save_external_agent,
            commands::ai::ai_remove_external_agent,
            commands::ai::ai_agent_presets,
            commands::ai::ai_detect_agent,
            commands::ai::ai_ask,
            commands::ai::ai_cancel,
            commands::ai::ai_threads,
            commands::ai::ai_open_thread,
            commands::ai::ai_rename_thread,
            commands::ai::ai_delete_thread,
            commands::ai::ai_select_version,
            commands::ai::ai_authenticate,
            commands::ai::ai_set_agent_setting,
            commands::ai::ai_start_agent,
            commands::ai::ai_stop_agent_start,
            commands::ai::ai_request_sample,
            commands::ai::ai_withdraw_sample,
            commands::ai::ai_answer_sample,
            commands::ai::ai_forget,
            commands::ai::ai_list_mentionable,
            commands::ai::ai_propose_schema_change,
        ])
        .build(tauri::generate_context!())
        .context("building the Tauri application")?
        // Closing waits for drafts and local writes before recording the
        // session close (ADR-0021).
        .run(|app, event| commands::recovery::on_run_event(app, &event));

    Ok(())
}
