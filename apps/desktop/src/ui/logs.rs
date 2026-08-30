//! Tool-calls pane: metadata-only invocation history from the local call log.

use chrono::{DateTime, Local, TimeZone};
use iced::widget::{column, container, row, space, text};
use iced::{Alignment, Element, Font, Length};

use mcp_core::{ToolCallEntry, TOOL_DELIMITER};

use crate::app::{App, Message};

use super::styles;
use super::{card, danger_button, pane_heading, secondary};

pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let heading = row![
        pane_heading("Tool calls", app.tool_calls.len()),
        space::horizontal(),
        danger_button("Clear").on_press(Message::ClearLogs),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let mut body = column![heading].spacing(16);

    if app.tool_calls.is_empty() {
        body = body.push(card(
            container(secondary(
                "No tool calls recorded yet. Requests through the aggregated endpoint appear here.",
            ))
            .width(Length::Fill)
            .align_x(iced::alignment::Horizontal::Center),
        ));
    }

    for entry in &app.tool_calls {
        body = body.push(call_card(entry));
    }

    body.into()
}

/// One invocation: status dot, local time, tool chip, failure kind, client, duration.
fn call_card(entry: &ToolCallEntry) -> container::Container<'_, Message> {
    let ok = entry.ok;
    let label = if entry.server.is_empty() {
        entry.tool.clone()
    } else {
        format!("{}{TOOL_DELIMITER}{}", entry.server, entry.tool)
    };

    let mut line = row![
        text("\u{25CF}").size(11).style(move |theme| text::Style {
            color: Some(if ok {
                super::theme::of(theme).success
            } else {
                super::theme::of(theme).danger
            }),
        }),
        text(clock_at(&Local, entry.called_at))
            .size(12)
            .font(Font::MONOSPACE),
        container(text(label).size(12).font(Font::MONOSPACE))
            .padding([2, 8])
            .style(styles::chip),
    ]
    .spacing(10)
    .align_y(Alignment::Center);

    if !entry.ok {
        if let Some(kind) = &entry.error_kind {
            line = line.push(
                container(text(kind).size(12).style(|theme| text::Style {
                    color: Some(super::theme::of(theme).danger),
                }))
                .padding([2, 8])
                .style(styles::chip),
            );
        }
    }

    line = line.push(space::horizontal());
    line = line.push(if entry.client.is_empty() {
        secondary("unknown client")
    } else {
        secondary(&entry.client)
    });
    line = line.push(
        text(format_duration(entry.duration_ms))
            .size(12)
            .font(Font::MONOSPACE),
    );

    card(line)
}

/// Milliseconds rendered compactly: sub-second stays in ms, otherwise seconds.
pub(crate) fn format_duration(duration_ms: i64) -> String {
    if duration_ms < 1000 {
        format!("{duration_ms} ms")
    } else {
        format!("{:.2} s", duration_ms as f64 / 1000.0)
    }
}

/// Local wall-clock rendering of a unix-millis stamp: `%m-%d %H:%M:%S`.
pub(crate) fn clock_at<Tz: TimeZone>(tz: &Tz, called_at_ms: i64) -> String
where
    Tz::Offset: std::fmt::Display,
{
    DateTime::from_timestamp_millis(called_at_ms)
        .map(|utc| utc.with_timezone(tz).format("%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| String::from("?"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    #[test]
    fn format_duration_stays_in_milliseconds_under_a_second() {
        assert_eq!(format_duration(0), "0 ms");
        assert_eq!(format_duration(12), "12 ms");
        assert_eq!(format_duration(999), "999 ms");
    }

    #[test]
    fn format_duration_switches_to_seconds_from_one_second() {
        assert_eq!(format_duration(1000), "1.00 s");
        assert_eq!(format_duration(1500), "1.50 s");
        assert_eq!(format_duration(62_500), "62.50 s");
    }

    #[test]
    fn clock_at_formats_utc_from_unix_millis() {
        // 2026-01-02 03:04:05.678 UTC
        let called_at = 1_767_323_045_678;
        let utc = FixedOffset::east_opt(0).unwrap();
        assert_eq!(clock_at(&utc, called_at), "01-02 03:04:05");
    }

    #[test]
    fn clock_at_applies_the_given_offset() {
        // 2026-01-02 03:04:05.678 UTC
        let called_at = 1_767_323_045_678;
        let jst = FixedOffset::east_opt(9 * 3600).unwrap();
        assert_eq!(clock_at(&jst, called_at), "01-02 12:04:05");
    }

    #[test]
    fn clock_at_marks_out_of_range_stamps() {
        let utc = FixedOffset::east_opt(0).unwrap();
        assert_eq!(clock_at(&utc, i64::MAX), "?");
    }
}
