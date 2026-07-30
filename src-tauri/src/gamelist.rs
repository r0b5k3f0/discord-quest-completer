//! Fetching Discord's "detectable games" list.
//!
//! The upstream list is roughly 12 MB for ~24k entries and ships 14 fields per
//! game, of which the UI reads five. Deserializing into a narrow struct here
//! means serde drops everything else before the list is ever handed to the
//! webview, which cuts both the IPC payload and the JSON the webview has to
//! parse to about a third.

use serde::{Deserialize, Serialize};

const GH_MIRROR_URL: &str =
    "https://markterence.github.io/discord-quest-completer/detectable.json";
const DISCORD_URL: &str = "https://discord.com/api/applications/detectable";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameExecutable {
    #[serde(default)]
    pub is_launcher: bool,
    pub name: String,
    pub os: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThirdPartySku {
    pub distributor: String,
    /// Discord sends `null` here for a fair number of entries.
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameEntry {
    /// Snowflake id. Accepts a JSON number too, in case the shape ever changes.
    #[serde(deserialize_with = "crate::runner::string_or_number")]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub executables: Vec<GameExecutable>,
    #[serde(default)]
    pub third_party_skus: Vec<ThirdPartySku>,
}

/// Downloads a game list and strips it down to the fields the UI uses.
///
/// `source` is either `github_mirror` or `discord`.
#[tauri::command(rename_all = "snake_case")]
pub async fn fetch_gamelist(source: String) -> Result<Vec<GameEntry>, String> {
    let url = match source.as_str() {
        "github_mirror" => GH_MIRROR_URL,
        "discord" => DISCORD_URL,
        other => return Err(format!("Unknown game list source: '{}'", other)),
    };

    let response = tauri_plugin_http::reqwest::get(url)
        .await
        .map_err(|e| format!("Request to {} failed: {}", url, e))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("{} returned HTTP {}", url, status));
    }

    let body = response
        .text()
        .await
        .map_err(|e| format!("Could not read the response from {}: {}", url, e))?;

    let games: Vec<GameEntry> = serde_json::from_str(&body)
        .map_err(|e| format!("Could not parse the game list from {}: {}", url, e))?;

    println!("Fetched {} games from {}", games.len(), url);

    Ok(games)
}
