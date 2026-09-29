//! Recently opened folders, most recent first, in the spike's own state file;
//! the first time, filled from the TypeScript app's (so the two can't
//! overwrite each other's list while both are in use). As History.ts.

use gtk::glib;
use std::path::{Path, PathBuf};

// $XDG_STATE_HOME, as g_get_user_state_dir (not in glib-rs 0.22).
fn state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .unwrap_or_else(|| glib::home_dir().join(".local/state"))
}

const LIMIT: usize = 10;

fn file() -> PathBuf {
    state_dir().join("vitrine-spike/history")
}

fn typescript_app_file() -> PathBuf {
    state_dir().join("vitrine/history")
}

pub struct History {
    pub entries: Vec<PathBuf>,
}

impl History {
    pub fn load() -> Self {
        let text = std::fs::read_to_string(file())
            .or_else(|_| std::fs::read_to_string(typescript_app_file()))
            .unwrap_or_default();
        let mut entries: Vec<PathBuf> = Vec::new();
        for line in text.lines().filter(|line| !line.is_empty()) {
            let path = PathBuf::from(line);
            if path.is_dir() && !entries.contains(&path) {
                entries.push(path);
            }
        }
        entries.truncate(LIMIT);
        Self { entries }
    }

    pub fn remember(&mut self, directory: &Path) {
        self.entries.retain(|entry| entry != directory);
        self.entries.insert(0, directory.to_owned());
        self.entries.truncate(LIMIT);
        let mut text = String::new();
        for entry in &self.entries {
            text.push_str(&entry.to_string_lossy());
            text.push('\n');
        }
        let path = file();
        let saved = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, text));
        if let Err(error) = saved {
            eprintln!("Could not save folder history: {error}");
        }
    }
}
