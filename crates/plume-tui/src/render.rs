use plume_core::Store;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, Paragraph};

use crate::app::{App, Connection, Focus, Prompt};
use crate::layout::{self, Areas};
use crate::sidebar::{self, Entry};
use crate::text;

const DIM: Style = Style::new().fg(Color::DarkGray);
const ACCENT: Color = Color::Rgb(88, 101, 242); // Discord blurple
const SELECTED: Style = Style::new().bg(Color::Rgb(50, 55, 80));
const UNREAD: Style = Style::new().fg(Color::White).add_modifier(Modifier::BOLD);
const MENTION_BADGE: Style = Style::new()
    .fg(Color::White)
    .bg(Color::Red)
    .add_modifier(Modifier::BOLD);

pub fn draw(frame: &mut Frame, app: &mut App) {
    let areas = layout::areas(frame.area(), app.focus == Focus::Sidebar);
    app.messages_height = areas.messages.height;

    if let Some(area) = areas.sidebar {
        draw_sidebar(frame, app, area);
    }
    if areas.messages.width > 0 {
        draw_chat(frame, app, &areas);
    }
    draw_status(frame, app, areas.status);
    if app.switcher.is_some() {
        draw_switcher(frame, app);
    }
}

fn draw_sidebar(frame: &mut Frame, app: &mut App, area: Rect) {
    let store = &app.store;
    let width = usize::from(area.width.saturating_sub(2));
    let active = store.active_channel();

    let items: Vec<ListItem> = app
        .sidebar
        .entries()
        .iter()
        .map(|&entry| ListItem::new(sidebar_row(store, app, entry, active, width)))
        .collect();

    let focused = app.focus == Focus::Sidebar;
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(if focused {
            Style::new().fg(ACCENT)
        } else {
            DIM
        })
        .title(" Plume ");
    let list = List::new(items).block(block).highlight_style(if focused {
        SELECTED
    } else {
        Style::new()
    });

    app.sidebar_state.select(Some(app.sidebar.selected_index()));
    frame.render_stateful_widget(list, area, &mut app.sidebar_state);
}

fn sidebar_row(
    store: &Store,
    app: &App,
    entry: Entry,
    active: Option<plume_core::Snowflake>,
    width: usize,
) -> Line<'static> {
    let (label, unread, base) = match entry {
        Entry::DmHeader => {
            let arrow = if app.sidebar.dms_collapsed() {
                "▸"
            } else {
                "▾"
            };
            (format!("{arrow} DIRECT MESSAGES"), Default::default(), DIM)
        }
        Entry::ServerHeader => ("  SERVERS".to_owned(), Default::default(), DIM),
        Entry::Dm(id) => {
            let name = store
                .channel(id)
                .map_or_else(String::new, |c| sidebar::dm_name(store, c));
            let base = if store.is_muted(id) {
                DIM
            } else {
                Style::new()
            };
            (format!("  @ {name}"), store.unread(id), base)
        }
        Entry::Guild(id) => {
            let arrow = if app.sidebar.is_expanded(id) {
                "▾"
            } else {
                "▸"
            };
            let name = store.guild(id).map_or("?", |g| g.name.as_str());
            let base = if store.is_guild_muted(id) {
                DIM
            } else {
                Style::new()
            };
            (format!("{arrow} {name}"), store.guild_unread(id), base)
        }
        Entry::Category(id) => {
            let name = store
                .channel(id)
                .and_then(|c| c.name.as_deref())
                .unwrap_or("");
            (
                format!("    {}", name.to_uppercase()),
                Default::default(),
                DIM,
            )
        }
        Entry::Channel(id) => {
            let name = store
                .channel(id)
                .and_then(|c| c.name.as_deref())
                .unwrap_or("?");
            let base = if store.is_muted(id) {
                DIM
            } else {
                Style::new().fg(Color::Gray)
            };
            (format!("    # {name}"), store.unread(id), base)
        }
    };

    let is_active = matches!(entry, Entry::Dm(id) | Entry::Channel(id) if Some(id) == active);
    let style = if is_active {
        Style::new().fg(Color::White).add_modifier(Modifier::BOLD)
    } else if unread.unread {
        UNREAD
    } else {
        base
    };

    let badge = (unread.mentions > 0).then(|| format!(" {} ", unread.mentions));
    let badge_width = badge.as_ref().map_or(0, |b| b.len() + 1);
    let mut spans = vec![Span::styled(
        text::truncate(&label, width.saturating_sub(badge_width)),
        style,
    )];
    if let Some(badge) = badge {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(badge, MENTION_BADGE));
    }
    Line::from(spans)
}

fn draw_chat(frame: &mut Frame, app: &mut App, areas: &Areas) {
    let store = &app.store;
    let Some(channel_id) = store.active_channel() else {
        let hint = Paragraph::new(vec![
            Line::raw(""),
            Line::styled("  Pick a channel on the left (↑/↓ and Enter),", DIM),
            Line::styled("  or press Ctrl+K to jump to one by name.", DIM),
        ]);
        frame.render_widget(hint, areas.messages);
        draw_input(frame, app, areas.input);
        return;
    };

    // Title: "#general · Server" or "@Alice".
    let mut title = vec![Span::styled(
        format!(" {}", sidebar::channel_label(store, channel_id)),
        Style::new().add_modifier(Modifier::BOLD),
    )];
    if let Some(guild) = store
        .channel(channel_id)
        .and_then(|c| c.guild_id)
        .and_then(|id| store.guild(id))
    {
        title.push(Span::styled(format!(" · {}", guild.name), DIM));
    }
    if app.loading_history || app.loading_older {
        title.push(Span::styled("  loading…", DIM));
    }
    if app.focus == Focus::Messages {
        title.push(Span::styled("  selecting", Style::new().fg(ACCENT)));
    }
    frame.render_widget(Paragraph::new(Line::from(title)), areas.title);

    // Messages, anchored to the bottom and scrolled up by `app.scroll` lines.
    let view = text::channel_lines(store, channel_id, areas.messages.width, app.selected);
    let lines = view.lines;
    let height = usize::from(areas.messages.height);
    let max_scroll = lines.len().saturating_sub(height);

    // After the selection moves, scroll just enough to show the whole message.
    if std::mem::take(&mut app.scroll_to_selected)
        && let Some((_, range)) = view.ranges.iter().find(|(id, _)| Some(*id) == app.selected)
    {
        let end = lines.len() - app.scroll.min(max_scroll);
        let start = end.saturating_sub(height);
        if range.end > end {
            app.scroll = lines.len() - range.end;
        } else if range.start < start {
            app.scroll = lines.len().saturating_sub(range.start + height);
        }
    }
    app.scroll = app.scroll.min(max_scroll);
    let scroll = app.scroll;
    let end = lines.len() - scroll;
    let start = end.saturating_sub(height);
    // Scrolled all the way up: time to fetch older messages.
    if start == 0 && scroll > 0 {
        app.at_top = true;
    }
    let visible: Vec<Line> = lines[start..end].to_vec();
    // Pad from the top so a short history still sits at the bottom.
    let padding = height.saturating_sub(visible.len()) as u16;
    let area = Rect {
        y: areas.messages.y + padding,
        height: areas.messages.height - padding,
        ..areas.messages
    };
    frame.render_widget(Paragraph::new(visible), area);

    // One status row above the input: reply/edit first, then "newer below", then typing.
    if let Some(compose) = app.compose_text() {
        frame.render_widget(
            Paragraph::new(Line::styled(format!(" {compose}"), Style::new().fg(ACCENT))),
            areas.typing,
        );
    } else if scroll > 0 && app.focus != Focus::Messages {
        let note = Paragraph::new(Line::styled(
            " ↓ newer messages below (End) ",
            Style::new().fg(Color::Black).bg(Color::Yellow),
        ))
        .right_aligned();
        frame.render_widget(
            note,
            Rect {
                height: 1,
                ..areas.typing
            },
        );
    } else if let Some(typing) = app.typing_text() {
        frame.render_widget(
            Paragraph::new(Line::styled(format!(" {typing}"), DIM)),
            areas.typing,
        );
    }

    draw_input(frame, app, areas.input);
}

fn draw_input(frame: &mut Frame, app: &App, area: Rect) {
    if let Some(Prompt::React { input, .. }) = &app.prompt {
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(Color::Yellow))
            .title(" React: an emoji, :server_emoji: or a word like thumbsup · Enter · Esc ");
        let inner = block.inner(area);
        frame.render_widget(
            Paragraph::new(Line::raw(input.text().to_owned())).block(block),
            area,
        );
        frame.set_cursor_position((inner.x + input.cursor_column() as u16, inner.y));
        return;
    }

    let focused = app.focus == Focus::Input;
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(if focused {
            Style::new().fg(ACCENT)
        } else {
            DIM
        });
    let inner = block.inner(area);

    let width = usize::from(inner.width.max(1));
    let cursor = app.input.cursor_column();
    // Scroll sideways so the cursor stays visible in long messages.
    let offset = cursor.saturating_sub(width.saturating_sub(1));

    let content = if app.input.is_empty() {
        let placeholder = match app.store.active_channel() {
            Some(id) if !app.store.can_send(id) => {
                "You can't send messages in this channel".to_owned()
            }
            Some(id) => format!("Message {}", sidebar::channel_label(&app.store, id)),
            None => "Open a channel to start chatting".to_owned(),
        };
        Line::styled(placeholder, DIM)
    } else {
        Line::raw(skip_columns(app.input.text(), offset))
    };
    frame.render_widget(Paragraph::new(content).block(block), area);

    if focused && app.switcher.is_none() && app.prompt.is_none() {
        let x = inner.x + (cursor - offset) as u16;
        frame.set_cursor_position((x, inner.y));
    }
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    let (state, color) = match app.connection {
        Connection::Connecting => ("connecting…", Color::Yellow),
        Connection::Online => ("online", Color::Green),
    };
    let mut left = vec![
        Span::styled(" ● ", Style::new().fg(color)),
        Span::raw(state),
    ];
    if let Some(me) = app.store.me() {
        left.push(Span::styled(format!(" as {}", me.display_name()), DIM));
    }

    let middle = if let Some(Prompt::ConfirmDelete { .. }) = &app.prompt {
        Span::styled(
            "  Delete this message? y = yes, any other key = no",
            Style::new().fg(Color::Yellow),
        )
    } else if let Some((error, _)) = &app.error {
        Span::styled(format!("  {error}"), Style::new().fg(Color::Red))
    } else {
        Span::styled(
            match app.focus {
                Focus::Sidebar => "  ↑↓ move · Enter open · Tab next · Ctrl+K jump · Ctrl+C quit",
                Focus::Messages => "  ↑↓ select · r reply · e edit · d delete · + react · Esc back",
                Focus::Input => "  Enter send · ↑ select messages · Esc sidebar · PgUp/PgDn scroll",
            },
            DIM,
        )
    };
    left.push(middle);
    frame.render_widget(Paragraph::new(Line::from(left)), area);

    let memory = match app.memory_kb {
        Some(kb) => format!("{:.1} MB · {:?} ", kb as f64 / 1024.0, app.options_tier),
        None => format!("{:?} ", app.options_tier),
    };
    frame.render_widget(
        Paragraph::new(Line::styled(memory, DIM)).right_aligned(),
        area,
    );
}

fn draw_switcher(frame: &mut Frame, app: &App) {
    let Some(switcher) = &app.switcher else {
        return;
    };
    let area = layout::centered(frame.area(), 60, 60);
    frame.render_widget(Clear, area);

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ACCENT))
        .title(" Jump to… (Esc to close) ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 2 {
        return;
    }

    let query_area = Rect { height: 1, ..inner };
    let query = if switcher.input.is_empty() {
        Line::styled("type a channel, server or person", DIM)
    } else {
        Line::raw(switcher.input.text().to_owned())
    };
    frame.render_widget(Paragraph::new(query), query_area);
    frame.set_cursor_position((inner.x + switcher.input.cursor_column() as u16, inner.y));

    let list_area = Rect {
        y: inner.y + 2,
        height: inner.height.saturating_sub(2),
        ..inner
    };
    let width = usize::from(list_area.width);
    let rows = usize::from(list_area.height);
    let first = switcher.selected.saturating_sub(rows.saturating_sub(1));
    let lines: Vec<Line> = switcher
        .matches
        .iter()
        .enumerate()
        .skip(first)
        .take(rows)
        .map(|(index, (_, label))| {
            let style = if index == switcher.selected {
                SELECTED
            } else {
                Style::new()
            };
            Line::styled(text::truncate(label, width), style)
        })
        .collect();
    if lines.is_empty() {
        frame.render_widget(Paragraph::new(Line::styled("no matches", DIM)), list_area);
    } else {
        frame.render_widget(Paragraph::new(lines), list_area);
    }
}

/// `text` without its first `columns` terminal columns.
fn skip_columns(text: &str, columns: usize) -> String {
    use unicode_width::UnicodeWidthChar;
    let mut skipped = 0;
    text.chars()
        .skip_while(|c| {
            let skip = skipped < columns;
            skipped += c.width().unwrap_or(0);
            skip
        })
        .collect()
}
