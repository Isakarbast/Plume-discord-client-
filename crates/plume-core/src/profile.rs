//! Usage tiers from `docs/11-usage-profiles.md`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Feather,
    Standard,
    Full,
}

/// The knobs a tier controls. More get added as subsystems land.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Profile {
    pub tier: Tier,
    /// How many channels keep their messages in memory at once.
    pub cached_channels: usize,
    /// How many recent messages each of those channels keeps.
    pub messages_per_channel: usize,
    /// How far back you can scroll in the open channel before the oldest
    /// messages are dropped again.
    pub max_scrollback: usize,
}

impl Profile {
    pub const FEATHER: Profile = Profile {
        tier: Tier::Feather,
        cached_channels: 2,
        messages_per_channel: 50,
        max_scrollback: 250,
    };

    pub const STANDARD: Profile = Profile {
        tier: Tier::Standard,
        cached_channels: 8,
        messages_per_channel: 200,
        max_scrollback: 1000,
    };

    pub const FULL: Profile = Profile {
        tier: Tier::Full,
        cached_channels: 16,
        messages_per_channel: 500,
        max_scrollback: 2500,
    };

    /// Looks a profile up by name (`"feather"`, `"standard"`, `"full"`).
    pub fn from_name(name: &str) -> Option<Profile> {
        match name.to_ascii_lowercase().as_str() {
            "feather" => Some(Profile::FEATHER),
            "standard" => Some(Profile::STANDARD),
            "full" => Some(Profile::FULL),
            _ => None,
        }
    }
}

impl Default for Profile {
    fn default() -> Self {
        Profile::STANDARD
    }
}
