//! The row of reaction buttons under a message. Click one to add or remove
//! your own; "+" opens the emoji chooser.

use std::rc::Rc;

use adw::prelude::*;
use plume_core::models::{Message, ReactionEmoji};

use crate::app::AppState;
use crate::widgets::{avatar, emoji_picker};
use plume_media::urls;

pub fn build(state: &Rc<AppState>, message: &Message) -> Option<gtk::Widget> {
    if message.reactions.is_empty() {
        return None;
    }
    // Wraps onto more lines instead of stretching the window (popular
    // messages can have dozens of reactions).
    let bar = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .column_spacing(4)
        .row_spacing(4)
        .max_children_per_line(40)
        .halign(gtk::Align::Fill)
        .hexpand(true)
        .build();
    let (channel_id, message_id) = (message.channel_id, message.id);

    for reaction in &message.reactions {
        let button = gtk::ToggleButton::new();
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        match reaction.emoji.id {
            // Server emoji: the picture, like Discord.
            Some(id) => {
                let picture = gtk::Picture::new();
                picture.set_size_request(18, 18);
                picture.set_can_shrink(true);
                picture.set_tooltip_text(Some(&format!(":{}:", reaction.emoji.name)));
                let weak = picture.downgrade();
                avatar::load(state, urls::emoji(id, false, 36), 36, move |texture| {
                    if let Some(picture) = weak.upgrade() {
                        picture.set_paintable(Some(texture));
                    }
                });
                content.append(&picture);
            }
            None => content.append(&gtk::Label::new(Some(&reaction.emoji.name))),
        }
        content.append(&gtk::Label::new(Some(&reaction.count.to_string())));
        button.set_child(Some(&content));
        button.add_css_class("reaction");
        // Set before connecting, so building the row doesn't send anything.
        button.set_active(reaction.me);
        if reaction.me {
            button.add_css_class("mine");
        }
        let emoji = reaction.emoji.clone();
        let state = state.clone();
        button.connect_clicked(move |_| state.toggle_reaction(channel_id, message_id, &emoji));
        bar.append(&button);
    }

    let add = gtk::Button::from_icon_name("face-smile-symbolic");
    add.add_css_class("reaction");
    add.add_css_class("flat");
    add.set_tooltip_text(Some("Add reaction"));
    let state = state.clone();
    add.connect_clicked(move |button| {
        let state = state.clone();
        emoji_picker::pick(button, move |emoji| {
            state.toggle_reaction(channel_id, message_id, &ReactionEmoji::unicode(&emoji));
        });
    });
    bar.append(&add);
    Some(bar.upcast())
}
