//! Style closures bridging the design tokens to iced widget styles.
//!
//! iced derives the effective theme from the OS appearance; every closure here
//! resolves [`theme::of`] first and never hardcodes a color.

use iced::theme::Theme;
use iced::widget::{button, container};
use iced::{Background, Border, Color, Shadow};

use super::theme::{self, Tokens};

/// The same color at a different opacity, for tinted fills.
fn tinted(color: Color, alpha: f32) -> Color {
    Color { a: alpha, ..color }
}

/// Card body inset, shared by cards and banners.
pub(crate) const CARD_PADDING: u16 = 16;

pub(crate) fn app_background(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(theme::of(theme).background)),
        ..container::Style::default()
    }
}

pub(crate) fn top_bar(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(theme::of(theme).surface)),
        ..container::Style::default()
    }
}

pub(crate) fn sidebar(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(theme::of(theme).surface)),
        ..container::Style::default()
    }
}

/// Card surface for grouped content.
pub(crate) fn card(theme: &Theme) -> container::Style {
    let tokens = theme::of(theme);
    container::Style {
        background: Some(Background::Color(tokens.surface)),
        border: Border {
            color: tokens.border,
            width: 1.0,
            radius: 10.0.into(),
        },
        ..container::Style::default()
    }
}

/// Code-chip surface for monospace values (endpoint, counts).
pub(crate) fn chip(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(theme::of(theme).background)),
        border: Border {
            color: theme::of(theme).border,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..container::Style::default()
    }
}

fn banner(fill: Color, border: Color) -> container::Style {
    container::Style {
        background: Some(Background::Color(tinted(fill, 0.10))),
        border: Border {
            color: tinted(border, 0.40),
            width: 1.0,
            radius: 8.0.into(),
        },
        ..container::Style::default()
    }
}

pub(crate) fn banner_danger(theme: &Theme) -> container::Style {
    let tokens = theme::of(theme);
    banner(tokens.danger, tokens.danger)
}

/// Accent-tinted banner for the one-time client secret reveal.
pub(crate) fn banner_accent(theme: &Theme) -> container::Style {
    let tokens = theme::of(theme);
    banner(tokens.accent, tokens.accent)
}

/// Accent-filled button for the one primary action of a pane.
pub(crate) fn primary(theme: &Theme, status: button::Status) -> button::Style {
    let tokens = theme::of(theme);
    filled(tokens.accent, tokens.accent_hover, tokens.on_accent, status)
}

/// Quiet bordered button for routine actions (Copy, Start, Stop, ...).
pub(crate) fn secondary(theme: &Theme, status: button::Status) -> button::Style {
    let tokens = theme::of(theme);
    outlined(tokens, tokens.text, tokens.border, status)
}

/// Bordered button in the danger color for destructive actions.
pub(crate) fn danger(theme: &Theme, status: button::Status) -> button::Style {
    let tokens = theme::of(theme);
    outlined(tokens, tokens.danger, tinted(tokens.danger, 0.50), status)
}

fn filled(fill: Color, hover: Color, text: Color, status: button::Status) -> button::Style {
    let base = button::Style {
        background: Some(Background::Color(fill)),
        text_color: text,
        border: Border {
            radius: 6.0.into(),
            ..Border::default()
        },
        shadow: Shadow::default(),
        snap: false,
    };
    match status {
        button::Status::Hovered | button::Status::Pressed => button::Style {
            background: Some(Background::Color(hover)),
            ..base
        },
        button::Status::Disabled => button::Style {
            background: base.background.map(|background| match background {
                Background::Color(color) => Background::Color(tinted(color, 0.5)),
                other => other,
            }),
            ..base
        },
        button::Status::Active => base,
    }
}

fn outlined(tokens: &Tokens, text: Color, border: Color, status: button::Status) -> button::Style {
    let base = button::Style {
        background: None,
        text_color: text,
        border: Border {
            color: border,
            width: 1.0,
            radius: 6.0.into(),
        },
        shadow: Shadow::default(),
        snap: false,
    };
    match status {
        button::Status::Hovered | button::Status::Pressed => button::Style {
            background: Some(Background::Color(tinted(tokens.accent, 0.10))),
            text_color: tokens.accent,
            ..base
        },
        button::Status::Disabled => button::Style {
            text_color: tinted(text, 0.5),
            ..base
        },
        button::Status::Active => base,
    }
}

/// Sidebar navigation entry; the selected section gets an accent tint.
pub(crate) fn nav(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let tokens = theme::of(theme);
        let base = button::Style {
            background: if selected {
                Some(Background::Color(tinted(tokens.accent, 0.14)))
            } else {
                None
            },
            text_color: if selected { tokens.accent } else { tokens.text },
            border: Border {
                radius: 6.0.into(),
                ..Border::default()
            },
            ..button::Style::default()
        };
        match status {
            button::Status::Hovered | button::Status::Pressed => button::Style {
                background: Some(Background::Color(if selected {
                    tinted(tokens.accent, 0.22)
                } else {
                    tokens.surface_hover
                })),
                ..base
            },
            button::Status::Disabled | button::Status::Active => base,
        }
    }
}
