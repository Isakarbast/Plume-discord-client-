//! First start: log in by scanning a QR code with the Discord app, or by
//! pasting a token. The token goes straight into the system keyring.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use plume_auth::{AuthError, QrStatus};
use plume_core::Token;
use qrcode::{Color, QrCode};

use crate::state::bridge;

const QR_SCALE: usize = 6;
const QR_QUIET_ZONE: usize = 4;

/// Shows the login window; calls `on_login` once a token is verified and saved.
pub fn show(app: &adw::Application, on_login: impl Fn(Token) + 'static) {
    let on_login: Rc<dyn Fn(Token)> = Rc::new(on_login);

    let qr = gtk::Picture::new();
    qr.set_size_request(240, 240);
    qr.set_halign(gtk::Align::Center);
    qr.add_css_class("qr-code");
    let status = gtk::Label::builder()
        .label("Connecting to Discord…")
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build();

    let token_row = adw::PasswordEntryRow::builder().title("Token").build();
    let token_list = gtk::ListBox::new();
    token_list.add_css_class("boxed-list");
    token_list.append(&token_row);
    let token_button = gtk::Button::with_label("Log In with Token");
    token_button.add_css_class("pill");
    token_button.set_halign(gtk::Align::Center);
    let token_box = gtk::Box::new(gtk::Orientation::Vertical, 12);
    token_box.append(&token_list);
    token_box.append(&token_button);
    let expander = gtk::Expander::builder()
        .label("Use a token instead")
        .child(&token_box)
        .build();

    let warning = gtk::Label::builder()
        .label("Third-party clients break Discord's Terms of Service. Use at your own risk.")
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build();
    warning.add_css_class("caption");
    warning.add_css_class("dim-label");

    let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
    content.set_width_request(360);
    content.set_halign(gtk::Align::Center);
    content.append(&qr);
    content.append(&status);
    content.append(&expander);
    content.append(&warning);

    let page = adw::StatusPage::builder()
        .title("Log in to Discord")
        .description(
            "Open the Discord app on your phone, go to Settings → Scan QR Code, and scan this.",
        )
        .child(&content)
        .build();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&page));
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Plume")
        .default_width(520)
        .default_height(760)
        .content(&view)
        .build();

    // Saving and switching windows, shared by both login methods.
    let finish = {
        let (window, status, on_login) = (window.clone(), status.clone(), on_login.clone());
        Rc::new(move |token: Token| {
            status.set_text("Checking the token…");
            let (window, status, on_login) = (window.clone(), status.clone(), on_login.clone());
            let saved = token.clone();
            bridge::spawn(
                async move {
                    let username = plume_auth::verify_token(&saved).await?;
                    let to_save = saved.clone();
                    tokio::task::spawn_blocking(move || plume_auth::keyring::save_token(&to_save))
                        .await
                        .map_err(|e| AuthError::Protocol(e.to_string()))??;
                    Ok::<_, AuthError>(username)
                },
                move |result| match result {
                    Ok(_) => {
                        window.close();
                        on_login(token);
                    }
                    Err(error) => status.set_text(&error.to_string()),
                },
            );
        })
    };

    let (finish_token, row) = (finish.clone(), token_row.clone());
    token_button.connect_clicked(move |_| {
        let text = row.text();
        if !text.trim().is_empty() {
            finish_token(Token::new(text.as_str()));
        }
    });

    start_qr(&qr, &status, finish);
    window.present();
}

/// Runs the QR login in the background and keeps the picture and status text updated.
fn start_qr(qr: &gtk::Picture, status: &gtk::Label, finish: Rc<dyn Fn(Token)>) {
    let (updates_tx, updates) = async_channel::unbounded();
    let (qr, status_label) = (qr.downgrade(), status.downgrade());
    glib::spawn_future_local(async move {
        while let Ok(update) = updates.recv().await {
            let (Some(qr), Some(status)) = (qr.upgrade(), status_label.upgrade()) else {
                break;
            };
            match update {
                QrStatus::ShowQr { url } => {
                    if let Some(texture) = qr_texture(&url) {
                        qr.set_paintable(Some(&texture));
                    }
                    status.set_text("Waiting for you to scan…");
                }
                QrStatus::Scanned { username } => {
                    status.set_text(&format!(
                        "Scanned by {username}. Confirm the login on your phone…"
                    ));
                }
            }
        }
    });

    let status = status.downgrade();
    bridge::spawn(
        async move {
            plume_auth::qr_login(move |update| {
                let _ = updates_tx.try_send(update);
            })
            .await
        },
        move |result| {
            let Some(status) = status.upgrade() else {
                return;
            };
            match result {
                Ok(token) => finish(token),
                Err(AuthError::CaptchaRequired) => status.set_text(
                    "Discord wants a captcha for QR logins right now, and Plume doesn't solve those. \
                     Use a token instead (see the README).",
                ),
                Err(error) => status.set_text(&format!("QR login stopped: {error}. Restart Plume to try again.")),
            }
        },
    );
}

/// Draws a QR code into a texture: black modules on white, with a quiet zone.
fn qr_texture(url: &str) -> Option<gdk::Texture> {
    let code = QrCode::new(url).ok()?;
    let modules = code.width();
    let colors = code.to_colors();
    let side = (modules + 2 * QR_QUIET_ZONE) * QR_SCALE;
    let mut rgba = vec![255u8; side * side * 4];
    for (index, color) in colors.iter().enumerate() {
        if *color != Color::Dark {
            continue;
        }
        let (mx, my) = (
            index % modules + QR_QUIET_ZONE,
            index / modules + QR_QUIET_ZONE,
        );
        for dy in 0..QR_SCALE {
            for dx in 0..QR_SCALE {
                let pixel = ((my * QR_SCALE + dy) * side + mx * QR_SCALE + dx) * 4;
                rgba[pixel..pixel + 3].fill(0);
            }
        }
    }
    let bytes = glib::Bytes::from_owned(rgba);
    Some(
        gdk::MemoryTexture::new(
            side as i32,
            side as i32,
            gdk::MemoryFormat::R8g8b8a8,
            &bytes,
            side * 4,
        )
        .upcast(),
    )
}
