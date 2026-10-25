//! The Friends page (top of the DM list): online / all / pending / blocked,
//! and adding friends by username.

use std::cell::Cell;
use std::rc::{Rc, Weak};

use adw::prelude::*;
use plume_core::Snowflake;
use plume_core::models::{RelationshipKind, Status};

use crate::app::AppState;
use crate::widgets::avatar;
use crate::widgets::message_row::open_profile_on_click;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Online,
    All,
    Pending,
    Blocked,
    Add,
}

pub struct FriendsView {
    pub root: gtk::Box,
    list: gtk::ListBox,
    heading: gtk::Label,
    pages: gtk::Stack,
    tab: Cell<Tab>,
}

impl FriendsView {
    pub fn new(state: Weak<AppState>) -> Rc<Self> {
        let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        tabs.add_css_class("friends-tabs");
        tabs.set_margin_start(16);
        tabs.set_margin_end(16);
        tabs.set_margin_top(10);
        tabs.set_margin_bottom(10);
        let title = gtk::Label::new(Some("Friends"));
        title.add_css_class("heading");
        title.set_margin_end(12);
        tabs.append(&gtk::Image::from_icon_name("system-users-symbolic"));
        tabs.append(&title);

        let heading = gtk::Label::builder().xalign(0.0).build();
        heading.add_css_class("member-group");

        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.add_css_class("member-list");
        let list_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list_page.append(&heading);
        list_page.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .vexpand(true)
                .child(&list)
                .build(),
        );

        // Add Friend page.
        let add_title = gtk::Label::new(Some("ADD FRIEND"));
        add_title.add_css_class("heading");
        add_title.set_xalign(0.0);
        let add_help = gtk::Label::new(Some("You can add friends with their Discord username."));
        add_help.add_css_class("dim-label");
        add_help.set_xalign(0.0);
        let entry = gtk::Entry::builder()
            .placeholder_text("You can add friends with their Discord username.")
            .hexpand(true)
            .build();
        let send = gtk::Button::with_label("Send Friend Request");
        send.add_css_class("suggested-action");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&entry);
        row.append(&send);
        let add_page = gtk::Box::new(gtk::Orientation::Vertical, 8);
        add_page.set_margin_start(24);
        add_page.set_margin_end(24);
        add_page.set_margin_top(20);
        add_page.append(&add_title);
        add_page.append(&add_help);
        add_page.append(&row);

        let pages = gtk::Stack::new();
        pages.add_named(&list_page, Some("list"));
        pages.add_named(&add_page, Some("add"));
        pages.set_vexpand(true);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&tabs);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&pages);

        let view = Rc::new(FriendsView {
            root,
            list,
            heading,
            pages,
            tab: Cell::new(Tab::Online),
        });

        let mut group: Option<gtk::ToggleButton> = None;
        for (label, tab) in [
            ("Online", Tab::Online),
            ("All", Tab::All),
            ("Pending", Tab::Pending),
            ("Blocked", Tab::Blocked),
            ("Add Friend", Tab::Add),
        ] {
            let button = gtk::ToggleButton::with_label(label);
            button.add_css_class("flat");
            if tab == Tab::Add {
                button.add_css_class("add-friend-button");
            }
            match &group {
                Some(first) => button.set_group(Some(first)),
                None => {
                    button.set_active(true);
                    group = Some(button.clone());
                }
            }
            let weak_view = Rc::downgrade(&view);
            let state = state.clone();
            button.connect_toggled(move |button| {
                if !button.is_active() {
                    return;
                }
                if let (Some(view), Some(state)) = (weak_view.upgrade(), state.upgrade()) {
                    view.tab.set(tab);
                    view.rebuild(&state);
                }
            });
            tabs.append(&button);
        }

        let send_request = {
            let state = state.clone();
            let entry = entry.clone();
            move || {
                let name = entry.text().trim().to_owned();
                if name.is_empty() {
                    return;
                }
                if let Some(state) = state.upgrade() {
                    state.send_friend_request(&name);
                    entry.set_text("");
                }
            }
        };
        let again = send_request.clone();
        send.connect_clicked(move |_| send_request());
        entry.connect_activate(move |_| again());
        view
    }

    pub fn rebuild(&self, state: &Rc<AppState>) {
        let tab = self.tab.get();
        if tab == Tab::Add {
            self.pages.set_visible_child_name("add");
            return;
        }
        self.pages.set_visible_child_name("list");
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let store = state.store.borrow();
        let mut people: Vec<(Snowflake, RelationshipKind, String)> = store
            .relationships()
            .filter(|r| match tab {
                Tab::Online => {
                    r.kind == RelationshipKind::Friend
                        && store.presence(r.user_id).status != Status::Offline
                }
                Tab::All => r.kind == RelationshipKind::Friend,
                Tab::Pending => {
                    matches!(
                        r.kind,
                        RelationshipKind::Incoming | RelationshipKind::Outgoing
                    )
                }
                Tab::Blocked => r.kind == RelationshipKind::Blocked,
                Tab::Add => false,
            })
            .map(|r| {
                let name = r
                    .nickname
                    .clone()
                    .or_else(|| store.user(r.user_id).map(|u| u.display_name().to_owned()))
                    .unwrap_or_else(|| "Unknown user".to_owned());
                (r.user_id, r.kind, name)
            })
            .collect();
        people.sort_by_cached_key(|(_, _, name)| name.to_lowercase());

        let title = match tab {
            Tab::Online => "ONLINE",
            Tab::All => "ALL FRIENDS",
            Tab::Pending => "PENDING",
            _ => "BLOCKED",
        };
        self.heading
            .set_text(&format!("{title} — {}", people.len()));
        if people.is_empty() {
            let empty = gtk::Label::new(Some("Nobody here yet."));
            empty.add_css_class("dim-label");
            empty.set_margin_top(40);
            self.list.append(&empty);
        }
        for (user_id, kind, name) in people {
            let Some(user) = store.user(user_id).cloned() else {
                continue;
            };
            let presence = store.presence(user_id);
            let subtitle = match kind {
                RelationshipKind::Incoming => "Incoming Friend Request".to_owned(),
                RelationshipKind::Outgoing => "Outgoing Friend Request".to_owned(),
                RelationshipKind::Blocked => "Blocked".to_owned(),
                RelationshipKind::Friend => presence
                    .activity
                    .clone()
                    .unwrap_or_else(|| presence.status.label().to_owned()),
            };

            let title = gtk::Label::builder()
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build();
            title.set_markup(&format!(
                "<b>{}</b>  <span alpha=\"60%\">{}</span>",
                gtk::glib::markup_escape_text(&name),
                gtk::glib::markup_escape_text(&user.username)
            ));
            let sub = gtk::Label::builder()
                .label(&subtitle)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build();
            sub.add_css_class("caption");
            sub.add_css_class("dim-label");
            let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
            text.set_hexpand(true);
            text.set_valign(gtk::Align::Center);
            text.append(&title);
            text.append(&sub);

            let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            content.set_margin_top(8);
            content.set_margin_bottom(8);
            content.set_margin_start(12);
            content.set_margin_end(12);
            let picture = avatar::with_status(state, &user, 36);
            open_profile_on_click(&picture, user_id);
            content.append(&picture);
            content.append(&text);

            let button = |icon: &str, tip: &str, run: fn(&Rc<AppState>, Snowflake)| {
                let button = gtk::Button::from_icon_name(icon);
                button.add_css_class("circular");
                button.set_tooltip_text(Some(tip));
                button.set_valign(gtk::Align::Center);
                let state = state.clone();
                button.connect_clicked(move |_| run(&state, user_id));
                button
            };
            match kind {
                RelationshipKind::Friend => {
                    content.append(&button(
                        "mail-unread-symbolic",
                        "Message",
                        AppState::open_dm,
                    ));
                    content.append(&button(
                        "user-trash-symbolic",
                        "Remove Friend",
                        AppState::remove_relationship,
                    ));
                }
                RelationshipKind::Incoming => {
                    content.append(&button(
                        "object-select-symbolic",
                        "Accept",
                        AppState::accept_friend,
                    ));
                    content.append(&button(
                        "window-close-symbolic",
                        "Ignore",
                        AppState::remove_relationship,
                    ));
                }
                RelationshipKind::Outgoing => content.append(&button(
                    "window-close-symbolic",
                    "Cancel",
                    AppState::remove_relationship,
                )),
                RelationshipKind::Blocked => content.append(&button(
                    "edit-undo-symbolic",
                    "Unblock",
                    AppState::remove_relationship,
                )),
            }
            self.list.append(
                &gtk::ListBoxRow::builder()
                    .child(&content)
                    .activatable(false)
                    .build(),
            );
        }
    }
}
