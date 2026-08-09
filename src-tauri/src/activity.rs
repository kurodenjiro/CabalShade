//! Small persisted activity feed for the profile screen.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEntry {
    pub id: String,
    pub kind: String,
    pub summary: String,
    pub created_at: u64,
}

fn path() -> std::path::PathBuf {
    crate::app_paths::in_data_dir("activity_log.json")
}

fn load_all() -> Vec<ActivityEntry> {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn append(kind: &str, summary: impl Into<String>) {
    let mut entries = load_all();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    entries.push(ActivityEntry {
        id: format!("activity-{now}-{}", entries.len()),
        kind: kind.to_string(),
        summary: summary.into(),
        created_at: now,
    });
    let entries = if entries.len() > 200 {
        entries.split_off(entries.len() - 200)
    } else {
        entries
    };
    let _ = cabal_store::JsonStore::new(path()).save(&entries);
}

pub fn recent(limit: usize) -> Vec<ActivityEntry> {
    let entries = load_all();
    entries.into_iter().rev().take(limit).collect()
}

/// Returns the full bounded history for deriving durable achievement state.
pub fn all() -> Vec<ActivityEntry> {
    load_all()
}
