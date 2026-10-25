//! The channel list for one server, or the DM list.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use adw::prelude::*;
use plume_core::models::VoiceState;
use plume_core::navigation::{self, ChannelEntry};
use plume_core::{Snowflake, Store, Unread};

use crate::app::AppState;
use crate::widgets::avatar;

pub struct ChannelList {
    pub root: gtk::Box,
    title: gtk::Label,
    list: gtk::ListBox,
    /// What each row does when picked.
    rows: RefCell<Vec<Row>>,
    /// People shown under voice channels, for the speaking highlight.
    voice_members: RefCell<HashMap<Snowflake, gtk::Box>>,
    /// Set while rebuilding, so our own row changes aren't taken as clicks.
    rebuilding: Cell<bool>,
}

impl ChannelList {
    pub fn new(state: Weak<AppState>) -> Rc<Self> {
        let title = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .margin_start(12)
            .margin_end(12)
            .margin_top(12)
            .margin_bottom(6)
            .build();
        title.add_css_class("heading");

        let list = gtk::ListBox::new();
        list.add_css_class("navigation-sidebar");
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build();

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&title);
        root.append(&scrolled);
        root.set_hexpand(true);

        let channel_list = Rc::new(ChannelList {
            root,
            title,
            list,
            rows: RefCell::new(Vec::new()),
            voice_members: RefCell::new(HashMap::new()),
            rebuilding: Cell::new(false),
        });
        let state_for_voice = state.clone();
        let weak_list = Rc::downgrade(&channel_list);
        // Selecting a row (click or keyboard) opens the channel.
        channel_list.list.connect_row_selected(move |_, row| {
            let (Some(state), Some(list), Some(row)) = (state.upgrade(), weak_list.upgrade(), row)
            else {
                return;
            };
            if list.rebuilding.get() {
                return;
            }
            let target = list
                .rows
                .borrow()
                .get(row.index() as usize)
                .copied()
                .unwrap_or(Row::Inert);
            // Rebuilding the list from inside its own signal confuses GTK; wait a moment.
            let state = state.clone();
            if matches!(target, Row::Friends) && !state.showing_friends.get() {
                gtk::glib::idle_add_local_once(move || state.show_friends());
                return;
            }
            // Voice rows aren't selectable: only a real click joins (below).
            if let Row::Text(channel_id) = target
                && Some(channel_id) != state.store.borrow().active_channel()
            {
                gtk::glib::idle_add_local_once(move || state.open_channel(channel_id));
            }
        });
        // Clicking (or Enter on) a voice channel joins it.
        let weak_list = Rc::downgrade(&channel_list);
        channel_list.list.connect_row_activated(move |_, row| {
            let (Some(state), Some(list)) = (state_for_voice.upgrade(), weak_list.upgrade()) else {
                return;
            };
            let target = list.rows.borrow().get(row.index() as usize).copied();
            if let Some(Row::Voice(channel_id)) = target {
                let name = state
                    .store
                    .borrow()
                    .channel(channel_id)
                    .and_then(|c| c.name.clone())
                    .unwrap_or_default();
                tracing::info!(
                    "voice channel clicked: {channel_id} ({name}), row {}",
                    row.index()
                );
                gtk::glib::idle_add_local_once(move || state.join_voice(channel_id));
            }
        });
        channel_list
    }

    pub fn rebuild(&self, state: &Rc<AppState>) {
        self.rebuilding.set(true);
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let store = state.store.borrow();
        let mut rows = Vec::new();
        let mut voice_members = HashMap::new();
        let active = store.active_channel();

        match state.selected_guild.get().and_then(|id| store.guild(id)) {
            Some(guild) => {
                self.title.set_text(&guild.name);
                for entry in navigation::guild_channels(&store, guild) {
                    match entry {
                        ChannelEntry::Category(id) => {
                            let name = store
                                .channel(id)
                                .and_then(|c| c.name.clone())
                                .unwrap_or_default();
                            let label = gtk::Label::builder()
                                .label(name.to_uppercase())
                                .xalign(0.0)
                                .build();
                            label.add_css_class("caption-heading");
                            label.add_css_class("dim-label");
                            label.set_margin_top(10);
                            let row = gtk::ListBoxRow::builder()
                                .child(&label)
                                .activatable(false)
                                .selectable(false)
                                .build();
                            self.list.append(&row);
                            rows.push(Row::Inert);
                        }
                        ChannelEntry::Channel(id) => {
                            let name = store
                                .channel(id)
                                .and_then(|c| c.name.clone())
                                .unwrap_or_default();
                            let icon = gtk::Label::new(Some("#"));
                            icon.add_css_class("dim-label");
                            self.list
                                .append(&channel_row(icon.upcast(), &name, &store, id));
                            rows.push(Row::Text(id));
                        }
                        ChannelEntry::Thread(id) => {
                            let name = store
                                .channel(id)
                                .and_then(|c| c.name.clone())
                                .unwrap_or_default();
                            let icon = gtk::Label::new(Some("└"));
                            icon.add_css_class("dim-label");
                            let row = channel_row(icon.upcast(), &name, &store, id);
                            if let Some(child) = row.child() {
                                child.set_margin_start(18);
                            }
                            self.list.append(&row);
                            rows.push(Row::Text(id));
                        }
                        ChannelEntry::Voice(id) => {
                            let name = store
                                .channel(id)
                                .and_then(|c| c.name.clone())
                                .unwrap_or_default();
                            let icon = gtk::Image::from_icon_name("audio-volume-high-symbolic");
                            icon.add_css_class("dim-label");
                            let row = channel_row(icon.upcast(), &name, &store, id);
                            // Selection can move on its own when the list
                            // changes; joining must only happen on a click.
                            row.set_selectable(false);
                            self.list.append(&row);
                            rows.push(Row::Voice(id));
                            for member in store.voice_members(id) {
                                let (row, content) = voice_member_row(state, &store, member);
                                self.list.append(&row);
                                rows.push(Row::Inert);
                                voice_members.insert(member.user_id, content);
                            }
                        }
                    }
                }
            }
            None => {
                self.title.set_text("Direct Messages");
                let icon = gtk::Image::from_icon_name("system-users-symbolic");
                let friends = channel_row(icon.upcast(), "Friends", &store, Snowflake(0));
                self.list.append(&friends);
                rows.push(Row::Friends);
                let heading = gtk::Label::builder()
                    .label("DIRECT MESSAGES")
                    .xalign(0.0)
                    .build();
                heading.add_css_class("caption-heading");
                heading.set_margin_top(10);
                self.list.append(
                    &gtk::ListBoxRow::builder()
                        .child(&heading)
                        .activatable(false)
                        .selectable(false)
                        .build(),
                );
                rows.push(Row::Inert);
                for id in navigation::sorted_dms(&store) {
                    let Some(channel) = store.channel(id) else {
                        continue;
                    };
                    let name = navigation::dm_name(&store, channel);
                    let picture: gtk::Widget =
                        match channel.recipients.first().and_then(|&u| store.user(u)) {
                            Some(user) if channel.recipients.len() == 1 => {
                                avatar::with_status(state, user, 28).upcast()
                            }
                            _ => adw::Avatar::new(28, Some(&name), true).upcast(),
                        };
                    self.list.append(&channel_row(picture, &name, &store, id));
                    rows.push(Row::Text(id));
                }
            }
        }

        let showing_friends = state.showing_friends.get();
        if let Some(index) = rows.iter().position(|r| match r {
            Row::Friends => showing_friends,
            Row::Text(id) => !showing_friends && Some(*id) == active,
            _ => false,
        }) {
            self.list
                .select_row(self.list.row_at_index(index as i32).as_ref());
        }
        *self.rows.borrow_mut() = rows;
        *self.voice_members.borrow_mut() = voice_members;
        self.rebuilding.set(false);
    }

    /// Light up (or dim) someone in a voice channel without a rebuild.
    pub fn set_speaking(&self, user_id: Snowflake, speaking: bool) {
        if let Some(row) = self.voice_members.borrow().get(&user_id) {
            if speaking {
                row.add_css_class("speaking");
            } else {
                row.remove_css_class("speaking");
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Row {
    /// Headers and voice participants.
    Inert,
    Friends,
    Text(Snowflake),
    Voice(Snowflake),
}

fn voice_member_row(
    state: &Rc<AppState>,
    store: &Store,
    member: &VoiceState,
) -> (gtk::ListBoxRow, gtk::Box) {
    let name = store.user(member.user_id).map_or_else(
        || "Unknown user".to_owned(),
        |u| u.display_name().to_owned(),
    );
    let picture: gtk::Widget = match store.user(member.user_id) {
        Some(user) => avatar::user_avatar(state, user, 20).upcast(),
        None => adw::Avatar::new(20, Some(&name), true).upcast(),
    };
    let label = gtk::Label::builder()
        .label(&name)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    label.add_css_class("caption");

    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    content.add_css_class("voice-member");
    content.set_margin_start(24);
    content.append(&picture);
    content.append(&label);
    let icon = if member.deaf || member.self_deaf {
        Some("audio-volume-muted-symbolic")
    } else if member.mute || member.self_mute {
        Some("microphone-disabled-symbolic")
    } else {
        None
    };
    if let Some(icon) = icon {
        let image = gtk::Image::from_icon_name(icon);
        image.add_css_class("dim-label");
        content.append(&image);
    }
    crate::widgets::message_row::open_profile_on_click(&content, member.user_id);
    if store.me().is_none_or(|me| me.id != member.user_id) {
        attach_volume_menu(state, &content, member.user_id);
    }
    if state.is_speaking(member.user_id) {
        content.add_css_class("speaking");
    }
    let row = gtk::ListBoxRow::builder()
        .child(&content)
        .activatable(false)
        .selectable(false)
        .build();
    (row, content)
}

fn channel_row(icon: gtk::Widget, name: &str, store: &Store, id: Snowflake) -> gtk::ListBoxRow {
    let unread: Unread = store.unread(id);
    let label = gtk::Label::builder()
        .label(name)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    if unread.unread {
        label.add_css_class("unread");
    } else if store.is_muted(id) {
        label.add_css_class("dim-label");
    }

    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    content.append(&icon);
    content.append(&label);
    if unread.mentions > 0 {
        let badge = gtk::Label::new(Some(&unread.mentions.min(99).to_string()));
        badge.add_css_class("badge");
        badge.set_valign(gtk::Align::Center);
        content.append(&badge);
    }
    let row = gtk::ListBoxRow::builder().child(&content).build();
    if id.0 != 0 {
        attach_menu(&row, id);
    }
    row
}

/// Right-click a channel: mark as read, copy its ID.
fn attach_menu(row: &gtk::ListBoxRow, id: Snowflake) {
    let click = gtk::GestureClick::new();
    click.set_button(gtk::gdk::BUTTON_SECONDARY);
    click.connect_pressed(move |gesture, _, x, y| {
        let Some(widget) = gesture.widget() else {
            return;
        };
        let menu = gtk::gio::Menu::new();
        menu.append(
            Some("Mark As Read"),
            Some(&format!("win.mark-channel-read::{id}")),
        );
        menu.append(Some("Copy Channel ID"), Some(&format!("win.copy-id::{id}")));
        popup_menu(&widget, &menu, x, y);
    });
    row.add_controller(click);
}

/// Show a menu where the pointer is.
pub fn popup_menu(widget: &gtk::Widget, menu: &gtk::gio::Menu, x: f64, y: f64) {
    let popover = gtk::PopoverMenu::from_model(Some(menu));
    popover.set_parent(widget);
    popover.set_has_arrow(false);
    popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    popover.connect_closed(|popover| {
        let popover = popover.clone();
        gtk::glib::idle_add_local_once(move || popover.unparent());
    });
    popover.popup();
}

/// Right-click someone in voice: a slider for how loud they are for you.
fn attach_volume_menu(state: &Rc<AppState>, content: &gtk::Box, user_id: Snowflake) {
    let click = gtk::GestureClick::new();
    click.set_button(gtk::gdk::BUTTON_SECONDARY);
    let state = Rc::downgrade(state);
    click.connect_pressed(move |gesture, _, x, y| {
        let (Some(widget), Some(state)) = (gesture.widget(), state.upgrade()) else {
            return;
        };
        let label = gtk::Label::new(Some("User Volume"));
        label.add_css_class("heading");
        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 200.0, 5.0);
        scale.set_value(f64::from(state.user_volume(user_id)));
        scale.set_draw_value(true);
        scale.set_format_value_func(|_, value| format!("{value:.0}%"));
        scale.add_mark(100.0, gtk::PositionType::Bottom, None);
        scale.set_size_request(220, -1);
        let weak = Rc::downgrade(&state);
        scale.connect_value_changed(move |scale| {
            if let Some(state) = weak.upgrade() {
                state.set_user_volume(user_id, scale.value().round() as u32);
            }
        });
        let profile = gtk::Button::with_label("Profile");
        profile.add_css_class("flat");
        profile.set_action_name(Some("win.profile"));
        profile.set_action_target_value(Some(&user_id.to_string().to_variant()));
        let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
        content.append(&label);
        content.append(&scale);
        content.append(&profile);
        let popover = gtk::Popover::builder().child(&content).build();
        popover.set_parent(&widget);
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.connect_closed(|popover| {
            let popover = popover.clone();
            gtk::glib::idle_add_local_once(move || popover.unparent());
        });
        popover.popup();
    });
    content.add_controller(click);
}
