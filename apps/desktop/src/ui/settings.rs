//! Settings pane: feature flags for the aggregated MCP endpoint.

use iced::widget::{checkbox, column, text};
use iced::{Element, Length};

use crate::app::{App, Message};

use super::{card, secondary, SEMIBOLD};

pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let heading = text("Settings").size(18).font(SEMIBOLD);

    let mut body = column![heading].spacing(16);

    body = body.push(card(
        column![
            checkbox(app.settings.list_stopped_from_cache)
                .label("List stopped servers from cache")
                .on_toggle(Message::ToggleListStoppedFromCache),
            secondary(
                "tools/list also serves the last-known tool lists of stopped \
                 servers, alongside live lists from running ones.",
            )
            .width(Length::Fill),
        ]
        .spacing(6),
    ));

    body = body.push(card(
        column![
            checkbox(app.settings.on_demand_start)
                .label("Start servers on demand")
                .on_toggle(Message::ToggleOnDemandStart),
            secondary(
                "tools/call starts the target server when it is stopped, then \
                 performs the call. Disabled servers are never started.",
            )
            .width(Length::Fill),
        ]
        .spacing(6),
    ));

    body.into()
}
