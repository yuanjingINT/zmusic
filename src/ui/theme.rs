use ratatui::style::{Color, Modifier, Style};

pub fn primary() -> Style {
    Style::new().fg(Color::Cyan)
}

pub fn title() -> Style {
    Style::new().fg(Color::LightBlue).add_modifier(Modifier::BOLD)
}

pub fn highlight() -> Style {
    Style::new().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
}

pub fn dim() -> Style {
    Style::new().fg(Color::DarkGray)
}

pub fn good() -> Style {
    Style::new().fg(Color::Green)
}

pub fn warn() -> Style {
    Style::new().fg(Color::Yellow)
}

pub fn err() -> Style {
    Style::new().fg(Color::Red)
}

pub fn lyric_current() -> Style {
    Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
}

pub fn lyric_normal() -> Style {
    Style::new().fg(Color::Gray)
}

pub fn song_title() -> Style {
    Style::new()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD)
}
