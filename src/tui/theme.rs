//! Color palette and styling helpers. One place to tune the look.
//! Palette is Catppuccin-Mocha-flavoured, on the terminal's own background.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Borders, Padding};

// ── palette ─────────────────────────────────────────────────────

pub const TEXT: Color = Color::Rgb(0xcd, 0xd6, 0xf4);
pub const SUBTEXT: Color = Color::Rgb(0xa6, 0xad, 0xc8);
pub const MUTED: Color = Color::Rgb(0x6c, 0x70, 0x86);
pub const SURFACE: Color = Color::Rgb(0x31, 0x32, 0x44);
pub const SURFACE_LO: Color = Color::Rgb(0x24, 0x25, 0x34);

pub const ACCENT: Color = Color::Indexed(1);
pub const MAUVE: Color = Color::Indexed(5);
pub const GREEN: Color = Color::Rgb(0xa6, 0xe3, 0xa1);
pub const RED: Color = Color::Rgb(0xf3, 0x8b, 0xa8);
pub const YELLOW: Color = Color::Rgb(0xf9, 0xe2, 0xaf);
pub const PEACH: Color = Color::Rgb(0xfa, 0xb3, 0x87);
pub const BLUE: Color = Color::Rgb(0x89, 0xb4, 0xfa);

// ── styles ──────────────────────────────────────────────────────

pub fn text() -> Style {
    Style::default().fg(TEXT)
}

pub fn dim() -> Style {
    Style::default().fg(MUTED)
}

pub fn subtle() -> Style {
    Style::default().fg(SUBTEXT)
}

pub fn label() -> Style {
    Style::default().fg(ACCENT)
}

pub fn title() -> Style {
    Style::default().fg(MAUVE).add_modifier(Modifier::BOLD)
}

pub fn table_header() -> Style {
    Style::default()
        .fg(MAUVE)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
}

/// Whole-card selection: a background tint over every line of the row,
/// leaving each span's own foreground color intact.
pub fn selected_row() -> Style {
    Style::default().bg(SURFACE)
}

// ── blocks ──────────────────────────────────────────────────────

/// Bare rounded panel with no title.
pub fn panel_bare() -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(SURFACE))
        .padding(Padding::horizontal(1))
}

/// Standard rounded panel with a styled title.
pub fn panel(title: &str) -> Block<'_> {
    panel_bare().title(Span::styled(format!(" {title} "), self::title()))
}

// ── badges ──────────────────────────────────────────────────────

/// Colored status dot + word, e.g. "● enabled".
pub fn badge<'a>(on: bool, on_word: &'a str, off_word: &'a str) -> Span<'a> {
    if on {
        Span::styled(format!("● {on_word}"), Style::default().fg(GREEN))
    } else {
        Span::styled(format!("○ {off_word}"), Style::default().fg(MUTED))
    }
}

/// Key-hint chip for the footer: highlighted key + dim label.
pub fn chip<'a>(key: &'a str, label: &'a str) -> Vec<Span<'a>> {
    vec![
        Span::styled(
            format!(" {key} "),
            Style::default()
                .fg(SURFACE_LO)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" {label}  "), dim()),
    ]
}

/// Color for a device class string (nvme/ssd/hdd).
pub fn device_class_color(class: &str) -> Color {
    match class {
        "nvme" => MAUVE,
        "ssd" => BLUE,
        "hdd" => PEACH,
        _ => SUBTEXT,
    }
}
