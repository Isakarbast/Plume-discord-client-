//! Discord's permission math, from
//! <https://discord.com/developers/docs/topics/permissions>.
//!
//! Your permissions in a channel = the server's `@everyone` role, plus your
//! roles, then the channel's overwrites for `@everyone`, your roles, and
//! you personally, in that order.

use crate::ids::Snowflake;
use crate::models::{Channel, Guild, OverwriteKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Permissions(pub u64);

impl Permissions {
    pub const ADMINISTRATOR: Permissions = Permissions(1 << 3);
    pub const VIEW_CHANNEL: Permissions = Permissions(1 << 10);
    pub const SEND_MESSAGES: Permissions = Permissions(1 << 11);
    pub const READ_MESSAGE_HISTORY: Permissions = Permissions(1 << 16);
    pub const ADD_REACTIONS: Permissions = Permissions(1 << 6);
    pub const CONNECT: Permissions = Permissions(1 << 20);
    pub const KICK_MEMBERS: Permissions = Permissions(1 << 1);
    pub const BAN_MEMBERS: Permissions = Permissions(1 << 2);
    pub const MANAGE_MESSAGES: Permissions = Permissions(1 << 13);
    pub const MODERATE_MEMBERS: Permissions = Permissions(1 << 40);
    pub const ALL: Permissions = Permissions(u64::MAX);

    pub fn contains(self, other: Permissions) -> bool {
        self.0 & other.0 == other.0
    }
}

/// Your permissions in the server as a whole (no channel overwrites).
pub fn guild_permissions(guild: &Guild, me: Snowflake) -> Permissions {
    if guild.owner_id == Some(me) {
        return Permissions::ALL;
    }
    // The @everyone role has the same ID as the server.
    let mut bits = role_bits(guild, guild.id);
    for role in &guild.my_roles {
        bits |= role_bits(guild, *role);
    }
    if bits & Permissions::ADMINISTRATOR.0 != 0 {
        return Permissions::ALL;
    }
    Permissions(bits)
}

/// Your permissions in one channel of that server.
pub fn channel_permissions(guild: &Guild, channel: &Channel, me: Snowflake) -> Permissions {
    let base = guild_permissions(guild, me);
    if base == Permissions::ALL {
        return base;
    }
    let mut bits = base.0;

    // 1. The @everyone overwrite.
    if let Some(everyone) = channel.overwrites.iter().find(|o| o.id == guild.id) {
        bits &= !everyone.deny;
        bits |= everyone.allow;
    }
    // 2. All your roles' overwrites together: denies first, then allows.
    let (mut allow, mut deny) = (0, 0);
    for overwrite in &channel.overwrites {
        if overwrite.kind == OverwriteKind::Role && guild.my_roles.contains(&overwrite.id) {
            allow |= overwrite.allow;
            deny |= overwrite.deny;
        }
    }
    bits &= !deny;
    bits |= allow;
    // 3. An overwrite for you specifically.
    if let Some(member) = channel
        .overwrites
        .iter()
        .find(|o| o.kind == OverwriteKind::Member && o.id == me)
    {
        bits &= !member.deny;
        bits |= member.allow;
    }
    Permissions(bits)
}

fn role_bits(guild: &Guild, role_id: Snowflake) -> u64 {
    guild
        .roles
        .iter()
        .find(|r| r.id == role_id)
        .map_or(0, |r| r.permissions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ChannelKind, Overwrite, Role};

    const GUILD: Snowflake = Snowflake(1);
    const ME: Snowflake = Snowflake(100);
    const MOD_ROLE: Snowflake = Snowflake(50);
    const VIEW: u64 = Permissions::VIEW_CHANNEL.0;
    const SEND: u64 = Permissions::SEND_MESSAGES.0;

    fn guild(everyone: u64, my_roles: Vec<Snowflake>) -> Guild {
        Guild {
            id: GUILD,
            name: "Test".into(),
            icon: None,
            channels: Vec::new(),
            owner_id: Some(Snowflake(999)),
            roles: vec![
                Role {
                    id: GUILD,
                    permissions: everyone,
                    ..Default::default()
                },
                Role {
                    id: MOD_ROLE,
                    permissions: VIEW | SEND,
                    ..Default::default()
                },
            ],
            my_roles,
            emojis: Vec::new(),
            voice_states: Vec::new(),
            stickers: Vec::new(),
        }
    }

    fn channel(overwrites: Vec<Overwrite>) -> Channel {
        Channel {
            id: Snowflake(10),
            guild_id: Some(GUILD),
            name: Some("secret".into()),
            kind: ChannelKind::Text,
            parent_id: None,
            position: 0,
            recipients: Vec::new(),
            last_message_id: None,
            overwrites,
            thread: None,
        }
    }

    fn overwrite(id: Snowflake, kind: OverwriteKind, allow: u64, deny: u64) -> Overwrite {
        Overwrite {
            id,
            kind,
            allow,
            deny,
        }
    }

    #[test]
    fn everyone_can_see_a_normal_channel() {
        let perms = channel_permissions(&guild(VIEW | SEND, vec![]), &channel(vec![]), ME);
        assert!(perms.contains(Permissions::VIEW_CHANNEL));
    }

    #[test]
    fn hidden_from_everyone_but_a_role() {
        let channel = channel(vec![
            overwrite(GUILD, OverwriteKind::Role, 0, VIEW),
            overwrite(MOD_ROLE, OverwriteKind::Role, VIEW, 0),
        ]);
        let outsider = channel_permissions(&guild(VIEW, vec![]), &channel, ME);
        let moderator = channel_permissions(&guild(VIEW, vec![MOD_ROLE]), &channel, ME);
        assert!(!outsider.contains(Permissions::VIEW_CHANNEL));
        assert!(moderator.contains(Permissions::VIEW_CHANNEL));
    }

    #[test]
    fn member_overwrite_beats_roles() {
        let channel = channel(vec![
            overwrite(MOD_ROLE, OverwriteKind::Role, VIEW, 0),
            overwrite(ME, OverwriteKind::Member, 0, VIEW),
        ]);
        let perms = channel_permissions(&guild(VIEW, vec![MOD_ROLE]), &channel, ME);
        assert!(!perms.contains(Permissions::VIEW_CHANNEL));
    }

    #[test]
    fn owner_and_admin_see_everything() {
        let hidden = channel(vec![overwrite(GUILD, OverwriteKind::Role, 0, VIEW)]);
        let mut owned = guild(0, vec![]);
        owned.owner_id = Some(ME);
        assert_eq!(channel_permissions(&owned, &hidden, ME), Permissions::ALL);

        let admin = guild(Permissions::ADMINISTRATOR.0, vec![]);
        assert_eq!(channel_permissions(&admin, &hidden, ME), Permissions::ALL);
    }
}
