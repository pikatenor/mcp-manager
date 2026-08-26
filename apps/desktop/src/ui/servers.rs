//! Servers pane: add-server form and the configured server list.

use iced::widget::{button, checkbox, column, pick_list, row, text, text_editor, text_input};
use iced::{Element, Length};

use mcp_core::{ServerStatus, ServerType};

use crate::app::{App, FormServerType, Message, FORM_SERVER_TYPES};

use super::theme::{of, status_color, status_label, type_label};
use super::{pane_heading, secondary};

pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let mut body = column![pane_heading("Servers", app.servers.len())].spacing(16);

    if app.show_add_form {
        body = body.push(add_form(app));
    }

    for server in &app.servers {
        let id = server.config.id.clone();
        let status = server.status;
        let mut card = column![row![
            text(&server.config.name).size(16),
            text(type_label(server.config.server_type))
                .size(13)
                .style(|theme| text::Style {
                    color: Some(of(theme).text_secondary),
                }),
            text(status_label(server.status)).size(13).style(move |theme| text::Style {
                color: Some(status_color(of(theme), status)),
            }),
        ]
        .spacing(8)];
        if let Some(error) = &server.last_error {
            card = card.push(text(error).size(13).style(|theme| text::Style {
                color: Some(of(theme).text_secondary),
            }));
        }
        let mut actions = row![].spacing(8);
        if server.status == ServerStatus::Running {
            actions = actions.push(button("Stop").on_press(Message::Stop(id.clone())));
        } else {
            actions = actions.push(button("Start").on_press(Message::Start(id.clone())));
        }
        actions = actions.push(button("Delete").on_press(Message::Delete(id.clone())));
        if server.config.server_type != ServerType::Local {
            let label = if app
                .oauth_by_server
                .get(&id)
                .copied()
                .unwrap_or(false)
            {
                "Re-auth"
            } else {
                "OAuth"
            };
            actions = actions.push(button(label).on_press(Message::Oauth(id.clone())));
        }
        card = card.push(actions);
        if server.status == ServerStatus::Running {
            if let Some(tools) = app.tools_by_server.get(&id) {
                for tool in tools {
                    let tool_id = id.clone();
                    let tool_name = tool.name.clone();
                    card = card.push(
                        checkbox(tool.public)
                            .label(format!(
                                "{} {}",
                                tool.name,
                                if tool.public { "public" } else { "hidden" }
                            ))
                            .on_toggle(move |public| Message::ToggleTool {
                                id: tool_id.clone(),
                                name: tool_name.clone(),
                                public,
                            }),
                    );
                }
            }
        }
        body = body.push(card.spacing(6));
    }

    body.push(secondary(
        "Local stdio, remote Streamable HTTP, and legacy SSE servers start from this list. Env values and bearer tokens stay in the keychain.",
    ))
    .into()
}

fn add_form(app: &App) -> iced::widget::Column<'_, Message> {
    let mut form = column![row![
        text_input("server name", &app.server_name).on_input(Message::ServerName),
        pick_list(
            FORM_SERVER_TYPES,
            Some(app.server_type),
            Message::ServerType,
        ),
    ]
    .spacing(8)]
    .spacing(8);

    if app.server_type == FormServerType::Local {
        form = form.push(
            row![
                text_input("command", &app.command).on_input(Message::Command),
                text_input("args", &app.args).on_input(Message::Args),
            ]
            .spacing(8),
        );
    } else {
        form = form.push(
            text_input("https://example.com/mcp", &app.remote_url)
                .on_input(Message::RemoteUrl),
        );
    }

    form = form.push(
        text_editor(&app.env)
            .placeholder("ENV_NAME=value (one per line)")
            .height(Length::Fixed(72.0))
            .on_action(Message::Env),
    );

    if app.server_type != FormServerType::Local {
        form = form.push(
            text_input("optional bearer token (stored in keychain)", &app.bearer)
                .on_input(Message::Bearer),
        );
    }

    form = form.push(
        checkbox(app.auto_start)
            .label("auto-start")
            .on_toggle(Message::AutoStart),
    );
    form.push(row![
        button("Add server").on_press(Message::AddServer),
        button("Cancel").on_press(Message::CancelAddForm),
    ]
    .spacing(8))
}
