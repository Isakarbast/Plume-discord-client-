use plume_core::Snowflake;

use crate::ast::Node;
use crate::emoji::parse_custom_emoji;

/// Tries to parse a `<…>` token at the start of `s`. Returns the node and how
/// many bytes it used.
pub fn parse_angle(s: &str) -> Option<(Node, usize)> {
    let end = s.find('>')?;
    let inner = &s[1..end];
    if inner.is_empty() || inner.contains(char::is_whitespace) {
        return None;
    }
    let id = |text: &str| text.parse::<u64>().ok().map(Snowflake);

    let node = if let Some(rest) = inner.strip_prefix("@&") {
        Node::RoleMention(id(rest)?)
    } else if let Some(rest) = inner.strip_prefix('@') {
        // `<@!id>` is an old nickname-mention form; same thing.
        Node::UserMention(id(rest.strip_prefix('!').unwrap_or(rest))?)
    } else if let Some(rest) = inner.strip_prefix('#') {
        Node::ChannelMention(id(rest)?)
    } else if let Some(rest) = inner.strip_prefix("t:") {
        let (unix, style) = match rest.split_once(':') {
            Some((unix, style)) => {
                let mut letters = style.chars();
                let style = letters.next().filter(|_| letters.next().is_none())?;
                (unix, Some(style))
            }
            None => (rest, None),
        };
        Node::Timestamp {
            unix: unix.parse().ok()?,
            style,
        }
    } else if inner.starts_with("http://") || inner.starts_with("https://") {
        // `<https://…>` is a link with its preview turned off.
        Node::Link(inner.to_owned())
    } else {
        parse_custom_emoji(inner)?
    };
    Some((node, end + 1))
}
