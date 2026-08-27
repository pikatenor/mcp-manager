//! Servers pane: add-server form and the configured server list.

use iced::widget::{
    checkbox, column, container, pick_list, row, space, text, text_editor, text_input,
};
use iced::{Alignment, Element, Length};

use mcp_core::{ServerStatus, ServerType};

use crate::app::{App, FormServerType, Message, FORM_SERVER_TYPES};

use super::theme::{of, status_color, status_label, type_label};
use super::{
    card, danger_button, form_label, pane_heading, primary_button, secondary, secondary_button,
    status_dot, SEMIBOLD,
};

pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let heading = row![
        pane_heading("Servers", app.servers.len()),
        space::horizontal(),
        primary_button("+ Add server").on_press(Message::ToggleAddForm),
    ]
    .align_y(Alignment::Center);

    let mut body = column![heading].spacing(16);

    if app.show_add_form {
        body = body.push(card(add_form(app)));
    }

    if app.servers.is_empty() && !app.show_add_form {
        body = body.push(card(
            container(secondary(
                "No servers yet. Add your first MCP server with “+ Add server”.",
            ))
            .width(Length::Fill)
            .align_x(iced::alignment::Horizontal::Center),
        ));
    }

    for server in &app.servers {
        let id = server.config.id.clone();
        let status = server.status;

        let mut content = column![row![
            status_dot(status),
            text(&server.config.name).size(16).font(SEMIBOLD),
            space::horizontal(),
            text(type_label(server.config.server_type))
                .size(13)
                .style(|theme| text::Style {
                    color: Some(of(theme).text_secondary),
                }),
            text(status_label(server.status)).size(13).style(move |theme| text::Style {
                color: Some(status_color(of(theme), status)),
            }),
        ]
        .spacing(8)
        .align_y(Alignment::Center)]
        .spacing(10);

        if let Some(error) = &server.last_error {
            content = content.push(text(error).size(13).style(|theme| text::Style {
                color: Some(of(theme).danger),
            }));
        }

        let mut actions = row![].spacing(8);
        actions = actions.push(secondary_button("Edit").on_press(Message::EditServer(id.clone())));
        if server.status == ServerStatus::Running {
            actions =
                actions.push(secondary_button("Stop").on_press(Message::Stop(id.clone())));
        } else {
            actions =
                actions.push(secondary_button("Start").on_press(Message::Start(id.clone())));
        }
        if server.config.server_type != ServerType::Local {
            let label = if app.oauth_by_server.get(&id).copied().unwrap_or(false) {
                "Re-auth"
            } else {
                "OAuth"
            };
            actions = actions.push(secondary_button(label).on_press(Message::Oauth(id.clone())));
        }
        actions = actions.push(danger_button("Delete").on_press(Message::Delete(id.clone())));
        content = content.push(actions);

        if server.status == ServerStatus::Running {
            if let Some(tools) = app.tools_by_server.get(&id) {
                let mut tool_list = column![].spacing(6);
                for tool in tools {
                    let tool_id = id.clone();
                    let tool_name = tool.name.clone();
                    tool_list = tool_list.push(
                        row![
                            checkbox(tool.public)
                                .label(tool.name.clone())
                                .on_toggle(move |public| Message::ToggleTool {
                                    id: tool_id.clone(),
                                    name: tool_name.clone(),
                                    public,
                                }),
                            space::horizontal(),
                            secondary(if tool.public { "public" } else { "hidden" }),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    );
                }
                content = content.push(tool_list);
            }
        }

        body = body.push(card(content));
    }

    body.push(secondary(
        "Local stdio, remote Streamable HTTP, and legacy SSE servers start from this list. Env values and bearer tokens stay in the keychain.",
    ))
    .into()
}

fn add_form(app: &App) -> iced::widget::Column<'_, Message> {
    let editing = app.editing_id.is_some();
    let (title, submit_label, submit) = if editing {
        ("Edit server", "Save", Message::UpdateServer)
    } else {
        ("Add server", "Add server", Message::AddServer)
    };
    let mut form = column![text(title).size(15).font(SEMIBOLD)].spacing(12);

    form = form.push(field("Name", {
        let mut inputs = row![text_input("server name", &app.server_name)
            .on_input(Message::ServerName)
            .width(Length::Fill)]
        .spacing(8);
        inputs = inputs.push(
            pick_list(
                FORM_SERVER_TYPES,
                Some(app.server_type),
                Message::ServerType,
            )
            .width(Length::FillPortion(2)),
        );
        inputs
    }));

    if app.server_type == FormServerType::Local {
        form = form.push(field(
            "Command",
            row![
                text_input("command", &app.command)
                    .on_input(Message::Command)
                    .width(Length::FillPortion(1)),
                text_input("args", &app.args)
                    .on_input(Message::Args)
                    .width(Length::FillPortion(2)),
            ]
            .spacing(8),
        ));
    } else {
        form = form.push(field(
            "URL",
            text_input("https://example.com/mcp", &app.remote_url)
                .on_input(Message::RemoteUrl),
        ));
    }

    form = form.push(field(
        "Environment",
        text_editor(&app.env)
            .placeholder("ENV_NAME=value (one per line)")
            .height(Length::Fixed(72.0))
            .on_action(Message::Env),
    ));

    if app.server_type != FormServerType::Local {
        // An OAuth-connected server keeps its token in the keychain; editing
        // the mirror by hand would only fight the OAuth flow.
        let oauth_managed = editing
            && app
                .editing_id
                .as_deref()
                .and_then(|id| app.oauth_by_server.get(id))
                .copied()
                .unwrap_or(false);
        let bearer_field: iced::Element<'_, Message> = if oauth_managed {
            secondary("Authentication is managed by OAuth.").into()
        } else {
            text_input("optional bearer token (stored in keychain)", &app.bearer)
                .on_input(Message::Bearer)
                .into()
        };
        form = form.push(field("Bearer", bearer_field));
    }

    form = form.push(
        checkbox(app.auto_start)
            .label("auto-start")
            .on_toggle(Message::AutoStart),
    );

    form.push(
        row![
            primary_button(submit_label).on_press(submit),
            secondary_button("Cancel").on_press(Message::CancelAddForm),
        ]
        .spacing(8),
    )
}

/// A labeled form field: small semibold label above the input row.
fn field<'a>(
    label: &'a str,
    input: impl Into<iced::Element<'a, Message>>,
) -> iced::widget::Column<'a, Message> {
    column![form_label(label), input.into()].spacing(4)
}
