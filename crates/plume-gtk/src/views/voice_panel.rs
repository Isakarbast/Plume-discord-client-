//! The strip under the channel list while you're in a call: where you are,
//! whether it's encrypted, and hang up. Mute and deafen live in the user
//! panel below it.

use std::rc::{Rc, Weak};

use adw::prelude::*;
use plume_core::navigation;

use crate::app::{AppState, VoiceStatus};

pub struct VoicePanel {
    pub root: gtk::Revealer,
    status: gtk::Label,
    place: gtk::Label,
    lock: gtk::Image,
}

impl VoicePanel {
    pub fn new(state: Weak<AppState>) -> Rc<Self> {
        let status = gtk::Label::builder().xalign(0.0).build();
        status.add_css_class("voice-status");
        status.add_css_class("caption-heading");
        let lock = gtk::Image::from_icon_name("channel-secure-symbolic");
        lock.add_css_class("dim-label");
        let status_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        status_row.append(&status);
        status_row.append(&lock);

        let place = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        place.add_css_class("caption");
        place.add_css_class("dim-label");

        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        text.append(&status_row);
        text.append(&place);

        let leave = gtk::Button::builder()
            .icon_name("call-stop-symbolic")
            .tooltip_text("Disconnect")
            .valign(gtk::Align::Center)
            .build();
        leave.add_css_class("destructive-action");
        leave.add_css_class("circular");
        leave.add_css_class("flat");

        let content = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        content.add_css_class("voice-panel");
        content.append(&text);
        content.append(&leave);

        let root = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideUp)
            .child(&content)
            .build();

        let panel = Rc::new(VoicePanel {
            root,
            status,
            place,
            lock,
        });

        leave.connect_clicked(move |_| {
            if let Some(state) = state.upgrade() {
                state.leave_voice();
            }
        });
        panel
    }

    pub fn update(&self, state: &AppState) {
        let voice = state.voice.borrow();
        let Some(call) = voice.as_ref() else {
            self.root.set_reveal_child(false);
            return;
        };
        self.root.set_reveal_child(true);

        let (text, class) = match call.status {
            VoiceStatus::Joining => ("Joining…", "connecting"),
            VoiceStatus::Connecting => ("Connecting…", "connecting"),
            VoiceStatus::Connected { .. } => ("Voice Connected", "connected"),
            VoiceStatus::Demo => ("Voice Connected (demo, no audio)", "connected"),
        };
        self.status.set_text(text);
        self.status
            .set_css_classes(&["voice-status", "caption-heading", class]);

        let encrypted = matches!(call.status, VoiceStatus::Connected { encrypted: true });
        self.lock.set_visible(encrypted);
        let tooltip = match &call.privacy_code {
            Some(code) => format!(
                "End-to-end encrypted\nPrivacy code: {}\nCompare it with the others in the call.",
                group_digits(code)
            ),
            None => "End-to-end encrypted".to_owned(),
        };
        self.lock.set_tooltip_text(Some(&tooltip));

        let store = state.store.borrow();
        let channel = navigation::channel_label(&store, call.channel_id);
        let place = match call.guild_id.and_then(|id| store.guild(id)) {
            Some(guild) => format!("{channel} · {}", guild.name),
            None => channel,
        };
        self.place.set_text(&place);
    }
}

/// "123451234512345…" → "12345 12345 12345 …".
fn group_digits(code: &str) -> String {
    code.as_bytes()
        .chunks(5)
        .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}
