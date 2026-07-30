//! Steam appinfo lookups: install dir, launch executables and AppID search.
//!
//! SteamDB itself has no API and blocks scraping, but the data its /config/
//! page shows is Steam's PICS appinfo, which api.steamcmd.net re-serves
//! publicly. That is a third-party community service, so every failure here
//! must stay non-fatal: the UI falls back to manual entry.

use serde::Serialize;
use std::time::Duration;

const APPINFO_URL: &str = "https://api.steamcmd.net/v1/info/";
const SEARCH_URL: &str = "https://steamcommunity.com/actions/SearchApps/";

#[derive(Debug, Clone, Serialize)]
pub struct SteamLaunchOption {
    /// Executable exactly as Steam lists it, possibly with a sub path
    /// (e.g. `game\bin\win64\cs2.exe`).
    pub executable: String,
    /// Just the file name part, which is what the dummy install needs.
    pub filename: String,
    /// The directory part of `executable`, relative to the install dir, using
    /// backslashes. Empty when the exe sits directly in the install dir.
    ///
    /// Games that nest their binary (Counter-Strike 2, Helldivers 2) are the
    /// reason this is surfaced: dropping the dummy at the top level would put
    /// it somewhere Discord is not looking.
    pub sub_dir: String,
    /// Steam launch option type. Empty or "default" is the entry the Play
    /// button uses; "option1"... are the alternatives.
    pub launch_type: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SteamAppInfo {
    pub steam_app_id: u64,
    pub name: Option<String>,
    pub install_dir: Option<String>,
    /// Windows `.exe` launch entries, deduplicated, default entry first.
    pub launch: Vec<SteamLaunchOption>,
    /// True when Steam only shows this app's config to accounts that own it.
    pub gated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SteamAppSearchResult {
    pub appid: String,
    pub name: String,
}

fn http_client() -> Result<tauri_plugin_http::reqwest::Client, String> {
    tauri_plugin_http::reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))
}

/// Truthiness the way this API mixes types: `true`, `1` and `"1"` all mean
/// yes (`public_only` in particular arrives as the string "1").
fn truthy(v: Option<&serde_json::Value>) -> bool {
    match v {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::Number(n)) => n.as_f64().unwrap_or(0.0) != 0.0,
        Some(serde_json::Value::String(s)) => s == "1" || s.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// Directory part of a Steam launch path, normalized to backslashes.
/// `game/bin/win64/cs2.exe` -> `game\bin\win64`, `cs2.exe` -> ``.
fn dirname(path: &str) -> String {
    let normalized = path.replace('/', "\\");
    match normalized.rfind('\\') {
        Some(i) => normalized[..i].trim_matches('\\').to_string(),
        None => String::new(),
    }
}

fn encode_path_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Fetches Steam's appinfo for one app and boils it down to what the dummy
/// install form needs: the install dir and the Windows launch executables.
#[tauri::command(rename_all = "snake_case")]
pub async fn fetch_steam_appinfo(steam_app_id: u64) -> Result<SteamAppInfo, String> {
    let url = format!("{}{}", APPINFO_URL, steam_app_id);
    let response = http_client()?
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Request to {} failed: {}", url, e))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("{} returned HTTP {}", url, status));
    }

    // .text() + from_str instead of .json(): the plugin's reqwest re-export
    // is built without the `json` feature.
    let body = response
        .text()
        .await
        .map_err(|e| format!("Could not read the appinfo response: {}", e))?;
    let body: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("Could not parse the appinfo response: {}", e))?;

    let app = body
        .get("data")
        .and_then(|d| d.get(steam_app_id.to_string()))
        .ok_or_else(|| format!("No appinfo returned for AppID {}", steam_app_id))?;

    let name = app
        .pointer("/common/name")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let install_dir = app
        .pointer("/config/installdir")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let gated = truthy(app.get("public_only"));

    // Launch entries come as an object keyed "0", "1", ... — order them
    // numerically, since the JSON map does not guarantee it.
    let mut entries: Vec<(u32, &serde_json::Value)> = app
        .pointer("/config/launch")
        .and_then(|v| v.as_object())
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| k.parse::<u32>().ok().map(|n| (n, v)))
                .collect()
        })
        .unwrap_or_default();
    entries.sort_by_key(|(n, _)| *n);

    let mut launch: Vec<SteamLaunchOption> = Vec::new();
    for (_, entry) in entries {
        let executable = match entry.get("executable").and_then(|v| v.as_str()) {
            Some(e) => e.to_string(),
            None => continue,
        };
        // Windows only. No oslist means "any platform", which includes it.
        let oslist = entry
            .pointer("/config/oslist")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !oslist.is_empty() && !oslist.to_lowercase().contains("windows") {
            continue;
        }
        let filename = basename(&executable).to_string();
        if !filename.to_lowercase().ends_with(".exe") {
            continue;
        }
        if launch
            .iter()
            .any(|l| l.filename.eq_ignore_ascii_case(&filename))
        {
            continue;
        }
        let launch_type = entry
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let sub_dir = dirname(&executable);
        launch.push(SteamLaunchOption {
            executable,
            filename,
            sub_dir,
            launch_type,
        });
    }
    // Stable sort: the entry Steam's Play button runs (no type, or "default")
    // moves to the front, everything else keeps its order.
    launch.sort_by_key(|l| !(l.launch_type.is_empty() || l.launch_type == "default"));

    Ok(SteamAppInfo {
        steam_app_id,
        name,
        install_dir,
        launch,
        gated,
    })
}

/// Searches Steam's storefront by name. Used to find the Steam AppID when
/// Discord's entry ships no Steam SKU.
#[tauri::command(rename_all = "snake_case")]
pub async fn search_steam_apps(query: String) -> Result<Vec<SteamAppSearchResult>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }

    let url = format!("{}{}", SEARCH_URL, encode_path_segment(query));
    let response = http_client()?
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Request to {} failed: {}", url, e))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("{} returned HTTP {}", url, status));
    }

    let body = response
        .text()
        .await
        .map_err(|e| format!("Could not read the Steam search response: {}", e))?;
    let body: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("Could not parse the Steam search response: {}", e))?;

    let results = body
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    // appid is a string today; stay tolerant of a number.
                    let appid = match item.get("appid") {
                        Some(serde_json::Value::String(s)) => s.clone(),
                        Some(serde_json::Value::Number(n)) => n.to_string(),
                        _ => return None,
                    };
                    let name = item.get("name")?.as_str()?.to_string();
                    Some(SteamAppSearchResult { appid, name })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(results)
}
