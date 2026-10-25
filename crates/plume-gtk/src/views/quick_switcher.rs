//! Ctrl+K: type part of a channel, server or person and press Enter.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use plume_core::Snowflake;

use crate::app::AppState;

const MAX_RESULTS: usize = 50;

pub fn open(state: &Rc<AppState>) {
    let Some(ui) = state.ui.get() else { return };
    let targets: Vec<(Snowflake, String, String)> = state
        .jump_targets()
        .into_iter()
        .map(|(id, label)| {
            let lower = label.to_lowercase();
            (id, label, lower)
        })
        .collect();

    let search = gtk::SearchEntry::builder()
        .placeholder_text("Where do you want to go?")
        .build();
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::Browse);
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&list)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.append(&search);
    content.append(&scrolled);

    let header = adw::HeaderBar::new();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&content));
    let dialog = adw::Dialog::builder()
        .title("Jump to")
        .content_width(460)
        .content_height(520)
        .child(&view)
        .build();

    let shown: Rc<RefCell<Vec<Snowflake>>> = Rc::new(RefCell::new(Vec::new()));
    let fill = {
        let (list, shown) = (list.clone(), shown.clone());
        move |query: &str| {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            let query = query.to_lowercase();
            let words: Vec<&str> = query.split_whitespace().collect();
            let mut ids = Vec::new();
            let matching = targets
                .iter()
                .filter(|(_, _, lower)| words.iter().all(|w| lower.contains(w)));
            for (id, label, _) in matching.take(MAX_RESULTS) {
                let row = adw::ActionRow::builder()
                    .title(gtk::glib::markup_escape_text(label))
                    .activatable(true)
                    .build();
                list.append(&row);
                ids.push(*id);
            }
            if let Some(first) = list.row_at_index(0) {
                list.select_row(Some(&first));
            }
            *shown.borrow_mut() = ids;
        }
    };
    fill("");
    let fill = Rc::new(fill);

    let go = {
        let (state, dialog, shown) = (state.clone(), dialog.clone(), shown.clone());
        Rc::new(move |index: i32| {
            let target = shown.borrow().get(index.max(0) as usize).copied();
            dialog.close();
            if let Some(channel_id) = target {
                state.go_to(channel_id);
            }
        })
    };

    let refill = fill.clone();
    search.connect_search_changed(move |entry| refill(&entry.text()));
    let (go_enter, list_for_enter) = (go.clone(), list.clone());
    search.connect_activate(move |_| {
        let index = list_for_enter.selected_row().map_or(0, |r| r.index());
        go_enter(index);
    });
    // ↓ from the search box moves into the results.
    let list_for_keys = list.clone();
    search.connect_next_match(move |_| {
        if let Some(row) = list_for_keys
            .selected_row()
            .and_then(|r| list_for_keys.row_at_index(r.index() + 1))
        {
            list_for_keys.select_row(Some(&row));
        }
    });
    let list_for_prev = list.clone();
    search.connect_previous_match(move |_| {
        if let Some(row) = list_for_prev
            .selected_row()
            .and_then(|r| list_for_prev.row_at_index(r.index() - 1))
        {
            list_for_prev.select_row(Some(&row));
        }
    });
    list.connect_row_activated(move |_, row| go(row.index()));

    dialog.present(Some(&ui.window));
    search.grab_focus();
}
