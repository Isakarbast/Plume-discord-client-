//! The narrow column of server icons on the far left. The first entry is
//! "Direct Messages".

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use plume_core::navigation;
use plume_core::{Snowflake, Unread};

use crate::app::AppState;
use crate::widgets::avatar;

pub struct GuildRail {
    pub root: gtk::ScrolledWindow,
    list: gtk::ListBox,
    /// What each row stands for (`None` = DMs), in row order.
    rows: RefCell<Vec<Option<Snowflake>>>,
    /// Set while rebuilding, so our own row changes aren't taken as clicks.
    rebuilding: Cell<bool>,
}

impl GuildRail {
    pub fn new(state: Weak<AppState>) -> Rc<Self> {
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Single);
        list.add_css_class("guild-rail");
        let root = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&list)
            .width_request(72)
            .build();
        let rail = Rc::new(GuildRail {
            root,
            list,
            rows: RefCell::new(Vec::new()),
            rebuilding: Cell::new(false),
        });

        let weak_rail = Rc::downgrade(&rail);
        // Selecting (click or keyboard) switches servers.
        rail.list.connect_row_selected(move |_, row| {
            let (Some(state), Some(rail), Some(row)) = (state.upgrade(), weak_rail.upgrade(), row)
            else {
                return;
            };
            if rail.rebuilding.get() {
                return;
            }
            let target = rail.rows.borrow().get(row.index() as usize).copied();
            if let Some(guild) = target
                && guild != state.selected_guild.get()
            {
                let state = state.clone();
                gtk::glib::idle_add_local_once(move || state.select_guild(guild));
            }
        });
        rail
    }

    /// Rebuilds the icons (with unread dots and mention badges).
    pub fn rebuild(&self, state: &Rc<AppState>) {
        self.rebuilding.set(true);
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let store = state.store.borrow();
        let mut rows = vec![None];

        let dm_unread =
            navigation::sorted_dms(&store)
                .into_iter()
                .fold(Unread::default(), |mut total, id| {
                    let unread = store.unread(id);
                    total.unread |= unread.unread;
                    total.mentions += unread.mentions;
                    total
                });
        let home = adw::Avatar::new(48, Some("Direct Messages"), true);
        home.set_icon_name(Some("chat-bubbles-text-symbolic"));
        home.set_show_initials(false);
        self.list
            .append(&row(home.upcast(), "Direct Messages", dm_unread));

        for guild in navigation::sorted_guilds(&store) {
            rows.push(Some(guild.id));
            let icon = avatar::guild_avatar(state, guild, 48);
            let row = row(icon.upcast(), &guild.name, store.guild_unread(guild.id));
            attach_menu(&row, guild.id, store.is_guild_muted(guild.id));
            self.list.append(&row);
        }
        drop(store);

        let selected = state.selected_guild.get();
        if let Some(index) = rows.iter().position(|r| *r == selected) {
            self.list
                .select_row(self.list.row_at_index(index as i32).as_ref());
        }
        *self.rows.borrow_mut() = rows;
        self.rebuilding.set(false);
    }
}

/// An icon with an unread pill on the left and a mention count on the corner.
fn row(icon: gtk::Widget, tooltip: &str, unread: Unread) -> gtk::ListBoxRow {
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&icon));
    overlay.set_halign(gtk::Align::Center);
    if unread.mentions > 0 {
        let badge = gtk::Label::new(Some(&unread.mentions.min(99).to_string()));
        badge.add_css_class("badge");
        badge.set_halign(gtk::Align::End);
        badge.set_valign(gtk::Align::End);
        overlay.add_overlay(&badge);
    }

    let pill = gtk::Box::new(gtk::Orientation::Vertical, 0);
    pill.add_css_class("unread-pill");
    pill.set_visible(unread.unread);
    pill.set_valign(gtk::Align::Center);

    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.append(&pill);
    content.append(&overlay);
    content.set_margin_top(4);
    content.set_margin_bottom(4);

    let row = gtk::ListBoxRow::new();
    row.set_child(Some(&content));
    row.set_tooltip_text(Some(tooltip));
    row
}

/// Right-click a server: mark as read, mute, copy its ID.
fn attach_menu(row: &gtk::ListBoxRow, id: Snowflake, muted: bool) {
    let click = gtk::GestureClick::new();
    click.set_button(gtk::gdk::BUTTON_SECONDARY);
    click.connect_pressed(move |gesture, _, x, y| {
        let Some(widget) = gesture.widget() else {
            return;
        };
        let menu = gtk::gio::Menu::new();
        menu.append(
            Some("Mark As Read"),
            Some(&format!("win.mark-guild-read::{id}")),
        );
        menu.append(
            Some(if muted {
                "Unmute Server"
            } else {
                "Mute Server"
            }),
            Some(&format!("win.mute-guild::{id}")),
        );
        let notify = gtk::gio::Menu::new();
        notify.append(
            Some("All Messages"),
            Some(&format!("win.notify-level::{id}:0")),
        );
        notify.append(
            Some("Only @mentions"),
            Some(&format!("win.notify-level::{id}:1")),
        );
        notify.append(Some("Nothing"), Some(&format!("win.notify-level::{id}:2")));
        menu.append_submenu(Some("Notification Settings"), &notify);
        menu.append(Some("Copy Server ID"), Some(&format!("win.copy-id::{id}")));
        crate::widgets::channel_tree::popup_menu(&widget, &menu, x, y);
    });
    row.add_controller(click);
}
