//! Suggestions while typing `@name`, `#channel` or `:emoji:`, like Discord.

use std::collections::HashSet;

use plume_core::models::ChannelKind;
use plume_core::navigation;
use plume_core::{Snowflake, Store};

/// Common emoji shortcodes (Discord accepts the same names).
const EMOJI: &[(&str, &str)] = &[
    ("thumbsup", "👍"),
    ("+1", "👍"),
    ("thumbsdown", "👎"),
    ("heart", "❤️"),
    ("joy", "😂"),
    ("rofl", "🤣"),
    ("sob", "😭"),
    ("smile", "😄"),
    ("grin", "😁"),
    ("slight_smile", "🙂"),
    ("wink", "😉"),
    ("blush", "😊"),
    ("heart_eyes", "😍"),
    ("kissing_heart", "😘"),
    ("thinking", "🤔"),
    ("eyes", "👀"),
    ("fire", "🔥"),
    ("tada", "🎉"),
    ("pray", "🙏"),
    ("skull", "💀"),
    ("100", "💯"),
    ("ok_hand", "👌"),
    ("wave", "👋"),
    ("clap", "👏"),
    ("muscle", "💪"),
    ("white_check_mark", "✅"),
    ("x", "❌"),
    ("warning", "⚠️"),
    ("open_mouth", "😮"),
    ("cry", "😢"),
    ("angry", "😠"),
    ("rage", "😡"),
    ("sunglasses", "😎"),
    ("nerd", "🤓"),
    ("sweat_smile", "😅"),
    ("upside_down", "🙃"),
    ("rolling_eyes", "🙄"),
    ("pleading_face", "🥺"),
    ("partying_face", "🥳"),
    ("star_struck", "🤩"),
    ("shrug", "🤷"),
    ("facepalm", "🤦"),
    ("heart_on_fire", "❤️‍🔥"),
    ("broken_heart", "💔"),
    ("sparkles", "✨"),
    ("star", "⭐"),
    ("zap", "⚡"),
    ("rocket", "🚀"),
    ("crab", "🦀"),
    ("cat", "🐱"),
    ("dog", "🐶"),
    ("pizza", "🍕"),
    ("coffee", "☕"),
    ("beer", "🍺"),
    ("trophy", "🏆"),
    ("gift", "🎁"),
    ("moneybag", "💰"),
    ("bulb", "💡"),
    ("lock", "🔒"),
    ("point_up", "☝️"),
    ("point_right", "👉"),
    ("raised_hands", "🙌"),
    ("handshake", "🤝"),
    ("salute", "🫡"),
    ("melting_face", "🫠"),
    ("skull_crossbones", "☠️"),
    ("clown", "🤡"),
    ("ghost", "👻"),
    ("poop", "💩"),
    ("sleeping", "😴"),
    ("yawning_face", "🥱"),
    ("hot_face", "🥵"),
    ("cold_face", "🥶"),
    ("exploding_head", "🤯"),
];

#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    /// What the list shows.
    pub label: String,
    pub detail: String,
    /// What goes in the message box.
    pub insert: String,
    /// What it becomes when sent (`<@id>` etc.), if different.
    pub wire: Option<String>,
    /// A server emoji to draw next to it.
    pub emoji: Option<(Snowflake, bool)>,
}

const MAX: usize = 10;

pub fn suggestions(
    store: &Store,
    channel_id: Snowflake,
    trigger: char,
    query: &str,
) -> Vec<Suggestion> {
    let query = query.to_lowercase();
    let guild_id = store.channel(channel_id).and_then(|c| c.guild_id);
    let mut out = Vec::new();
    match trigger {
        '@' => {
            let mut seen = HashSet::new();
            let mut people: Vec<Snowflake> = Vec::new();
            // Recent authors first: who you're most likely talking to.
            for message in store
                .messages(channel_id)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
            {
                if seen.insert(message.author_id) {
                    people.push(message.author_id);
                }
            }
            if let Some(channel) = store.channel(channel_id) {
                for &id in &channel.recipients {
                    if seen.insert(id) {
                        people.push(id);
                    }
                }
            }
            if let Some(guild_id) = guild_id {
                for member in store.guild_members(guild_id) {
                    if seen.insert(member.user_id) {
                        people.push(member.user_id);
                    }
                }
            }
            for id in people {
                let Some(user) = store.user(id) else { continue };
                let name = store.display_name_in(guild_id, id);
                if !(name.to_lowercase().starts_with(&query)
                    || user.username.to_lowercase().starts_with(&query))
                {
                    continue;
                }
                out.push(Suggestion {
                    label: name.clone(),
                    detail: user.username.clone(),
                    insert: format!("@{name}"),
                    wire: Some(format!("<@{id}>")),
                    emoji: None,
                });
                if out.len() == MAX {
                    break;
                }
            }
            for (word, detail) in [
                ("everyone", "Notify everyone in this channel"),
                ("here", "Notify everyone online"),
            ] {
                if guild_id.is_some() && word.starts_with(&query) && out.len() < MAX {
                    out.push(Suggestion {
                        label: format!("@{word}"),
                        detail: detail.to_owned(),
                        insert: format!("@{word}"),
                        wire: None,
                        emoji: None,
                    });
                }
            }
        }
        '#' => {
            let Some(guild) = guild_id.and_then(|g| store.guild(g)) else {
                return out;
            };
            for entry in navigation::guild_channels(store, guild) {
                let navigation::ChannelEntry::Channel(id) = entry else {
                    continue;
                };
                let Some(channel) = store.channel(id) else {
                    continue;
                };
                let name = channel.name.clone().unwrap_or_default();
                if channel.kind != ChannelKind::Text || !name.to_lowercase().starts_with(&query) {
                    continue;
                }
                let category = channel
                    .parent_id
                    .and_then(|p| store.channel(p))
                    .and_then(|c| c.name.clone())
                    .unwrap_or_default();
                out.push(Suggestion {
                    label: format!("#{name}"),
                    detail: category.to_uppercase(),
                    insert: format!("#{name}"),
                    wire: Some(format!("<#{id}>")),
                    emoji: None,
                });
                if out.len() == MAX {
                    break;
                }
            }
        }
        ':' if query.len() >= 2 => {
            if let Some(guild) = guild_id.and_then(|g| store.guild(g)) {
                for emoji in &guild.emojis {
                    if emoji.name.to_lowercase().contains(&query) {
                        let code = if emoji.animated { "a" } else { "" };
                        out.push(Suggestion {
                            label: format!(":{}:", emoji.name),
                            detail: guild.name.clone(),
                            insert: format!(":{}:", emoji.name),
                            wire: Some(format!("<{code}:{}:{}>", emoji.name, emoji.id)),
                            emoji: Some((emoji.id, emoji.animated)),
                        });
                    }
                    if out.len() == MAX / 2 {
                        break;
                    }
                }
            }
            for (name, emoji) in EMOJI {
                if out.len() == MAX {
                    break;
                }
                if name.contains(&query) {
                    out.push(Suggestion {
                        label: format!("{emoji}  :{name}:"),
                        detail: String::new(),
                        insert: (*emoji).to_owned(),
                        wire: None,
                        emoji: None,
                    });
                }
            }
        }
        _ => {}
    }
    out
}

/// The `@ali` / `#gen` / `:fi` being typed right before the cursor:
/// (trigger, query, its length in characters).
pub fn token_before_cursor(text_before: &str) -> Option<(char, String, usize)> {
    let word: String = text_before
        .chars()
        .rev()
        .take_while(|c| !c.is_whitespace())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let mut chars = word.chars();
    let trigger = chars.next()?;
    if !matches!(trigger, '@' | '#' | ':') {
        return None;
    }
    let query: String = chars.collect();
    // `:)` and finished `:name:` aren't emoji searches.
    if trigger == ':'
        && (query.contains(':') || !query.chars().all(|c| c.is_alphanumeric() || c == '_'))
    {
        return None;
    }
    Some((trigger, query, word.chars().count()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_token_being_typed() {
        assert_eq!(token_before_cursor("hi @al"), Some(('@', "al".into(), 3)));
        assert_eq!(
            token_before_cursor("see #gen"),
            Some(('#', "gen".into(), 4))
        );
        assert_eq!(token_before_cursor("lol :fi"), Some((':', "fi".into(), 3)));
        assert_eq!(token_before_cursor("done :fire:"), None);
        assert_eq!(token_before_cursor("smile :)"), None);
        assert_eq!(token_before_cursor("plain words"), None);
    }

    #[test]
    fn emoji_shortcodes() {
        let store = Store::default();
        let found = suggestions(&store, Snowflake(1), ':', "fir");
        assert!(found.iter().any(|s| s.insert == "🔥"));
    }
}
