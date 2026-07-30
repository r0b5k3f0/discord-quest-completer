// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
use std::env;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use tauri::async_runtime::JoinHandle;
use tauri::{path::BaseDirectory, AppHandle, Emitter, Listener, Manager};

mod gamelist;
mod rpc;
mod runner;
mod steam;

const EVENT_CONNECTING: &str = "client_connecting";
const EVENT_CONNECTED: &str = "client_connected";
const EVENT_ERROR: &str = "client_error";
const EVENT_DISCONNECT: &str = "event_disconnect";

/// The single live Discord RPC session, if any.
static DISCORD_CLIENT: Mutex<Option<rpc::Client>> = Mutex::new(None);
/// The in-flight connect task, so a disconnect can cancel a pending connect.
static CONNECT_TASK: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

/// A poisoned mutex here only means a previous connect panicked; the data behind
/// it stays perfectly usable, so recover instead of cascading the panic.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Takes the current client out of the global slot and closes its connection.
/// A no-op when nothing is connected.
async fn disconnect_current_client() {
    // The guard must be dropped before the await, hence the inner scope.
    let client = { lock(&DISCORD_CLIENT).take() };
    if let Some(client) = client {
        client.discord.disconnect().await;
        println!("Disconnected from Discord RPC");
    }
}

fn abort_pending_connect() {
    if let Some(task) = lock(&CONNECT_TASK).take() {
        task.abort();
    }
}

/// Resolves `<exe dir>/games/<app_id>/<path>`, the folder a dummy executable
/// lives in.
fn game_folder_path(app_id: i64, path: &str) -> std::path::PathBuf {
    // Must be created next to the executable to avoid permission issues.
    let exe_path = env::current_exe().unwrap_or_default();
    let exe_dir = exe_path
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .to_path_buf();

    exe_dir
        .join("games")
        .join(app_id.to_string())
        .join(Path::new(path).to_string_lossy().to_string())
}

#[tauri::command(rename_all = "snake_case")]
async fn create_fake_game(
    handle: tauri::AppHandle,
    path: &str,
    executable_name: &str,
    app_id: i64,
) -> Result<String, String> {
    let game_folder_path = game_folder_path(app_id, path);
    let target_executable_path = game_folder_path.join(executable_name);

    println!("Game folder path: {:?}", game_folder_path);
    println!("Game full path: {:?}", target_executable_path);

    std::fs::create_dir_all(&game_folder_path)
        .map_err(|e| format!("Failed to create game folder: {}", e))?;

    // Copy the dummy executable into the folder we just created. It ships
    // alongside the final build as `data/src-win.exe`.
    let resource_path = handle
        .path()
        .resolve("data/src-win.exe", BaseDirectory::Resource)
        .map_err(|e| format!("Failed to resolve the dummy executable resource: {}", e))?;

    println!("Creating dummy game executable: {:?}", resource_path);
    std::fs::copy(&resource_path, &target_executable_path)
        .map_err(|e| format!("Failed to copy dummy executable: {}", e))?;

    Ok(format!(
        "Dummy executable copied to: {:?}",
        target_executable_path
    ))
}

#[tauri::command(rename_all = "snake_case")]
async fn run_background_process(
    name: &str,
    path: &str,
    executable_name: &str,
    app_id: i64,
) -> Result<String, String> {
    let game_folder_path = game_folder_path(app_id, path);
    let executable_path = game_folder_path.join(executable_name);

    std::process::Command::new(&executable_path)
        .args(["--title", name])
        .current_dir(game_folder_path) // Set working directory to the game folder
        .spawn()
        .map_err(|e| format!("Failed to start process: {}", e))?;

    Ok("Process started successfully".to_string())
}

#[tauri::command(rename_all = "snake_case")]
async fn stop_process(exec_name: String) -> Result<(), String> {
    // Stop the process using taskkill command
    let output = std::process::Command::new("taskkill")
        .arg("/F")
        .arg("/IM")
        .arg(exec_name)
        .output()
        .map_err(|e| format!("Failed to execute taskkill: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Failed to stop process: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

/// Connects to (or disconnects from) Discord's RPC socket.
///
/// Connecting is asynchronous: this returns as soon as the attempt has been
/// scheduled, and the outcome is reported through the `client_connected` /
/// `client_error` events.
///
/// Usage from JS:
/// ```javascript
/// await invoke('connect_to_discord_rpc_3', { activity_json, action: 'connect' | 'disconnect' });
/// ```
#[tauri::command(rename_all = "snake_case")]
fn connect_to_discord_rpc_3(
    handle: AppHandle,
    activity_json: String,
    action: String,
) -> Result<(), String> {
    if action == "disconnect" {
        abort_pending_connect();
        tauri::async_runtime::spawn(disconnect_current_client());
        return Ok(());
    }

    // Validate up front so a malformed payload rejects the invoke call instead
    // of panicking inside the spawned task, where nothing can observe it.
    let app_id = runner::parse_activity_json(&activity_json)?.app_id;

    abort_pending_connect();

    let task = tauri::async_runtime::spawn(async move {
        // Close any previous session first. Dropping the client without
        // disconnecting leaves the old Discord IPC connection open.
        disconnect_current_client().await;

        let _ = handle.emit(EVENT_CONNECTING, serde_json::json!({ "app_id": &app_id }));

        match runner::set_activity(activity_json).await {
            Ok(client) => {
                *lock(&DISCORD_CLIENT) = Some(client);
                let _ = handle.emit(EVENT_CONNECTED, serde_json::json!({ "app_id": &app_id }));
            }
            Err(e) => {
                eprintln!("Failed to set activity: {}", e);
                let _ = handle.emit(
                    EVENT_ERROR,
                    serde_json::json!({ "app_id": &app_id, "message": e }),
                );
            }
        }
    });

    *lock(&CONNECT_TASK) = Some(task);
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // No `tauri_plugin_http::init()`: the frontend never calls the plugin's
        // fetch commands. The Rust side only uses the reqwest re-export, which
        // does not need the plugin to be registered.
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Registered exactly once for the lifetime of the app. Registering
            // it per connect meant a single disconnect event fired one handler
            // per connect that had ever happened.
            app.handle().listen(EVENT_DISCONNECT, |_| {
                println!("Disconnecting from Discord RPC...");
                abort_pending_connect();
                tauri::async_runtime::spawn(disconnect_current_client());
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            create_fake_game,
            stop_process,
            connect_to_discord_rpc_3,
            run_background_process,
            gamelist::fetch_gamelist,
            steam::find_steam_libraries,
            steam::install_steam_dummy_game,
            steam::get_steam_dummy_status,
            steam::remove_steam_dummy_game,
            steam::run_steam_dummy_game
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
