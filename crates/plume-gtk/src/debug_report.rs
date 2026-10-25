//! "Copy Debug Report": system info, the call state and the recent log, as
//! text you can paste into a bug report. Never includes your token or
//! message contents (neither is logged), and anything token-shaped is
//! blanked out just in case.

use std::rc::Rc;

use adw::prelude::*;

use crate::app::AppState;
use crate::settings;

/// How much of the log to include.
const LOG_LINES: usize = 300;

pub fn copy(state: &Rc<AppState>) {
    let report = build(state);
    let saved = settings::state_dir().and_then(|dir| {
        let name = chrono::Local::now().format("debug-report-%Y%m%d-%H%M%S.txt");
        let path = dir.join(name.to_string());
        std::fs::write(&path, &report).ok().map(|()| path)
    });
    if let Some(ui) = state.ui.get() {
        ui.window.clipboard().set_text(&report);
    }
    match saved {
        Some(path) => state.toast(&format!(
            "Debug report copied (also saved to {})",
            path.display()
        )),
        None => state.toast("Debug report copied"),
    }
}

fn build(state: &AppState) -> String {
    let mut out = String::from("Plume debug report\n==================\n");
    out.push_str(&format!("version: {}\n", env!("CARGO_PKG_VERSION")));
    out.push_str(&format!("os: {}\n", os_name()));
    out.push_str(&format!(
        "desktop: {}\n",
        std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default()
    ));
    out.push_str(&format!(
        "gtk: {}.{}.{}, libadwaita: {}.{}.{}\n",
        gtk::major_version(),
        gtk::minor_version(),
        gtk::micro_version(),
        adw::major_version(),
        adw::minor_version(),
        adw::micro_version()
    ));
    out.push_str(&format!(
        "profile: {:?}, online: {}, demo: {}\n",
        state.options.borrow().profile.tier,
        state.online.get(),
        state.options.borrow().demo
    ));

    {
        let store = state.store.borrow();
        out.push_str(&format!(
            "servers: {}, channels: {}, open channel: {:?}\n",
            store.guilds().count(),
            store.channels().count(),
            store.active_channel()
        ));
        match state.voice.borrow().as_ref() {
            Some(call) => out.push_str(&format!(
                "voice: in {} ({}), server {:?}, status {:?}, muted {}, deafened {}\n",
                call.channel_id,
                store
                    .channel(call.channel_id)
                    .and_then(|c| c.name.clone())
                    .unwrap_or_default(),
                call.guild_id,
                call.status,
                state.voice_muted(),
                state.voice_deafened()
            )),
            None => out.push_str("voice: not in a call\n"),
        }
        if let Some(mine) = store.my_voice_state() {
            out.push_str(&format!(
                "voice according to Discord: channel {:?}, server {:?}\n",
                mine.channel_id, mine.guild_id
            ));
        }
    }

    out.push_str(&format!(
        "\nlog (last {LOG_LINES} lines)\n------------------------\n"
    ));
    let log = settings::state_dir()
        .and_then(|dir| std::fs::read_to_string(dir.join("plume-gtk.log")).ok())
        .unwrap_or_else(|| "(no log file)".to_owned());
    let lines: Vec<&str> = log.lines().collect();
    for line in &lines[lines.len().saturating_sub(LOG_LINES)..] {
        out.push_str(&scrub(line));
        out.push('\n');
    }
    out
}

fn os_name() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|l| l.strip_prefix("PRETTY_NAME="))
                .map(|name| name.trim_matches('"').to_owned())
        })
        .unwrap_or_else(|| std::env::consts::OS.to_owned())
}

/// Blank out anything that looks like a Discord token (three dot-separated
/// base64 parts), in case one ever ends up in a log line.
fn scrub(line: &str) -> String {
    line.split(' ')
        .map(|word| {
            let parts = word.split('.').count();
            let token_chars = word
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
            if parts == 3 && word.len() >= 50 && token_chars {
                "[redacted]"
            } else {
                word
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::scrub;

    #[test]
    fn token_shaped_words_are_removed() {
        let fake = format!("{}.{}.{}", "A".repeat(26), "b".repeat(6), "c".repeat(38));
        assert_eq!(
            scrub(&format!("token {fake} here")),
            "token [redacted] here"
        );
        assert_eq!(
            scrub("voice connected to 123.45.67"),
            "voice connected to 123.45.67"
        );
    }
}
