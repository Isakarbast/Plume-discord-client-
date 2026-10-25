//! User Settings: account, voice, notifications, appearance. Changes are
//! saved to `~/.config/plume/settings` straight away.

use std::rc::Rc;

use adw::prelude::*;
use plume_core::{Profile, Tier};

use crate::app::AppState;
use crate::settings::Options;
use crate::widgets::avatar;

pub fn show(state: &Rc<AppState>) {
    let Some(ui) = state.ui.get() else { return };
    let options = state.options.borrow().clone();
    let dialog = adw::PreferencesDialog::new();
    dialog.set_title("User Settings");

    // My Account
    let account = adw::PreferencesPage::builder()
        .title("My Account")
        .icon_name("avatar-default-symbolic")
        .build();
    let who = adw::PreferencesGroup::new();
    let me = state.store.borrow().me().cloned();
    let me_row = adw::ActionRow::builder()
        .title(me.as_ref().map_or("Not logged in", |u| u.display_name()))
        .subtitle(me.as_ref().map(|u| u.username.clone()).unwrap_or_default())
        .use_markup(false)
        .build();
    if let Some(me) = &me {
        me_row.add_prefix(&avatar::with_status(state, me, 48));
    }
    let profile_button = gtk::Button::with_label("View Profile");
    profile_button.set_valign(gtk::Align::Center);
    profile_button.set_action_name(Some("win.my-profile"));
    me_row.add_suffix(&profile_button);
    who.add(&me_row);
    let display_name = adw::EntryRow::builder()
        .title("Display name")
        .text(
            me.as_ref()
                .and_then(|u| u.global_name.clone())
                .unwrap_or_default(),
        )
        .show_apply_button(true)
        .build();
    {
        let state = state.clone();
        display_name.connect_apply(move |row| state.set_display_name(row.text().to_string()));
    }
    who.add(&display_name);
    let avatar_row = adw::ButtonRow::builder().title("Change Avatar…").build();
    avatar_row.set_action_name(Some("win.change-avatar"));
    who.add(&avatar_row);
    let logout = adw::ButtonRow::builder().title("Log Out").build();
    logout.add_css_class("destructive-action");
    logout.set_action_name(Some("app.logout"));
    who.add(&logout);
    account.add(&who);
    dialog.add(&account);

    // Voice & Audio
    let voice = adw::PreferencesPage::builder()
        .title("Voice & Audio")
        .icon_name("audio-input-microphone-symbolic")
        .build();
    let input = adw::PreferencesGroup::builder()
        .title("Input")
        .description("Devices apply to your next call; the rest right away.")
        .build();
    let threshold = adw::SpinRow::with_range(-100.0, 0.0, 1.0);
    threshold.set_title("Input sensitivity (dB)");
    threshold
        .set_subtitle("How loud you must be to transmit. Higher ignores more background noise.");
    threshold.set_value(f64::from(options.voice_threshold));
    let gain = adw::SpinRow::with_range(-20.0, 20.0, 1.0);
    gain.set_title("Input volume (dB)");
    gain.set_value(f64::from(options.mic_gain));
    let auto_gain = adw::SwitchRow::builder()
        .title("Automatic gain control")
        .subtitle("Brings a quiet microphone up to a normal level")
        .active(options.auto_gain)
        .build();
    let noise = adw::SwitchRow::builder()
        .title("Noise suppression")
        .subtitle("Removes fans, keyboards and background chatter (RNNoise)")
        .active(options.noise_suppression)
        .build();
    let ptt = adw::SwitchRow::builder()
        .title("Push to talk")
        .subtitle("Only transmit while holding the key below (while Plume is focused)")
        .active(options.push_to_talk)
        .build();
    let ptt_key = adw::EntryRow::builder()
        .title("Push-to-talk key (grave is the ` key)")
        .text(options.push_to_talk_key.as_str())
        .show_apply_button(true)
        .build();

    // Devices: the first entry means "whatever the system uses".
    let device_row = |title: &str, names: Vec<String>, current: &str| {
        let mut items = vec!["System default".to_owned()];
        items.extend(names);
        let model = gtk::StringList::new(&items.iter().map(String::as_str).collect::<Vec<_>>());
        let selected = items.iter().position(|n| n == current).unwrap_or(0);
        adw::ComboRow::builder()
            .title(title)
            .model(&model)
            .selected(selected as u32)
            .build()
    };
    let mic = device_row(
        "Input device",
        plume_voice::audio_io::input_devices(),
        &options.input_device,
    );
    let speakers = device_row(
        "Output device",
        plume_voice::audio_io::output_devices(),
        &options.output_device,
    );
    let devices = adw::PreferencesGroup::builder().title("Devices").build();
    devices.add(&mic);
    devices.add(&speakers);
    voice.add(&devices);

    input.add(&threshold);
    input.add(&gain);
    input.add(&auto_gain);
    input.add(&noise);
    input.add(&ptt);
    input.add(&ptt_key);
    voice.add(&input);
    dialog.add(&voice);

    // Notifications
    let notifications = adw::PreferencesPage::builder()
        .title("Notifications")
        .icon_name("preferences-system-notifications-symbolic")
        .build();
    let notify_group = adw::PreferencesGroup::new();
    let desktop = adw::SwitchRow::builder()
        .title("Desktop notifications")
        .subtitle("For DMs and mentions, unless muted")
        .active(options.notifications)
        .build();
    let silent = adw::SwitchRow::builder()
        .title("Hide that I'm typing")
        .active(options.silent_typing)
        .build();
    notify_group.add(&desktop);
    notify_group.add(&silent);
    notifications.add(&notify_group);
    dialog.add(&notifications);

    // Appearance
    let appearance = adw::PreferencesPage::builder()
        .title("Appearance")
        .icon_name("applications-graphics-symbolic")
        .build();
    let look = adw::PreferencesGroup::new();
    let theme = adw::SwitchRow::builder()
        .title("Discord-style dark theme")
        .subtitle("Off uses your system's style. Takes effect after a restart.")
        .active(options.discord_theme)
        .build();
    let members = adw::SwitchRow::builder()
        .title("Show member list")
        .active(options.member_list)
        .build();
    let hardware = adw::SwitchRow::builder()
        .title("Hardware acceleration")
        .subtitle("Off draws on the CPU, which uses less memory. Takes effect after a restart.")
        .active(options.hardware_acceleration)
        .build();
    look.add(&theme);
    look.add(&members);
    look.add(&hardware);
    appearance.add(&look);
    dialog.add(&appearance);

    // Advanced
    let advanced = adw::PreferencesPage::builder()
        .title("Advanced")
        .icon_name("applications-engineering-symbolic")
        .build();
    let memory = adw::PreferencesGroup::new();
    let tiers = gtk::StringList::new(&["Standard (50–100 MB)", "Full (more cached)"]);
    let profile = adw::ComboRow::builder()
        .title("Memory profile")
        .subtitle("Takes effect after a restart")
        .model(&tiers)
        .selected(u32::from(options.profile.tier == Tier::Full))
        .build();
    memory.add(&profile);
    let report = adw::ButtonRow::builder().title("Copy Debug Report").build();
    report.set_action_name(Some("win.debug-report"));
    memory.add(&report);
    advanced.add(&memory);
    dialog.add(&advanced);

    // Save whenever anything changes.
    let save = {
        let state = state.clone();
        let (threshold, gain, auto_gain) = (threshold.clone(), gain.clone(), auto_gain.clone());
        let (noise, ptt, ptt_key, mic, speakers) = (
            noise.clone(),
            ptt.clone(),
            ptt_key.clone(),
            mic.clone(),
            speakers.clone(),
        );
        let (desktop, silent, theme, members, profile, hardware) = (
            desktop.clone(),
            silent.clone(),
            theme.clone(),
            members.clone(),
            profile.clone(),
            hardware.clone(),
        );
        Rc::new(move || {
            let updated = {
                let mut options = state.options.borrow_mut();
                options.voice_threshold = threshold.value() as f32;
                options.mic_gain = gain.value() as f32;
                options.auto_gain = auto_gain.is_active();
                options.noise_suppression = noise.is_active();
                options.push_to_talk = ptt.is_active();
                let key = ptt_key.text().trim().to_owned();
                if !key.is_empty() {
                    options.push_to_talk_key = key;
                }
                let device = |row: &adw::ComboRow| {
                    let index = row.selected();
                    if index == 0 {
                        return String::new();
                    }
                    row.model()
                        .and_downcast::<gtk::StringList>()
                        .and_then(|list| list.string(index))
                        .map(|s| s.to_string())
                        .unwrap_or_default()
                };
                options.input_device = device(&mic);
                options.output_device = device(&speakers);
                options.notifications = desktop.is_active();
                options.silent_typing = silent.is_active();
                options.discord_theme = theme.is_active();
                options.member_list = members.is_active();
                options.hardware_acceleration = hardware.is_active();
                options.profile = if profile.selected() == 1 {
                    Profile::FULL
                } else {
                    Profile::STANDARD
                };
                options.clone()
            };
            save_options(&state, &updated);
            state.settings_changed();
        })
    };
    for row in [&threshold, &gain] {
        let save = save.clone();
        row.connect_value_notify(move |_| save());
    }
    for row in [
        &auto_gain, &desktop, &silent, &theme, &members, &hardware, &noise, &ptt,
    ] {
        let save = save.clone();
        row.connect_active_notify(move |_| save());
    }
    for row in [&mic, &speakers] {
        let save = save.clone();
        row.connect_selected_notify(move |_| save());
    }
    {
        let save = save.clone();
        ptt_key.connect_apply(move |_| save());
    }
    profile.connect_selected_notify(move |_| save());

    dialog.present(Some(&ui.window));
}

fn save_options(state: &AppState, options: &Options) {
    if options.demo {
        return;
    }
    if let Err(error) = options.save() {
        state.toast(&format!("Couldn't save settings: {error}"));
    }
}
