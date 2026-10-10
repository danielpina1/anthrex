//! The empty main pane's splash: the octopus mascot (`assets/logo-mark.svg`), the
//! name and version, and the "no agents" hint, centred. Each arm is an agent, so its
//! tip wears a status colour — the same roles the sidebar draws statuses in.

use crate::theme::{self, Palette, Role};
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// The body rows, every one `WIDTH` columns so centring keeps them aligned.
const BODY: [&str; 6] = [
    "    ▄▄████▄▄    ",
    "   ███ ██ ███   ",
    "   ██████████   ",
    "  ▟██▀▟██▙▀██▙  ",
    " ▟▛  ▟▛  ▜▙  ▜▙ ",
    "▐▛  ▐▛    ▜▌  ▜▌",
];
const BODY_ASCII: [&str; 6] = [
    "    .------.    ",
    "   (  o  o  )   ",
    "    )      (    ",
    r"   /  /  \  \   ",
    r"  /  /    \  \  ",
    r" /  /      \  \ ",
];
/// The tips row: each `*` is the next arm's tip, under that arm's end.
const TIPS_ROW: &str = " *   *    *   * ";
const TIPS_ROW_ASCII: &str = " *  *      *  * ";
const WIDTH: u16 = 16;
/// The arm tips' colours, left to right.
const TIPS: [Role; 4] = [Role::Done, Role::Working, Role::Attention, Role::Paused];

/// Draws the splash in `area`, or only `hint` when the art does not fit.
pub fn render(frame: &mut Frame, area: Rect, hint: &str, p: Palette) {
    let muted = theme::role(Role::Muted, p);
    let mut lines = art(p);
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled(
            "anthrex",
            theme::role(Role::Accent, p).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" v{}", env!("CARGO_PKG_VERSION")), muted),
    ]));
    lines.push(Line::raw(""));
    lines.push(Line::styled(hint.to_string(), muted));

    let rows = lines.len() as u16;
    if area.height < rows || area.width < WIDTH + 2 {
        let fallback = vec![Line::raw(""), Line::styled(format!("  {hint}"), muted)];
        frame.render_widget(Paragraph::new(fallback), area);
        return;
    }
    let top = area.y + (area.height - rows) / 2;
    let centred = Rect::new(area.x, top, area.width, rows);
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), centred);
}

fn art(p: Palette) -> Vec<Line<'static>> {
    let body = theme::role(Role::Accent, p);
    let (rows, tips_row, tip) = if p.ascii {
        (BODY_ASCII, TIPS_ROW_ASCII, "o")
    } else {
        (BODY, TIPS_ROW, "●")
    };
    let mut lines: Vec<Line> = rows.iter().map(|r| Line::styled(*r, body)).collect();
    let mut roles = TIPS.into_iter();
    let spans = tips_row
        .split_inclusive('*')
        .flat_map(|part| match part.strip_suffix('*') {
            Some(gap) => {
                let role = roles.next().expect("one role per tip");
                vec![Span::raw(gap), Span::styled(tip, theme::role(role, p))]
            }
            None => vec![Span::raw(part)],
        });
    lines.push(Line::from(spans.collect::<Vec<_>>()));
    lines
}

#[cfg(test)]
#[path = "splash_tests.rs"]
mod tests;
