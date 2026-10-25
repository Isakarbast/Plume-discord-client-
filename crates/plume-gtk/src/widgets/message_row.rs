//! One message: avatar, name, time, text, files, previews and reactions.
//! Right-click opens a menu (reply, react, copy, edit, delete).

use std::rc::Rc;

use adw::prelude::*;
use chrono::{DateTime, Local, TimeZone};
use gtk::{gdk, gio, glib};
use plume_core::models::{Attachment, Message};
use plume_core::{Snowflake, Store};
use plume_media::urls;

use crate::app::AppState;
use crate::markup::{self, Block};
use crate::widgets::{avatar, embed, reaction_bar};

/// Messages from one person this close together share a header.
const GROUP_WINDOW_MS: u64 = 7 * 60 * 1000;
const AVATAR_SIZE: i32 = 36;
const MAX_IMAGE_WIDTH: u32 = 400;
const MAX_IMAGE_HEIGHT: u32 = 300;
/// Emoji-only messages show their emoji this big, like Discord.
const JUMBO_EMOJI: u32 = 48;

pub fn build(state: &Rc<AppState>, id: Snowflake, previous: Option<Snowflake>) -> gtk::Widget {
    let store = state.store.borrow();
    let Some(channel_id) = store.active_channel() else {
        return gtk::Box::new(gtk::Orientation::Vertical, 0).upcast();
    };
    let Some(message) = store.message(channel_id, id) else {
        return gtk::Box::new(gtk::Orientation::Vertical, 0).upcast();
    };
    let previous = previous.and_then(|p| store.message(channel_id, p));

    let time = local_time(message.timestamp_ms());
    let new_day =
        previous.is_none_or(|p| local_time(p.timestamp_ms()).date_naive() != time.date_naive());
    let grouped = !new_day
        && message.reply_to.is_none()
        && previous.is_some_and(|p| {
            p.author_id == message.author_id
                && message.timestamp_ms() - p.timestamp_ms() < GROUP_WINDOW_MS
        });

    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    if new_day {
        outer.append(&day_separator(&time));
    }
    // The red "NEW" line above the first message we hadn't read.
    if let Some((marker_channel, read)) = state.unread_marker.get()
        && marker_channel == channel_id
        && message.id > read
        && previous.is_none_or(|p| p.id <= read)
    {
        outer.append(&new_messages_line());
    }

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("message-row");
    if !grouped {
        row.add_css_class("first-in-group");
    }
    if store.me().is_some_and(|me| message.mentions_user(me.id)) {
        row.add_css_class("mentioned");
    }

    // Left column: avatar, or the time on hover-less grouped rows.
    if grouped {
        let spacer = gtk::Label::new(None);
        spacer.set_size_request(AVATAR_SIZE, -1);
        row.append(&spacer);
    } else {
        let picture: gtk::Widget = match store.user(message.author_id) {
            Some(user) => avatar::user_avatar(state, user, AVATAR_SIZE).upcast(),
            None => adw::Avatar::new(AVATAR_SIZE, Some("?"), true).upcast(),
        };
        picture.set_valign(gtk::Align::Start);
        open_profile_on_click(&picture, message.author_id);
        let guild_id = store.channel(channel_id).and_then(|c| c.guild_id);
        crate::widgets::person_menu::attach(state, &picture, guild_id, message.author_id);
        row.append(&picture);
    }

    let body = gtk::Box::new(gtk::Orientation::Vertical, 2);
    body.set_hexpand(true);
    if let Some(reply_to) = message.reply_to {
        body.append(&reply_preview(&store, channel_id, reply_to));
    }
    if !grouped {
        body.append(&header(&store, message, &time));
    }
    if !append_jumbo_emoji(state, &body, message) {
        append_content(&body, &store, message);
    }
    for attachment in &message.attachments {
        body.append(&attachment_widget(state, attachment));
    }
    for sticker in &message.stickers {
        body.append(&sticker_widget(state, sticker));
    }
    for preview in &message.embeds {
        if let Some(card) = embed::build(state, preview) {
            body.append(&card);
        }
    }
    if let Some(reactions) = reaction_bar::build(state, message) {
        body.append(&reactions);
    }
    row.append(&body);
    // Discord's hover toolbar: react, reply, edit, more.
    let hovered = gtk::Overlay::new();
    hovered.set_child(Some(&row));
    let mine = store.me().is_some_and(|me| me.id == message.author_id);
    let toolbar = hover_toolbar(id, mine);
    hovered.add_overlay(&toolbar);
    let motion = gtk::EventControllerMotion::new();
    let shown = toolbar.clone();
    motion.connect_enter(move |_, _, _| shown.set_visible(true));
    let hidden = toolbar.clone();
    motion.connect_leave(move |_| hidden.set_visible(false));
    hovered.add_controller(motion);
    outer.append(&hovered);

    let channel = store.channel(channel_id);
    let manage = channel.is_some_and(|c| c.guild_id.is_none())
        || store
            .permissions(channel_id)
            .contains(plume_core::Permissions::MANAGE_MESSAGES);
    attach_menu(&row, id, message.author_id, mine, manage, message.pinned);
    outer.upcast()
}

fn header(store: &Store, message: &Message, time: &DateTime<Local>) -> gtk::Widget {
    // Nickname and role colour in servers, like Discord.
    let guild_id = store.channel(message.channel_id).and_then(|c| c.guild_id);
    let name = store.display_name_in(guild_id, message.author_id);
    let name_label = gtk::Label::new(None);
    match guild_id.and_then(|g| store.role_color(g, message.author_id)) {
        Some(color) => name_label.set_markup(&format!(
            "<span foreground=\"#{color:06x}\">{}</span>",
            glib::markup_escape_text(&name)
        )),
        None => name_label.set_text(&name),
    }
    name_label.add_css_class("heading");
    name_label.add_css_class("profile-link");
    name_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name_label.set_max_width_chars(40);
    open_profile_on_click(&name_label, message.author_id);

    let time_label = gtk::Label::new(Some(&friendly_time(time)));
    time_label.add_css_class("caption");
    time_label.add_css_class("dim-label");
    time_label.set_tooltip_text(Some(&time.format("%A, %-d %B %Y %H:%M").to_string()));

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.append(&name_label);
    header.append(&time_label);
    header.upcast()
}

fn reply_preview(store: &Store, channel_id: Snowflake, reply_to: Snowflake) -> gtk::Widget {
    let text = match store.message(channel_id, reply_to) {
        Some(original) => {
            let name = store
                .user(original.author_id)
                .map_or("unknown", |u| u.display_name());
            format!("↪ {name}: {}", markup::snippet(&original.content, 80))
        }
        None => "↪ replying to an older message".to_owned(),
    };
    let label = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    label.add_css_class("caption");
    label.add_css_class("dim-label");
    label.upcast()
}

fn append_content(body: &gtk::Box, store: &Store, message: &Message) {
    let blocks = markup::message_blocks(store, &message.content);
    let last_text = blocks.iter().rposition(|b| matches!(b, Block::Text(_)));
    for (index, block) in blocks.iter().enumerate() {
        match block {
            Block::Text(text) => {
                let mut text = text.clone();
                if message.edited && Some(index) == last_text {
                    text.push_str(" <span size=\"small\" alpha=\"55%\">(edited)</span>");
                }
                body.append(&text_label(&text));
            }
            Block::Code { language, code } => {
                let label = gtk::Label::builder()
                    .label(code)
                    .xalign(0.0)
                    .selectable(true)
                    .wrap(true)
                    .wrap_mode(gtk::pango::WrapMode::Char)
                    .build();
                label.add_css_class("monospace");
                let frame = gtk::Box::new(gtk::Orientation::Vertical, 4);
                frame.add_css_class("code-block");
                if let Some(language) = language {
                    let tag = gtk::Label::builder().label(language).xalign(0.0).build();
                    tag.add_css_class("caption");
                    tag.add_css_class("dim-label");
                    frame.append(&tag);
                }
                frame.append(&label);
                body.append(&frame);
            }
        }
    }
    if blocks.is_empty() && message.attachments.is_empty() && message.embeds.is_empty() {
        let empty = text_label("<i>(no text)</i>");
        empty.add_css_class("dim-label");
        body.append(&empty);
    }
}

/// Messages that are only emoji (up to 27) show them large. Returns whether
/// it handled the message.
fn append_jumbo_emoji(state: &Rc<AppState>, body: &gtk::Box, message: &Message) -> bool {
    if message.content.is_empty() || message.edited && message.content.len() > 400 {
        return false;
    }
    let nodes = plume_markdown::parse(&message.content);
    let spans = plume_markdown::flatten(&nodes);
    let mut custom = Vec::new();
    let mut unicode = String::new();
    for span in &spans {
        match &span.piece {
            plume_markdown::Piece::CustomEmoji { name, id, animated } => {
                custom.push((name.to_string(), *id, *animated));
            }
            plume_markdown::Piece::Text(text) if text.trim().is_empty() => {}
            plume_markdown::Piece::Text(text)
                if custom.is_empty() && text.chars().all(is_emoji_char) =>
            {
                unicode.push_str(text);
            }
            _ => return false,
        }
    }
    if !custom.is_empty() && unicode.trim().is_empty() && custom.len() <= 27 {
        let row = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .max_children_per_line(30)
            .column_spacing(2)
            .hexpand(true)
            .build();
        for (name, id, animated) in custom {
            let picture = gtk::Picture::new();
            picture.set_size_request(JUMBO_EMOJI as i32, JUMBO_EMOJI as i32);
            picture.set_can_shrink(true);
            picture.set_tooltip_text(Some(&format!(":{name}:")));
            picture.add_css_class("jumbo-emoji");
            let weak = picture.downgrade();
            avatar::load(
                state,
                urls::emoji(id, animated, JUMBO_EMOJI * avatar::scale()),
                JUMBO_EMOJI * avatar::scale(),
                move |texture| {
                    if let Some(picture) = weak.upgrade() {
                        picture.set_paintable(Some(texture));
                    }
                },
            );
            row.append(&picture);
        }
        body.append(&row);
        return true;
    }
    let count = unicode.chars().filter(|&c| is_emoji(c)).count();
    if count > 0 && count <= 27 && custom.is_empty() {
        let label = text_label(&format!(
            "<span size=\"xx-large\">{}</span>",
            glib::markup_escape_text(unicode.trim())
        ));
        body.append(&label);
        return true;
    }
    false
}

/// Emoji, the joiners and modifiers inside them, or spaces.
fn is_emoji_char(c: char) -> bool {
    c.is_whitespace() || is_emoji(c) || matches!(c as u32, 0x200D | 0xFE0F | 0x20E3)
}

fn is_emoji(c: char) -> bool {
    matches!(c as u32, 0x2600..=0x27BF | 0x2B00..=0x2BFF | 0x1F000..=0x1FAFF)
}

fn text_label(markup: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .selectable(true)
        .build();
    label.set_markup(markup);
    // Selectable labels grab focus on click; this keeps keyboard focus in the composer.
    label.set_focus_on_click(false);
    label
}

/// Images inline (shrunk by Discord's media proxy), anything else as a link.
fn attachment_widget(state: &Rc<AppState>, attachment: &Attachment) -> gtk::Widget {
    if urls::is_image(&attachment.filename) && state.media.is_some() {
        let picture = image_widget(
            state,
            &attachment.url,
            attachment.width.unwrap_or(MAX_IMAGE_WIDTH),
            attachment.height.unwrap_or(MAX_IMAGE_HEIGHT),
        );
        picture.set_tooltip_text(Some(&attachment.filename));
        return picture.upcast();
    }
    let icon = if urls::is_media(&attachment.filename) {
        "▶"
    } else {
        "📎"
    };
    let label = format!(
        "{icon} {} ({})",
        attachment.filename,
        human_size(attachment.size)
    );
    // Videos and music open in your media player: lighter than an
    // embedded one, and it's probably better at it.
    let link = gtk::LinkButton::with_label(&attachment.url, &label);
    link.set_halign(gtk::Align::Start);
    if let Some(text) = link.child().and_downcast::<gtk::Label>() {
        text.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        text.set_max_width_chars(60);
    }
    link.upcast()
}

/// An image shrunk by Discord's media proxy to the size it's shown at.
/// Clicking opens the full one.
pub fn image_widget(state: &Rc<AppState>, url: &str, width: u32, height: u32) -> gtk::Picture {
    let (width, height) = fit(width.max(1), height.max(1));
    let picture = gtk::Picture::new();
    picture.set_size_request(width as i32, height as i32);
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_halign(gtk::Align::Start);
    picture.add_css_class("attachment-image");
    let weak = picture.downgrade();
    let scale = avatar::scale();
    let preview = urls::attachment_preview(url, width * scale, height * scale);
    avatar::load(state, preview, width.max(height) * scale, move |texture| {
        if let Some(picture) = weak.upgrade() {
            picture.set_paintable(Some(texture));
        }
    });
    let click = gtk::GestureClick::new();
    let url = url.to_owned();
    click.connect_released(move |gesture, _, _, _| {
        let window = gesture
            .widget()
            .and_then(|w| w.root())
            .and_downcast::<gtk::Window>();
        gtk::UriLauncher::new(&url).launch(window.as_ref(), gio::Cancellable::NONE, |_| {});
    });
    picture.add_controller(click);
    picture
}

fn sticker_widget(state: &Rc<AppState>, sticker: &plume_core::models::Sticker) -> gtk::Widget {
    const SIZE: u32 = 160;
    let Some(url) = urls::sticker(sticker.id, sticker.format, SIZE * avatar::scale()) else {
        let label = gtk::Label::new(Some(&format!("[sticker: {}]", sticker.name)));
        label.add_css_class("dim-label");
        label.set_xalign(0.0);
        return label.upcast();
    };
    let picture = gtk::Picture::new();
    picture.set_size_request(SIZE as i32, SIZE as i32);
    picture.set_can_shrink(true);
    picture.set_halign(gtk::Align::Start);
    picture.set_tooltip_text(Some(&sticker.name));
    let weak = picture.downgrade();
    avatar::load(state, url, SIZE * avatar::scale(), move |texture| {
        if let Some(picture) = weak.upgrade() {
            picture.set_paintable(Some(texture));
        }
    });
    picture.upcast()
}

/// Scales an image's size down to fit the preview box.
fn fit(width: u32, height: u32) -> (u32, u32) {
    let scale = (MAX_IMAGE_WIDTH as f64 / width.max(1) as f64)
        .min(MAX_IMAGE_HEIGHT as f64 / height.max(1) as f64)
        .min(1.0);
    (
        ((width as f64) * scale).round().max(1.0) as u32,
        ((height as f64) * scale).round().max(1.0) as u32,
    )
}

/// The right-click menu. Actions live on the window (see window.rs).
fn attach_menu(
    row: &gtk::Box,
    id: Snowflake,
    author: Snowflake,
    mine: bool,
    manage: bool,
    pinned: bool,
) {
    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_SECONDARY);
    click.connect_pressed(move |gesture, _, x, y| {
        let Some(widget) = gesture.widget() else {
            return;
        };
        let menu = gio::Menu::new();
        let target = id.to_string();
        menu.append(Some("Reply"), Some(&format!("win.reply::{target}")));
        menu.append(Some("Add Reaction"), Some(&format!("win.react::{target}")));
        menu.append(Some("Copy Text"), Some(&format!("win.copy::{target}")));
        menu.append(
            Some("View Profile"),
            Some(&format!("win.profile::{author}")),
        );
        let own = gio::Menu::new();
        if mine {
            own.append(Some("Edit"), Some(&format!("win.edit::{target}")));
        }
        if manage {
            if pinned {
                own.append(Some("Unpin Message"), Some(&format!("win.unpin::{target}")));
            } else {
                own.append(Some("Pin Message"), Some(&format!("win.pin::{target}")));
            }
        }
        if mine || manage {
            own.append(Some("Delete"), Some(&format!("win.delete::{target}")));
        }
        if own.n_items() > 0 {
            menu.append_section(None, &own);
        }
        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.set_parent(&widget);
        popover.set_has_arrow(false);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.connect_closed(|popover| {
            let popover = popover.clone();
            glib::idle_add_local_once(move || popover.unparent());
        });
        popover.popup();
    });
    row.add_controller(click);
}

fn hover_toolbar(id: Snowflake, mine: bool) -> gtk::Box {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    bar.add_css_class("hover-toolbar");
    bar.set_halign(gtk::Align::End);
    bar.set_valign(gtk::Align::Start);
    bar.set_margin_end(16);
    bar.set_visible(false);
    let target = id.to_string().to_variant();
    let mut buttons = vec![
        ("face-smile-symbolic", "Add Reaction", "win.react"),
        ("mail-reply-sender-symbolic", "Reply", "win.reply"),
    ];
    if mine {
        buttons.push(("document-edit-symbolic", "Edit", "win.edit"));
    }
    buttons.push(("edit-copy-symbolic", "Copy Text", "win.copy"));
    if mine {
        buttons.push(("user-trash-symbolic", "Delete", "win.delete"));
    }
    for (icon, tip, action) in buttons {
        let button = gtk::Button::from_icon_name(icon);
        button.add_css_class("flat");
        button.set_tooltip_text(Some(tip));
        button.set_action_name(Some(action));
        button.set_action_target_value(Some(&target));
        bar.append(&button);
    }
    bar
}

/// Clicking a name or avatar shows that person's profile.
pub fn open_profile_on_click(widget: &impl IsA<gtk::Widget>, user_id: Snowflake) {
    let click = gtk::GestureClick::new();
    click.connect_released(move |gesture, _, _, _| {
        if let Some(widget) = gesture.widget() {
            let _ = widget.activate_action("win.profile", Some(&user_id.to_string().to_variant()));
        }
    });
    widget.add_controller(click);
    widget.set_cursor_from_name(Some("pointer"));
}

fn new_messages_line() -> gtk::Widget {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    line.add_css_class("new-messages");
    line.set_margin_start(16);
    line.set_margin_end(16);
    let rule = gtk::Separator::new(gtk::Orientation::Horizontal);
    rule.set_hexpand(true);
    rule.set_valign(gtk::Align::Center);
    line.append(&rule);
    line.append(&gtk::Label::new(Some("NEW")));
    line.upcast()
}

fn day_separator(time: &DateTime<Local>) -> gtk::Widget {
    let label = gtk::Label::new(Some(&time.format("%A, %-d %B %Y").to_string()));
    label.add_css_class("caption");
    label.add_css_class("dim-label");
    let left = gtk::Separator::new(gtk::Orientation::Horizontal);
    let right = gtk::Separator::new(gtk::Orientation::Horizontal);
    left.set_hexpand(true);
    right.set_hexpand(true);
    left.set_valign(gtk::Align::Center);
    right.set_valign(gtk::Align::Center);
    let separator = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    separator.add_css_class("day-separator");
    separator.append(&left);
    separator.append(&label);
    separator.append(&right);
    separator.upcast()
}

/// "Today at 18:02", "Yesterday at 09:15", or a date.
fn friendly_time(time: &DateTime<Local>) -> String {
    let today = Local::now().date_naive();
    let date = time.date_naive();
    if date == today {
        format!("Today at {}", time.format("%H:%M"))
    } else if today.pred_opt() == Some(date) {
        format!("Yesterday at {}", time.format("%H:%M"))
    } else {
        time.format("%d/%m/%Y %H:%M").to_string()
    }
}

fn local_time(unix_ms: u64) -> DateTime<Local> {
    Local
        .timestamp_millis_opt(unix_ms as i64)
        .single()
        .unwrap_or_else(Local::now)
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_fit_the_preview_box() {
        assert_eq!(fit(800, 600), (400, 300));
        assert_eq!(fit(100, 50), (100, 50), "never scaled up");
        assert_eq!(fit(1000, 100), (400, 40));
    }
}
