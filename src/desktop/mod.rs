//! The desktop around the app: the file manager (over D-Bus), the wallpaper
//! command, and the trash (`trash.rs`, the freedesktop.org trash).

pub mod trash;

use gtk::{gio, glib, prelude::*};
use std::{ffi::OsString, path::Path};

/// VITRINE_WALLPAPER_COMMAND, e.g. "set-wallpaper", split into arguments;
/// None (hiding "Set as wallpaper") when it's unset, empty or unparsable.
pub fn wallpaper_command() -> Option<Vec<OsString>> {
    let command = std::env::var_os("VITRINE_WALLPAPER_COMMAND")?;
    if command.to_string_lossy().trim().is_empty() {
        return None;
    }
    match glib::shell_parse_argv(&command) {
        Ok(argv) if !argv.is_empty() => Some(argv),
        Ok(_) => None,
        Err(error) => {
            eprintln!("Invalid VITRINE_WALLPAPER_COMMAND: {error}");
            None
        }
    }
}

/// Runs the wallpaper command with `path` appended, until it exits.
pub async fn set_wallpaper(argv: &[OsString], path: &Path) -> Result<(), glib::Error> {
    let mut argv: Vec<&std::ffi::OsStr> = argv.iter().map(|arg| arg.as_os_str()).collect();
    argv.push(path.as_os_str());
    gio::Subprocess::newv(&argv, gio::SubprocessFlags::NONE)?
        .wait_check_future()
        .await
}

/// Shows `path` selected in the file manager (org.freedesktop.FileManager1).
pub fn show_in_file_manager(path: &Path) {
    let uri = gio::File::for_path(path).uri().to_string();
    let path = path.to_owned();
    glib::spawn_future_local(async move {
        let shown = async {
            let bus = gio::bus_get_future(gio::BusType::Session).await?;
            bus.call_future(
                Some("org.freedesktop.FileManager1"),
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1",
                "ShowItems",
                Some(&(vec![uri], "").to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                -1,
            )
            .await
        };
        if let Err(error) = shown.await {
            eprintln!("Could not show {} in file manager: {error}", path.display());
        }
    });
}
