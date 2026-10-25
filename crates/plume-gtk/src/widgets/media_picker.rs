//! The GIF and sticker picker (the button next to emoji in the composer).

use std::rc::Rc;

use adw::prelude::*;
use plume_core::models::Sticker;
use plume_media::urls;

use crate::app::AppState;
use crate::state::bridge;
use crate::widgets::avatar;

const TILE: i32 = 120;

pub fn open(state: &Rc<AppState>, anchor: &impl IsA<gtk::Widget>) {
    let gifs = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .min_children_per_line(3)
        .max_children_per_line(3)
        .homogeneous(true)
        .build();
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search Tenor")
        .build();
    let gif_page = gtk::Box::new(gtk::Orientation::Vertical, 6);
    gif_page.append(&search);
    gif_page.append(&scrolled(&gifs));

    let stickers = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .min_children_per_line(3)
        .max_children_per_line(4)
        .homogeneous(true)
        .build();

    let pages = adw::ViewStack::new();
    pages.add_titled_with_icon(&gif_page, Some("gifs"), "GIFs", "image-x-generic-symbolic");
    pages.add_titled_with_icon(
        &scrolled(&stickers),
        Some("stickers"),
        "Stickers",
        "face-smile-big-symbolic",
    );
    let switcher = adw::ViewSwitcher::builder()
        .stack(&pages)
        .policy(adw::ViewSwitcherPolicy::Wide)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
    content.set_size_request(420, 440);
    content.append(&switcher);
    content.append(&pages);

    let popover = gtk::Popover::builder().child(&content).build();
    popover.set_parent(anchor.as_ref());
    popover.connect_closed(|popover| {
        let popover = popover.clone();
        gtk::glib::idle_add_local_once(move || popover.unparent());
    });

    fill_stickers(state, &stickers, &popover);
    load_gifs(state, &gifs, &popover, "");
    {
        let state = state.clone();
        let gifs = gifs.clone();
        let popover = popover.clone();
        search.connect_activate(move |entry| load_gifs(&state, &gifs, &popover, &entry.text()));
    }
    popover.popup();
}

fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(child)
        .build()
}

fn clear(flow: &gtk::FlowBox) {
    while let Some(child) = flow.first_child() {
        flow.remove(&child);
    }
}

fn note(flow: &gtk::FlowBox, text: &str) {
    clear(flow);
    let label = gtk::Label::new(Some(text));
    label.add_css_class("dim-label");
    label.set_wrap(true);
    label.set_margin_top(30);
    flow.append(&label);
}

/// A clickable square that shows `url` and runs `pick`.
fn tile(
    state: &Rc<AppState>,
    url: String,
    tooltip: &str,
    pick: impl Fn() + 'static,
) -> gtk::Button {
    let picture = gtk::Picture::new();
    picture.set_size_request(TILE, TILE);
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk::ContentFit::Cover);
    let weak = picture.downgrade();
    avatar::load(state, url, TILE as u32 * avatar::scale(), move |texture| {
        if let Some(picture) = weak.upgrade() {
            picture.set_paintable(Some(texture));
        }
    });
    let button = gtk::Button::builder()
        .child(&picture)
        .tooltip_text(tooltip)
        .build();
    button.add_css_class("flat");
    button.connect_clicked(move |_| pick());
    button
}

fn fill_stickers(state: &Rc<AppState>, flow: &gtk::FlowBox, popover: &gtk::Popover) {
    let stickers: Vec<Sticker> = {
        let store = state.store.borrow();
        let guild = store
            .active_channel()
            .and_then(|c| store.channel(c))
            .and_then(|c| c.guild_id)
            .and_then(|g| store.guild(g));
        guild.map(|g| g.stickers.clone()).unwrap_or_default()
    };
    let drawable: Vec<Sticker> = stickers.into_iter().filter(|s| s.format != 3).collect();
    if drawable.is_empty() {
        note(flow, "This server has no stickers.");
        return;
    }
    for sticker in drawable {
        let Some(url) = urls::sticker(sticker.id, sticker.format, TILE as u32 * avatar::scale())
        else {
            continue;
        };
        let state_for_pick = state.clone();
        let popover = popover.clone();
        let id = sticker.id;
        flow.append(&tile(state, url, &sticker.name, move || {
            popover.popdown();
            state_for_pick.send_sticker(id);
        }));
    }
}

fn load_gifs(state: &Rc<AppState>, flow: &gtk::FlowBox, popover: &gtk::Popover, query: &str) {
    let Some(rest) = state.rest() else {
        note(flow, "GIFs need a Discord login (not in demo mode).");
        return;
    };
    note(flow, "Loading…");
    let query = query.to_owned();
    let state = state.clone();
    let flow = flow.clone();
    let popover = popover.clone();
    bridge::spawn(async move { rest.gifs(&query).await }, move |result| {
        let gifs = match result {
            Ok(gifs) if gifs.is_empty() => return note(&flow, "No GIFs found."),
            Ok(gifs) => gifs,
            Err(error) => return note(&flow, &format!("Couldn't load GIFs: {error}")),
        };
        clear(&flow);
        for gif in gifs {
            let preview = if gif.preview.is_empty() {
                gif.url.clone()
            } else {
                gif.preview.clone()
            };
            let state_for_pick = state.clone();
            let popover = popover.clone();
            let url = gif.url.clone();
            flow.append(&tile(&state, preview, "Send", move || {
                popover.popdown();
                state_for_pick.submit(url.clone(), Vec::new());
            }));
        }
    });
}
