mod ai;
mod capture;
mod cdp;
mod display;
mod encoding;
mod engine;
mod hooks;
mod input;
mod locate;
mod matrix;
mod mkv;
mod obs;
mod page;
mod ripple;
mod script;
mod secrets;
mod settings;
mod setup;
mod take;
mod target;
mod teach;

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
            target::launch_target,
            target::launch_chrome,
            target::target_profile,
            target::stop_target,
            script::ai_status,
            script::draft_script,
            script::draft_from_teach,
            script::current_script,
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
            take::obs_status,
            take::take_preflight,
            take::start_take,
            take::stop_take,
            take::restore_obs_profile,
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
