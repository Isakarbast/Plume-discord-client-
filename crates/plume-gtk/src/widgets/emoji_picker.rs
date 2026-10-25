//! GTK's built-in emoji chooser, as a popover on any widget.

use adw::prelude::*;

pub fn pick(parent: &impl IsA<gtk::Widget>, on_pick: impl Fn(String) + 'static) {
    let chooser = gtk::EmojiChooser::new();
    chooser.set_parent(parent);
    chooser.connect_emoji_picked(move |_, emoji| on_pick(emoji.to_owned()));
    // A popover must be unparented when it closes, or it leaks.
    chooser.connect_closed(|chooser| {
        let chooser = chooser.clone();
        gtk::glib::idle_add_local_once(move || chooser.unparent());
    });
    chooser.popup();
}
