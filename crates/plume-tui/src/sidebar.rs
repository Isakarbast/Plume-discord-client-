use std::collections::HashSet;

use plume_core::models::Guild;
use plume_core::navigation::{self, ChannelEntry};
use plume_core::{Snowflake, Store};

/// One row in the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    DmHeader,
    ServerHeader,
    Dm(Snowflake),
    Guild(Snowflake),
    Category(Snowflake),
    Channel(Snowflake),
}

/// The tree of DMs, servers and channels, flattened into rows.
///
/// Rebuilding is cheap (it's just sorting IDs), so instead of patching the
/// list on every change we mark it dirty and rebuild before the next draw.
pub struct Sidebar {
    entries: Vec<Entry>,
    selected: usize,
    expanded: HashSet<Snowflake>,
    dms_collapsed: bool,
    dirty: bool,
    /// Entry to select after the next rebuild (it may not exist yet).
    pending_select: Option<Entry>,
}

/// DMs beyond this many are hidden; use the quick switcher (Ctrl+K) for older ones.
const MAX_DMS: usize = 40;

impl Default for Sidebar {
    fn default() -> Self {
        Sidebar {
            entries: Vec::new(),
            selected: 0,
            expanded: HashSet::new(),
            dms_collapsed: false,
            dirty: true,
            pending_select: None,
        }
    }
}

impl Sidebar {
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn selected(&self) -> Option<Entry> {
        self.entries.get(self.selected).copied()
    }

    pub fn is_expanded(&self, guild_id: Snowflake) -> bool {
        self.expanded.contains(&guild_id)
    }

    pub fn dms_collapsed(&self) -> bool {
        self.dms_collapsed
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn move_by(&mut self, delta: isize) {
        if self.entries.is_empty() {
            return;
        }
        let last = self.entries.len() - 1;
        self.selected = self.selected.saturating_add_signed(delta).min(last);
    }

    pub fn select_first(&mut self) {
        self.selected = 0;
    }

    pub fn select_last(&mut self) {
        self.selected = self.entries.len().saturating_sub(1);
    }

    pub fn toggle_dms(&mut self) {
        self.dms_collapsed = !self.dms_collapsed;
        self.dirty = true;
    }

    pub fn toggle_guild(&mut self, guild_id: Snowflake) {
        if !self.expanded.remove(&guild_id) {
            self.expanded.insert(guild_id);
        }
        self.dirty = true;
    }

    pub fn collapse_guild(&mut self, guild_id: Snowflake) {
        self.expanded.remove(&guild_id);
        self.dirty = true;
        self.pending_select = Some(Entry::Guild(guild_id));
    }

    /// Selects a channel, expanding its server if needed.
    pub fn reveal_channel(&mut self, store: &Store, channel_id: Snowflake) {
        let entry = match store.channel(channel_id) {
            Some(channel) if is_dm(channel) => {
                self.dms_collapsed = false;
                Entry::Dm(channel_id)
            }
            Some(channel) => {
                if let Some(guild_id) = channel.guild_id {
                    self.expanded.insert(guild_id);
                }
                Entry::Channel(channel_id)
            }
            None => return,
        };
        self.dirty = true;
        self.pending_select = Some(entry);
    }

    pub fn rebuild_if_dirty(&mut self, store: &Store) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let keep = self.pending_select.take().or_else(|| self.selected());

        self.entries.clear();
        self.entries.push(Entry::DmHeader);
        if !self.dms_collapsed {
            self.entries
                .extend(sorted_dms(store).into_iter().take(MAX_DMS).map(Entry::Dm));
        }
        self.entries.push(Entry::ServerHeader);
        for guild in sorted_guilds(store) {
            self.entries.push(Entry::Guild(guild.id));
            if self.expanded.contains(&guild.id) {
                self.entries.extend(guild_channels(store, guild));
            }
        }

        self.selected = keep
            .and_then(|entry| self.entries.iter().position(|e| *e == entry))
            .unwrap_or(self.selected)
            .min(self.entries.len().saturating_sub(1));
    }
}

pub use plume_core::navigation::{channel_label, dm_name, is_dm, sorted_dms, sorted_guilds};

/// A server's channels as sidebar rows.
pub fn guild_channels(store: &Store, guild: &Guild) -> Vec<Entry> {
    navigation::guild_channels(store, guild)
        .into_iter()
        .filter_map(|entry| match entry {
            ChannelEntry::Category(id) => Some(Entry::Category(id)),
            ChannelEntry::Channel(id) | ChannelEntry::Thread(id) => Some(Entry::Channel(id)),
            // Voice is GTK-only for now.
            ChannelEntry::Voice(_) => None,
        })
        .collect()
}
