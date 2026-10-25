//! Settings from the command line and from `~/.config/plume/settings`.
//!
//! The file is simple `key = value` lines, e.g.
//!
//! ```text
//! profile = full
//! notifications = false
//! silent_typing = true
//! voice_threshold = -45
//! mic_gain = 6
//! auto_gain = true
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;

use plume_core::models::Status;
use plume_core::{Profile, Snowflake, Tier};

#[derive(Debug, Clone)]
pub struct Options {
    pub profile: Profile,
    pub notifications: bool,
    pub silent_typing: bool,
    /// Microphone level (dBFS) above which voice transmits.
    pub voice_threshold: f32,
    /// Extra microphone gain in dB.
    pub mic_gain: f32,
    /// Automatic gain control for the microphone.
    pub auto_gain: bool,
    /// RNNoise noise suppression on the microphone.
    pub noise_suppression: bool,
    /// Only transmit while `push_to_talk_key` is held (while Plume is focused).
    pub push_to_talk: bool,
    /// A GDK key name, like `grave` (the ` key) or `Control_L`.
    pub push_to_talk_key: String,
    /// Device names; empty = the system default.
    pub input_device: String,
    pub output_device: String,
    /// Discord's dark look (`true`) or the system's GNOME style.
    pub discord_theme: bool,
    /// The status we set when logging in.
    pub status: Status,
    /// "Coding something cool", shown under our name.
    pub custom_status: String,
    /// Show the member list on the right.
    pub member_list: bool,
    /// Draw with the GPU (`false`: software, which saves memory).
    pub hardware_acceleration: bool,
    /// Per-person voice volume in percent (100 = normal).
    pub user_volumes: BTreeMap<Snowflake, u32>,
    /// Made-up data, no network (`--demo`).
    pub demo: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            profile: Profile::STANDARD,
            notifications: true,
            silent_typing: false,
            voice_threshold: -50.0,
            mic_gain: 0.0,
            auto_gain: true,
            noise_suppression: true,
            push_to_talk: false,
            push_to_talk_key: "grave".to_owned(),
            input_device: String::new(),
            output_device: String::new(),
            discord_theme: true,
            status: Status::Online,
            custom_status: String::new(),
            member_list: true,
            hardware_acceleration: true,
            user_volumes: BTreeMap::new(),
            demo: false,
        }
    }
}

pub const HELP: &str = "\
plume: a lightweight native Discord client

Usage: plume [options]

Options:
  --profile <name>    standard (default) or full: how much to keep in memory
  --no-notifications  don't show desktop notifications
  --silent-typing     don't tell others when you're typing
  --demo              try the app with made-up data (no login, no network)
  --logout            forget the saved token and the cache, then quit
  --help              show this help

Settings can also go in ~/.config/plume/settings (key = value), including
voice_threshold = -50 (microphone level in dB that counts as talking),
mic_gain = 0 (extra microphone volume in dB) and auto_gain = true.
Your own CSS goes in ~/.config/plume/style.css.";

impl Options {
    /// The settings file, then command-line flags on top.
    pub fn load(args: &[String]) -> Result<Options, String> {
        let mut options = Options::default();
        if let Some(text) =
            config_dir().and_then(|dir| std::fs::read_to_string(dir.join("settings")).ok())
        {
            options.apply_file(&text)?;
        }
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--profile" => {
                    let name = args.next().ok_or("--profile needs a name")?;
                    options.profile = Profile::from_name(name)
                        .ok_or_else(|| format!("unknown profile `{name}`"))?;
                }
                "--no-notifications" => options.notifications = false,
                "--silent-typing" => options.silent_typing = true,
                "--demo" => options.demo = true,
                "--logout" | "--help" | "-h" => {}
                other => return Err(format!("unknown option `{other}`. See --help")),
            }
        }
        Ok(options)
    }

    fn apply_file(&mut self, text: &str) -> Result<(), String> {
        for (number, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| format!("settings line {}: expected `key = value`", number + 1))?;
            let (key, value) = (key.trim(), value.trim());
            let flag = || match value {
                "true" | "yes" | "on" => Ok(true),
                "false" | "no" | "off" => Ok(false),
                _ => Err(format!("settings: `{key}` should be true or false")),
            };
            match key {
                "profile" => {
                    self.profile = Profile::from_name(value)
                        .ok_or_else(|| format!("settings: unknown profile `{value}`"))?;
                }
                "notifications" => self.notifications = flag()?,
                "silent_typing" => self.silent_typing = flag()?,
                "voice_threshold" => {
                    self.voice_threshold = value
                        .parse::<f32>()
                        .ok()
                        .filter(|db| (-100.0..=0.0).contains(db))
                        .ok_or("settings: `voice_threshold` should be a number from -100 to 0")?;
                }
                "mic_gain" => {
                    self.mic_gain = value
                        .parse::<f32>()
                        .ok()
                        .filter(|db| (-20.0..=20.0).contains(db))
                        .ok_or("settings: `mic_gain` should be a number from -20 to 20")?;
                }
                "auto_gain" => self.auto_gain = flag()?,
                "noise_suppression" => self.noise_suppression = flag()?,
                "push_to_talk" => self.push_to_talk = flag()?,
                "push_to_talk_key" => self.push_to_talk_key = value.to_owned(),
                "input_device" => self.input_device = value.to_owned(),
                "output_device" => self.output_device = value.to_owned(),
                "theme" => {
                    self.discord_theme = match value {
                        "discord" => true,
                        "system" => false,
                        _ => return Err("settings: `theme` should be discord or system".into()),
                    }
                }
                "status" => {
                    self.status = match value {
                        "online" => Status::Online,
                        "idle" => Status::Idle,
                        "dnd" => Status::Dnd,
                        "invisible" => Status::Offline,
                        _ => return Err("settings: unknown `status`".into()),
                    }
                }
                "member_list" => self.member_list = flag()?,
                "custom_status" => self.custom_status = value.to_owned(),
                "hardware_acceleration" => self.hardware_acceleration = flag()?,
                key if key.starts_with("volume.") => {
                    let user = key["volume.".len()..]
                        .parse::<u64>()
                        .map_err(|_| format!("settings: bad user ID in `{key}`"))?;
                    let percent = value
                        .parse::<u32>()
                        .ok()
                        .filter(|p| *p <= 200)
                        .ok_or("settings: volumes go from 0 to 200")?;
                    self.user_volumes.insert(Snowflake(user), percent);
                }
                _ => return Err(format!("settings: unknown setting `{key}`")),
            }
        }
        Ok(())
    }
}

impl Options {
    /// The settings file as written by the settings window.
    pub fn to_file(&self) -> String {
        let tier = match self.profile.tier {
            Tier::Feather => "feather",
            Tier::Standard => "standard",
            Tier::Full => "full",
        };
        let status = match self.status {
            Status::Online => "online",
            Status::Idle => "idle",
            Status::Dnd => "dnd",
            Status::Offline => "invisible",
        };
        let mut out = format!(
            "# Written by Plume's settings window; edit freely.\n\
             profile = {tier}\n\
             notifications = {}\n\
             silent_typing = {}\n\
             voice_threshold = {}\n\
             mic_gain = {}\n\
             auto_gain = {}\n\
             theme = {}\n\
             status = {status}\n\
             member_list = {}\n\
             hardware_acceleration = {}\n",
            self.notifications,
            self.silent_typing,
            self.voice_threshold,
            self.mic_gain,
            self.auto_gain,
            if self.discord_theme {
                "discord"
            } else {
                "system"
            },
            self.member_list,
            self.hardware_acceleration,
        );
        out.push_str(&format!(
            "noise_suppression = {}\npush_to_talk = {}\npush_to_talk_key = {}\n",
            self.noise_suppression, self.push_to_talk, self.push_to_talk_key
        ));
        for (key, value) in [
            ("input_device", &self.input_device),
            ("output_device", &self.output_device),
        ] {
            if !value.is_empty() {
                out.push_str(&format!("{key} = {value}\n"));
            }
        }
        if !self.custom_status.is_empty() {
            out.push_str(&format!(
                "custom_status = {}\n",
                self.custom_status.replace('\n', " ")
            ));
        }
        for (user, percent) in &self.user_volumes {
            out.push_str(&format!("volume.{user} = {percent}\n"));
        }
        out
    }

    /// Save to `~/.config/plume/settings`.
    pub fn save(&self) -> std::io::Result<()> {
        let dir = config_dir().ok_or_else(|| std::io::Error::other("no home directory"))?;
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("settings"), self.to_file())
    }
}

/// `$XDG_CONFIG_HOME/plume` or `~/.config/plume`.
pub fn config_dir() -> Option<PathBuf> {
    xdg("XDG_CONFIG_HOME", ".config")
}

/// `$XDG_CACHE_HOME/plume` or `~/.cache/plume`.
pub fn cache_dir() -> Option<PathBuf> {
    xdg("XDG_CACHE_HOME", ".cache")
}

/// `$XDG_STATE_HOME/plume` or `~/.local/state/plume`.
pub fn state_dir() -> Option<PathBuf> {
    xdg("XDG_STATE_HOME", ".local/state")
}

fn xdg(variable: &str, fallback: &str) -> Option<PathBuf> {
    let base = match std::env::var_os(variable) {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(fallback),
    };
    Some(base.join("plume"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_then_flags() {
        let mut options = Options::default();
        options
            .apply_file("# comment\nprofile = full\nnotifications = off\n")
            .unwrap();
        assert_eq!(options.profile, Profile::FULL);
        assert!(!options.notifications);
        options.apply_file("voice_threshold = -42").unwrap();
        assert_eq!(options.voice_threshold, -42.0);
        assert!(options.apply_file("voice_threshold = 3").is_err());

        // What we write, we can read back.
        options.user_volumes.insert(Snowflake(42), 150);
        options.status = Status::Dnd;
        let mut again = Options::default();
        again.apply_file(&options.to_file()).unwrap();
        assert_eq!(again.user_volumes[&Snowflake(42)], 150);
        assert_eq!(again.status, Status::Dnd);
        assert_eq!(again.profile, Profile::FULL);
        assert!(options.apply_file("nonsense").is_err());
        assert!(options.apply_file("colour = blue").is_err());
    }
}
