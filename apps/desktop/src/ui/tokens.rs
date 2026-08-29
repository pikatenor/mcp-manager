//! Client-tokens pane: issue form, one-time secret, and the token list.

use iced::widget::{column, container, row, space, text, text_input};
use iced::{Alignment, Element, Font, Length};

use crate::app::{App, Message};

use super::styles::{self, CARD_PADDING};
use super::{card, danger_button, pane_heading, primary_button, secondary, SEMIBOLD};

pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let mut heading = row![pane_heading("Client tokens", app.tokens.len()), space::horizontal()]
        .spacing(10)
        .align_y(Alignment::Center);
    if app.tokens.iter().any(|token| token.revoked_at.is_some()) {
        heading = heading.push(
            danger_button("Clear revoked").on_press(Message::ClearRevokedTokens),
        );
    }
    let mut body = column![heading].spacing(16);

    if let Some(notice) = &app.notice {
        body = body.push(super::notice_banner(notice));
    }

    body = body.push(card(
        row![
            text_input("client name", &app.client_name)
                .on_input(Message::ClientName)
                .width(Length::Fill),
            primary_button("Issue").on_press(Message::IssueToken),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    ));

    if let Some(plaintext) = &app.plaintext {
        body = body.push(secret_banner(plaintext));
    }

    if app.tokens.is_empty() {
        body = body.push(card(
            container(secondary(
                "No client tokens yet. Issue one to let a client call the aggregated endpoint.",
            ))
            .width(Length::Fill)
            .align_x(iced::alignment::Horizontal::Center),
        ));
    }

    for token in &app.tokens {
        let mut line = row![
            text(&token.client_name).size(14),
            space::horizontal(),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        if token.revoked_at.is_some() {
            line = line.push(secondary("revoked"));
            line = line.push(
                danger_button("Delete").on_press(Message::DeleteToken(token.id.clone())),
            );
        } else {
            line = line.push(
                danger_button("Revoke").on_press(Message::Revoke(token.id.clone())),
            );
        }

        body = body.push(card(line));
    }

    body.push(secondary(
        "Treat issued tokens like passwords; the plaintext is shown exactly once.",
    ))
    .into()
}

/// Accent-tinted reveal for the one-time client secret, with a copy button.
fn secret_banner(plaintext: &str) -> container::Container<'_, Message> {
    let secret = container(
        text(plaintext)
            .size(13)
            .font(Font::MONOSPACE)
            .style(|theme| text::Style {
                color: Some(super::theme::of(theme).accent),
            }),
    )
    .padding([8, 12])
    .style(styles::chip);

    container(
        column![
            text("Copy this secret now \u{2014} it will not be shown again")
                .size(13)
                .font(SEMIBOLD),
            row![secret, primary_button("Copy").on_press(Message::CopyPlaintext),]
                .spacing(8)
                .align_y(Alignment::Center),
        ]
        .spacing(10),
    )
    .padding(CARD_PADDING)
    .style(styles::banner_accent)
}
