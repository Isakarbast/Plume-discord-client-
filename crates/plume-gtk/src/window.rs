//! The main window: server bar + channel list on the left (they collapse
//! into their own page on narrow windows), messages and composer on the right.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use plume_core::navigation;
use plume_core::{Snowflake, Store};

use crate::app::AppState;
use crate::views::friends::FriendsView;
use crate::views::voice_panel::VoicePanel;
use crate::views::{quick_switcher, search, settings_dialog};
use crate::widgets::channel_tree::ChannelList;
use crate::widgets::composer::Composer;
use crate::widgets::emoji_picker;
use crate::widgets::guild_rail::GuildRail;
use crate::widgets::member_list::MemberList;
use crate::widgets::message_list::MessageList;
use crate::widgets::user_panel::UserPanel;
use plume_core::models::ReactionEmoji;

/// The widgets the app logic needs to reach.
pub struct Ui {
    pub window: adw::ApplicationWindow,
    pub toasts: adw::ToastOverlay,
    pub split: adw::NavigationSplitView,
    pub title: adw::WindowTitle,
    pub guild_rail: Rc<GuildRail>,
    pub channels: Rc<ChannelList>,
    pub messages: Rc<MessageList>,
    pub composer: Rc<Composer>,
    pub voice_panel: Rc<VoicePanel>,
    pub user_panel: Rc<UserPanel>,
    pub member_list: Rc<MemberList>,
    /// Chat with the member list on its right.
    pub members_split: adw::OverlaySplitView,
    pub members_button: gtk::ToggleButton,
    /// "chat" or "friends".
    pub content: gtk::Stack,
    pub friends: Rc<FriendsView>,
    /// "Start a call", shown in DMs.
    pub call_button: gtk::Button,
    pub pins_button: gtk::Button,
    pub threads_button: gtk::Button,
    pub search: gtk::SearchEntry,
}

impl Ui {
    pub fn set_title(&self, store: &Store, channel_id: Snowflake) {
        let subtitle = store
            .channel(channel_id)
            .and_then(|c| c.guild_id)
            .and_then(|id| store.guild(id))
            .map_or_else(String::new, |g| g.name.clone());
        self.title
            .set_title(&navigation::channel_label(store, channel_id));
        self.title.set_subtitle(&subtitle);
    }
}

pub fn build(state: &Rc<AppState>) -> adw::ApplicationWindow {
    let weak = Rc::downgrade(state);
    let guild_rail = GuildRail::new(weak.clone());
    let channels = ChannelList::new(weak.clone());
    let messages = MessageList::new(weak.clone());
    let composer = Composer::new(weak.clone());
    let voice_panel = VoicePanel::new(weak.clone());
    let user_panel = UserPanel::new(weak.clone());
    let member_list = MemberList::new(weak.clone());
    let friends = FriendsView::new(weak);

    // Left: server icons | channels, with the call and user panels below.
    let channel_column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    channel_column.append(&channels.root);
    channel_column.append(&voice_panel.root);
    channel_column.append(&user_panel.root);
    channel_column.set_hexpand(true);
    let rail_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    rail_box.add_css_class("guild-rail-box");
    rail_box.append(&guild_rail.root);
    guild_rail.root.set_vexpand(true);
    let join = gtk::Button::from_icon_name("list-add-symbolic");
    join.add_css_class("circular");
    join.add_css_class("join-server");
    join.set_tooltip_text(Some("Join a Server"));
    join.set_action_name(Some("win.join-server"));
    join.set_halign(gtk::Align::Center);
    join.set_margin_bottom(8);
    join.set_margin_top(4);
    rail_box.append(&join);
    let sidebar_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    sidebar_box.append(&rail_box);
    sidebar_box.append(&channel_column);
    let menu_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&main_menu())
        .tooltip_text("Menu")
        .primary(true)
        .build();
    let sidebar_header = adw::HeaderBar::new();
    sidebar_header.pack_end(&menu_button);
    let sidebar_view = adw::ToolbarView::new();
    sidebar_view.add_top_bar(&sidebar_header);
    sidebar_view.set_content(Some(&sidebar_box));
    let sidebar_page = adw::NavigationPage::new(&sidebar_view, "Plume");

    // Right: header, then chat (or Friends) with the member list beside it.
    let title = adw::WindowTitle::new("Plume", "");
    let switcher_button = gtk::Button::from_icon_name("system-search-symbolic");
    switcher_button.set_tooltip_text(Some("Jump to a channel (Ctrl+K)"));
    switcher_button.set_action_name(Some("win.switcher"));
    let call_button = gtk::Button::from_icon_name("call-start-symbolic");
    call_button.set_tooltip_text(Some("Start a voice call"));
    call_button.set_action_name(Some("win.call"));
    call_button.set_visible(false);
    let pins_button = gtk::Button::from_icon_name("view-pin-symbolic");
    pins_button.set_tooltip_text(Some("Pinned Messages"));
    let threads_button = gtk::Button::from_icon_name("view-list-symbolic");
    threads_button.set_tooltip_text(Some("Threads"));
    let members_button = gtk::ToggleButton::builder()
        .icon_name("system-users-symbolic")
        .tooltip_text("Member List")
        .active(state.options.borrow().member_list)
        .build();
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search")
        .width_chars(14)
        .build();
    let content_header = adw::HeaderBar::new();
    content_header.set_title_widget(Some(&title));
    content_header.pack_end(&switcher_button);
    content_header.pack_end(&search);
    content_header.pack_end(&members_button);
    content_header.pack_end(&pins_button);
    content_header.pack_end(&threads_button);
    content_header.pack_end(&call_button);

    let chat = gtk::Box::new(gtk::Orientation::Vertical, 0);
    chat.append(&messages.root);
    chat.append(&composer.root);
    let members_split = adw::OverlaySplitView::builder()
        .content(&chat)
        .sidebar(&member_list.root)
        .sidebar_position(gtk::PackType::End)
        .min_sidebar_width(240.0)
        .max_sidebar_width(240.0)
        .show_sidebar(state.options.borrow().member_list)
        .build();
    let content = gtk::Stack::new();
    content.add_named(&members_split, Some("chat"));
    content.add_named(&friends.root, Some("friends"));

    let content_view = adw::ToolbarView::new();
    content_view.add_top_bar(&content_header);
    content_view.set_content(Some(&content));
    let content_page = adw::NavigationPage::new(&content_view, "Chat");

    {
        let state = Rc::downgrade(state);
        let split = members_split.clone();
        members_button.connect_toggled(move |button| {
            split.set_show_sidebar(button.is_active());
            if let Some(state) = state.upgrade() {
                state.options.borrow_mut().member_list = button.is_active();
                let options = state.options.borrow().clone();
                if !options.demo {
                    let _ = options.save();
                }
                if button.is_active() {
                    state.refresh_people_now();
                }
            }
        });
    }
    {
        let state = Rc::downgrade(state);
        pins_button.connect_clicked(move |button| {
            if let Some(state) = state.upgrade() {
                search::show_pins(&state, button);
            }
        });
    }
    {
        let state = Rc::downgrade(state);
        threads_button.connect_clicked(move |button| {
            if let Some(state) = state.upgrade() {
                search::show_threads(&state, button);
            }
        });
    }
    {
        let state = Rc::downgrade(state);
        search.connect_activate(move |entry| {
            let text = entry.text().trim().to_owned();
            if let (Some(state), false) = (state.upgrade(), text.is_empty()) {
                search::search(&state, entry, &text);
            }
        });
    }

    let split = adw::NavigationSplitView::new();
    split.set_sidebar(Some(&sidebar_page));
    split.set_content(Some(&content_page));
    split.set_min_sidebar_width(300.0);
    split.set_max_sidebar_width(380.0);

    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&split));

    let window = adw::ApplicationWindow::builder()
        .application(&state.app)
        .title("Plume")
        .default_width(1100)
        .default_height(720)
        .content(&toasts)
        .build();

    // Narrow window: one pane at a time, with a back button.
    if let Ok(condition) = adw::BreakpointCondition::parse("max-width: 720sp") {
        let breakpoint = adw::Breakpoint::new(condition);
        breakpoint.add_setter(&split, "collapsed", Some(&true.to_value()));
        window.add_breakpoint(breakpoint);
    }

    add_actions(state, &window);

    // Push-to-talk: catch the key before any text box sees it.
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(state);
    keys.connect_key_pressed(move |_, key, _, _| match weak.upgrade() {
        Some(state) if state.push_to_talk_key(key, true) => glib::Propagation::Stop,
        _ => glib::Propagation::Proceed,
    });
    let weak = Rc::downgrade(state);
    keys.connect_key_released(move |_, key, _, _| {
        if let Some(state) = weak.upgrade() {
            state.push_to_talk_key(key, false);
        }
    });
    window.add_controller(keys);
    state
        .app
        .set_accels_for_action("win.switcher", &["<Control>k"]);
    state
        .app
        .set_accels_for_action("win.settings", &["<Control>comma"]);

    let weak = Rc::downgrade(state);
    window.connect_is_active_notify(move |window| {
        // Deferred: focus can change while a list is being rebuilt (and the store is borrowed).
        if window.is_active() {
            let weak = weak.clone();
            glib::idle_add_local_once(move || {
                if let Some(state) = weak.upgrade() {
                    state.window_focused();
                }
            });
        }
    });
    let weak = Rc::downgrade(state);
    window.connect_close_request(move |_| {
        if let Some(state) = weak.upgrade() {
            state.shutdown();
        }
        glib::Propagation::Proceed
    });

    let ui = Ui {
        window: window.clone(),
        toasts,
        split,
        title,
        guild_rail,
        channels,
        messages,
        composer,
        voice_panel,
        user_panel,
        member_list,
        members_split,
        members_button,
        content,
        friends,
        call_button,
        pins_button,
        threads_button,
        search,
    };
    if state.ui.set(ui).is_err() {
        tracing::warn!("window built twice");
    }
    window
}

fn main_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append(Some("Jump to Channel…"), Some("win.switcher"));
    menu.append(Some("Friends"), Some("win.friends"));
    menu.append(Some("My Profile"), Some("win.my-profile"));
    menu.append(Some("User Settings"), Some("win.settings"));
    let account = gio::Menu::new();
    account.append(Some("Log Out"), Some("app.logout"));
    account.append(Some("Copy Debug Report"), Some("win.debug-report"));
    account.append(Some("About Plume"), Some("app.about"));
    menu.append_section(None, &account);
    menu
}

/// Window actions; the message right-click menu calls these with the
/// message ID as a string.
fn add_actions(state: &Rc<AppState>, window: &adw::ApplicationWindow) {
    let message_action = |name: &str, run: fn(&Rc<AppState>, Snowflake)| {
        let action = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
        let weak = Rc::downgrade(state);
        action.connect_activate(move |_, parameter| {
            let id = parameter
                .and_then(|p| p.get::<String>())
                .and_then(|s| s.parse().ok());
            if let (Some(state), Some(id)) = (weak.upgrade(), id) {
                run(&state, id);
            }
        });
        window.add_action(&action);
    };

    message_action("reply", |state, id| state.start_reply(id));
    // Takes a user ID, not a message ID.
    message_action("profile", |state, id| {
        crate::views::profile::show(state, id)
    });
    message_action("edit", |state, id| state.start_edit(id));
    message_action("copy", |state, id| {
        if let (Some(text), Some(ui)) = (state.message_text(id), state.ui.get()) {
            ui.window.clipboard().set_text(&text);
            state.toast("Copied");
        }
    });
    message_action("react", |state, id| {
        let Some(ui) = state.ui.get() else { return };
        let Some(channel_id) = state.store.borrow().active_channel() else {
            return;
        };
        let state = state.clone();
        emoji_picker::pick(&ui.messages.root, move |emoji| {
            state.toggle_reaction(channel_id, id, &ReactionEmoji::unicode(&emoji));
        });
    });
    message_action("delete", |state, id| {
        let Some(ui) = state.ui.get() else { return };
        let dialog = adw::AlertDialog::new(Some("Delete message?"), Some("This can't be undone."));
        dialog.add_responses(&[("cancel", "Cancel"), ("delete", "Delete")]);
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        let state = state.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "delete" {
                state.delete_message(id);
            }
        });
        dialog.present(Some(&ui.window));
    });

    let switcher = gio::SimpleAction::new("switcher", None);
    let weak = Rc::downgrade(state);
    switcher.connect_activate(move |_, _| {
        if let Some(state) = weak.upgrade() {
            quick_switcher::open(&state);
        }
    });
    window.add_action(&switcher);

    let simple = |name: &str, run: fn(&Rc<AppState>)| {
        let action = gio::SimpleAction::new(name, None);
        let weak = Rc::downgrade(state);
        action.connect_activate(move |_, _| {
            if let Some(state) = weak.upgrade() {
                run(&state);
            }
        });
        window.add_action(&action);
    };
    simple("settings", settings_dialog::show);
    simple("friends", AppState::show_friends);
    simple("custom-status", AppState::edit_custom_status);

    let set_status = gio::SimpleAction::new("set-status", Some(glib::VariantTy::STRING));
    let weak = Rc::downgrade(state);
    set_status.connect_activate(move |_, parameter| {
        use plume_core::models::Status;
        let status = match parameter.and_then(|p| p.get::<String>()).as_deref() {
            Some("online") => Status::Online,
            Some("idle") => Status::Idle,
            Some("dnd") => Status::Dnd,
            Some("invisible") => Status::Offline,
            _ => return,
        };
        if let Some(state) = weak.upgrade() {
            state.set_status(status);
        }
    });
    window.add_action(&set_status);

    // Right-click menus on servers and channels pass an ID.
    message_action("mark-guild-read", |state, id| state.mark_guild_read(id));
    message_action("mute-guild", |state, id| state.toggle_guild_mute(id));
    message_action("mark-channel-read", |state, id| {
        state.mark_channel_read(id);
        state.refresh_sidebar();
    });
    message_action("dm", |state, id| state.open_dm(id));
    message_action("mention", |state, id| state.mention(id));
    message_action("pin", |state, id| state.set_pinned(id, true));
    message_action("unpin", |state, id| state.set_pinned(id, false));
    simple("join-server", AppState::join_server_dialog);
    simple("change-avatar", AppState::change_avatar);

    // "kind:guild:user" → kick, ban, timeout, untimeout.
    let moderate = gio::SimpleAction::new("moderate", Some(glib::VariantTy::STRING));
    let weak = Rc::downgrade(state);
    moderate.connect_activate(move |_, parameter| {
        let Some(text) = parameter.and_then(|p| p.get::<String>()) else {
            return;
        };
        let parts: Vec<&str> = text.split(':').collect();
        if let ([kind, guild, user], Some(state)) = (&parts[..], weak.upgrade())
            && let (Ok(guild), Ok(user)) = (guild.parse(), user.parse())
        {
            state.moderate(Snowflake(guild), Snowflake(user), kind);
        }
    });
    window.add_action(&moderate);

    // "guild:level" → 0 all, 1 mentions, 2 nothing.
    let notify = gio::SimpleAction::new("notify-level", Some(glib::VariantTy::STRING));
    let weak = Rc::downgrade(state);
    notify.connect_activate(move |_, parameter| {
        let Some(text) = parameter.and_then(|p| p.get::<String>()) else {
            return;
        };
        if let (Some((guild, level)), Some(state)) = (text.split_once(':'), weak.upgrade())
            && let (Ok(guild), Ok(level)) = (guild.parse(), level.parse())
        {
            state.set_notification_level(Snowflake(guild), level);
        }
    });
    window.add_action(&notify);

    message_action("copy-id", |state, id| {
        if let Some(ui) = state.ui.get() {
            ui.window.clipboard().set_text(&id.to_string());
            state.toast("ID copied");
        }
    });
    message_action("copy-link", |state, id| {
        let link = state.message_link(id);
        if let (Some(ui), Some(link)) = (state.ui.get(), link) {
            ui.window.clipboard().set_text(&link);
            state.toast("Link copied");
        }
    });

    let report = gio::SimpleAction::new("debug-report", None);
    let weak = Rc::downgrade(state);
    report.connect_activate(move |_, _| {
        if let Some(state) = weak.upgrade() {
            crate::debug_report::copy(&state);
        }
    });
    window.add_action(&report);

    let my_profile = gio::SimpleAction::new("my-profile", None);
    let weak = Rc::downgrade(state);
    my_profile.connect_activate(move |_, _| {
        if let Some(state) = weak.upgrade() {
            let me = state.store.borrow().me().map(|u| u.id);
            if let Some(me) = me {
                crate::views::profile::show(&state, me);
            }
        }
    });
    window.add_action(&my_profile);

    let call = gio::SimpleAction::new("call", None);
    let weak = Rc::downgrade(state);
    call.connect_activate(move |_, _| {
        if let Some(state) = weak.upgrade() {
            let channel = state.store.borrow().active_channel();
            if let Some(channel_id) = channel {
                state.join_voice(channel_id);
            }
        }
    });
    window.add_action(&call);
}
