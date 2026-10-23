use ratatui::layout::{Constraint, Layout, Rect};

/// Where each part of the screen goes.
///
/// ```text
/// ┌─sidebar─┬─title──────────────┐
/// │         │ messages           │
/// │         │                    │
/// │         │ typing indicator   │
/// │         ├─input──────────────┤
/// │         │                    │
/// ├─status──┴────────────────────┤
/// ```
pub struct Areas {
    pub sidebar: Option<Rect>,
    pub title: Rect,
    pub messages: Rect,
    pub typing: Rect,
    pub input: Rect,
    pub status: Rect,
}

/// Below this width there's no room for both, so the sidebar takes the whole
/// screen while it has focus and hides otherwise.
const NARROW: u16 = 60;

pub fn areas(screen: Rect, sidebar_focused: bool) -> Areas {
    let [main, status] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(screen);

    let (sidebar, chat) = if main.width < NARROW {
        if sidebar_focused {
            (Some(main), Rect::default())
        } else {
            (None, main)
        }
    } else {
        let width = (main.width / 3).clamp(20, 32);
        let [sidebar, chat] = Layout::horizontal([Constraint::Length(width), Constraint::Fill(1)]).areas(main);
        (Some(sidebar), chat)
    };

    let [title, messages, typing, input] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(3),
    ])
    .areas(chat);

    Areas {
        sidebar,
        title,
        messages,
        typing,
        input,
        status,
    }
}

/// A box in the middle of `area`, for popups.
pub fn centered(area: Rect, width_percent: u16, height_percent: u16) -> Rect {
    let width = (area.width * width_percent / 100).max(30).min(area.width);
    let height = (area.height * height_percent / 100).max(8).min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}
