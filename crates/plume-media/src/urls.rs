//! Where Discord's CDN keeps each kind of image.

use plume_core::Snowflake;
use plume_core::models::{Guild, User};

const CDN: &str = "https://cdn.discordapp.com";

/// The CDN only serves power-of-two sizes from 16 to 4096.
pub fn cdn_size(pixels: u32) -> u32 {
    pixels.clamp(16, 4096).next_power_of_two()
}

/// A user's avatar, or one of Discord's default ones if they have none.
pub fn avatar(user: &User, pixels: u32) -> String {
    match &user.avatar {
        Some(hash) => format!(
            "{CDN}/avatars/{}/{hash}.webp?size={}",
            user.id,
            cdn_size(pixels)
        ),
        // New usernames pick the default avatar from the ID.
        None => format!("{CDN}/embed/avatars/{}.png", (user.id.0 >> 22) % 6),
    }
}

/// Whether the avatar has an animated version ("a_" hashes).
pub fn is_animated(hash: &str) -> bool {
    hash.starts_with("a_")
}

pub fn guild_icon(guild: &Guild, pixels: u32) -> Option<String> {
    let hash = guild.icon.as_ref()?;
    Some(format!(
        "{CDN}/icons/{}/{hash}.webp?size={}",
        guild.id,
        cdn_size(pixels)
    ))
}

pub fn emoji(id: Snowflake, animated: bool, pixels: u32) -> String {
    let ext = if animated { "gif" } else { "webp" };
    format!("{CDN}/emojis/{id}.{ext}?size={}", cdn_size(pixels))
}

/// A sticker image (`format` 4 is GIF; Lottie stickers can't be drawn).
pub fn sticker(id: Snowflake, format: u8, pixels: u32) -> Option<String> {
    let ext = match format {
        1 | 2 => "png",
        4 => "gif",
        _ => return None,
    };
    Some(format!(
        "https://media.discordapp.net/stickers/{id}.{ext}?size={}",
        cdn_size(pixels)
    ))
}

/// Video and audio files, which open in your media player.
pub fn is_media(filename: &str) -> bool {
    let lower = filename.to_ascii_lowercase();
    [
        ".mp4", ".webm", ".mov", ".mkv", ".mp3", ".ogg", ".wav", ".flac", ".m4a", ".opus",
    ]
    .iter()
    .any(|ext| lower.ends_with(ext))
}

/// A resized preview of an image attachment from Discord's media proxy,
/// so we never download a 12 MB photo to show it 400 pixels wide.
pub fn attachment_preview(url: &str, width: u32, height: u32) -> String {
    let proxied = url.replacen(
        "https://cdn.discordapp.com/",
        "https://media.discordapp.net/",
        1,
    );
    let separator = if proxied.contains('?') { '&' } else { '?' };
    format!("{proxied}{separator}width={width}&height={height}")
}

/// Whether a filename looks like an image we can show inline.
pub fn is_image(filename: &str) -> bool {
    let lower = filename.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".webp"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(avatar: Option<&str>) -> User {
        User {
            id: Snowflake(80351110224678912),
            username: "nelly".into(),
            global_name: None,
            avatar: avatar.map(str::to_owned),
        }
    }

    #[test]
    fn avatar_urls() {
        assert_eq!(
            avatar(&user(Some("8342729096ea3675442027381ff50dfe")), 40),
            "https://cdn.discordapp.com/avatars/80351110224678912/8342729096ea3675442027381ff50dfe.webp?size=64"
        );
        assert!(avatar(&user(None), 40).starts_with("https://cdn.discordapp.com/embed/avatars/"));
        assert!(is_animated("a_123"));
    }

    #[test]
    fn sizes_are_powers_of_two_in_range() {
        assert_eq!(cdn_size(1), 16);
        assert_eq!(cdn_size(100), 128);
        assert_eq!(cdn_size(10_000), 4096);
    }

    #[test]
    fn attachment_previews_go_through_the_media_proxy() {
        assert_eq!(
            attachment_preview(
                "https://cdn.discordapp.com/attachments/1/2/cat.png?ex=abc",
                400,
                300
            ),
            "https://media.discordapp.net/attachments/1/2/cat.png?ex=abc&width=400&height=300"
        );
        assert!(is_image("Cat.JPG"));
        assert!(!is_image("notes.txt"));
    }
}
