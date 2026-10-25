//! The member list on the right: who's in this server (grouped by role,
//! online first, as Discord sends it) or this DM, with status and activity.

use std::rc::{Rc, Weak};

use adw::prelude::*;
use plume_core::models::{MemberListItem, Status};
use plume_core::{Snowflake, Store};

use crate::app::AppState;
use crate::widgets::avatar;
use crate::widgets::message_row::open_profile_on_click;

pub struct MemberList {
    pub root: gtk::ScrolledWindow,
    list: gtk::ListBox,
}

impl MemberList {
    pub fn new(_state: Weak<AppState>) -> Rc<Self> {
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.add_css_class("member-list");
        let root = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build();
        root.add_css_class("member-list");
        Rc::new(MemberList { root, list })
    }

    pub fn rebuild(&self, state: &Rc<AppState>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let store = state.store.borrow();
        let Some(channel) = store.active_channel().and_then(|id| store.channel(id)) else {
            return;
        };

        match channel.guild_id {
            Some(guild_id) => {
                let items = store.member_list(guild_id);
                if items.is_empty() {
                    self.list.append(&group_row("MEMBERS — loading…"));
                }
                let mut offline = false;
                for item in items {
                    match item {
                        MemberListItem::Group { id, count } => {
                            offline = id == "offline";
                            let name = match id.as_str() {
                                "online" => "Online".to_owned(),
                                "offline" => "Offline".to_owned(),
                                role => role
                                    .parse::<u64>()
                                    .ok()
                                    .and_then(|r| {
                                        store.guild(guild_id)?.roles.iter().find(|x| x.id.0 == r)
                                    })
                                    .map_or_else(|| "Members".to_owned(), |r| r.name.clone()),
                            };
                            self.list
                                .append(&group_row(&format!("{} — {count}", name.to_uppercase())));
                        }
                        MemberListItem::Member(user_id) if user_id.0 != 0 => {
                            if let Some(row) =
                                member_row(state, &store, Some(guild_id), *user_id, offline)
                            {
                                self.list.append(&row);
                            }
                        }
                        MemberListItem::Member(_) => {}
                    }
                }
            }
            None => {
                let mut people: Vec<Snowflake> = channel.recipients.clone();
                people.extend(store.me().map(|u| u.id));
                self.list
                    .append(&group_row(&format!("MEMBERS — {}", people.len())));
                for user_id in people {
                    let offline = store.presence(user_id).status == Status::Offline;
                    if let Some(row) = member_row(state, &store, None, user_id, offline) {
                        self.list.append(&row);
                    }
                }
            }
        }
    }
}

fn group_row(text: &str) -> gtk::ListBoxRow {
    let label = gtk::Label::builder().label(text).xalign(0.0).build();
    label.add_css_class("member-group");
    gtk::ListBoxRow::builder()
        .child(&label)
        .activatable(false)
        .build()
}

fn member_row(
    state: &Rc<AppState>,
    store: &Store,
    guild_id: Option<Snowflake>,
    user_id: Snowflake,
    offline: bool,
) -> Option<gtk::ListBoxRow> {
    let user = store.user(user_id)?;
    let picture = avatar::with_status(state, user, 32);

    let name = gtk::Label::builder()
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    let display = store.display_name_in(guild_id, user_id);
    let escaped = gtk::glib::markup_escape_text(&display);
    match guild_id.and_then(|g| store.role_color(g, user_id)) {
        Some(color) => name.set_markup(&format!(
            "<span foreground=\"#{color:06x}\">{escaped}</span>"
        )),
        None => name.set_text(&display),
    }

    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_valign(gtk::Align::Center);
    text.set_hexpand(true);
    text.append(&name);
    if let Some(activity) = store.presence(user_id).activity {
        let line = gtk::Label::builder()
            .label(&activity)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        line.add_css_class("caption");
        line.add_css_class("dim-label");
        text.append(&line);
    }

    let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    content.set_margin_top(3);
    content.set_margin_bottom(3);
    content.set_margin_start(8);
    content.set_margin_end(8);
    content.append(&picture);
    content.append(&text);
    if offline {
        content.add_css_class("member-offline");
    }
    open_profile_on_click(&content, user_id);
    crate::widgets::person_menu::attach(state, &content, guild_id, user_id);
    Some(
        gtk::ListBoxRow::builder()
            .child(&content)
            .activatable(false)
            .build(),
    )
}
