//! Steam workaround support (see GitHub issue #48).
//!
//! Discord's quest/game detection for newer titles no longer relies on the
//! `executables` field of the detectable-games list. Instead, it looks at the
//! installation state reported by distribution platforms such as Steam.
//!
//! The community-verified workaround is:
//!   1. Create a folder under `<Steam>/steamapps/common/<install_dir>/`.
//!   2. Place the dummy (renamed) executable inside that folder.
//!   3. Drop an `appmanifest_<app_id>.acf` file in `<Steam>/steamapps/`
//!      describing the fake installation (appid, name, installdir).
//!   4. Run the dummy executable directly from that folder.
//!
//! This module implements the above in a *safe* way:
//!   - It never overwrites an existing `appmanifest_*.acf` (that would mean a
//!     real installation or leftovers we do not own).
//!   - It never touches a `common/<install_dir>` folder that contains files
//!     we did not create (tracked through a `.dqc-steam.json` marker file).
//!   - Input values are validated (no path traversal, only sane characters).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::Manager;

/// Marker file placed inside the fake game folder. Lets us know which files
/// were created by us, so uninstall never deletes anything else.
const MARKER_FILE: &str = ".dqc-steam.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SteamLibrary {
    /// Root of the Steam installation/library (folder that contains steamapps).
    pub root: String,
    /// Full path of the `steamapps` folder for this library.
    pub steamapps: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SteamDummyStatus {
    pub installed: bool,
    pub acf_exists: bool,
    pub marker_exists: bool,
    pub folder: String,
    pub exes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DummyMarker {
    app_id: u64,
    game_name: String,
    install_dir: String,
    exes: Vec<String>,
    created_at_unix: u64,
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

/// Names (install dir / exe names) must be plain file system segments:
/// no separators and no characters that are illegal on Windows.
fn is_valid_segment(name: &str) -> bool {
    if name.is_empty() || name.len() > 128 {
        return false;
    }
    if name == "." || name == ".." {
        return false;
    }
    let illegal = ['>', '<', ':', '"', '|', '?', '*', '/', '\\'];
    !name.chars().any(|c| illegal.contains(&c) || c.is_control())
}

fn is_valid_exe_name(name: &str) -> bool {
    is_valid_segment(name) && name.to_lowercase().ends_with(".exe")
}

fn is_valid_steamapps_path(path: &str) -> bool {
    if path.is_empty() || path.contains("..") {
        return false;
    }
    // The folder must actually be named "steamapps" (case-insensitive) so a
    // typo can't make us write acf files into random places on disk.
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase() == "steamapps")
        .unwrap_or(false)
}

fn normalize_slashes(p: &str) -> String {
    p.replace('/', "\\")
}

// ---------------------------------------------------------------------------
// Steam library discovery
// ---------------------------------------------------------------------------

/// Read `steamapps/libraryfolders.vdf` and return every `steamapps` path of
/// the additional libraries registered in it.
fn parse_libraryfolders_vdf(steam_root: &Path) -> Vec<PathBuf> {
    let vdf_path = steam_root.join("steamapps").join("libraryfolders.vdf");
    let content = match std::fs::read_to_string(&vdf_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut roots = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        // Lines look like: "path"		"D:\\Games\\Steam"
        if let Some(rest) = line.strip_prefix("\"path\"") {
            let rest = rest.trim().trim_matches('"');
            // unescape the double backslashes used by the VDF format
            let unescaped = rest.replace("\\\\", "\\");
            let candidate = PathBuf::from(&unescaped);
            if !candidate.as_os_str().is_empty() {
                roots.push(candidate);
            }
        }
    }
    roots
}

/// Candidate Steam install roots gathered from the registry and well-known
/// default locations.
#[cfg(target_os = "windows")]
fn steam_root_candidates() -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    // Registry: HKCU\Software\Valve\Steam -> SteamPath
    if let Ok(hkcu) = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey("Software\\Valve\\Steam")
    {
        if let Ok(steam_path) = hkcu.get_value::<String, _>("SteamPath") {
            candidates.push(PathBuf::from(steam_path));
        }
    }

    // Registry: HKLM\SOFTWARE\WOW6432Node\Valve\Steam -> InstallPath
    if let Ok(hklm) = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
        .open_subkey("SOFTWARE\\WOW6432Node\\Valve\\Steam")
    {
        if let Ok(install_path) = hklm.get_value::<String, _>("InstallPath") {
            candidates.push(PathBuf::from(install_path));
        }
    }

    candidates.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
    candidates.push(PathBuf::from(r"C:\Program Files\Steam"));
    candidates
}

#[cfg(not(target_os = "windows"))]
fn steam_root_candidates() -> Vec<PathBuf> {
    Vec::new()
}

#[tauri::command(rename_all = "snake_case")]
pub async fn find_steam_libraries() -> Result<Vec<SteamLibrary>, String> {
    let mut seen: Vec<String> = Vec::new();
    let mut libraries: Vec<SteamLibrary> = Vec::new();

    let mut roots = steam_root_candidates();

    // Also include libraries registered in each root's libraryfolders.vdf
    let roots_snapshot = roots.clone();
    for root in roots_snapshot {
        roots.extend(parse_libraryfolders_vdf(&root));
    }

    for root in roots {
        let steamapps = root.join("steamapps");
        if !steamapps.is_dir() {
            continue;
        }
        let key = steamapps.to_string_lossy().to_lowercase();
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        libraries.push(SteamLibrary {
            root: root.to_string_lossy().to_string(),
            steamapps: steamapps.to_string_lossy().to_string(),
        });
    }

    Ok(libraries)
}

// ---------------------------------------------------------------------------
// acf (app manifest) generation
// ---------------------------------------------------------------------------

fn escape_acf_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "")
}

fn build_acf_content(
    app_id: u64,
    game_name: &str,
    install_dir: &str,
    steam_root: &Path,
    size_on_disk: u64,
) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let launcher_path = normalize_slashes(&steam_root.join("steam.exe").to_string_lossy());

    // StateFlags "4" = fully installed. Keep the file small and plausible so
    // neither Steam nor Discord decides to "repair" or re-download anything.
    format!(
        "\"AppState\"\n{{\n\
        \t\"appid\"\t\t\"{app_id}\"\n\
        \t\"universe\"\t\t\"1\"\n\
        \t\"LauncherPath\"\t\t\"{launcher}\"\n\
        \t\"name\"\t\t\"{name}\"\n\
        \t\"StateFlags\"\t\t\"4\"\n\
        \t\"installdir\"\t\t\"{installdir}\"\n\
        \t\"LastUpdated\"\t\t\"{now}\"\n\
        \t\"LastPlayed\"\t\t\"{now}\"\n\
        \t\"SizeOnDisk\"\t\t\"{size}\"\n\
        \t\"StagingSize\"\t\t\"0\"\n\
        \t\"buildid\"\t\t\"1\"\n\
        \t\"DownloadType\"\t\t\"4\"\n\
        \t\"UpdateResult\"\t\t\"0\"\n\
        \t\"BytesToDownload\"\t\t\"0\"\n\
        \t\"BytesDownloaded\"\t\t\"0\"\n\
        \t\"BytesToStage\"\t\t\"0\"\n\
        \t\"BytesStaged\"\t\t\"0\"\n\
        \t\"AutoUpdateBehavior\"\t\t\"0\"\n\
        \t\"AllowOtherDownloadsWhileRunning\"\t\t\"0\"\n\
        \t\"UserConfig\"\n\t{{\n\
        \t\t\"language\"\t\t\"english\"\n\
        \t}}\n\
        \t\"MountedConfig\"\n\t{{\n\
        \t\t\"language\"\t\t\"english\"\n\
        \t}}\n\
        }}\n",
        app_id = app_id,
        launcher = escape_acf_value(&launcher_path),
        name = escape_acf_value(game_name),
        installdir = escape_acf_value(install_dir),
        now = now,
        size = size_on_disk,
    )
}

// ---------------------------------------------------------------------------
// Install / status / uninstall / run
// ---------------------------------------------------------------------------

fn marker_path(game_folder: &Path) -> PathBuf {
    game_folder.join(MARKER_FILE)
}

fn read_marker(game_folder: &Path) -> Option<DummyMarker> {
    let content = std::fs::read_to_string(marker_path(game_folder)).ok()?;
    serde_json::from_str(&content).ok()
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Files inside a folder we manage. `Ok(true)` when every file in the folder
/// is something we created (dummy exes listed in the marker + marker itself).
fn folder_contains_only_our_files(game_folder: &Path, marker: &DummyMarker) -> bool {
    let entries = match std::fs::read_dir(game_folder) {
        Ok(e) => e,
        Err(_) => return true, // cannot read -> treat as empty/absent
    };

    let mut allowed: Vec<String> = marker
        .exes
        .iter()
        .map(|e| e.to_lowercase())
        .collect();
    allowed.push(MARKER_FILE.to_lowercase());

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if !allowed.contains(&name) {
            return false;
        }
    }
    true
}

pub struct InstallParams<'a> {
    pub steamapps: &'a str,
    pub app_id: u64,
    pub game_name: &'a str,
    pub install_dir: &'a str,
    pub exe_names: Vec<String>,
}

#[tauri::command(rename_all = "snake_case")]
pub async fn install_steam_dummy_game(
    handle: tauri::AppHandle,
    steamapps: String,
    app_id: u64,
    game_name: String,
    install_dir: String,
    exe_names: Vec<String>,
) -> Result<SteamDummyStatus, String> {
    let params = InstallParams {
        steamapps: &steamapps,
        app_id,
        game_name: &game_name,
        install_dir: &install_dir,
        exe_names,
    };

    // ---- validation -------------------------------------------------------
    if !is_valid_steamapps_path(params.steamapps) {
        return Err(format!(
            "Invalid steamapps path: '{}'. The folder must be named 'steamapps'.",
            params.steamapps
        ));
    }
    if !is_valid_segment(params.install_dir) {
        return Err(format!("Invalid install dir: '{}'", params.install_dir));
    }
    if params.exe_names.is_empty() {
        return Err("At least one executable name is required".to_string());
    }
    for exe in &params.exe_names {
        if !is_valid_exe_name(exe) {
            return Err(format!(
                "Invalid executable name: '{}'. It must be a plain file name ending with .exe",
                exe
            ));
        }
    }

    let steamapps_path = PathBuf::from(normalize_slashes(params.steamapps));
    if !steamapps_path.is_dir() {
        return Err(format!(
            "steamapps folder does not exist: {}",
            steamapps_path.display()
        ));
    }
    let steam_root = steamapps_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from(""));

    let game_folder = steamapps_path.join("common").join(params.install_dir);
    let acf_path = steamapps_path.join(format!("appmanifest_{}.acf", params.app_id));

    // ---- safety checks ----------------------------------------------------
    let existing_marker = read_marker(&game_folder);

    if game_folder.exists() && existing_marker.is_none() {
        // Folder exists but we did not create it.
        let has_any_file = std::fs::read_dir(&game_folder)
            .map(|mut d| d.next().is_some())
            .unwrap_or(true);
        if has_any_file {
            return Err(format!(
                "Refusing to use '{}': the folder already exists and was not created by this app. \
                 It may be a real game installation.",
                game_folder.display()
            ));
        }
    }

    if acf_path.exists() && existing_marker.is_none() {
        return Err(format!(
            "'{}' already exists. This looks like a real Steam installation (or a leftover). \
             Remove it manually if you are sure it is safe.",
            acf_path.display()
        ));
    }

    if let Some(ref marker) = existing_marker {
        if !folder_contains_only_our_files(&game_folder, marker) {
            return Err(format!(
                "'{}' contains files that were not created by this app. Refusing to touch it.",
                game_folder.display()
            ));
        }
    }

    // ---- create folder + dummy executables --------------------------------
    std::fs::create_dir_all(&game_folder)
        .map_err(|e| format!("Failed to create {}: {}", game_folder.display(), e))?;

    let resource_path = handle
        .path()
        .resolve(
            "data/src-win.exe",
            tauri::path::BaseDirectory::Resource,
        )
        .map_err(|e| format!("Failed to resolve dummy executable resource: {}", e))?;
    if !resource_path.exists() {
        return Err(format!(
            "Dummy executable template not found at {}",
            resource_path.display()
        ));
    }

    let mut exes: Vec<String> = Vec::new();
    let mut total_size: u64 = 0;
    for exe_name in &params.exe_names {
        let target = game_folder.join(exe_name);
        if target.exists() && existing_marker.is_none() {
            return Err(format!(
                "Refusing to overwrite existing file '{}'.",
                target.display()
            ));
        }
        std::fs::copy(&resource_path, &target)
            .map_err(|e| format!("Failed to copy dummy executable to {}: {}", target.display(), e))?;
        total_size += std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
        if !exes.iter().any(|e| e == exe_name) {
            exes.push(exe_name.clone());
        }
    }

    // ---- marker + acf ------------------------------------------------------
    let marker = DummyMarker {
        app_id: params.app_id,
        game_name: params.game_name.to_string(),
        install_dir: params.install_dir.to_string(),
        exes: exes.clone(),
        created_at_unix: existing_marker
            .as_ref()
            .map(|m| m.created_at_unix)
            .unwrap_or_else(now_unix),
    };
    let marker_json = serde_json::to_string_pretty(&marker)
        .map_err(|e| format!("Failed to serialize marker: {}", e))?;
    std::fs::write(marker_path(&game_folder), marker_json)
        .map_err(|e| format!("Failed to write marker file: {}", e))?;

    let acf_content = build_acf_content(
        params.app_id,
        params.game_name,
        params.install_dir,
        &steam_root,
        total_size,
    );
    std::fs::write(&acf_path, acf_content)
        .map_err(|e| format!("Failed to write acf file {}: {}", acf_path.display(), e))?;

    Ok(SteamDummyStatus {
        installed: true,
        acf_exists: true,
        marker_exists: true,
        folder: game_folder.to_string_lossy().to_string(),
        exes,
    })
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_steam_dummy_status(
    steamapps: String,
    app_id: u64,
    install_dir: String,
) -> Result<SteamDummyStatus, String> {
    if !is_valid_steamapps_path(&steamapps) || !is_valid_segment(&install_dir) {
        return Err("Invalid path or install dir".to_string());
    }
    let steamapps_path = PathBuf::from(normalize_slashes(&steamapps));
    let game_folder = steamapps_path.join("common").join(&install_dir);
    let acf_path = steamapps_path.join(format!("appmanifest_{}.acf", app_id));

    let marker = read_marker(&game_folder);
    let acf_exists = acf_path.exists();
    let installed = marker.is_some()
        && acf_exists
        && marker
            .as_ref()
            .map(|m| m.exes.iter().all(|e| game_folder.join(e).exists()))
            .unwrap_or(false);

    Ok(SteamDummyStatus {
        installed,
        acf_exists,
        marker_exists: marker.is_some(),
        folder: game_folder.to_string_lossy().to_string(),
        exes: marker.map(|m| m.exes).unwrap_or_default(),
    })
}

#[tauri::command(rename_all = "snake_case")]
pub async fn remove_steam_dummy_game(
    steamapps: String,
    app_id: u64,
    install_dir: String,
) -> Result<String, String> {
    if !is_valid_steamapps_path(&steamapps) || !is_valid_segment(&install_dir) {
        return Err("Invalid path or install dir".to_string());
    }
    let steamapps_path = PathBuf::from(normalize_slashes(&steamapps));
    let game_folder = steamapps_path.join("common").join(&install_dir);
    let acf_path = steamapps_path.join(format!("appmanifest_{}.acf", app_id));

    let marker = match read_marker(&game_folder) {
        Some(m) => m,
        None => {
            return Err(format!(
                "'{}' was not created by this app (no marker file). Refusing to delete anything.",
                game_folder.display()
            ));
        }
    };

    if !folder_contains_only_our_files(&game_folder, &marker) {
        return Err(format!(
            "'{}' contains files that were not created by this app. \
             Removed nothing; delete the folder and acf manually if you know what you are doing.",
            game_folder.display()
        ));
    }

    // Stop leftover dummy processes first (best effort).
    for exe in &marker.exes {
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/IM", exe])
            .output();
    }

    let mut removed: Vec<String> = Vec::new();
    for exe in &marker.exes {
        let p = game_folder.join(exe);
        if p.exists() {
            std::fs::remove_file(&p).map_err(|e| format!("Failed to remove {}: {}", p.display(), e))?;
            removed.push(p.display().to_string());
        }
    }

    let mp = marker_path(&game_folder);
    if mp.exists() {
        std::fs::remove_file(&mp).map_err(|e| format!("Failed to remove marker: {}", e))?;
    }

    // Remove the folder only if it is now empty.
    let _ = std::fs::remove_dir(&game_folder);

    if acf_path.exists() {
        std::fs::remove_file(&acf_path)
            .map_err(|e| format!("Failed to remove {}: {}", acf_path.display(), e))?;
        removed.push(acf_path.display().to_string());
    }

    Ok(format!("Removed {} file(s).", removed.len()))
}

#[tauri::command(rename_all = "snake_case")]
pub async fn run_steam_dummy_game(
    steamapps: String,
    install_dir: String,
    exe_name: String,
    title: String,
) -> Result<String, String> {
    if !is_valid_steamapps_path(&steamapps)
        || !is_valid_segment(&install_dir)
        || !is_valid_exe_name(&exe_name)
    {
        return Err("Invalid arguments".to_string());
    }
    let steamapps_path = PathBuf::from(normalize_slashes(&steamapps));
    let game_folder = steamapps_path.join("common").join(&install_dir);

    if read_marker(&game_folder).is_none() {
        return Err(
            "This folder was not installed by this app. Use 'Install in Steam library' first."
                .to_string(),
        );
    }

    let executable_path = game_folder.join(&exe_name);
    if !executable_path.exists() {
        return Err(format!("Executable not found: {}", executable_path.display()));
    }

    std::process::Command::new(&executable_path)
        .args(["--title", &title])
        .current_dir(&game_folder)
        .spawn()
        .map_err(|e| format!("Failed to start process: {}", e))?;

    Ok(format!("Started {}", executable_path.display()))
}
