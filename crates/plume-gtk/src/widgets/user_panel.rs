//! Bottom-left: you. Avatar with your status (click to change it), your
//! name, and the mute, deafen and settings buttons.

use std::cell::Cell;
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::gio;

use crate::app::AppState;
use crate::widgets::avatar;

pub struct UserPanel {
    pub root: gtk::Box,
    picture: gtk::Box,
    name: gtk::Label,
    status: gtk::Label,
    mute: gtk::ToggleButton,
    deafen: gtk::ToggleButton,
    updating: Cell<bool>,
}

impl UserPanel {
    pub fn new(state: Weak<AppState>) -> Rc<Self> {
        let picture = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let name = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        name.add_css_class("heading");
        let status = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        status.add_css_class("caption");
        status.add_css_class("dim-label");
        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        text.set_valign(gtk::Align::Center);
        text.append(&name);
        text.append(&status);

        let me = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        me.append(&picture);
        me.append(&text);
        let me_button = gtk::MenuButton::builder()
            .child(&me)
            .menu_model(&status_menu())
            .tooltip_text("Set your status")
            .hexpand(true)
            .build();
        me_button.add_css_class("flat");

        let mute = gtk::ToggleButton::builder()
            .icon_name("audio-input-microphone-symbolic")
            .tooltip_text("Mute")
            .valign(gtk::Align::Center)
            .build();
        let deafen = gtk::ToggleButton::builder()
            .icon_name("audio-headphones-symbolic")
            .tooltip_text("Deafen")
            .valign(gtk::Align::Center)
            .build();
        let settings = gtk::Button::builder()
            .icon_name("emblem-system-symbolic")
            .tooltip_text("User Settings")
            .action_name("win.settings")
            .valign(gtk::Align::Center)
            .build();
        for button in [
            mute.upcast_ref::<gtk::Widget>(),
            deafen.upcast_ref(),
            settings.upcast_ref(),
        ] {
            button.add_css_class("flat");
            button.add_css_class("circular");
        }

        let root = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        root.add_css_class("user-panel");
        root.append(&me_button);
        root.append(&mute);
        root.append(&deafen);
        root.append(&settings);

        let panel = Rc::new(UserPanel {
            root,
            picture,
            name,
            status,
            mute,
            deafen,
            updating: Cell::new(false),
        });
        let on_toggle = |action: fn(&Rc<AppState>)| {
            let state = state.clone();
            let panel = Rc::downgrade(&panel);
            move |_: &gtk::ToggleButton| {
                if panel.upgrade().is_some_and(|p| p.updating.get()) {
                    return;
                }
                if let Some(state) = state.upgrade() {
                    action(&state);
                }
            }
        };
        panel.mute.connect_toggled(on_toggle(AppState::toggle_mute));
        panel
            .deafen
            .connect_toggled(on_toggle(AppState::toggle_deafen));
        panel
    }

    pub fn update(&self, state: &Rc<AppState>) {
        let me = state.store.borrow().me().cloned();
        while let Some(child) = self.picture.first_child() {
            self.picture.remove(&child);
        }
        if let Some(me) = &me {
            self.picture.append(&avatar::with_status(state, me, 32));
            self.name.set_text(me.display_name());
        } else {
            self.name.set_text("Connecting…");
        }
        let presence = me
            .map(|u| state.store.borrow().presence(u.id))
            .unwrap_or_default();
        let status = state.status.get();
        let line = presence.activity.unwrap_or_else(|| match status {
            plume_core::models::Status::Offline => "Invisible".to_owned(),
            other => other.label().to_owned(),
        });
        self.status.set_text(&line);

        self.updating.set(true);
        let muted = state.voice_muted();
        let deafened = state.voice_deafened();
        self.mute.set_active(muted);
        self.mute.set_icon_name(if muted {
            "microphone-disabled-symbolic"
        } else {
            "audio-input-microphone-symbolic"
        });
        self.mute
            .set_tooltip_text(Some(if muted { "Unmute" } else { "Mute" }));
        self.deafen.set_active(deafened);
        self.deafen.set_icon_name(if deafened {
            "audio-volume-muted-symbolic"
        } else {
            "audio-headphones-symbolic"
        });
        self.deafen
            .set_tooltip_text(Some(if deafened { "Undeafen" } else { "Deafen" }));
        self.updating.set(false);
    }
}

fn status_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let statuses = gio::Menu::new();
    statuses.append(Some("🟢 Online"), Some("win.set-status::online"));
    statuses.append(Some("🌙 Idle"), Some("win.set-status::idle"));
    statuses.append(Some("⛔ Do Not Disturb"), Some("win.set-status::dnd"));
    statuses.append(Some("⚫ Invisible"), Some("win.set-status::invisible"));
    menu.append_section(None, &statuses);
    let more = gio::Menu::new();
    more.append(Some("Set Custom Status…"), Some("win.custom-status"));
    more.append(Some("My Profile"), Some("win.my-profile"));
    more.append(Some("User Settings"), Some("win.settings"));
    menu.append_section(None, &more);
    menu
}
