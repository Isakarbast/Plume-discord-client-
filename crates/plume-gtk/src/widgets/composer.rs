//! The message box at the bottom: typing indicator, reply/edit bar,
//! attachments, the text field, and the emoji button.
//!
//! Enter sends, Shift+Enter adds a line, ↑ in an empty box edits your last
//! message, Esc cancels a reply or edit.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::app::AppState;
use crate::widgets::autocomplete::{self, Suggestion};
use crate::widgets::{avatar, emoji_picker};

pub struct Composer {
    pub root: gtk::Box,
    pub typing: gtk::Label,
    pub text: gtk::TextView,
    context: gtk::Box,
    context_label: gtk::Label,
    /// "@ ON": ping the person being replied to.
    ping: gtk::ToggleButton,
    files_box: gtk::Box,
    files: RefCell<Vec<PathBuf>>,
    state: Weak<AppState>,
    suggest: gtk::Popover,
    suggest_list: gtk::ListBox,
    suggestions: RefCell<Vec<Suggestion>>,
    /// Length (in characters) of the `@ali` being completed.
    token_len: Cell<usize>,
    /// "@Alice" → "<@123>" for everything picked from the suggestions.
    replacements: RefCell<Vec<(String, String)>>,
}

impl Composer {
    pub fn new(state: Weak<AppState>) -> Rc<Self> {
        let typing = gtk::Label::builder()
            .xalign(0.0)
            .margin_start(16)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        typing.add_css_class("caption");
        typing.add_css_class("dim-label");
        typing.set_size_request(-1, 18);

        let context_label = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        context_label.add_css_class("caption");
        let cancel = gtk::Button::from_icon_name("window-close-symbolic");
        cancel.add_css_class("flat");
        cancel.add_css_class("circular");
        cancel.set_tooltip_text(Some("Cancel (Esc)"));
        let ping = gtk::ToggleButton::builder()
            .label("@ ON")
            .active(true)
            .tooltip_text("Mention the person you're replying to")
            .visible(false)
            .build();
        ping.add_css_class("flat");
        ping.add_css_class("reply-ping");
        ping.connect_toggled(|button| {
            button.set_label(if button.is_active() { "@ ON" } else { "@ OFF" });
        });
        let context = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        context.add_css_class("composer-context");
        context.append(&context_label);
        context.append(&ping);
        context.append(&cancel);
        context.set_visible(false);

        let files_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        files_box.set_visible(false);
        files_box.set_margin_start(12);

        let text = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .accepts_tab(false)
            .top_margin(8)
            .bottom_margin(8)
            .left_margin(4)
            .right_margin(4)
            .build();
        text.add_css_class("composer-text");
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(200)
            .hexpand(true)
            .child(&text)
            .build();

        let attach = gtk::Button::from_icon_name("mail-attachment-symbolic");
        attach.add_css_class("flat");
        attach.set_tooltip_text(Some("Attach a file"));
        attach.set_valign(gtk::Align::End);
        let emoji = gtk::Button::from_icon_name("face-smile-symbolic");
        emoji.add_css_class("flat");
        emoji.set_tooltip_text(Some("Emoji"));
        emoji.set_valign(gtk::Align::End);

        let gif = gtk::Button::with_label("GIF");
        gif.add_css_class("flat");
        gif.add_css_class("gif-button");
        gif.set_tooltip_text(Some("GIFs and stickers"));
        gif.set_valign(gtk::Align::End);

        let field = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        field.add_css_class("composer");
        field.append(&attach);
        field.append(&scrolled);
        field.append(&gif);
        field.append(&emoji);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 2);
        root.set_margin_start(12);
        root.set_margin_end(12);
        root.set_margin_bottom(12);
        root.append(&typing);
        root.append(&context);
        root.append(&files_box);
        root.append(&field);

        let suggest_list = gtk::ListBox::new();
        suggest_list.add_css_class("autocomplete");
        suggest_list.set_selection_mode(gtk::SelectionMode::Single);
        let suggest = gtk::Popover::builder()
            .child(&suggest_list)
            .autohide(false)
            .has_arrow(false)
            .position(gtk::PositionType::Top)
            .build();
        suggest.set_parent(&text);

        let composer = Rc::new(Composer {
            root,
            typing,
            text,
            context,
            context_label,
            ping,
            files_box,
            files: RefCell::new(Vec::new()),
            state: state.clone(),
            suggest,
            suggest_list,
            suggestions: RefCell::new(Vec::new()),
            token_len: Cell::new(0),
            replacements: RefCell::new(Vec::new()),
        });
        let weak = Rc::downgrade(&composer);
        composer.suggest_list.connect_row_activated(move |_, row| {
            if let Some(composer) = weak.upgrade() {
                composer.accept(row.index() as usize);
            }
        });

        // Enter / Shift+Enter / ↑ / Esc.
        let keys = gtk::EventControllerKey::new();
        let key_state = state.clone();
        let weak = Rc::downgrade(&composer);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let (Some(state), Some(composer)) = (key_state.upgrade(), weak.upgrade()) else {
                return glib::Propagation::Proceed;
            };
            if composer.suggest.is_visible() {
                let count = composer.suggestions.borrow().len() as i32;
                let current = composer
                    .suggest_list
                    .selected_row()
                    .map_or(0, |r| r.index());
                let select = |index: i32| {
                    let row = composer
                        .suggest_list
                        .row_at_index(index.rem_euclid(count.max(1)));
                    composer.suggest_list.select_row(row.as_ref());
                };
                match key {
                    gdk::Key::Up => {
                        select(current - 1);
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::Down => {
                        select(current + 1);
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::Tab | gdk::Key::Return | gdk::Key::KP_Enter => {
                        composer.accept(current as usize);
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::Escape => {
                        composer.suggest.popdown();
                        return glib::Propagation::Stop;
                    }
                    _ => {}
                }
            }
            match key {
                gdk::Key::Return | gdk::Key::KP_Enter
                    if !modifiers.contains(gdk::ModifierType::SHIFT_MASK) =>
                {
                    state.submit(composer.take_text(), composer.take_files());
                    glib::Propagation::Stop
                }
                gdk::Key::Up if composer.current_text().is_empty() => {
                    state.edit_last_message();
                    glib::Propagation::Stop
                }
                gdk::Key::Escape => {
                    state.cancel_compose();
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        composer.text.add_controller(keys);

        let typing_state = state.clone();
        let weak = Rc::downgrade(&composer);
        composer.text.buffer().connect_changed(move |buffer| {
            if buffer.char_count() > 0
                && let Some(state) = typing_state.upgrade()
            {
                state.notify_typing();
            }
            if let Some(composer) = weak.upgrade() {
                // Wait until the cursor has moved past the new text.
                glib::idle_add_local_once(move || composer.update_suggestions());
            }
        });

        let cancel_state = state.clone();
        cancel.connect_clicked(move |_| {
            if let Some(state) = cancel_state.upgrade() {
                state.cancel_compose();
            }
        });

        let weak = Rc::downgrade(&composer);
        emoji.connect_clicked(move |button| {
            let weak = weak.clone();
            emoji_picker::pick(button, move |emoji| {
                if let Some(composer) = weak.upgrade() {
                    composer.text.buffer().insert_at_cursor(&emoji);
                    composer.text.grab_focus();
                }
            });
        });

        let gif_state = state.clone();
        gif.connect_clicked(move |button| {
            if let Some(state) = gif_state.upgrade() {
                crate::widgets::media_picker::open(&state, button);
            }
        });

        let weak = Rc::downgrade(&composer);
        attach.connect_clicked(move |button| {
            let weak = weak.clone();
            let window = button.root().and_downcast::<gtk::Window>();
            gtk::FileDialog::builder()
                .title("Attach a file")
                .modal(true)
                .build()
                .open(window.as_ref(), gio::Cancellable::NONE, move |result| {
                    if let (Ok(file), Some(composer)) = (result, weak.upgrade())
                        && let Some(path) = file.path()
                    {
                        composer.add_file(path);
                    }
                });
        });
        composer
    }

    pub fn current_text(&self) -> String {
        let buffer = self.text.buffer();
        buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string()
    }

    pub fn set_text(&self, text: &str) {
        self.text.buffer().set_text(text);
        self.text.grab_focus();
    }

    fn take_text(&self) -> String {
        let mut text = self.current_text();
        self.text.buffer().set_text("");
        self.suggest.popdown();
        // Longest first, so "@Al" doesn't eat part of "@Alice".
        let mut replacements = std::mem::take(&mut *self.replacements.borrow_mut());
        replacements.sort_by_key(|(visible, _)| std::cmp::Reverse(visible.len()));
        for (visible, wire) in replacements {
            text = text.replace(&visible, &wire);
        }
        text
    }

    /// Show (or hide) suggestions for the word being typed.
    fn update_suggestions(self: &Rc<Self>) {
        let Some(state) = self.state.upgrade() else {
            return;
        };
        let buffer = self.text.buffer();
        let cursor = buffer.iter_at_mark(&buffer.get_insert());
        let mut line_start = cursor;
        line_start.set_line_offset(0);
        let before = buffer.text(&line_start, &cursor, false).to_string();
        let channel = state.store.borrow().active_channel();
        let found = match (autocomplete::token_before_cursor(&before), channel) {
            (Some((trigger, query, len)), Some(channel_id)) => {
                self.token_len.set(len);
                autocomplete::suggestions(&state.store.borrow(), channel_id, trigger, &query)
            }
            _ => Vec::new(),
        };
        if found.is_empty() {
            self.suggest.popdown();
            self.suggestions.borrow_mut().clear();
            return;
        }
        while let Some(child) = self.suggest_list.first_child() {
            self.suggest_list.remove(&child);
        }
        for suggestion in &found {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            if let Some((id, animated)) = suggestion.emoji {
                let picture = gtk::Picture::new();
                picture.set_size_request(20, 20);
                picture.set_can_shrink(true);
                let weak = picture.downgrade();
                avatar::load(
                    &state,
                    plume_media::urls::emoji(id, animated, 40),
                    40,
                    move |texture| {
                        if let Some(picture) = weak.upgrade() {
                            picture.set_paintable(Some(texture));
                        }
                    },
                );
                row.append(&picture);
            }
            let label = gtk::Label::builder()
                .label(&suggestion.label)
                .xalign(0.0)
                .hexpand(true)
                .build();
            row.append(&label);
            let detail = gtk::Label::new(Some(&suggestion.detail));
            detail.add_css_class("dim-label");
            detail.add_css_class("caption");
            row.append(&detail);
            self.suggest_list.append(&row);
        }
        self.suggest_list
            .select_row(self.suggest_list.row_at_index(0).as_ref());
        *self.suggestions.borrow_mut() = found;

        let location = self.text.iter_location(&cursor);
        let (x, y) = self.text.buffer_to_window_coords(
            gtk::TextWindowType::Widget,
            location.x(),
            location.y(),
        );
        self.suggest
            .set_pointing_to(Some(&gdk::Rectangle::new(x, y, 1, location.height())));
        self.suggest.set_size_request(320, -1);
        self.suggest.popup();
    }

    /// Replace the word being typed with suggestion `index`.
    fn accept(&self, index: usize) {
        let Some(suggestion) = self.suggestions.borrow().get(index).cloned() else {
            return;
        };
        let buffer = self.text.buffer();
        let mut end = buffer.iter_at_mark(&buffer.get_insert());
        let mut start = end;
        start.backward_chars(self.token_len.get() as i32);
        buffer.delete(&mut start, &mut end);
        buffer.insert(&mut start, &format!("{} ", suggestion.insert));
        if let Some(wire) = suggestion.wire {
            self.replacements
                .borrow_mut()
                .push((suggestion.insert, wire));
        }
        self.suggest.popdown();
        self.text.grab_focus();
    }

    fn take_files(self: &Rc<Self>) -> Vec<PathBuf> {
        let files = std::mem::take(&mut *self.files.borrow_mut());
        self.show_files();
        files
    }

    fn add_file(self: &Rc<Self>, path: PathBuf) {
        self.files.borrow_mut().push(path);
        self.show_files();
        self.text.grab_focus();
    }

    /// One chip per pending file, each with a remove button.
    fn show_files(self: &Rc<Self>) {
        while let Some(child) = self.files_box.first_child() {
            self.files_box.remove(&child);
        }
        let files = self.files.borrow();
        self.files_box.set_visible(!files.is_empty());
        for (index, path) in files.iter().enumerate() {
            let name = path
                .file_name()
                .map_or_else(|| "file".into(), |n| n.to_string_lossy().into_owned());
            let chip = gtk::Button::with_label(&format!("📎 {name}  ✕"));
            chip.add_css_class("pill");
            chip.add_css_class("small");
            chip.set_tooltip_text(Some("Remove"));
            let weak = Rc::downgrade(self);
            chip.connect_clicked(move |_| {
                if let Some(composer) = weak.upgrade() {
                    let mut files = composer.files.borrow_mut();
                    if index < files.len() {
                        files.remove(index);
                    }
                    drop(files);
                    composer.show_files();
                }
            });
            self.files_box.append(&chip);
        }
    }

    /// "Replying to Alice" / "Editing message", or nothing.
    pub fn set_context(&self, text: Option<&str>) {
        self.context.set_visible(text.is_some());
        self.context_label.set_text(text.unwrap_or(""));
        self.ping.set_visible(false);
    }

    /// Insert "@Name " at the cursor; it's sent as a real mention.
    pub fn insert_mention(&self, name: &str, user_id: plume_core::Snowflake) {
        let visible = format!("@{name}");
        self.text.buffer().insert_at_cursor(&format!("{visible} "));
        self.replacements
            .borrow_mut()
            .push((visible, format!("<@{user_id}>")));
        self.text.grab_focus();
    }

    pub fn show_reply_ping(&self, visible: bool) {
        self.ping.set_visible(visible);
        self.ping.set_active(true);
    }

    pub fn reply_ping(&self) -> bool {
        self.ping.is_active()
    }

    pub fn set_placeholder_enabled(&self, can_send: bool) {
        self.text.set_sensitive(can_send);
        self.text
            .set_tooltip_text((!can_send).then_some("You can't send messages in this channel"));
    }
}
