use discord_sdk::activity::{ActivityBuilder, ActivityKind};

use crate::rpc::{self, Client};
use serde::Deserialize;

/// Accepts an id encoded either as a JSON string or as a JSON number and
/// always normalizes it to a `String`.
pub(crate) fn string_or_number<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrNumber {
        String(String),
        Number(u64),
        Float(f64),
    }

    match StringOrNumber::deserialize(deserializer)? {
        StringOrNumber::String(s) => Ok(s),
        StringOrNumber::Number(n) => Ok(n.to_string()),
        // JS f64 rounding can produce values like 1.4514096096329114e18 when an
        // id larger than 2^53 is passed as a raw number. Format without
        // scientific notation so Rust-side parsing keeps working.
        StringOrNumber::Float(f) => Ok(format!("{:.0}", f)),
    }
}

#[derive(Deserialize)]
pub struct ActivityParams {
    #[serde(deserialize_with = "string_or_number")]
    pub app_id: String,
    pub details: Option<String>,
    pub state: Option<String>,
    #[serde(rename = "largeImageKey")]
    pub large_image_key: Option<String>,
    #[serde(rename = "largeImageText")]
    pub large_image_text: Option<String>,
    pub timestamp: Option<i64>,
    pub activity_kind: Option<i32>,
}

pub struct CreateActivityResult {
    pub activity: ActivityBuilder,
    pub app_id: u64,
}

pub fn parse_activity_json(activity_json: &str) -> Result<ActivityParams, String> {
    serde_json::from_str(activity_json).map_err(|e| {
        eprintln!("Failed to parse activity JSON: {}", e);
        format!("Failed to parse activity JSON: {}", e)
    })
}

pub fn create_activity(activity_json: String) -> Result<CreateActivityResult, String> {
    let activity: ActivityParams = parse_activity_json(&activity_json)?;

    let app_id: u64 = activity
        .app_id
        .parse::<u64>()
        .map_err(|e| format!("Failed to parse app_id '{}': {}", activity.app_id, e))?;

    let kind = match activity.activity_kind.unwrap_or(0) {
        2 => ActivityKind::Listening,
        3 => ActivityKind::Watching,
        5 => ActivityKind::Competing,
        _ => ActivityKind::Playing,
    };

    let mut rp = ActivityBuilder::default().kind(kind);

    if let Some(details) = activity.details.filter(|d| !d.is_empty()) {
        rp = rp.details(details);
    }

    if let Some(state) = activity.state.filter(|s| !s.is_empty()) {
        rp = rp.state(state);
    }

    if let Some(ts) = activity.timestamp {
        rp = rp.start_timestamp(ts);
    }

    if let Some(key) = activity.large_image_key.filter(|k| !k.is_empty()) {
        rp = rp.assets(rpc::ds::activity::Assets::default().large(&key, activity.large_image_text));
    }

    Ok(CreateActivityResult {
        activity: rp,
        app_id,
    })
}

pub async fn set_activity(activity_json: String) -> Result<Client, String> {
    let activity_result = create_activity(activity_json)?;
    let app_id: i64 = activity_result.app_id as i64;

    let client = rpc::make_client(app_id, rpc::ds::Subscriptions::ACTIVITY).await?;
    client
        .discord
        .update_activity(activity_result.activity)
        .await
        .map_err(|e| format!("Failed to update activity: {}", e))?;

    Ok(client)
}
