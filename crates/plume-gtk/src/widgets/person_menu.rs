//! Right-click someone (member list, voice, a message's avatar): profile,
//! message, mention, copy ID, and moderation if you're allowed.

use std::rc::Rc;

use adw::prelude::*;
use gtk::gio;
use plume_core::{Permissions, Snowflake};

use crate::app::AppState;
use crate::widgets::channel_tree::popup_menu;

pub fn attach(
    state: &Rc<AppState>,
    widget: &impl IsA<gtk::Widget>,
    guild_id: Option<Snowflake>,
    user_id: Snowflake,
) {
    let click = gtk::GestureClick::new();
    click.set_button(gtk::gdk::BUTTON_SECONDARY);
    let state = Rc::downgrade(state);
    click.connect_pressed(move |gesture, _, x, y| {
        let (Some(widget), Some(state)) = (gesture.widget(), state.upgrade()) else {
            return;
        };
        popup_menu(&widget, &menu(&state, guild_id, user_id), x, y);
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    widget.add_controller(click);
}

pub fn menu(state: &AppState, guild_id: Option<Snowflake>, user_id: Snowflake) -> gio::Menu {
    let store = state.store.borrow();
    let is_me = store.me().is_some_and(|me| me.id == user_id);
    let menu = gio::Menu::new();
    let main = gio::Menu::new();
    main.append(Some("Profile"), Some(&format!("win.profile::{user_id}")));
    if !is_me {
        main.append(Some("Message"), Some(&format!("win.dm::{user_id}")));
    }
    main.append(Some("Mention"), Some(&format!("win.mention::{user_id}")));
    menu.append_section(None, &main);

    if let Some(guild_id) = guild_id.filter(|_| !is_me) {
        let perms = store.guild_permissions(guild_id);
        let target = format!("{guild_id}:{user_id}");
        let moderation = gio::Menu::new();
        if perms.contains(Permissions::MODERATE_MEMBERS) {
            moderation.append(
                Some("Timeout for 1 Hour"),
                Some(&format!("win.moderate::timeout:{target}")),
            );
            moderation.append(
                Some("Remove Timeout"),
                Some(&format!("win.moderate::untimeout:{target}")),
            );
        }
        if perms.contains(Permissions::KICK_MEMBERS) {
            moderation.append(Some("Kick"), Some(&format!("win.moderate::kick:{target}")));
        }
        if perms.contains(Permissions::BAN_MEMBERS) {
            moderation.append(Some("Ban"), Some(&format!("win.moderate::ban:{target}")));
        }
        if moderation.n_items() > 0 {
            menu.append_section(None, &moderation);
        }
    }
    let copy = gio::Menu::new();
    copy.append(
        Some("Copy User ID"),
        Some(&format!("win.copy-id::{user_id}")),
    );
    menu.append_section(None, &copy);
    menu
}
