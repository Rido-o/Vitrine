//! Recently opened folders, most recent first, in
//! ~/.local/state/vitrine/history (one path per line).

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
    state_dir().join("vitrine/history")
}

// The Rust port kept its own list while both apps were in use; being the
// newer, it replaces the TypeScript app's, once.
fn migrate() {
    let old = state_dir().join("vitrine-spike");
    let old_file = old.join("history");
    if !old_file.exists() {
        return;
    }
    let moved = file()
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::rename(&old_file, file()));
    match moved {
        Ok(()) => {
            let _ = std::fs::remove_dir(&old);
        }
        Err(error) => eprintln!("Could not move {}: {error}", old_file.display()),
    }
}

pub struct History {
    pub entries: Vec<PathBuf>,
}

impl History {
    pub fn load() -> Self {
        migrate();
        let text = std::fs::read_to_string(file()).unwrap_or_default();
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
