//! Link previews: a card with a coloured bar, like Discord's.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib::markup_escape_text;
use plume_core::models::Embed;

use crate::app::AppState;
use crate::widgets::message_row::image_widget;

/// Long descriptions are cut here.
const MAX_DESCRIPTION: usize = 400;

/// `None` for previews with nothing to show (the link is in the message).
pub fn build(state: &Rc<AppState>, embed: &Embed) -> Option<gtk::Widget> {
    let picture = embed.image.as_ref().or(embed.thumbnail.as_ref());
    let has_text = embed.title.is_some() || embed.description.is_some() || !embed.fields.is_empty();
    // A link to a picture or gif: Discord shows just the picture.
    if !has_text {
        let image = picture.filter(|_| state.media.is_some())?;
        return Some(image_widget(state, &image.url, image.width, image.height).upcast());
    }
    let card = gtk::Box::new(gtk::Orientation::Vertical, 4);
    card.add_css_class("embed");
    card.set_halign(gtk::Align::Start);

    let line = |markup: String, classes: &[&str]| {
        let label = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .selectable(true)
            .max_width_chars(70)
            .build();
        label.set_markup(&markup);
        for class in classes {
            label.add_css_class(class);
        }
        label
    };

    if let Some(provider) = &embed.provider {
        card.append(&line(
            markup_escape_text(provider).to_string(),
            &["caption", "dim-label"],
        ));
    }
    if let Some(author) = &embed.author {
        card.append(&line(
            format!("<b>{}</b>", markup_escape_text(author)),
            &["caption"],
        ));
    }
    if let Some(title) = &embed.title {
        let title = markup_escape_text(title);
        let markup = match &embed.url {
            Some(url) => format!("<b><a href=\"{}\">{title}</a></b>", markup_escape_text(url)),
            None => format!("<b>{title}</b>"),
        };
        card.append(&line(markup, &[]));
    }
    if let Some(description) = &embed.description {
        let short: String = description.chars().take(MAX_DESCRIPTION).collect();
        let ellipsis = if short.len() < description.len() {
            "…"
        } else {
            ""
        };
        card.append(&line(
            format!("{}{ellipsis}", markup_escape_text(&short)),
            &[],
        ));
    }
    let big = embed.image.as_ref().or(match embed.kind.as_str() {
        // Videos and articles: their thumbnail is the picture.
        "video" | "gifv" | "article" => embed.thumbnail.as_ref(),
        _ => None,
    });
    for (name, value) in &embed.fields {
        card.append(&line(
            format!(
                "<b>{}</b>\n{}",
                markup_escape_text(name),
                markup_escape_text(value)
            ),
            &[],
        ));
    }
    if let Some(image) = big.filter(|_| state.media.is_some()) {
        card.append(&image_widget(state, &image.url, image.width, image.height));
    }
    // Coloured bar on the left, like Discord.
    let Some(color) = embed.color else {
        return Some(card.upcast());
    };
    let bar = gtk::DrawingArea::new();
    bar.set_content_width(4);
    bar.set_draw_func(move |_, cr, width, height| {
        let channel = |shift: u32| f64::from((color >> shift) & 0xff) / 255.0;
        cr.set_source_rgb(channel(16), channel(8), channel(0));
        cr.rectangle(0.0, 0.0, f64::from(width), f64::from(height));
        let _ = cr.fill();
    });
    card.add_css_class("embed-colored");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.set_halign(gtk::Align::Start);
    row.append(&bar);
    row.append(&card);
    Some(row.upcast())
}
