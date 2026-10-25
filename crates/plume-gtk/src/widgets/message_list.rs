//! The scrolling list of messages in the open channel.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::glib::BoxedAnyObject;
use plume_core::Snowflake;

use crate::app::AppState;
use crate::state::view_models::MessageListModel;
use crate::widgets::message_row;

/// How close to the bottom (in pixels) still counts as "at the bottom".
const BOTTOM_SLACK: f64 = 60.0;

pub struct MessageList {
    pub root: gtk::Stack,
    /// "Jump to Present", shown when scrolled up.
    jump: gtk::Button,
    scrolled: gtk::ScrolledWindow,
    model: RefCell<MessageListModel>,
    /// Keep following new messages (true while the user is at the bottom).
    stick_to_bottom: Cell<bool>,
    /// Distance from the bottom to restore after older messages are prepended.
    anchor: Cell<Option<f64>>,
}

impl MessageList {
    pub fn new(state: Weak<AppState>) -> Rc<Self> {
        let model = MessageListModel::new();
        let store_model = model.model.clone();

        let factory = gtk::SignalListItemFactory::new();
        let bind_state = state.clone();
        factory.connect_bind(move |_, item| {
            let (Some(item), Some(state)) =
                (item.downcast_ref::<gtk::ListItem>(), bind_state.upgrade())
            else {
                return;
            };
            let Some(id) = item
                .item()
                .and_downcast::<BoxedAnyObject>()
                .map(|o| *o.borrow::<Snowflake>())
            else {
                return;
            };
            let position = item.position();
            let previous = (position > 0)
                .then(|| store_model.item(position - 1))
                .flatten()
                .and_downcast::<BoxedAnyObject>()
                .map(|o| *o.borrow::<Snowflake>());
            item.set_child(Some(&message_row::build(&state, id, previous)));
        });
        factory.connect_unbind(|_, item| {
            if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
                item.set_child(None::<&gtk::Widget>);
            }
        });

        let selection = gtk::NoSelection::new(Some(model.model.clone()));
        let view = gtk::ListView::new(Some(selection), Some(factory));
        view.add_css_class("message-list");
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&view)
            .build();

        let empty = adw::StatusPage::builder()
            .icon_name("mail-message-new-symbolic")
            .title("Pick a channel")
            .description("Choose a server on the left, or press Ctrl+K to jump to any channel.")
            .build();
        let jump = gtk::Button::with_label("Jump to Present");
        jump.add_css_class("pill");
        jump.add_css_class("suggested-action");
        jump.set_halign(gtk::Align::Center);
        jump.set_valign(gtk::Align::End);
        jump.set_margin_bottom(12);
        jump.set_visible(false);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&scrolled));
        overlay.add_overlay(&jump);
        let root = gtk::Stack::new();
        root.add_named(&empty, Some("empty"));
        root.add_named(&overlay, Some("messages"));

        let list = Rc::new(MessageList {
            root,
            jump,
            scrolled,
            model: RefCell::new(model),
            stick_to_bottom: Cell::new(true),
            anchor: Cell::new(None),
        });

        let adjustment = list.scrolled.vadjustment();
        let weak = Rc::downgrade(&list);
        adjustment.connect_value_changed(move |adj| {
            let Some(list) = weak.upgrade() else { return };
            let at_bottom = adj.value() + adj.page_size() >= adj.upper() - BOTTOM_SLACK;
            list.stick_to_bottom.set(at_bottom);
            // More than a few screens up: offer a way back.
            list.jump
                .set_visible(adj.upper() - (adj.value() + adj.page_size()) > adj.page_size() * 2.0);
            let scrollable = adj.upper() > adj.page_size() + 1.0;
            if scrollable
                && adj.value() <= adj.lower() + 1.0
                && let Some(state) = state.upgrade()
            {
                list.anchor.set(Some(adj.upper() - adj.value()));
                state.load_older();
            }
        });
        let weak = Rc::downgrade(&list);
        list.jump.connect_clicked(move |_| {
            if let Some(list) = weak.upgrade() {
                list.scroll_to_bottom();
            }
        });
        // When content grows: follow the bottom, or keep the reading position.
        let weak = Rc::downgrade(&list);
        adjustment.connect_upper_notify(move |adj| {
            let Some(list) = weak.upgrade() else { return };
            if let Some(distance) = list.anchor.take().filter(|_| !list.stick_to_bottom.get()) {
                adj.set_value(adj.upper() - distance);
            } else if list.stick_to_bottom.get() {
                adj.set_value(adj.upper() - adj.page_size());
            }
        });
        list
    }

    /// Matches the list to the store's open channel.
    pub fn sync(&self, state: &AppState) {
        let store = state.store.borrow();
        let channel = store.active_channel();
        let switched = channel != self.model.borrow().channel();
        self.model.borrow_mut().sync(&store, channel);
        self.root.set_visible_child_name(if channel.is_some() {
            "messages"
        } else {
            "empty"
        });
        if switched {
            self.anchor.set(None);
            self.scroll_to_bottom();
        }
    }

    pub fn refresh(&self, id: Snowflake) {
        self.model.borrow().refresh(id);
    }

    pub fn scroll_to_bottom(&self) {
        self.stick_to_bottom.set(true);
        let adjustment = self.scrolled.vadjustment();
        // After GTK has measured the new rows.
        gtk::glib::idle_add_local_once(move || {
            adjustment.set_value(adjustment.upper() - adjustment.page_size())
        });
    }
}
