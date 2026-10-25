//! Keeps the GTK list model of the open channel in step with the store.
//!
//! The model holds only message IDs (wrapped in `BoxedAnyObject`); rows look
//! the message up in the store when they're drawn. So a list of thousands of
//! messages costs a few bytes per message, and GtkListView only builds
//! widgets for the rows on screen.

use gtk::gio;
use gtk::glib::BoxedAnyObject;
use gtk::prelude::*;
use plume_core::{Snowflake, Store};

pub struct MessageListModel {
    pub model: gio::ListStore,
    channel: Option<Snowflake>,
}

impl MessageListModel {
    pub fn new() -> Self {
        MessageListModel {
            model: gio::ListStore::new::<BoxedAnyObject>(),
            channel: None,
        }
    }

    pub fn channel(&self) -> Option<Snowflake> {
        self.channel
    }

    pub fn id_at(&self, position: u32) -> Option<Snowflake> {
        let item = self
            .model
            .item(position)?
            .downcast::<BoxedAnyObject>()
            .ok()?;
        let id = *item.borrow::<Snowflake>();
        Some(id)
    }

    pub fn len(&self) -> u32 {
        self.model.n_items()
    }

    /// Makes the model match the store's messages for `channel`, touching
    /// only what changed so scrolling and selection survive.
    pub fn sync(&mut self, store: &Store, channel: Option<Snowflake>) {
        let wanted: Vec<Snowflake> = match channel {
            Some(channel) => store.messages(channel).map(|m| m.id).collect(),
            None => Vec::new(),
        };
        if channel != self.channel {
            self.channel = channel;
            self.replace_all(&wanted);
            return;
        }
        let current: Vec<Snowflake> = (0..self.len()).filter_map(|i| self.id_at(i)).collect();
        if current == wanted {
            return;
        }
        // The common cases: new messages at the end, or older ones loaded at the start.
        if wanted.starts_with(&current) {
            let added: Vec<BoxedAnyObject> = wanted[current.len()..]
                .iter()
                .map(|id| BoxedAnyObject::new(*id))
                .collect();
            self.model.splice(self.len(), 0, &added);
        } else if wanted.ends_with(&current) && !current.is_empty() {
            let added: Vec<BoxedAnyObject> = wanted[..wanted.len() - current.len()]
                .iter()
                .map(|id| BoxedAnyObject::new(*id))
                .collect();
            self.model.splice(0, 0, &added);
        } else {
            self.replace_all(&wanted);
        }
    }

    /// Redraws one message (after an edit or a reaction).
    pub fn refresh(&self, id: Snowflake) {
        if let Some(position) = (0..self.len()).find(|&i| self.id_at(i) == Some(id)) {
            // Replacing an item with an identical one makes GTK rebind its row.
            self.model.splice(position, 1, &[BoxedAnyObject::new(id)]);
        }
    }

    fn replace_all(&self, ids: &[Snowflake]) {
        let items: Vec<BoxedAnyObject> = ids.iter().map(|id| BoxedAnyObject::new(*id)).collect();
        self.model.splice(0, self.len(), &items);
    }
}
