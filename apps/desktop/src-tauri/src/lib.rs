//! PageLamp desktop shell.
//!
//! ─── Tauri boundary ──────────────────────────────────────────────────────────────────────────
//! The React UI (apps/desktop/src) calls the commands in `commands.rs` through
//! `src/api/tauri.ts`. Each command is a thin wrapper over one `pagelamp_app::App` method —
//! NO business logic lives here (docs/ARCHITECTURE.md §6). If the UI needs something new, it
//! is added to the facade in crates/pagelamp-app first.
//!
//! Permissions granted to the webview are in `capabilities/default.json`: the folder picker
//! (`dialog:allow-open`) and opening http(s) links. No shell and no filesystem access; the
//! data and logs folders are opened by commands that take no path from the UI.
//! ─────────────────────────────────────────────────────────────────────────────────────────────

pub mod backend;
pub mod background;
mod commands;
pub mod reminders;
pub mod updates;
pub mod window;

pub use backend::Backend;

use tauri::Manager;

/// Every command the UI can call (see `src/api/tauri.ts`). Kept apart from `run` so the IPC
/// contract test (`tests/ipc_contract.rs`) builds the app with exactly the same handlers.
pub fn with_commands<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.invoke_handler(tauri::generate_handler![
        commands::status,
        commands::activity,
        commands::list_sources,
        commands::add_canvas_source,
        commands::add_folder_source,
        commands::add_ical_source,
        commands::update_source_secret,
        commands::remove_source,
        commands::sync_all,
        commands::sync_source,
        commands::download_course_files,
        commands::cancel_sync,
        commands::list_courses,
        commands::course_overview,
        commands::week_materials,
        commands::list_deadlines,
        commands::search,
        commands::latest_study_plan,
        commands::plan_limits,
        commands::set_course_policy,
        commands::set_course_term,
        commands::set_course_ai_access,
        commands::set_course_hidden,
        commands::set_course_material_sharing,
        commands::keep_course_current,
        commands::clear_keep_course_current,
        commands::confirm_course_dates,
        commands::set_course_dates,
        commands::lifecycle_summary,
        commands::snooze_lifecycle_banner,
        commands::snooze_removal_suggestions,
        commands::snooze_calendar_offers,
        commands::clear_removal_snooze,
        commands::removal_preview,
        commands::remove_courses,
        commands::removed_courses,
        commands::restore_course,
        commands::purge_removed_courses,
        commands::forget_removed_course,
        commands::course_calendar,
        commands::set_calendar_sources,
        commands::download_material_files,
        commands::scan_course_calendar,
        commands::accept_calendar_proposal,
        commands::accept_passing_proposals,
        commands::dismiss_calendar_proposal,
        commands::syllabus_reading_offers,
        commands::read_course_calendar,
        commands::read_course_calendars,
        commands::cancel_generation,
        commands::explain_week,
        commands::saved_explanations,
        commands::delete_explanation,
        commands::ai_output_language,
        commands::set_ai_output_language,
        commands::generate_study_plan,
        commands::accept_study_plan,
        commands::set_study_plan_item_done,
        commands::write_weekly_note,
        commands::weekly_notes,
        commands::delete_weekly_note,
        commands::weekly_note_settings,
        commands::set_prepare_weekly_note_on_monday,
        commands::mcp_client_configs,
        commands::diagnostic_report,
        commands::doctor,
        commands::last_crash,
        commands::clear_last_crash,
        commands::open_material,
        commands::reveal_material,
        commands::reveal_data_dir,
        commands::reveal_logs_dir,
        commands::log_ui_error,
        commands::update_prefs,
        commands::set_update_prefs,
        commands::effective_update_channel,
        commands::startup_tasks,
        commands::acknowledge_whats_new,
        commands::acknowledge_update_disclosure,
        commands::last_update_check,
        commands::ai_status,
        commands::model_provider_presets,
        commands::add_model_provider,
        commands::update_model_provider_key,
        commands::remove_model_provider,
        commands::detect_local_servers,
        commands::list_models,
        commands::test_model,
        commands::set_feature_model,
        commands::acknowledge_ai_disclosure,
        commands::acknowledge_unpriced_model,
        commands::set_monthly_budget,
        commands::estimate_generation,
        commands::usage_summary,
        commands::remove_all_ai_data,
        commands::codex_status,
        commands::install_codex,
        commands::cancel_codex_install,
        commands::remove_codex,
        commands::codex_login,
        commands::cancel_codex_login,
        commands::codex_logout,
        commands::set_mode_a_weekly_cap,
        commands::set_codex_source,
        background::background_status,
        background::set_tray_labels,
        reminders::reminder_settings,
        reminders::set_reminder_settings,
        reminders::due_reminders,
        reminders::show_reminders,
        reminders::mark_reminders_shown,
        reminders::show_reminders_on_notice,
        reminders::open_notification_settings,
        updates::updates_status,
        updates::updates_check,
        updates::updates_install,
    ])
}

pub fn run() {
    // First of all: log files, redacted stderr and the panic hook, so a failure while opening
    // the core below is logged and a crash is recorded for the next launch's notice.
    pagelamp_app::diagnostics::init(pagelamp_app::diagnostics::ProcessKind::App, false);
    updates::remove_old_sidecar();
    let context = tauri::generate_context!();
    let identifier = context.config().identifier.clone();
    let hidden = background::launch_hidden(std::env::args(), &identifier);
    let mut builder = tauri::Builder::default();
    // First: a second launch hands over to the running PageLamp before anything else starts.
    if let Some(single_instance) = background::single_instance_plugin() {
        builder = builder.plugin(single_instance);
    }
    let builder = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(updates::plugin())
        .plugin(background::autostart_plugin(&identifier))
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            updates::manage(app.handle());
            // Built here, not from the config, so Windows 11 can get Mica (window.rs).
            window::create_main(app)?;
            // The stored "Remind me" answer; unknown while the core can't open.
            let run_in_background = app
                .state::<Backend>()
                .app()
                .and_then(|facade| facade.reminder_settings())
                .map(|settings| settings.run_in_background)
                .ok();
            background::start(app.handle(), run_in_background);
            reminders::start(app.handle().clone());
            #[cfg(debug_assertions)]
            background::debug_restart_once(app.handle());
            Ok(())
        })
        .manage(Backend::open())
        .manage(background::Background::new(hidden));
    let app = with_commands(builder).build(context).unwrap_or_else(|err| {
        panic!(
            "error while running the {} desktop app: {err}",
            pagelamp_core::brand::PRODUCT_NAME
        )
    });
    app.run(|_app, _event| {
        // The plugins have seen Exit (the single-instance lock is gone): an update's restart.
        #[cfg(target_os = "linux")]
        if let tauri::RunEvent::Exit = _event {
            updates::relaunch_if_asked(_app);
        }
        #[cfg(target_os = "macos")]
        match _event {
            // macOS doesn't start a second PageLamp when it's opened again (Finder, the Dock,
            // Spotlight, a login item): the running one gets Reopen. Show its window, as the
            // single-instance hand-over does elsewhere; a window in the tray would stay hidden.
            tauri::RunEvent::Reopen { .. } => {
                tracing::info!(target: "pagelamp::background", "reopened: showing the window");
                background::show_main(_app);
            }
            // The plugins have seen Exit (the single-instance lock is gone): an update's restart.
            tauri::RunEvent::Exit => updates::relaunch_if_asked(_app),
            _ => {}
        }
    });
}
