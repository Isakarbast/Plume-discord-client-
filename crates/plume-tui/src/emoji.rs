//! Turns what the user typed in the reaction prompt into an emoji.

use plume_core::models::ReactionEmoji;
use plume_core::{Snowflake, Store};

/// Common shortcodes, so you can type `thumbsup` instead of finding 👍.
const ALIASES: &[(&str, &str)] = &[
    ("+1", "👍"),
    ("thumbsup", "👍"),
    ("-1", "👎"),
    ("thumbsdown", "👎"),
    ("heart", "❤️"),
    ("joy", "😂"),
    ("sob", "😭"),
    ("smile", "😄"),
    ("eyes", "👀"),
    ("fire", "🔥"),
    ("tada", "🎉"),
    ("pray", "🙏"),
    ("skull", "💀"),
    ("100", "💯"),
    ("thinking", "🤔"),
    ("ok", "👌"),
    ("wave", "👋"),
    ("check", "✅"),
    ("white_check_mark", "✅"),
    ("x", "❌"),
    ("open_mouth", "😮"),
    ("cry", "😢"),
    ("rofl", "🤣"),
    ("clap", "👏"),
];

/// Accepts an emoji itself ("🔥"), a server emoji (":pepe:"), or an alias
/// ("fire" or ":fire:"). Server emoji are looked up in `guild_id` first,
/// then in every server you're in.
pub fn resolve(input: &str, store: &Store, guild_id: Option<Snowflake>) -> Option<ReactionEmoji> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    // Something with no letters or digits is probably the emoji itself.
    if !input.chars().any(|c| c.is_ascii_alphanumeric()) && !input.contains(':') {
        return Some(ReactionEmoji::unicode(input));
    }

    let name = input.trim_matches(':').to_lowercase();
    if let Some(custom) = find_custom(store, guild_id, &name) {
        return Some(custom);
    }
    ALIASES
        .iter()
        .find(|(alias, _)| *alias == name)
        .map(|(_, emoji)| ReactionEmoji::unicode(emoji))
}

fn find_custom(store: &Store, guild_id: Option<Snowflake>, name: &str) -> Option<ReactionEmoji> {
    let in_guild = |guild: &plume_core::models::Guild| {
        guild
            .emojis
            .iter()
            .find(|e| e.name.to_lowercase() == name)
            .map(|e| ReactionEmoji {
                id: Some(e.id),
                name: e.name.clone(),
            })
    };
    guild_id
        .and_then(|id| store.guild(id))
        .and_then(in_guild)
        .or_else(|| store.guilds().find_map(in_guild))
}

#[cfg(test)]
mod tests {
    use super::*;
    use plume_core::Profile;

    #[test]
    fn unicode_and_aliases() {
        let store = Store::new(Profile::FEATHER);
        assert_eq!(
            resolve("🔥", &store, None),
            Some(ReactionEmoji::unicode("🔥"))
        );
        assert_eq!(
            resolve("fire", &store, None),
            Some(ReactionEmoji::unicode("🔥"))
        );
        assert_eq!(
            resolve(":thumbsup:", &store, None),
            Some(ReactionEmoji::unicode("👍"))
        );
        assert_eq!(resolve("no-such-emoji", &store, None), None);
        assert_eq!(resolve("  ", &store, None), None);
    }
}
