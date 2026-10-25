//! Discord markdown → Pango markup (what GTK labels render), with code
//! blocks split out so they can get their own boxed widget.

use chrono::{Local, TimeZone};
use gtk::glib::markup_escape_text;
use plume_core::Store;
use plume_markdown::{Piece, Style};

pub enum Block {
    /// Pango markup for a label.
    Text(String),
    Code {
        language: Option<String>,
        code: String,
    },
}

const MENTION: &str = "foreground=\"#8c9eff\" background=\"#5865f233\" weight=\"bold\"";
const MENTION_ME: &str = "foreground=\"#ffd27f\" background=\"#faa61a40\" weight=\"bold\"";
const CODE: &str = "font_family=\"monospace\" background=\"#8080802e\"";
const DIM: &str = "alpha=\"60%\"";
const SPOILER: &str = "foreground=\"#2b2d31\" background=\"#2b2d31\"";

pub fn message_blocks(store: &Store, content: &str) -> Vec<Block> {
    let me = store.me().map(|u| u.id);
    let nodes = plume_markdown::parse(content);
    let spans = plume_markdown::flatten(&nodes);

    let mut blocks = Vec::new();
    let mut text = String::new();
    let mut line_start = true;

    for span in spans {
        if span.style.quote && line_start && !matches!(span.piece, Piece::LineBreak) {
            text.push_str(&format!("<span {DIM}>▎ </span>"));
        }
        line_start = false;
        match span.piece {
            Piece::Text(t) => text.push_str(&styled(&markup_escape_text(t), span.style)),
            Piece::Code(code) => {
                text.push_str(&format!("<span {CODE}>{}</span>", markup_escape_text(code)))
            }
            Piece::CodeBlock { language, code } => {
                let finished = std::mem::take(&mut text);
                if !finished.trim().is_empty() {
                    blocks.push(Block::Text(finished.trim_end_matches('\n').to_owned()));
                }
                blocks.push(Block::Code {
                    language: language.map(str::to_owned),
                    code: code.to_owned(),
                });
                line_start = true;
            }
            Piece::LineBreak => {
                text.push('\n');
                line_start = true;
            }
            Piece::UserMention(id) => {
                let name = store.user(id).map_or("unknown-user", |u| u.display_name());
                let style = if Some(id) == me { MENTION_ME } else { MENTION };
                text.push_str(&format!(
                    "<span {style}>@{}</span>",
                    markup_escape_text(name)
                ));
            }
            Piece::ChannelMention(id) => {
                let name = store
                    .channel(id)
                    .and_then(|c| c.name.as_deref())
                    .unwrap_or("unknown");
                text.push_str(&format!(
                    "<span {MENTION}>#{}</span>",
                    markup_escape_text(name)
                ));
            }
            Piece::RoleMention(_) => text.push_str(&format!("<span {MENTION}>@role</span>")),
            Piece::Everyone => text.push_str(&format!("<span {MENTION_ME}>@everyone</span>")),
            Piece::Here => text.push_str(&format!("<span {MENTION_ME}>@here</span>")),
            Piece::CustomEmoji { name, .. } => {
                text.push_str(&format!(
                    "<span foreground=\"#d08cff\">:{}:</span>",
                    markup_escape_text(name)
                ));
            }
            Piece::Timestamp { unix, style } => {
                text.push_str(&format!(
                    "<span {CODE}>{}</span>",
                    markup_escape_text(&format_timestamp(unix, style))
                ));
            }
            Piece::Link(url) => {
                let url = markup_escape_text(url);
                text.push_str(&format!("<a href=\"{url}\">{url}</a>"));
            }
        }
    }
    if !text.trim().is_empty() {
        blocks.push(Block::Text(text.trim_end_matches('\n').to_owned()));
    }
    blocks
}

fn styled(escaped: &str, style: Style) -> String {
    let mut out = escaped.to_owned();
    let mut wrap = |open: &str, close: &str| out = format!("{open}{out}{close}");
    if style.bold {
        wrap("<b>", "</b>");
    }
    if style.italic {
        wrap("<i>", "</i>");
    }
    if style.underline {
        wrap("<u>", "</u>");
    }
    if style.strikethrough {
        wrap("<s>", "</s>");
    }
    if style.heading {
        wrap("<span size=\"larger\">", "</span>");
    }
    if style.subtext {
        wrap(&format!("<span size=\"small\" {DIM}>"), "</span>");
    }
    if style.spoiler {
        wrap(&format!("<span {SPOILER}>"), "</span>");
    }
    out
}

/// Discord's `<t:…:style>` formats.
pub fn format_timestamp(unix: i64, style: Option<char>) -> String {
    let Some(time) = Local.timestamp_opt(unix, 0).single() else {
        return "(invalid date)".to_owned();
    };
    match style {
        Some('t') => time.format("%H:%M").to_string(),
        Some('T') => time.format("%H:%M:%S").to_string(),
        Some('d') => time.format("%d/%m/%Y").to_string(),
        Some('D') => time.format("%-d %B %Y").to_string(),
        Some('F') => time.format("%A, %-d %B %Y %H:%M").to_string(),
        Some('R') => relative(unix - Local::now().timestamp()),
        _ => time.format("%-d %B %Y %H:%M").to_string(),
    }
}

fn relative(seconds: i64) -> String {
    let amount = seconds.unsigned_abs();
    let (n, unit) = match amount {
        0..60 => (amount, "second"),
        60..3600 => (amount / 60, "minute"),
        3600..86_400 => (amount / 3600, "hour"),
        86_400..2_592_000 => (amount / 86_400, "day"),
        2_592_000..31_536_000 => (amount / 2_592_000, "month"),
        _ => (amount / 31_536_000, "year"),
    };
    let plural = if n == 1 { "" } else { "s" };
    if seconds >= 0 {
        format!("in {n} {unit}{plural}")
    } else {
        format!("{n} {unit}{plural} ago")
    }
}

/// Plain text for a reply preview: the first line, without markup.
pub fn snippet(content: &str, max: usize) -> String {
    let line = content.lines().next().unwrap_or("");
    if line.chars().count() <= max {
        return line.to_owned();
    }
    let mut short: String = line.chars().take(max.saturating_sub(1)).collect();
    short.push('…');
    short
}

#[cfg(test)]
mod tests {
    use super::*;
    use plume_core::Profile;

    fn text_of(blocks: &[Block]) -> Vec<String> {
        blocks
            .iter()
            .map(|b| match b {
                Block::Text(t) => t.clone(),
                Block::Code { code, .. } => format!("CODE[{code}]"),
            })
            .collect()
    }

    #[test]
    fn escapes_and_styles() {
        let store = Store::new(Profile::STANDARD);
        let blocks = message_blocks(&store, "**a < b** & <script>");
        assert_eq!(text_of(&blocks), ["<b>a &lt; b</b> &amp; &lt;script&gt;"]);
    }

    #[test]
    fn code_blocks_become_their_own_block() {
        let store = Store::new(Profile::STANDARD);
        let blocks = message_blocks(&store, "look:\n```rust\nfn x() {}\n```\nnice");
        assert_eq!(text_of(&blocks), ["look:", "CODE[fn x() {}]", "nice"]);
    }

    #[test]
    fn links_are_clickable_and_escaped() {
        let store = Store::new(Profile::STANDARD);
        let blocks = message_blocks(&store, "https://x.y/?a=1&b=2");
        assert_eq!(
            text_of(&blocks),
            ["<a href=\"https://x.y/?a=1&amp;b=2\">https://x.y/?a=1&amp;b=2</a>"]
        );
    }

    #[test]
    fn snippets_are_cut() {
        assert_eq!(snippet("hello world\nsecond", 6), "hello…");
    }
}
