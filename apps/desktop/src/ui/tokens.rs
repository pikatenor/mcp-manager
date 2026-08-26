//! Client-tokens pane: issue form, one-time secret, and the token list.

use iced::widget::{button, column, row, text, text_input};
use iced::Element;

use crate::app::{App, Message};

use super::{pane_heading, secondary};

pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let mut body = column![pane_heading("Client tokens", app.tokens.len())].spacing(16);

    body = body.push(
        row![
            text_input("client name", &app.client_name).on_input(Message::ClientName),
            button("Issue").on_press(Message::IssueToken),
        ]
        .spacing(8),
    );

    if let Some(plaintext) = &app.plaintext {
        body = body.push(secondary("Copy this secret now; it will not be shown again:"));
        body = body.push(
            row![
                text(plaintext),
                button("Copy").on_press(Message::CopyPlaintext),
            ]
            .spacing(8),
        );
    }

    for token in &app.tokens {
        let mut line = row![text(&token.client_name)].spacing(8);
        if token.revoked_at.is_some() {
            line = line.push(secondary("revoked"));
        } else {
            line = line.push(button("Revoke").on_press(Message::Revoke(token.id.clone())));
        }
        body = body.push(line);
    }

    body.into()
}
