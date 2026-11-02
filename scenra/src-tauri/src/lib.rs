mod ai;
mod capture;
mod cdp;
mod display;
mod dte;
mod editor;
mod engine;
mod hooks;
mod input;
mod locate;
mod matrix;
mod page;
mod recorder;
mod ripple;
mod script;
mod secrets;
mod session;
mod settings;
mod setup;
mod take;
mod target;
mod teach;
mod teach_code;
mod uia;
mod steps;
mod vscode;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            ripple::start()?;
            let handle = app.handle().clone();
            hooks::start(move || {
                // On the hook thread: let go of everything now, tell the UI off-thread.
                input::release_all();
                let handle = handle.clone();
                std::thread::spawn(move || {
                    use tauri::Emitter;
                    let _ = handle.emit("emergency-stop", ());
                });
            })?;
            use tauri::Manager;
            app.state::<teach::TeachState>().restore(app.handle());
            Ok(())
        })
        .manage(target::LaunchedTarget::default())
        .manage(script::CurrentScript::default())
        .manage(script::ScriptInitialState::default())
        .manage(script::CurrentSession::default())
        .manage(take::CurrentTake::default())
        .manage(setup::SetupState::default())
        .manage(teach::TeachState::default())
        .manage(editor::ExportState::default())
        .invoke_handler(tauri::generate_handler![
            capture::open_teleprompter,
            capture::resize_teleprompter,
            capture::close_teleprompter,
            capture::set_capture_excluded,
            capture::capture_status,
            capture::probe_capture,
            matrix::run_capture_matrix,
            display::display_info,
            display::set_overlay_zoom,
            display::set_capture_monitor,
            target::launch_target,
            target::launch_chrome,
            target::launch_vscode,
            target::target_profile,
            target::stop_target,
            script::ai_status,
            script::draft_script,
            script::draft_from_teach,
            script::current_script,
            script::set_script,
            steps::capture_code_position,
            script::check_scene_start,
            setup::prepare_scene,
            setup::cancel_scene_setup,
            script::reset_scene_page,
            script::approve_scene_start,
            script::start_session,
            script::current_session,
            script::approach_action,
            script::press_action,
            engine::probe_element,
            engine::click_element,
            engine::type_into,
            engine::slow_move_test,
            engine::slow_type_test,
            engine::simulate_hang,
            engine::guard_status,
            hooks::playback_guard,
            engine::clear_stop,
            engine::guard_key,
            take::take_preflight,
            take::recorder_status,
            take::start_take,
            take::stop_take,
            editor::open_editor,
            editor::list_sessions,
            editor::load_session,
            editor::save_edit,
            editor::tidy_session,
            editor::assemble_session,
            editor::editor_ai_status,
            editor::set_transcription,
            editor::export_session,
            editor::cancel_export,
            editor::reveal_in_explorer,
            editor::retake_scene,
            editor::describe_session,
            teach::start_teach,
            teach::stop_teach,
            teach::teach_state,
            teach::delete_teach_step,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                target::shutdown(app);
            }
        });
}
