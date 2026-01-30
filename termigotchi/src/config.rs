use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Settings {
    pub(crate) skin_name: String,
    pub(crate) fps_cap: u32,
    pub(crate) enable_color: bool,
    pub(crate) enable_braille: bool,
    pub(crate) seed: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            skin_name: "default".to_string(),
            fps_cap: 30,
            enable_color: true,
            enable_braille: true,
            seed: 0xC0FFEE_u64,
        }
    }
}

pub(crate) struct Paths {
    pub(crate) save_path: PathBuf,
    pub(crate) settings_path: PathBuf,
}

pub(crate) fn project_paths() -> Result<Paths> {
    let proj = ProjectDirs::from("com", "termigotchi", "Termigotchi")
        .context("could not resolve project directories")?;
    let dir = proj.data_local_dir().to_path_buf();
    fs::create_dir_all(&dir).ok();
    Ok(Paths {
        save_path: dir.join("save.json"),
        settings_path: dir.join("settings.json"),
    })
}

pub(crate) fn load_settings(path: &Path) -> Settings {
    if let Ok(s) = fs::read_to_string(path) {
        if let Ok(v) = serde_json::from_str::<Settings>(&s) {
            return v;
        }
    }
    Settings::default()
}

pub(crate) fn save_settings_atomic(path: &Path, s: &Settings) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    let data = serde_json::to_vec_pretty(s)?;
    fs::write(&tmp, data)?;
    atomic_rename(&tmp, path)?;
    Ok(())
}

pub(crate) fn atomic_rename(from: &Path, to: &Path) -> Result<()> {
    // Best-effort atomic replace on same filesystem.
    // On Windows, rename-over-existing is trickier; this is still fine for Linux server usage.
    if to.exists() {
        let _ = fs::remove_file(to);
    }
    fs::rename(from, to)?;
    Ok(())
}
