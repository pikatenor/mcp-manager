//! Design tokens for the desktop shell.
//!
//! The app follows the OS appearance: iced derives the effective theme from the
//! window, and [`of`] picks the matching token set. Style closures receive only
//! an `&iced::Theme`, so every custom color flows through here.

use iced::{Color, Theme};
use mcp_core::{ServerStatus, ServerType};

/// One palette of concrete UI colors for a single appearance mode.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Tokens {
    /// App canvas behind cards.
    pub background: Color,
    /// Cards, top bar, sidebar.
    pub surface: Color,
    /// Hover fill for neutral surfaces and nav items.
    pub surface_hover: Color,
    /// Hairline borders on surfaces.
    pub border: Color,
    /// Primary text.
    pub text: Color,
    /// Hints, metadata, secondary labels.
    pub text_secondary: Color,
    /// Primary actions and selection.
    pub accent: Color,
    pub accent_hover: Color,
    /// Text drawn on an accent fill.
    pub on_accent: Color,
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
}

pub(crate) const LIGHT: Tokens = Tokens {
    background: Color::from_rgb8(0xF4, 0xF4, 0xF6),
    surface: Color::from_rgb8(0xFF, 0xFF, 0xFF),
    surface_hover: Color::from_rgb8(0xEF, 0xEF, 0xF3),
    border: Color::from_rgb8(0xE2, 0xE2, 0xE7),
    text: Color::from_rgb8(0x1B, 0x1B, 0x1F),
    text_secondary: Color::from_rgb8(0x6E, 0x6E, 0x78),
    accent: Color::from_rgb8(0x5B, 0x5B, 0xD6),
    accent_hover: Color::from_rgb8(0x4E, 0x4E, 0xC4),
    on_accent: Color::WHITE,
    success: Color::from_rgb8(0x21, 0x83, 0x58),
    warning: Color::from_rgb8(0xB7, 0x7E, 0x33),
    danger: Color::from_rgb8(0xC3, 0x42, 0x3F),
};

pub(crate) const DARK: Tokens = Tokens {
    background: Color::from_rgb8(0x1C, 0x1C, 0x20),
    surface: Color::from_rgb8(0x26, 0x26, 0x2B),
    surface_hover: Color::from_rgb8(0x2E, 0x2E, 0x34),
    border: Color::from_rgb8(0x38, 0x38, 0x3F),
    text: Color::from_rgb8(0xF2, 0xF2, 0xF4),
    text_secondary: Color::from_rgb8(0x9E, 0x9E, 0xA7),
    accent: Color::from_rgb8(0x8B, 0x8B, 0xF4),
    accent_hover: Color::from_rgb8(0x9D, 0x9D, 0xF7),
    on_accent: Color::from_rgb8(0x1C, 0x1C, 0x20),
    success: Color::from_rgb8(0x3D, 0xD6, 0x8C),
    warning: Color::from_rgb8(0xFF, 0xC1, 0x4E),
    danger: Color::from_rgb8(0xE5, 0x48, 0x4D),
};

/// Tokens for the theme iced derived from the OS appearance.
pub(crate) fn of(theme: &Theme) -> &'static Tokens {
    if theme.extended_palette().is_dark {
        &DARK
    } else {
        &LIGHT
    }
}

/// Dot and label color for a server lifecycle state.
pub(crate) fn status_color(tokens: &Tokens, status: ServerStatus) -> Color {
    match status {
        ServerStatus::Running => tokens.success,
        ServerStatus::Error => tokens.danger,
        ServerStatus::Starting | ServerStatus::Stopping => tokens.warning,
        ServerStatus::Stopped => tokens.text_secondary,
    }
}

pub(crate) fn status_label(status: ServerStatus) -> &'static str {
    match status {
        ServerStatus::Stopped => "stopped",
        ServerStatus::Starting => "starting",
        ServerStatus::Running => "running",
        ServerStatus::Stopping => "stopping",
        ServerStatus::Error => "error",
    }
}

pub(crate) fn type_label(server_type: ServerType) -> &'static str {
    match server_type {
        ServerType::Local => "local",
        ServerType::Remote => "remote",
        ServerType::RemoteStreamable => "remote-streamable",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn of_follows_the_effective_theme_mode() {
        assert_eq!(of(&Theme::Dark).background, DARK.background);
        assert_eq!(of(&Theme::Light).background, LIGHT.background);
    }

    #[test]
    fn status_color_matches_the_state() {
        let tokens = &LIGHT;
        assert_eq!(status_color(tokens, ServerStatus::Running), tokens.success);
        assert_eq!(status_color(tokens, ServerStatus::Error), tokens.danger);
        assert_eq!(status_color(tokens, ServerStatus::Starting), tokens.warning);
        assert_eq!(status_color(tokens, ServerStatus::Stopping), tokens.warning);
        assert_eq!(status_color(tokens, ServerStatus::Stopped), tokens.text_secondary);
    }

    #[test]
    fn light_and_dark_differ_on_every_role() {
        assert_ne!(LIGHT.background, DARK.background);
        assert_ne!(LIGHT.surface, DARK.surface);
        assert_ne!(LIGHT.surface_hover, DARK.surface_hover);
        assert_ne!(LIGHT.border, DARK.border);
        assert_ne!(LIGHT.text, DARK.text);
        assert_ne!(LIGHT.text_secondary, DARK.text_secondary);
        assert_ne!(LIGHT.accent, DARK.accent);
        assert_ne!(LIGHT.accent_hover, DARK.accent_hover);
        assert_ne!(LIGHT.on_accent, DARK.on_accent);
        assert_ne!(LIGHT.success, DARK.success);
        assert_ne!(LIGHT.warning, DARK.warning);
        assert_ne!(LIGHT.danger, DARK.danger);
    }
}
