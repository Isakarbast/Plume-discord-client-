use plume_core::Snowflake;

use crate::ast::Node;

/// Parses the inside of `<:name:id>` or `<a:name:id>` (without the brackets).
pub fn parse_custom_emoji(inner: &str) -> Option<Node> {
    let (animated, rest) = match inner.strip_prefix("a:") {
        Some(rest) => (true, rest),
        None => (false, inner.strip_prefix(':')?),
    };
    let (name, id) = rest.split_once(':')?;
    let valid_name = !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_');
    if !valid_name {
        return None;
    }
    Some(Node::CustomEmoji {
        name: name.to_owned(),
        id: id.parse().ok().map(Snowflake)?,
        animated,
    })
}
