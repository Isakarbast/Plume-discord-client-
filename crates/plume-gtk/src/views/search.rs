//! Pinned messages and message search, shown in a popover under the header
//! buttons. Clicking a result opens its channel.

use std::rc::Rc;

use adw::prelude::*;
use plume_core::models::{Message, User};
use plume_core::navigation;

use crate::app::AppState;
use crate::markup;
use crate::state::bridge;
use crate::widgets::avatar;

/// The pinned messages of the open channel.
pub fn show_pins(state: &Rc<AppState>, anchor: &impl IsA<gtk::Widget>) {
    let Some(channel_id) = state.store.borrow().active_channel() else {
        return;
    };
    let (popover, list) = results_popover(anchor, "Pinned Messages");
    let Some(rest) = state.rest() else {
        placeholder(&list, "Not available in demo mode");
        return;
    };
    placeholder(&list, "Loading…");
    let state = state.clone();
    bridge::spawn(async move { rest.pins(channel_id).await }, move |result| {
        fill(
            &state,
            &popover,
            &list,
            result,
            "This channel doesn't have any pinned messages yet.",
        );
    });
}

/// Active threads in the open channel (or its parent, inside a thread).
pub fn show_threads(state: &Rc<AppState>, anchor: &impl IsA<gtk::Widget>) {
    let (popover, list) = results_popover(anchor, "Threads");
    let store = state.store.borrow();
    let Some(channel) = store.active_channel().and_then(|id| store.channel(id)) else {
        return;
    };
    let parent = if channel.kind == plume_core::models::ChannelKind::Thread {
        channel.parent_id.unwrap_or(channel.id)
    } else {
        channel.id
    };
    let threads = store.threads(parent);
    if threads.is_empty() {
        placeholder(&list, "There are no active threads here.");
        return;
    }
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    for thread in threads {
        let info = thread.thread.unwrap_or(plume_core::models::ThreadInfo {
            archived: false,
            joined: false,
            message_count: 0,
        });
        let title = gtk::Label::builder()
            .label(format!("🧵 {}", thread.name.as_deref().unwrap_or("thread")))
            .xalign(0.0)
            .build();
        title.add_css_class("heading");
        let detail = gtk::Label::builder()
            .label(format!(
                "{} messages{}",
                info.message_count,
                if info.joined { " · joined" } else { "" }
            ))
            .xalign(0.0)
            .build();
        detail.add_css_class("caption");
        detail.add_css_class("dim-label");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
        content.add_css_class("search-result");
        content.append(&title);
        content.append(&detail);
        let row = gtk::ListBoxRow::builder().child(&content).build();
        let id = thread.id;
        let click = gtk::GestureClick::new();
        let state = state.clone();
        let popover = popover.clone();
        click.connect_released(move |_, _, _, _| {
            popover.popdown();
            state.go_to(id);
        });
        row.add_controller(click);
        list.append(&row);
    }
}

/// Search the open server (or DM) for `text`.
pub fn search(state: &Rc<AppState>, anchor: &impl IsA<gtk::Widget>, text: &str) {
    let (guild_id, channel_id) = {
        let store = state.store.borrow();
        let Some(channel_id) = store.active_channel() else {
            return;
        };
        (
            store.channel(channel_id).and_then(|c| c.guild_id),
            channel_id,
        )
    };
    let (popover, list) = results_popover(anchor, &format!("Results for “{text}”"));
    let Some(rest) = state.rest() else {
        placeholder(&list, "Not available in demo mode");
        return;
    };
    placeholder(&list, "Searching…");
    let state = state.clone();
    let text = text.to_owned();
    bridge::spawn(
        async move { rest.search(guild_id, channel_id, &text).await },
        move |result| fill(&state, &popover, &list, result, "No results."),
    );
}

fn results_popover(anchor: &impl IsA<gtk::Widget>, title: &str) -> (gtk::Popover, gtk::ListBox) {
    let heading = gtk::Label::new(Some(title));
    heading.add_css_class("heading");
    heading.set_margin_top(6);
    heading.set_margin_bottom(6);
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_width(420)
        .min_content_height(120)
        .max_content_height(520)
        .propagate_natural_height(true)
        .child(&list)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&heading);
    content.append(&scrolled);
    let popover = gtk::Popover::builder().child(&content).build();
    popover.set_parent(anchor.as_ref());
    popover.connect_closed(|popover| {
        let popover = popover.clone();
        gtk::glib::idle_add_local_once(move || popover.unparent());
    });
    popover.popup();
    (popover, list)
}

fn placeholder(list: &gtk::ListBox, text: &str) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let label = gtk::Label::new(Some(text));
    label.add_css_class("dim-label");
    label.set_margin_top(24);
    label.set_margin_bottom(24);
    label.set_wrap(true);
    list.append(&label);
}

fn fill(
    state: &Rc<AppState>,
    popover: &gtk::Popover,
    list: &gtk::ListBox,
    result: Result<Vec<(Message, User)>, plume_rest::RestError>,
    empty: &str,
) {
    let messages = match result {
        Ok(messages) if messages.is_empty() => return placeholder(list, empty),
        Ok(messages) => messages,
        Err(error) => return placeholder(list, &format!("Couldn't load: {error}")),
    };
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    for (message, author) in messages {
        let store = state.store.borrow();
        let header = gtk::Label::builder().xalign(0.0).build();
        let place = navigation::channel_label(&store, message.channel_id);
        header.set_markup(&format!(
            "<b>{}</b>  <span size=\"small\" alpha=\"60%\">{}</span>",
            gtk::glib::markup_escape_text(author.display_name()),
            gtk::glib::markup_escape_text(&place)
        ));
        let body = gtk::Label::builder()
            .label(markup::snippet(&message.content, 300))
            .xalign(0.0)
            .wrap(true)
            .build();
        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_hexpand(true);
        text.append(&header);
        text.append(&body);
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        content.add_css_class("search-result");
        content.append(&avatar::user_avatar(state, &author, 32));
        content.append(&text);
        drop(store);

        let row = gtk::ListBoxRow::builder().child(&content).build();
        let channel_id = message.channel_id;
        let click = gtk::GestureClick::new();
        let state_for_click = state.clone();
        let popover_for_click = popover.clone();
        click.connect_released(move |_, _, _, _| {
            popover_for_click.popdown();
            state_for_click.go_to(channel_id);
        });
        row.add_controller(click);
        list.append(&row);
    }
}
