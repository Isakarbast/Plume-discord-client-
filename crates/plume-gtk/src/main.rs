//! Plume's 💬 Standard / 🧩 Full desktop client (GTK4 + libadwaita).
//!
//!     plume              start (log in on first run)
//!     plume --demo       try it with made-up data, no network
//!     plume --logout     forget the token and the cache

mod app;
mod debug_report;
mod demo;
mod markup;
mod memory;
mod settings;
mod state;
mod views;
mod widgets;
mod window;

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use plume_core::Token;

use crate::app::AppState;
use crate::settings::Options;

const APP_ID: &str = "dev.plume.Plume";

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{}", settings::HELP);
        return glib::ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--logout") {
        logout();
        println!("Logged out: removed the saved token and the cache.");
        return glib::ExitCode::SUCCESS;
    }
    let options = match Options::load(&args) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("error: {error}");
            return glib::ExitCode::FAILURE;
        }
    };
    memory::tune();
    if !options.hardware_acceleration && std::env::var_os("GSK_RENDERER").is_none() {
        // SAFETY: still single-threaded; GTK reads this when it starts.
        unsafe { std::env::set_var("GSK_RENDERER", "cairo") };
    }
    init_logging();

    // The demo runs on its own, instead of handing off to a running Plume.
    let flags = if options.demo {
        gio::ApplicationFlags::NON_UNIQUE
    } else {
        gio::ApplicationFlags::empty()
    };
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(flags)
        .build();
    let discord_theme = options.discord_theme;
    app.connect_startup(move |_| load_css(discord_theme));
    app.connect_activate(move |app| activate(app, options.clone()));
    // Our flags were handled above; don't let GTK try to parse them.
    app.run_with_args(&["plume"])
}

fn activate(app: &adw::Application, options: Options) {
    if let Some(window) = app.active_window() {
        window.present();
        return;
    }
    if options.demo {
        start_main(app, options, None);
        return;
    }
    let token =
        plume_auth::token_from_env().or_else(|| plume_auth::keyring::load_token().ok().flatten());
    match token {
        Some(token) => start_main(app, options, Some(token)),
        None => {
            let app_for_login = app.clone();
            views::login::show(app, move |token| {
                start_main(&app_for_login, options.clone(), Some(token))
            });
        }
    }
}

fn start_main(app: &adw::Application, options: Options, token: Option<Token>) {
    let state = AppState::new(app, options, token.as_ref());
    let window = window::build(&state);
    // Widgets only hold weak references to the state; the window owns it.
    let owner = state.clone();
    window.connect_destroy(move |_| {
        let _ = &owner;
    });
    install_app_actions(app, &state);
    window.present();
    state.start(token);
}

fn install_app_actions(app: &adw::Application, state: &Rc<AppState>) {
    // Clicking a notification opens its channel.
    let open = gio::SimpleAction::new("open-channel", Some(glib::VariantTy::STRING));
    let weak = Rc::downgrade(state);
    open.connect_activate(move |_, parameter| {
        let Some(state) = weak.upgrade() else { return };
        if let Some(id) = parameter
            .and_then(|p| p.get::<String>())
            .and_then(|s| s.parse().ok())
        {
            if let Some(ui) = state.ui.get() {
                ui.window.present();
            }
            state.open_channel(id);
        }
    });
    app.add_action(&open);

    // Asks first, like Discord. Never available in the demo, which must not
    // touch the real saved login.
    let logout_action = gio::SimpleAction::new("logout", None);
    logout_action.set_enabled(!state.options.borrow().demo);
    let app_for_logout = app.clone();
    let weak = Rc::downgrade(state);
    logout_action.connect_activate(move |_, _| {
        let Some(state) = weak.upgrade() else { return };
        if state.options.borrow().demo {
            return;
        }
        let Some(ui) = state.ui.get() else { return };
        let dialog = adw::AlertDialog::new(
            Some("Log Out"),
            Some("Are you sure you want to log out? You'll need to log in again next time."),
        );
        dialog.add_responses(&[("cancel", "Cancel"), ("logout", "Log Out")]);
        dialog.set_response_appearance("logout", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let app = app_for_logout.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "logout" {
                logout();
                app.quit();
            }
        });
        dialog.present(Some(&ui.window));
    });
    app.add_action(&logout_action);

    let about = gio::SimpleAction::new("about", None);
    let weak = Rc::downgrade(state);
    about.connect_activate(move |_, _| {
        let Some(state) = weak.upgrade() else { return };
        let dialog = adw::AboutDialog::builder()
            .application_name("Plume")
            .application_icon(APP_ID)
            .version(env!("CARGO_PKG_VERSION"))
            .comments("A lightweight native Discord client. Not affiliated with Discord.")
            .license_type(gtk::License::Gpl30)
            .build();
        if let Some(ui) = state.ui.get() {
            dialog.present(Some(&ui.window));
        }
    });
    app.add_action(&about);
}

/// Forgets the token and deletes the cache (it holds your messages).
fn logout() {
    if let Err(error) = plume_auth::keyring::delete_token() {
        eprintln!("couldn't remove the token from the keyring: {error}");
    }
    if let Some(dir) = settings::cache_dir() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Built-in styling, then the user's own CSS on top.
fn load_css(discord_theme: bool) {
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    let mut css = include_str!("../resources/css/style.css").to_owned();
    if discord_theme {
        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
        css.push_str(include_str!("../resources/css/discord.css"));
    }
    let builtin = gtk::CssProvider::new();
    builtin.load_from_string(&css);
    gtk::style_context_add_provider_for_display(
        &display,
        &builtin,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    if let Some(path) = settings::config_dir().map(|dir| dir.join("style.css"))
        && path.exists()
    {
        let user = gtk::CssProvider::new();
        user.load_from_path(&path);
        gtk::style_context_add_provider_for_display(
            &display,
            &user,
            gtk::STYLE_PROVIDER_PRIORITY_USER,
        );
    }
}

fn init_logging() {
    let Some(dir) = settings::state_dir() else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let Ok(file) = std::fs::File::create(dir.join("plume-gtk.log")) else {
        return;
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        // Voice details by default, so debug reports are useful.
        .unwrap_or_else(|_| "info,plume_voice=debug".into());
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(std::sync::Mutex::new(file))
        .init();
}
