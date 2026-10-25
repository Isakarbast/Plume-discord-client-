//! Someone's profile: who they are, their bio, the servers you share, and
//! buttons to message them or copy their ID.

use std::rc::Rc;

use adw::prelude::*;
use chrono::{Local, TimeZone};
use plume_core::Snowflake;

use crate::app::AppState;
use crate::widgets::avatar;

pub fn show(state: &Rc<AppState>, user_id: Snowflake) {
    let Some(ui) = state.ui.get() else { return };
    let (user, is_me) = {
        let store = state.store.borrow();
        let is_me = store.me().is_some_and(|me| me.id == user_id);
        (store.user(user_id).cloned(), is_me)
    };

    let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
    content.add_css_class("profile");
    content.set_margin_top(12);
    content.set_margin_bottom(24);
    content.set_margin_start(24);
    content.set_margin_end(24);

    let picture: gtk::Widget = match &user {
        Some(user) => avatar::user_avatar(state, user, 80).upcast(),
        None => adw::Avatar::new(80, Some("?"), true).upcast(),
    };
    picture.set_halign(gtk::Align::Center);
    content.append(&picture);

    let name = gtk::Label::new(Some(
        user.as_ref().map_or("Unknown user", |u| u.display_name()),
    ));
    name.add_css_class("title-2");
    name.set_wrap(true);
    content.append(&name);

    let handle = gtk::Label::new(user.as_ref().map(|u| format!("@{}", u.username)).as_deref());
    handle.add_css_class("dim-label");
    handle.set_selectable(true);
    content.append(&handle);

    let pronouns = gtk::Label::new(None);
    pronouns.add_css_class("caption");
    pronouns.add_css_class("dim-label");
    pronouns.set_visible(false);
    content.append(&pronouns);

    let details = gtk::ListBox::new();
    details.add_css_class("boxed-list");
    details.set_selection_mode(gtk::SelectionMode::None);
    details.set_margin_top(12);

    let bio = adw::ActionRow::builder()
        .title("About me")
        .subtitle("Loading…")
        .build();
    bio.set_subtitle_selectable(true);
    // Bios and server names are plain text, not markup ("&", "<t:…>").
    bio.set_use_markup(false);
    let since = Local
        .timestamp_millis_opt(user_id.timestamp_ms() as i64)
        .single()
        .map_or_else(String::new, |t| t.format("%-d %B %Y").to_string());
    let member_since = adw::ActionRow::builder()
        .title("Discord member since")
        .subtitle(&since)
        .build();
    let mutual = adw::ActionRow::builder().title("Servers in common").build();
    mutual.set_use_markup(false);
    mutual.set_visible(false);
    details.append(&bio);
    details.append(&member_since);
    details.append(&mutual);
    content.append(&details);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    buttons.set_halign(gtk::Align::Center);
    buttons.set_margin_top(12);
    let message = gtk::Button::with_label("Message");
    message.add_css_class("pill");
    message.add_css_class("suggested-action");
    message.set_visible(!is_me);
    let copy = gtk::Button::with_label("Copy User ID");
    copy.add_css_class("pill");
    buttons.append(&message);
    buttons.append(&copy);
    content.append(&buttons);

    let header = adw::HeaderBar::new();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&content));
    let dialog = adw::Dialog::builder()
        .title("Profile")
        .content_width(380)
        .child(&view)
        .build();

    {
        let state = state.clone();
        let dialog = dialog.clone();
        message.connect_clicked(move |_| {
            dialog.close();
            state.open_dm(user_id);
        });
    }
    {
        let state = state.clone();
        copy.connect_clicked(move |button| {
            button.clipboard().set_text(&user_id.to_string());
            state.toast("User ID copied");
        });
    }

    match state.rest() {
        Some(rest) => {
            let state = state.clone();
            crate::state::bridge::spawn(
                async move { rest.user_profile(user_id).await },
                move |result| match result {
                    Ok(profile) => {
                        bio.set_subtitle(profile.bio.as_deref().unwrap_or("No bio"));
                        if let Some(text) = &profile.pronouns {
                            pronouns.set_text(text);
                            pronouns.set_visible(true);
                        }
                        let store = state.store.borrow();
                        let names: Vec<&str> = profile
                            .mutual_guilds
                            .iter()
                            .filter_map(|&id| store.guild(id))
                            .map(|g| g.name.as_str())
                            .collect();
                        if !names.is_empty() {
                            mutual.set_subtitle(&names.join(", "));
                            mutual.set_visible(true);
                        }
                    }
                    Err(error) => {
                        tracing::debug!("profile unavailable: {error}");
                        bio.set_subtitle("Couldn't load the profile");
                    }
                },
            );
        }
        None => bio.set_subtitle("Not available in demo mode"),
    }

    dialog.present(Some(&ui.window));
}
