//! Status, friends, the member list, mark-as-read, muting and per-person
//! voice volume.

use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;
use plume_core::models::{Presence, Status};
use plume_core::{Event, Snowflake};

use super::AppState;
use crate::state::bridge;

impl AppState {
    pub fn set_status(self: &Rc<Self>, status: Status) {
        self.status.set(status);
        let options = {
            let mut options = self.options.borrow_mut();
            options.status = status;
            options.clone()
        };
        if !options.demo
            && let Err(error) = options.save()
        {
            tracing::warn!("couldn't save settings: {error}");
        }
        self.send_presence();
    }

    /// Set (or clear, with "") the line under our name.
    pub fn set_custom_status(self: &Rc<Self>, text: &str) {
        let options = {
            let mut options = self.options.borrow_mut();
            options.custom_status = text.trim().to_owned();
            options.clone()
        };
        if !options.demo
            && let Err(error) = options.save()
        {
            tracing::warn!("couldn't save settings: {error}");
        }
        self.send_presence();
    }

    fn send_presence(self: &Rc<Self>) {
        let status = self.status.get();
        let custom = self.options.borrow().custom_status.clone();
        if let Some(gateway) = &*self.gateway.borrow() {
            gateway.set_status(status, Some(custom.clone()));
        }
        // Show it right away; Discord doesn't echo our own presence.
        let me = self.store.borrow().me().map(|u| u.id);
        if let Some(me) = me {
            self.store.borrow_mut().apply(Event::PresenceUpdate {
                user_id: me,
                presence: Presence {
                    status,
                    activity: (!custom.is_empty()).then_some(custom),
                },
            });
        }
        self.refresh_people_now();
    }

    /// A small dialog to type a custom status.
    pub fn edit_custom_status(self: &Rc<Self>) {
        let Some(ui) = self.ui.get() else { return };
        let entry = gtk::Entry::builder()
            .text(self.options.borrow().custom_status.as_str())
            .placeholder_text("What's happening?")
            .max_length(128)
            .activates_default(true)
            .build();
        let dialog = adw::AlertDialog::new(Some("Set a custom status"), None);
        dialog.set_extra_child(Some(&entry));
        dialog.add_responses(&[("clear", "Clear"), ("cancel", "Cancel"), ("save", "Save")]);
        dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("save"));
        dialog.set_close_response("cancel");
        let state = self.clone();
        dialog.connect_response(None, move |_, response| match response {
            "save" => state.set_custom_status(&entry.text()),
            "clear" => state.set_custom_status(""),
            _ => {}
        });
        dialog.present(Some(&ui.window));
    }

    /// Statuses change often; redraw the people views at most a few
    /// times a second.
    pub fn refresh_people(self: &Rc<Self>) {
        if self.people_pending.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(400), move || {
            if let Some(state) = weak.upgrade() {
                state.people_pending.set(false);
                state.refresh_people_now();
            }
        });
    }

    pub fn refresh_people_now(self: &Rc<Self>) {
        let ui = self.ui();
        ui.user_panel.update(self);
        if self.showing_friends.get() {
            ui.friends.rebuild(self);
        } else if ui.members_split.shows_sidebar() {
            ui.member_list.rebuild(self);
        }
    }

    /// The Friends page, from the top of the DM list.
    pub fn show_friends(self: &Rc<Self>) {
        self.showing_friends.set(true);
        self.selected_guild.set(None);
        let ui = self.ui();
        ui.content.set_visible_child_name("friends");
        ui.title.set_title("Friends");
        ui.title.set_subtitle("");
        ui.split.set_show_content(true);
        ui.friends.rebuild(self);
        self.update_header_buttons();
        self.refresh_sidebar();
    }

    pub fn send_friend_request(self: &Rc<Self>, username: &str) {
        let Some(rest) = self.rest() else {
            self.toast("Not available in demo mode");
            return;
        };
        let username = username.to_owned();
        let weak = Rc::downgrade(self);
        bridge::spawn(
            async move {
                let result = rest.send_friend_request(&username).await;
                (username, result)
            },
            move |(username, result)| {
                let Some(state) = weak.upgrade() else { return };
                match result {
                    Ok(()) => state.toast(&format!("Friend request sent to {username}")),
                    Err(error) => state.toast(&format!("Couldn't send the request: {error}")),
                }
            },
        );
    }

    pub fn accept_friend(self: &Rc<Self>, user_id: Snowflake) {
        let Some(rest) = self.rest() else {
            self.toast("Not available in demo mode");
            return;
        };
        let weak = Rc::downgrade(self);
        bridge::spawn(
            async move { rest.accept_friend(user_id).await },
            move |result| {
                if let (Some(state), Err(error)) = (weak.upgrade(), result) {
                    state.toast(&format!("Couldn't accept: {error}"));
                }
            },
        );
    }

    pub fn remove_relationship(self: &Rc<Self>, user_id: Snowflake) {
        let Some(rest) = self.rest() else {
            self.toast("Not available in demo mode");
            return;
        };
        let weak = Rc::downgrade(self);
        bridge::spawn(
            async move { rest.remove_relationship(user_id).await },
            move |result| {
                if let (Some(state), Err(error)) = (weak.upgrade(), result) {
                    state.toast(&format!("Couldn't do that: {error}"));
                }
            },
        );
    }

    /// Mark every unread channel in a server as read.
    pub fn mark_guild_read(self: &Rc<Self>, guild_id: Snowflake) {
        let unread: Vec<Snowflake> = {
            let store = self.store.borrow();
            store
                .guild(guild_id)
                .map(|g| g.channels.clone())
                .unwrap_or_default()
                .into_iter()
                .filter(|&c| store.unread(c).unread || store.unread(c).mentions > 0)
                .collect()
        };
        for channel_id in unread {
            self.mark_channel_read(channel_id);
        }
        self.refresh_sidebar();
    }

    pub fn mark_channel_read(&self, channel_id: Snowflake) {
        let ack = self.store.borrow_mut().mark_read(channel_id);
        if let (Some(message_id), Some(rest)) = (ack, self.rest()) {
            bridge::spawn_detached(async move {
                if let Err(error) = rest.ack_message(channel_id, message_id).await {
                    tracing::debug!("ack failed: {error}");
                }
            });
        }
    }

    pub fn toggle_guild_mute(self: &Rc<Self>, guild_id: Snowflake) {
        let muted = self.store.borrow().is_guild_muted(guild_id);
        let Some(rest) = self.rest() else {
            self.toast("Not available in demo mode");
            return;
        };
        let weak = Rc::downgrade(self);
        bridge::spawn(
            async move { rest.set_guild_muted(guild_id, !muted).await },
            move |result| {
                let Some(state) = weak.upgrade() else { return };
                match result {
                    Ok(()) => state.toast(if muted {
                        "Server unmuted"
                    } else {
                        "Server muted"
                    }),
                    Err(error) => state.toast(&format!("Couldn't change that: {error}")),
                }
            },
        );
    }

    pub fn send_sticker(self: &Rc<Self>, sticker_id: Snowflake) {
        let Some(channel_id) = self.store.borrow().active_channel() else {
            return;
        };
        let Some(rest) = self.rest() else {
            self.toast("Not available in demo mode");
            return;
        };
        let weak = Rc::downgrade(self);
        bridge::spawn(
            async move { rest.send_sticker(channel_id, sticker_id).await },
            move |result| {
                let Some(state) = weak.upgrade() else { return };
                match result {
                    Ok((message, author)) => {
                        state.on_event(Event::MessageCreate { message, author })
                    }
                    Err(error) => state.toast(&format!("Couldn't send the sticker: {error}")),
                }
            },
        );
    }

    /// Run a REST call and toast what happened.
    fn rest_action<F, Fut>(self: &Rc<Self>, done: &'static str, call: F)
    where
        F: FnOnce(plume_rest::RestClient) -> Fut,
        Fut: std::future::Future<Output = Result<(), plume_rest::RestError>> + Send + 'static,
    {
        let Some(rest) = self.rest() else {
            self.toast("Not available in demo mode");
            return;
        };
        let future = call(rest);
        let weak = Rc::downgrade(self);
        bridge::spawn(future, move |result| {
            let Some(state) = weak.upgrade() else { return };
            match result {
                Ok(()) => state.toast(done),
                Err(error) => state.toast(&format!("That didn't work: {error}")),
            }
        });
    }

    /// Kick, ban or time out someone, after asking.
    pub fn moderate(self: &Rc<Self>, guild_id: Snowflake, user_id: Snowflake, kind: &str) {
        let Some(ui) = self.ui.get() else { return };
        let name = self.store.borrow().display_name_in(Some(guild_id), user_id);
        let (title, body, button) = match kind {
            "kick" => (
                "Kick",
                format!("Kick {name} from the server? They can rejoin with an invite."),
                "Kick",
            ),
            "ban" => (
                "Ban",
                format!("Ban {name}? They won't be able to rejoin."),
                "Ban",
            ),
            "timeout" => (
                "Timeout",
                format!("Time out {name} for 1 hour? They can't talk until then."),
                "Timeout",
            ),
            "untimeout" => (
                "Remove Timeout",
                format!("Let {name} talk again?"),
                "Remove",
            ),
            _ => return,
        };
        let dialog = adw::AlertDialog::new(Some(title), Some(&body));
        dialog.add_responses(&[("cancel", "Cancel"), ("go", button)]);
        dialog.set_response_appearance("go", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let state = self.clone();
        let kind = kind.to_owned();
        dialog.connect_response(None, move |_, response| {
            if response != "go" {
                return;
            }
            match kind.as_str() {
                "kick" => state.rest_action("Kicked", move |rest| async move {
                    rest.kick(guild_id, user_id).await
                }),
                "ban" => state.rest_action("Banned", move |rest| async move {
                    rest.ban(guild_id, user_id).await
                }),
                "timeout" => {
                    let until = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_secs() as i64)
                        + 3600;
                    state.rest_action("Timed out for 1 hour", move |rest| async move {
                        rest.timeout(guild_id, user_id, until).await
                    })
                }
                _ => state.rest_action("Timeout removed", move |rest| async move {
                    rest.timeout(guild_id, user_id, 0).await
                }),
            }
        });
        dialog.present(Some(&ui.window));
    }

    /// Put "@Name " in the message box.
    pub fn mention(self: &Rc<Self>, user_id: Snowflake) {
        let guild = self
            .store
            .borrow()
            .active_channel()
            .and_then(|c| self.store.borrow().channel(c).and_then(|c| c.guild_id));
        let name = self.store.borrow().display_name_in(guild, user_id);
        self.ui().composer.insert_mention(&name, user_id);
    }

    pub fn set_pinned(self: &Rc<Self>, message_id: Snowflake, pinned: bool) {
        let Some(channel_id) = self.store.borrow().active_channel() else {
            return;
        };
        let Some(rest) = self.rest() else {
            self.toast("Not available in demo mode");
            return;
        };
        let weak = Rc::downgrade(self);
        bridge::spawn(
            async move { rest.set_pinned(channel_id, message_id, pinned).await },
            move |result| {
                let Some(state) = weak.upgrade() else { return };
                match result {
                    Ok(()) => {
                        state
                            .store
                            .borrow_mut()
                            .set_pinned(channel_id, message_id, pinned);
                        state.ui().messages.refresh(message_id);
                        state.toast(if pinned {
                            "Message pinned"
                        } else {
                            "Message unpinned"
                        });
                    }
                    Err(error) => state.toast(&format!("Couldn't change the pin: {error}")),
                }
            },
        );
    }

    pub fn set_notification_level(self: &Rc<Self>, guild_id: Snowflake, level: u8) {
        let label = match level {
            0 => "All messages",
            2 => "Nothing",
            _ => "Only @mentions",
        };
        self.rest_action(label, move |rest| async move {
            rest.set_notification_level(guild_id, level).await
        });
    }

    /// Ask for an invite link and join that server.
    pub fn join_server_dialog(self: &Rc<Self>) {
        let Some(ui) = self.ui.get() else { return };
        let entry = gtk::Entry::builder()
            .placeholder_text("https://discord.gg/hTKzmak")
            .activates_default(true)
            .build();
        let dialog = adw::AlertDialog::new(
            Some("Join a Server"),
            Some("Enter an invite below to join an existing server."),
        );
        dialog.set_extra_child(Some(&entry));
        dialog.add_responses(&[("cancel", "Cancel"), ("join", "Join Server")]);
        dialog.set_response_appearance("join", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("join"));
        dialog.set_close_response("cancel");
        let state = self.clone();
        dialog.connect_response(None, move |_, response| {
            let code = entry.text().to_string();
            if response == "join" && !code.trim().is_empty() {
                state.rest_action(
                    "Joined! It'll show up in a moment.",
                    move |rest| async move { rest.join_invite(&code).await },
                );
            }
        });
        dialog.present(Some(&ui.window));
    }

    pub fn set_display_name(self: &Rc<Self>, name: String) {
        self.rest_action("Display name updated", move |rest| async move {
            rest.set_display_name(&name).await
        });
    }

    /// Pick an image file and make it our avatar.
    pub fn change_avatar(self: &Rc<Self>) {
        let Some(ui) = self.ui.get() else { return };
        let filter = gtk::FileFilter::new();
        filter.add_mime_type("image/png");
        filter.add_mime_type("image/jpeg");
        filter.add_mime_type("image/gif");
        filter.add_mime_type("image/webp");
        let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let state = self.clone();
        gtk::FileDialog::builder()
            .title("Choose an avatar")
            .filters(&filters)
            .modal(true)
            .build()
            .open(
                Some(&ui.window),
                gtk::gio::Cancellable::NONE,
                move |result| {
                    let Some(path) = result.ok().and_then(|f| f.path()) else {
                        return;
                    };
                    let mime = match path
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(str::to_ascii_lowercase)
                        .as_deref()
                    {
                        Some("png") => "image/png",
                        Some("gif") => "image/gif",
                        Some("webp") => "image/webp",
                        _ => "image/jpeg",
                    };
                    match std::fs::read(&path) {
                        Ok(bytes) if bytes.len() <= 8 * 1024 * 1024 => {
                            state.rest_action("Avatar updated", move |rest| async move {
                                rest.set_avatar(&bytes, mime).await
                            });
                        }
                        Ok(_) => state.toast("That image is bigger than 8 MB"),
                        Err(error) => state.toast(&format!("Couldn't read it: {error}")),
                    }
                },
            );
    }

    /// Someone's voice volume, in percent (100 = normal).
    pub fn user_volume(&self, user_id: Snowflake) -> u32 {
        self.options
            .borrow()
            .user_volumes
            .get(&user_id)
            .copied()
            .unwrap_or(100)
    }

    pub fn set_user_volume(&self, user_id: Snowflake, percent: u32) {
        let options = {
            let mut options = self.options.borrow_mut();
            if percent == 100 {
                options.user_volumes.remove(&user_id);
            } else {
                options.user_volumes.insert(user_id, percent);
            }
            options.clone()
        };
        if !options.demo {
            let _ = options.save();
        }
        self.apply_user_volume(user_id, percent);
    }

    /// Apply settings that can change while running.
    pub fn settings_changed(self: &Rc<Self>) {
        let options = self.options.borrow().clone();
        self.apply_voice_options(&options);
        let ui = self.ui();
        ui.members_split.set_show_sidebar(options.member_list);
        ui.members_button.set_active(options.member_list);
    }
}
