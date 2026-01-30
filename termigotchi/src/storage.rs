use crate::config::atomic_rename;
use crate::config::Settings;
use crate::model::{GameState, SaveFile};
use anyhow::Result;
use chrono::{DateTime, Utc};
use std::{fs, path::Path};

pub(crate) fn load_or_init_save(
    path: &Path,
    settings: &Settings,
) -> Result<(GameState, Option<DateTime<Utc>>)> {
    if let Ok(s) = fs::read_to_string(path) {
        if let Ok(save) = serde_json::from_str::<SaveFile>(&s) {
            return Ok((save.state, Some(save.last_seen_utc)));
        }
    }
    Ok((GameState::new(settings.seed), None))
}

pub(crate) fn save_atomic(path: &Path, save: &SaveFile) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    let data = serde_json::to_vec_pretty(save)?;
    fs::write(&tmp, data)?;
    atomic_rename(&tmp, path)?;
    Ok(())
}
