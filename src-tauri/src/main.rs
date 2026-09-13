#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod controller;
mod engine;
mod settings;
mod updates;

use controller::Controller;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_opener::OpenerExt;

struct Desktop {
    queue: Arc<Controller>,
    updater: updates::Updater,
    profile: Value,
    version: String,
}

fn clipper_path() -> Option<PathBuf> {
    let mut paths = vec![];
    #[cfg(windows)]
    {
        if let Ok(key)=winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{A67FDBB5-CF45-489D-8A41-0E7576A446F1}_is1") {
            if let Ok(folder)=key.get_value::<String,_>("InstallLocation"){paths.push(PathBuf::from(folder).join("FlipperClipper.exe"));}
        }
        for key in ["LOCALAPPDATA", "ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(root) = std::env::var_os(key) {
                paths.push(PathBuf::from(&root).join("Programs/FlipperClipper/FlipperClipper.exe"));
                paths.push(PathBuf::from(root).join("FlipperClipper/FlipperClipper.exe"));
            }
        }
    }
    if let Some(value) = std::env::var_os("PATH") {
        for folder in std::env::split_paths(&value) {
            paths.push(folder.join(if cfg!(windows) {
                "FlipperClipper.exe"
            } else {
                "flipperclipper"
            }));
        }
    }
    #[cfg(target_os = "macos")]
    paths.push(PathBuf::from(
        "/Applications/FlipperClipper.app/Contents/MacOS/flipperclipper",
    ));
    paths.into_iter().find(|p| p.is_file())
}

fn snapshot(app: &tauri::AppHandle, state: &Desktop) -> Value {
    let mut result = state.queue.snapshot();
    result["branding"] = state.profile.clone();
    result["version"] = json!(state.version);
    result["tools"] = engine::find_tools();
    result["flipperclipper"] = json!(clipper_path().is_some());
    result["updates"] = state.updater.snapshot();
    for key in ["source", "path"] {
        if let Some(path) = result["preview"][key].as_str().filter(|p| !p.is_empty()) {
            if Path::new(path).is_file() {
                let _ = app.asset_protocol_scope().allow_file(path);
            }
        }
    }
    result
}

fn open_window(
    app: &tauri::AppHandle,
    label: &str,
    title: &str,
    width: f64,
    height: f64,
    min_width: f64,
    min_height: f64,
) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(label) {
        window.show().map_err(|e| e.to_string())?;
        let _ = window.unminimize();
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }
    WebviewWindowBuilder::new(app, label, WebviewUrl::App(format!("{label}.html").into()))
        .title(title)
        .inner_size(width, height)
        .min_inner_size(min_width, min_height)
        .center()
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn arg(args: &Value, index: usize) -> Result<&str, String> {
    args.get(index)
        .and_then(Value::as_str)
        .ok_or_else(|| "Invalid desktop request.".into())
}

fn dispatch(app: &tauri::AppHandle, method: &str, args: Value) -> Result<Value, String> {
    let state = app.state::<Desktop>();
    let queue = &state.queue;
    match method {
        "get_state" => return Ok(snapshot(app, &state)),
        "get_updates" => return Ok(state.updater.snapshot()),
        "add_paths" => {
            let values = args[0]
                .as_array()
                .ok_or("Choose files or a folder from this computer.")?;
            let paths = values
                .iter()
                .map(|v| v.as_str().map(str::to_owned).ok_or("Invalid file path."))
                .collect::<Result<Vec<_>, _>>()?;
            queue.add_paths(paths);
        }
        "add_files" => {
            if let Some(files) = app
                .dialog()
                .file()
                .add_filter(
                    "Media",
                    &[
                        "mp4", "mkv", "mov", "webm", "avi", "m4v", "wmv", "mp3", "m4a", "aac",
                        "wav", "flac", "ogg", "opus", "wma", "jpg", "jpeg", "png", "webp", "bmp",
                        "tif", "tiff", "gif", "avif",
                    ],
                )
                .add_filter("All files", &["*"])
                .blocking_pick_files()
            {
                let paths = files
                    .into_iter()
                    .filter_map(|p| p.into_path().ok())
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                queue.add_paths(paths);
            }
        }
        "add_folder" | "choose_output" => {
            if let Some(folder) = app.dialog().file().blocking_pick_folder() {
                let folder = folder.into_path().map_err(|e| e.to_string())?;
                if method == "add_folder" {
                    queue.add_paths(vec![folder.to_string_lossy().into_owned()]);
                } else {
                    queue.update_settings(json!({"output_dir":folder.to_string_lossy()}));
                }
            }
        }
        "use_source_folder" => queue.update_settings(json!({"output_dir":""})),
        "update_settings" => queue.update_settings(args[0].clone()),
        "start_queue" => queue.start_queue(None),
        "cancel_current" => queue.cancel_current(),
        "stop_queue" => queue.stop_queue(),
        "remove_job" | "retry_job" => queue.mutate_job(method, arg(&args, 0)?),
        "clear_finished" => queue.mutate_job(method, ""),
        "rerun_job" => queue.rerun(arg(&args, 0)?, arg(&args, 1)?),
        "start_preview" => queue.start_preview(
            arg(&args, 0)?,
            args.get(1)
                .map_or(Some(0.0), Value::as_f64)
                .ok_or("Invalid preview start.")?,
            args.get(2)
                .map_or(Some(5.0), Value::as_f64)
                .ok_or("Invalid preview duration.")?,
        ),
        "cancel_preview" => queue.cancel_preview(),
        "open_preview" | "preview_source" | "open_output" | "show_output" | "open_in_clipper" => {
            let path = if method == "open_preview" {
                queue.preview_path()?
            } else {
                queue.job_path(arg(&args, 0)?, method != "preview_source")?
            };
            if method == "show_output" {
                app.opener()
                    .reveal_item_in_dir(path)
                    .map_err(|e| e.to_string())?;
            } else if method == "open_in_clipper" {
                let exe = clipper_path().ok_or("FlipperClipper is not installed.")?;
                let mut command = std::process::Command::new(exe);
                command.arg(path);
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    command.creation_flags(0x08000000);
                }
                command.spawn().map_err(|e| e.to_string())?;
            } else {
                app.opener()
                    .open_path(path.to_string_lossy(), None::<&str>)
                    .map_err(|e| e.to_string())?;
            }
            return Ok(json!({"ok":true}));
        }
        "open_updates" => {
            open_window(
                app,
                "updates",
                &format!(
                    "Updates - {}",
                    state.profile["display_name"]
                        .as_str()
                        .unwrap_or("BlubberBound")
                ),
                740.0,
                700.0,
                650.0,
                540.0,
            )?;
            return state
                .updater
                .action("check_updates", json!([]), queue.busy());
        }
        "close_updates" => {
            if let Some(window) = app.get_webview_window("updates") {
                window.close().map_err(|e| e.to_string())?;
            }
            return Ok(state.updater.snapshot());
        }
        "read_whats_new" => {
            open_window(
                app,
                "notes",
                &format!(
                    "What's new - {}",
                    state.profile["display_name"]
                        .as_str()
                        .unwrap_or("BlubberBound")
                ),
                640.0,
                560.0,
                440.0,
                350.0,
            )?;
            return Ok(state.updater.snapshot());
        }
        "close_whats_new" => {
            if let Some(window) = app.get_webview_window("notes") {
                window.close().map_err(|e| e.to_string())?;
            }
            return state
                .updater
                .action("dismiss_whats_new", json!([]), queue.busy());
        }
        "check_updates"
        | "set_update_channel"
        | "set_automatic_updates"
        | "install_update"
        | "disarm_downgrade"
        | "dismiss_update"
        | "open_update_page"
        | "dismiss_whats_new" => {
            let result = state.updater.action(method, args, queue.busy())?;
            if let Some(url) = result["open_url"].as_str() {
                app.opener()
                    .open_url(url, None::<&str>)
                    .map_err(|e| e.to_string())?;
                return Ok(json!({"ok":true}));
            }
            return Ok(result);
        }
        _ => return Err("Unknown desktop request.".into()),
    }
    Ok(snapshot(app, &state))
}

#[tauri::command]
async fn desktop_command(
    app: tauri::AppHandle,
    method: String,
    args: Value,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || dispatch(&app, &method, args))
        .await
        .map_err(|e| e.to_string())?
}

fn main() {
    let profile: Value =
        serde_json::from_str(include_str!("../../app_profile.json")).expect("Invalid app profile");
    let version = option_env!("SQUEEZE_BUILD_VERSION")
        .unwrap_or(include_str!("../../version.txt").trim())
        .to_string();
    let mut args = std::env::args().skip(1);
    let mut files = vec![];
    let mut state_override = None;
    while let Some(value) = args.next() {
        if value == "--state-dir" {
            state_override = args.next().map(PathBuf::from);
        } else if value != "--debug" {
            files.push(value);
        }
    }
    tauri::Builder::default().plugin(tauri_plugin_dialog::init()).plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![desktop_command])
        .setup(move|app|{
            let directory=if let Some(directory)=state_override {directory} else {
                let base=std::env::var_os("LOCALAPPDATA").map(PathBuf::from).or_else(||app.path().local_data_dir().ok()).unwrap_or_else(std::env::temp_dir);
                base.join(profile["storage_id"].as_str().unwrap_or("BlubberBound"))
            };
            std::fs::create_dir_all(&directory)?;
            let queue=Controller::new(directory.join("state.json"));if !files.is_empty(){queue.add_paths(files);}
            let updater=updates::Updater::new(profile.clone(),directory,version.clone());
            if let Some(identity)=option_env!("SQUEEZE_BUILD_IDENTITY_JSON").and_then(|value|serde_json::from_str(value).ok()){updater.set_identity(identity);}
            updater.start();
            if let Some(window)=app.get_webview_window("main"){window.set_title(profile["display_name"].as_str().unwrap_or("BlubberBound"))?;}
            app.manage(Desktop{queue,updater,profile,version});
            let handle=app.handle().clone();std::thread::spawn(move||loop{
                std::thread::sleep(Duration::from_millis(500));let state=handle.state::<Desktop>();
                if let Some(path)=state.updater.poll_ready_install(){if state.queue.reserve_install(){match state.updater.launch_ready_install(&path){Ok(true)=>{state.updater.close();state.queue.close();handle.exit(0);break;},_=>state.queue.release_install()}}else{state.updater.defer_install();}}
            });
            Ok(())
        })
        .on_window_event(|window,event|{
            if let tauri::WindowEvent::CloseRequested{api,..}=event{
                if window.label()=="main"{
                    api.prevent_close();let handle=window.app_handle().clone();
                    tauri::async_runtime::spawn_blocking(move||{let state=handle.state::<Desktop>();if state.queue.busy()&&!handle.dialog().message("Closing the app cancels the current compression or preview. Pending files will be kept for next time.").title("Stop compression?").kind(MessageDialogKind::Warning).buttons(MessageDialogButtons::OkCancel).blocking_show(){return;}state.updater.close();state.queue.close();handle.exit(0);});
                }else if window.label()=="notes"{let state=window.state::<Desktop>();let _=state.updater.action("dismiss_whats_new",json!([]),state.queue.busy());}
            }
        }).run(tauri::generate_context!()).expect("Desktop application failed");
}
