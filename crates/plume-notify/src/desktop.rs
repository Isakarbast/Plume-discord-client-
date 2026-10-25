//! Shows a notification through the desktop's notification service
//! (D-Bus `org.freedesktop.Notifications` on Linux).

use crate::unread::Notification;

/// Shows a notification. This blocks briefly while talking to D-Bus, so from
/// async code call it with `spawn_blocking`.
pub fn show(notification: &Notification) -> Result<(), String> {
    notify_rust::Notification::new()
        .appname("Plume")
        .summary(&notification.title)
        .body(&escape_markup(&notification.body))
        .icon("mail-message-new")
        .show()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Many notification servers render a little HTML in the body, so a message
/// containing `<b>` would turn bold. Escape it so it shows as typed.
fn escape_markup(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_html() {
        assert_eq!(
            escape_markup("<b>hi</b> & bye"),
            "&lt;b&gt;hi&lt;/b&gt; &amp; bye"
        );
    }
}
