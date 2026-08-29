//! View layer for the desktop shell: design tokens, style closures, and panes.

pub(crate) mod logs;
pub(crate) mod servers;
pub(crate) mod styles;
pub(crate) mod theme;
pub(crate) mod tokens;

use iced::widget::{button, container, row, text, text::IntoFragment};
use iced::{Alignment, Element, Font, Length};

use mcp_core::ServerStatus;

use crate::app::Message;

use styles::CARD_PADDING;

/// Semibold system font for titles and emphasized labels.
pub(crate) const SEMIBOLD: Font = Font {
    weight: iced::font::Weight::Semibold,
    ..Font::DEFAULT
};

/// Secondary-size, secondary-color text for hints and metadata.
pub(crate) fn secondary(label: &str) -> text::Text<'_> {
    text(label).size(13).style(|theme| text::Style {
        color: Some(theme::of(theme).text_secondary),
    })
}

/// Small semibold secondary label above form fields.
pub(crate) fn form_label(label: &str) -> text::Text<'_> {
    text(label)
        .size(12)
        .font(SEMIBOLD)
        .style(|theme| text::Style {
            color: Some(theme::of(theme).text_secondary),
        })
}

/// Status dot for a server lifecycle state, colored via the active theme.
pub(crate) fn status_dot(status: ServerStatus) -> text::Text<'static> {
    text("\u{25CF}").size(11).style(move |theme| text::Style {
        color: Some(theme::status_color(theme::of(theme), status)),
    })
}

/// Card surface for grouped content.
pub(crate) fn card<'a, M>(content: impl Into<Element<'a, M>>) -> container::Container<'a, M> {
    container(content).padding(CARD_PADDING).style(styles::card)
}

/// Pane title with a count chip, e.g. "Servers  3".
pub(crate) fn pane_heading(label: &str, count: usize) -> row::Row<'_, Message> {
    let chip = container(
        text(count.to_string()).size(12).style(|theme| text::Style {
            color: Some(theme::of(theme).text_secondary),
        }),
    )
    .padding([2, 8])
    .style(styles::chip);

    row![text(label).size(22).font(SEMIBOLD), chip]
        .spacing(10)
        .align_y(Alignment::Center)
}

/// Sidebar navigation entry; the active section gets an accent tint.
pub(crate) fn nav_item<'a>(
    selected: bool,
    label: &'a str,
    message: Message,
) -> button::Button<'a, Message> {
    button(
        container(
            text(label)
                .size(13)
                .font(if selected { SEMIBOLD } else { Font::DEFAULT }),
        )
        .padding([8, 10]),
    )
    .width(Length::Fill)
    .style(styles::nav(selected))
    .on_press(message)
}

/// Accent-filled button for the one primary action of a pane.
pub(crate) fn primary_button<'a, M: Clone>(label: &'a str) -> button::Button<'a, M> {
    button(text(label).size(13))
        .padding([8, 14])
        .style(styles::primary)
}

/// Quiet bordered button for routine actions (Copy, Start, Stop, ...).
pub(crate) fn secondary_button<'a, M: Clone>(
    label: impl IntoFragment<'a>,
) -> button::Button<'a, M> {
    button(text(label).size(13)).padding([8, 14]).style(styles::secondary)
}

/// Bordered button in the danger color for destructive actions.
pub(crate) fn danger_button<'a, M: Clone>(label: &'a str) -> button::Button<'a, M> {
    button(text(label).size(13)).padding([8, 14]).style(styles::danger)
}

/// Full-width danger banner for the latest failed operation.
pub(crate) fn error_banner(error: &str) -> container::Container<'_, Message> {
    container(
        text(format!("\u{26A0} {error}")).size(13).style(|theme| text::Style {
            color: Some(theme::of(theme).danger),
        }),
    )
    .padding(CARD_PADDING)
    .style(styles::banner_danger)
}

/// Full-width accent banner for operation reports (e.g. import results).
pub(crate) fn notice_banner(notice: &str) -> container::Container<'_, Message> {
    container(text(notice).size(13))
        .padding(CARD_PADDING)
        .style(styles::banner_accent)
}
