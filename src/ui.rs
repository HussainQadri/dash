use crate::app::App;
use crate::model::{Item, SourceStatus, days_from_today, local, now};
use crate::theme::*;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, List, ListItem, Padding, Paragraph, Wrap};

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const WIDE: u16 = 120;

mod icon {
    pub const DASH: &str = "󰃰";
    pub const DUE: &str = "󰔟";
    pub const NEWS: &str = "󰃦";
    pub const FORUM: &str = "󰊌";
    pub const GMAIL: &str = "󰇰";
    pub const KEATS: &str = "󰑴";
    pub const CODE: &str = "󰅩";
    pub const VOICE: &str = "󰗋";
    pub const CHECK: &str = "󰅈";
    pub const TASK: &str = "󰅇";
    pub const CAL: &str = "󰃭";
    pub const OK: &str = "󰗠";
    pub const ALERT: &str = "󰀨";
    pub const FIRE: &str = "󰈸";
    pub const LINK: &str = "󰌷";
}

/// Pad or cut to exactly `w` columns (every glyph used here is one cell wide).
fn fit(s: &str, w: usize) -> String {
    match s.chars().count() {
        n if n <= w => format!("{s:<w$}"),
        _ if w == 0 => String::new(),
        _ => s.chars().take(w - 1).collect::<String>() + "…",
    }
}

fn ago(ts: i64) -> String {
    match now() - ts {
        d if d < 60 => "just now".into(),
        d if d < 3600 => format!("{}m ago", d / 60),
        d if d < 86400 => format!("{}h ago", d / 3600),
        d => format!("{}d ago", d / 86400),
    }
}

fn span_text(d: i64) -> String {
    match d.abs() {
        a if a < 3600 => format!("{}m", a / 60),
        a if a < 86400 => format!("{}h", a / 3600),
        a => format!("{}d", a / 86400),
    }
}

/// Countdown badge: under a day is a solid red pill, then red, yellow, grey.
fn countdown(i: &Item) -> Span<'static> {
    let Some(due) = i.due else { return Span::styled("    — ", FAINT) };
    let d = due - now();
    let text = format!(" {:>4} ", if d < 0 { format!("-{}", span_text(d)) } else { span_text(d) });
    let style = match d {
        _ if !i.pending => Style::new().fg(DIM),
        d if d < 0 => Style::new().fg(RED).bold(),
        d if d <= 86400 => Style::new().fg(BG).bg(RED).bold(),
        d if d <= 3 * 86400 => Style::new().fg(RED).bold(),
        d if d <= 7 * 86400 => Style::new().fg(YELLOW).bold(),
        _ => Style::new().fg(DIM),
    };
    Span::styled(text, style)
}

fn urgency(due: i64) -> ratatui::style::Color {
    match due - now() {
        d if d <= 3 * 86400 => RED,
        d if d <= 7 * 86400 => YELLOW,
        _ => DIM,
    }
}

fn bucket(due: Option<i64>) -> &'static str {
    match due {
        None => "no date",
        Some(d) if d < now() => "overdue",
        Some(d) => match days_from_today(d) {
            0 => "today",
            1 => "tomorrow",
            2..=6 => "this week",
            _ => "later",
        },
    }
}

fn kind_icon(i: &Item) -> &'static str {
    match i.tag.as_str() {
        "OA" => icon::CODE,
        "interview" => icon::VOICE,
        "assessment" | "quiz" => icon::CHECK,
        "assign" | "coursework" => icon::TASK,
        _ => icon::CAL,
    }
}

fn marker(selected: bool, focused: bool) -> Span<'static> {
    if selected && focused { Span::styled("▌ ", TEAL) } else { Span::raw("  ") }
}

fn panel<'a>(ic: &'a str, name: &'a str, count: usize, focused: bool, warn: Option<&'a str>) -> Block<'a> {
    let mut title = vec![
        Span::styled(format!(" {ic} "), if focused { TEAL } else { DIM }),
        Span::styled(format!("{name} "), if focused { Style::new().fg(FG).bold() } else { Style::new().fg(DIM) }),
    ];
    if let Some(w) = warn {
        title.push(Span::styled(format!("{} {w} ", icon::ALERT), YELLOW));
    }
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(if focused { DIM } else { FAINT })
        .title(Line::from(title))
        .padding(Padding::horizontal(1));
    if count > 0 {
        block = block.title_top(Line::styled(format!(" {count} "), FAINT).right_aligned());
    }
    block
}

fn empty(f: &mut Frame, area: Rect, ic: &str, text: &str, color: ratatui::style::Color) {
    let y = area.y + area.height.saturating_sub(1) / 2;
    let line = Line::from(vec![Span::styled(format!("{ic}  "), color), Span::styled(text.to_string(), DIM)]);
    f.render_widget(line.centered(), Rect { y, height: 1.min(area.height), ..area });
}

fn header(f: &mut Frame, app: &App, top: Rect, sub: Rect) {
    let left = Line::from(vec![
        Span::styled(format!("{} ", icon::DASH), TEAL),
        "dash".bold().fg(FG),
        Span::styled(format!("   {}", local(now(), "%a %d %b").to_lowercase()), DIM),
        Span::styled("  ·  ", FAINT),
        Span::styled(local(now(), "%H:%M"), DIM),
    ]);
    f.render_widget(left, top);

    let s = &app.data.status;
    let mut right = source("keats", &s.keats, app);
    right.push(Span::raw("    "));
    right.extend(source("gmail", &s.gmail, app));
    f.render_widget(Line::from(right).right_aligned(), top);

    let due = &app.data.sections[0];
    let soon = due.iter().filter(|i| i.pending && i.due.is_some_and(|d| d - now() <= 2 * 86400)).count();
    let mut spans = if soon > 0 {
        vec![
            Span::styled(format!("{} ", icon::FIRE), RED),
            Span::styled(format!("{soon} due within 48h"), Style::new().fg(RED).bold()),
        ]
    } else {
        vec![Span::styled(format!("{} ", icon::OK), GREEN), Span::styled("nothing urgent", GREEN)]
    };
    if let Some(next) = due.iter().find(|i| i.pending && i.due.is_some_and(|d| d >= now())) {
        let d = next.due.unwrap_or(0);
        spans.extend([
            Span::styled("   ·   next  ", FAINT),
            Span::styled(next.org.clone(), Style::new().fg(FG).bold()),
            Span::styled(format!(" {} ", next.tag), DIM),
            Span::styled(format!("in {}", span_text(d - now())), Style::new().fg(urgency(d)).bold()),
        ]);
    }
    f.render_widget(Line::from(spans), sub);
}

fn source<'a>(name: &'a str, s: &'a SourceStatus, app: &App) -> Vec<Span<'a>> {
    let running = s.state == "running" && app.syncing();
    let err = |e: &str| match e.chars().count() {
        n if n > 32 => e.chars().take(31).collect::<String>() + "…",
        _ => e.to_string(),
    };
    let (dot, color, text) = match s.state.as_str() {
        _ if running => (SPINNER[app.tick % SPINNER.len()], YELLOW, "syncing".to_string()),
        "running" => ("●", RED, "interrupted".into()),
        "error" => ("●", RED, err(&s.error)),
        "ok" => ("●", GREEN, ago(s.at)),
        _ => ("●", FAINT, "never synced".into()),
    };
    let ic = if name == "gmail" { icon::GMAIL } else { icon::KEATS };
    vec![
        Span::styled(format!("{dot} "), color),
        Span::styled(format!("{ic} {name} "), DIM),
        Span::styled(text, if s.state == "error" { RED } else { FAINT }),
    ]
}

fn footer(f: &mut Frame, area: Rect) {
    let keys = [("↵", "open"), ("d", "done"), ("u", "undo"), ("r", "refresh"), ("tab", "panel"), ("j/k", "move"), ("q", "quit")];
    let spans: Vec<Span> = keys
        .iter()
        .flat_map(|(k, v)| [Span::styled(format!(" {k} "), Style::new().fg(FG).bg(SEL).bold()), Span::styled(format!(" {v}   "), DIM)])
        .collect();
    f.render_widget(Line::from(spans), area);
}

fn due_panel(f: &mut Frame, app: &mut App, area: Rect, compact: bool) {
    let focused = app.focus == 0;
    let items = &app.data.sections[0];
    let block = panel(icon::DUE, "due soon", items.len(), focused, None);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if items.is_empty() {
        return empty(f, inner, icon::OK, "nothing due, enjoy it", GREEN);
    }

    let w = inner.width.saturating_sub(2) as usize;
    let org_w = items.iter().map(|i| i.org.chars().count()).max().unwrap_or(0).min(18);
    let title_w = w.saturating_sub(6 + 2 + 13 + org_w + 2 + 2);
    let sel = app.tables[0].selected();
    let (mut rows, mut row_of, mut last) = (Vec::new(), Vec::new(), "");
    for (n, i) in items.iter().enumerate() {
        let b = bucket(i.due);
        if b != last {
            if !rows.is_empty() && !compact {
                rows.push(ListItem::new(""));
            }
            let color = if b == "overdue" { RED } else { DIM };
            rows.push(ListItem::new(Line::from(vec![
                Span::raw("  "),
                Span::styled(b, Style::new().fg(color).bold()),
                Span::styled(format!(" {}", "─".repeat(w.saturating_sub(b.len() + 1))), FAINT),
            ])));
            last = b;
        }
        row_of.push(rows.len());
        let text = if i.pending { FG } else { DIM };
        let kind_color = if i.gmail { PINK } else { TEAL };
        rows.push(ListItem::new(Line::from(vec![
            marker(sel == Some(n), focused),
            countdown(i),
            Span::raw("  "),
            Span::styled(fit(&format!("{} {}", kind_icon(i), i.tag), 13), kind_color),
            Span::styled(fit(&i.org, org_w), Style::new().fg(text).bold()),
            Span::raw("  "),
            Span::styled(fit(&i.title, title_w), text),
            Span::styled(format!(" {}", if i.gmail { icon::GMAIL } else { icon::KEATS }), FAINT),
        ]))
        .style(row_style(sel == Some(n), focused)));
    }
    app.views[0].select(sel.map(|s| row_of[s]));
    f.render_stateful_widget(List::new(rows), inner, &mut app.views[0]);
}

/// Selection as a row background (not a List highlight) so badge colours survive.
fn row_style(selected: bool, focused: bool) -> Style {
    if selected && focused { Style::new().bg(SEL) } else { Style::new() }
}

/// Announcements (idx 1) and forum threads (idx 2): two lines per item when
/// there's room for it, one line in narrow layouts.
fn feed_panel(f: &mut Frame, app: &mut App, idx: usize, area: Rect, two_line: bool) {
    let focused = app.focus == idx;
    let items = &app.data.sections[idx];
    let warn = if idx == 2 { app.data.forum_note.as_ref().map(|_| "session expired") } else { None };
    let (ic, name) = if idx == 1 { (icon::NEWS, "announcements") } else { (icon::FORUM, "q&a forums") };
    let block = panel(ic, name, items.len(), focused, warn);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if items.is_empty() {
        let text = if warn.is_some() { "run dash login" } else { "quiet for now" };
        return empty(f, inner, if warn.is_some() { icon::ALERT } else { icon::OK }, text, FAINT);
    }

    let w = inner.width.saturating_sub(2) as usize;
    let sel = app.tables[idx].selected();
    let rows: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(n, i)| {
            let m = || marker(sel == Some(n), focused);
            let date = local(i.when.unwrap_or(0), "%d %b");
            let right = if idx == 1 { date.clone() } else { i.note.clone() };
            let right_color = if i.note.contains("solved") { GREEN } else { DIM };
            if two_line {
                let org_w = w.saturating_sub(right.chars().count() + 1);
                ListItem::new(Text::from(vec![
                    Line::from(vec![m(), Span::styled(fit(&i.org, org_w), TEAL), Span::raw(" "), Span::styled(right, right_color)]),
                    Line::from(vec![m(), Span::styled(fit(&i.title, w), FG)]),
                ]))
                .style(row_style(sel == Some(n), focused))
            } else {
                let org_w = 14;
                let note_w = if idx == 2 { 18 } else { 0 };
                let title_w = w.saturating_sub(6 + 2 + org_w + 2 + note_w);
                let mut spans = vec![
                    m(),
                    Span::styled(fit(&date, 6), DIM),
                    Span::raw("  "),
                    Span::styled(fit(&i.org, org_w), TEAL),
                    Span::raw("  "),
                    Span::styled(fit(&i.title, title_w), FG),
                ];
                if idx == 2 {
                    spans.push(Span::styled(format!("{:>18}", fit(&i.note, 18).trim_end()), right_color));
                }
                ListItem::new(Line::from(spans)).style(row_style(sel == Some(n), focused))
            }
        })
        .collect();
    app.views[idx].select(sel);
    f.render_stateful_widget(List::new(rows), inner, &mut app.views[idx]);
}

fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split('/').next().unwrap_or(rest)
}

fn details(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(FAINT)
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let Some(i) = app.selected() else { return };

    let sep = || Span::styled("  ·  ", FAINT);
    let mut meta = vec![Span::styled(i.org.clone(), TEAL)];
    if app.focus == 0 {
        meta.extend([sep(), Span::styled(format!("{} {}", kind_icon(i), i.tag), if i.gmail { PINK } else { TEAL })]);
        if let Some(d) = i.due {
            let rel = if d < now() { format!("{} ago", span_text(d - now())) } else { format!("in {}", span_text(d - now())) };
            meta.extend([
                sep(),
                Span::styled(format!("due {}  ", local(d, "%a %d %b %H:%M").to_lowercase()), FG),
                Span::styled(rel, Style::new().fg(urgency(d)).bold()),
            ]);
        }
        if !i.pending {
            meta.extend([sep(), Span::styled("no action pending on keats", DIM)]);
        }
    } else if let Some(w) = i.when {
        meta.extend([sep(), Span::styled(local(w, "%a %d %b %H:%M").to_lowercase(), DIM)]);
    }
    let src = if i.gmail { format!("{} gmail", icon::GMAIL) } else { format!("{} keats", icon::KEATS) };
    meta.extend([sep(), Span::styled(src, DIM)]);

    let mut lines = vec![Line::styled(i.title.clone(), Style::new().fg(FG).bold()), Line::from(meta)];
    if app.focus == 1 && !i.note.is_empty() {
        lines.push(Line::styled(i.note.clone(), DIM));
    } else {
        let mut last = Vec::new();
        if app.focus == 0 && !i.note.is_empty() {
            last.extend([Span::styled(i.note.clone(), YELLOW), sep()]);
        }
        if !i.url.is_empty() {
            last.extend([Span::styled(format!("{} ", icon::LINK), FAINT), Span::styled(host(&i.url).to_string(), FAINT)]);
        }
        lines.push(Line::from(last));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

pub fn draw(f: &mut Frame, app: &mut App) {
    f.render_widget(Block::new().style(Style::new().bg(BG).fg(FG)), f.area());
    let area = f.area().inner(Margin::new(2, 1));
    let [top, sub, _, body, _, bottom] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    header(f, app, top, sub);
    footer(f, bottom);

    let wide = body.width >= WIDE;
    let details_h = if body.height >= if wide { 20 } else { 32 } { 5 } else { 0 };
    let [main, det] = Layout::vertical([Constraint::Fill(1), Constraint::Length(details_h)]).areas(body);
    if details_h > 0 {
        details(f, app, det);
    }

    if wide {
        let [left, right] = Layout::horizontal([Constraint::Percentage(58), Constraint::Fill(1)]).spacing(1).areas(main);
        let forum_h = (app.data.sections[2].len() as u16 * 2 + 2).clamp(4, right.height / 2);
        let [news, forum] = Layout::vertical([Constraint::Fill(1), Constraint::Length(forum_h)]).areas(right);
        due_panel(f, app, left, false);
        feed_panel(f, app, 1, news, true);
        feed_panel(f, app, 2, forum, true);
    } else {
        // Stacked: due gets what it needs (items + group headers), feeds share the rest.
        let due = &app.data.sections[0];
        let groups = due.iter().map(|i| bucket(i.due)).collect::<std::collections::BTreeSet<_>>().len() as u16;
        let due_need = (due.len() as u16 + groups).max(1) + 2;
        let forum_h = (app.data.sections[2].len() as u16 + 2).clamp(3, 6);
        let due_h = due_need.min(main.height.saturating_sub(forum_h + 4));
        let [d, n, fo] = Layout::vertical([Constraint::Length(due_h), Constraint::Fill(1), Constraint::Length(forum_h)]).areas(main);
        due_panel(f, app, d, true);
        feed_panel(f, app, 1, n, false);
        feed_panel(f, app, 2, fo, false);
    }
}
