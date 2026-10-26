use crate::app::App;
use crate::fx::{self, lerp, wave};
use crate::week;
use crate::model::{Item, SourceStatus, days_from_today, local, now};
use crate::theme::*;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, List, ListItem, ListState, Padding, Paragraph, Wrap};

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const WIDE: u16 = 120;

pub mod icon {
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
pub fn fit(s: &str, w: usize) -> String {
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

pub fn span_text(d: i64) -> String {
    match d.abs() {
        a if a < 3600 => format!("{}m", a / 60),
        a if a < 86400 => format!("{}h", a / 3600),
        a => format!("{}d", a / 86400),
    }
}

/// Countdown badge: under a day is a solid red pill, then red, yellow, grey.
fn countdown(i: &Item) -> Span<'static> {
    if i.email_done() {
        return Span::styled(" done ", Style::new().fg(GREEN).bold());
    }
    let Some(due) = i.due else { return Span::styled("    — ", FAINT) };
    let d = due - now();
    let text = format!(" {:>4} ", if d < 0 { format!("-{}", span_text(d)) } else { span_text(d) });
    if d < 0 && i.pending && let Some(g) = glitch(&i.id) {
        return Span::styled(g.apply(&text), Style::new().fg(g.color).bold());
    }
    let style = match d {
        _ if !i.pending => Style::new().fg(DIM),
        d if d < 0 => Style::new().fg(RED).bold(),
        d if d <= 86400 => Style::new().fg(BG).bg(lerp(RED, Color::Rgb(0x9e, 0x3f, 0x3f), wave(1800))).bold(),
        d if d <= 3 * 86400 => Style::new().fg(RED).bold(),
        d if d <= 7 * 86400 => Style::new().fg(YELLOW).bold(),
        _ => Style::new().fg(DIM),
    };
    Span::styled(text, style)
}

struct Glitch {
    frame: usize,
    color: ratatui::style::Color,
}

impl Glitch {
    fn apply(&self, text: &str) -> String {
        const NOISE: [char; 6] = ['▚', '▞', '░', '▒', '¦', '╳'];
        text.chars()
            .enumerate()
            .map(|(n, c)| if c != ' ' && (n + self.frame).is_multiple_of(3) { NOISE[(n + self.frame) % NOISE.len()] } else { c })
            .collect()
    }
}

/// Overdue badges glitch for ~150ms every couple of seconds, staggered per item.
fn glitch(id: &str) -> Option<Glitch> {
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as usize;
    let offset = id.bytes().map(usize::from).sum::<usize>() * 37;
    let phase = (ms + offset) % 2600;
    (phase < 150).then(|| Glitch { frame: ms / 40, color: if (ms / 50).is_multiple_of(2) { PINK } else { RED } })
}

pub fn urgency(due: i64) -> ratatui::style::Color {
    match due - now() {
        d if d <= 3 * 86400 => RED,
        d if d <= 7 * 86400 => YELLOW,
        _ => DIM,
    }
}

fn bucket(i: &Item) -> &'static str {
    if i.email_done() {
        return "done from email";
    }
    match i.due {
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

pub fn panel<'a>(ic: &'a str, name: &'a str, count: usize, focused: bool, warn: Option<&'a str>) -> Block<'a> {
    let mut title = vec![
        Span::styled(format!(" {ic} "), if focused { TEAL } else { DIM }),
        Span::styled(format!("{name} "), if focused { Style::new().fg(FG).bold() } else { Style::new().fg(DIM) }),
    ];
    if let Some(w) = warn {
        title.push(Span::styled(format!("{} {w} ", icon::ALERT), YELLOW));
    }
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(if focused { lerp(DIM, TEAL, 0.55 * wave(4200)) } else { FAINT })
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
    right.push(Span::raw("    "));
    right.extend(source("calendar", &s.calendar, app));
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
        _ if running => (SPINNER[app.tick() % SPINNER.len()], YELLOW, "syncing".to_string()),
        "running" => ("●", RED, "interrupted".into()),
        "error" => ("●", RED, err(&s.error)),
        "ok" => ("●", GREEN, ago(s.at)),
        _ => ("●", FAINT, "never synced".into()),
    };
    let ic = match name {
        "gmail" => icon::GMAIL,
        "calendar" => icon::CAL,
        _ => icon::KEATS,
    };
    let mut spans = vec![Span::styled(format!("{dot} "), color), Span::styled(format!("{ic} {name} "), DIM)];
    if running {
        spans.extend(shimmer(&text));
    } else {
        spans.push(Span::styled(text, if s.state == "error" { RED } else { FAINT }));
    }
    spans
}

/// A highlight that runs across `text` and loops.
fn shimmer(text: &str) -> Vec<Span<'static>> {
    let n = text.chars().count() as f32;
    let head = wave(1400) * (n + 6.0) - 3.0;
    text.chars()
        .enumerate()
        .map(|(i, c)| {
            let t = 1.0 - ((i as f32 - head).abs() / 3.0).min(1.0);
            Span::styled(c.to_string(), lerp(DIM, YELLOW, t))
        })
        .collect()
}

/// Where row `row` of a list landed on screen, given the list's scroll offset.
fn row_rect(inner: Rect, row: usize, offset: usize) -> Option<Rect> {
    let y = row.checked_sub(offset)?;
    (y < inner.height as usize).then(|| Rect { y: inner.y + y as u16, height: 1, ..inner })
}

fn footer(f: &mut Frame, area: Rect, calendar: bool) {
    let keys: &[(&str, &str)] = if calendar {
        &[("h/l", "week"), ("t", "this week"), ("j/k", "event"), ("c", "back"), ("r", "refresh"), ("q", "quit")]
    } else {
        &[("↵", "open"), ("d", "done"), ("u", "undo"), ("c", "calendar"), ("r", "refresh"), ("tab", "panel"), ("j/k", "move"), ("q", "quit")]
    };
    let spans: Vec<Span> = keys
        .iter()
        .flat_map(|(k, v)| [Span::styled(format!(" {k} "), Style::new().fg(FG).bg(SEL).bold()), Span::styled(format!(" {v}   "), DIM)])
        .collect();
    f.render_widget(Line::from(spans), area);
}

/// "5CCS2PEP" -> "PEP"; names that aren't module codes are left alone.
pub fn short_org(org: &str) -> &str {
    let b = org.as_bytes();
    let run = |from: usize, f: fn(&u8) -> bool| b[from..].iter().take_while(|c| f(c)).count();
    let year = run(0, u8::is_ascii_digit);
    let dept = run(year, u8::is_ascii_uppercase);
    let level = run(year + dept, u8::is_ascii_digit);
    let end = year + dept + level;
    if year == 0 || dept == 0 || level == 0 || end == b.len() { org } else { &org[end..] }
}

fn width(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.content.chars().count()).sum()
}

/// `  label ───────── right`, spanning the row width `w` (after the marker).
fn group_header(label: &str, color: Color, right: Vec<Span<'static>>, w: usize) -> ListItem<'static> {
    let gap = if right.is_empty() { 1 } else { 2 };
    let rule = w.saturating_sub(label.chars().count() + width(&right) + gap);
    let mut spans = vec![
        Span::raw("  "),
        Span::styled(label.to_string(), Style::new().fg(color).bold()),
        Span::styled(format!(" {}{}", "─".repeat(rule), if right.is_empty() { "" } else { " " }), FAINT),
    ];
    spans.extend(right);
    ListItem::new(Line::from(spans))
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
    let org_w = items.iter().map(|i| short_org(&i.org).chars().count()).max().unwrap_or(0).min(18);
    let title_w = w.saturating_sub(6 + 2 + 2 + org_w + 2 + 11);
    let sel = app.tables[0].selected();
    let (mut rows, mut row_of, mut last, mut last_org) = (Vec::new(), Vec::new(), "", "");
    let (mut header_of, mut header) = (Vec::new(), 0);
    for (n, i) in items.iter().enumerate() {
        let b = bucket(i);
        if b != last {
            if !rows.is_empty() && !compact {
                rows.push(ListItem::new(""));
            }
            header = rows.len();
            let color = match b {
                "overdue" => RED,
                "done from email" => GREEN,
                _ => DIM,
            };
            rows.push(group_header(b, color, vec![], w));
            (last, last_org) = (b, "");
        }
        header_of.push(header);
        row_of.push(rows.len());
        let text = if i.pending { FG } else { DIM };
        let org = short_org(&i.org);
        let org_span = if org == last_org {
            Span::styled(fit("·", org_w), FAINT)
        } else {
            Span::styled(fit(org, org_w), Style::new().fg(text).bold())
        };
        last_org = org;
        let date = i.due.map(|d| local(d, "%a %d %b").to_lowercase()).unwrap_or_default();
        rows.push(ListItem::new(Line::from(vec![
            marker(sel == Some(n), focused),
            countdown(i),
            Span::raw("  "),
            Span::styled(format!("{} ", kind_icon(i)), if i.gmail { PINK } else { TEAL }),
            org_span,
            Span::raw("  "),
            Span::styled(fit(&i.title, title_w), text),
            Span::styled(format!(" {date:>10}"), DIM),
        ]))
        .style(row_style(app, i, sel == Some(n), focused)));
    }
    app.views[0].select(sel.map(|s| row_of[s]));
    keep_header(&mut app.views[0], sel.map(|s| header_of[s]));
    f.render_stateful_widget(List::new(rows), inner, &mut app.views[0]);
    if focused && let Some(s) = sel {
        app.rects.row = row_rect(inner, row_of[s], app.views[0].offset());
    }
}

/// Scroll up far enough to show the selected item's group header. The list only
/// scrolls to the selected row itself, which leaves the header hidden after `gg`;
/// if the group is too long, rendering scrolls back down to the selection.
fn keep_header(view: &mut ListState, header: Option<usize>) {
    if let Some(h) = header {
        let offset = view.offset_mut();
        *offset = (*offset).min(h);
    }
}

/// Selection as a row background (not a List highlight) so badge colours survive,
/// with any glow on the item mixed in.
fn row_style(app: &App, item: &Item, selected: bool, focused: bool) -> Style {
    let base = if selected && focused { SEL } else { BG };
    match app.glows.get(&item.key()) {
        Some(g) => Style::new().bg(g.over(base)),
        None if selected && focused => Style::new().bg(SEL),
        None => Style::new(),
    }
}

pub fn day_label(ts: i64) -> String {
    match days_from_today(ts) {
        0 => "today".into(),
        1 => "tomorrow".into(),
        _ => local(ts, "%a %d %b").to_lowercase(),
    }
}

pub fn hm(ts: i64) -> String {
    local(ts, "%H:%M")
}

fn news_panel(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == 1;
    let items = &app.data.sections[1];
    let block = panel(icon::NEWS, "announcements", items.len(), focused, None);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if items.is_empty() {
        return empty(f, inner, icon::OK, "quiet for now", FAINT);
    }

    let w = inner.width.saturating_sub(2) as usize;
    let org_w = items.iter().map(|i| short_org(&i.org).chars().count()).max().unwrap_or(0).min(14);
    let sel = app.tables[1].selected();
    let rows: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(n, i)| {
            ListItem::new(Line::from(vec![
                marker(sel == Some(n), focused),
                Span::styled(local(i.when.unwrap_or(0), "%d %b").to_lowercase(), DIM),
                Span::raw("  "),
                Span::styled(fit(short_org(&i.org), org_w), TEAL),
                Span::raw("  "),
                Span::styled(fit(&i.title, w.saturating_sub(6 + 2 + org_w + 2)), FG),
            ]))
            .style(row_style(app, i, sel == Some(n), focused))
        })
        .collect();
    app.views[1].select(sel);
    f.render_stateful_widget(List::new(rows), inner, &mut app.views[1]);
    if focused && let Some(s) = sel {
        app.rects.row = row_rect(inner, s, app.views[1].offset());
    }
}

/// Q&A threads under one header per module (items arrive grouped, see `Data::load`).
fn forum_panel(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == 2;
    let items = &app.data.sections[2];
    let warn = app.data.forum_note.as_ref().map(|_| "session expired");
    let block = panel(icon::FORUM, "q&a forums", items.len(), focused, warn);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if items.is_empty() {
        let text = if warn.is_some() { "run hud login" } else { "quiet for now" };
        return empty(f, inner, if warn.is_some() { icon::ALERT } else { icon::OK }, text, FAINT);
    }

    let w = inner.width.saturating_sub(2) as usize;
    let note_w = items.iter().map(|i| i.note.chars().count()).max().unwrap_or(0);
    let title_w = w.saturating_sub(2 + note_w + 2 + 2 + 6);
    let unanswered = |i: &Item| i.note.starts_with("0 ");
    let sel = app.tables[2].selected();
    let (mut rows, mut row_of) = (Vec::new(), Vec::new());
    let (mut header_of, mut header) = (Vec::new(), 0);
    for (n, i) in items.iter().enumerate() {
        if n == 0 || items[n - 1].org != i.org {
            header = rows.len();
            let (total, open) = app.data.forum_counts.get(&i.org).copied().unwrap_or_default();
            let mut right = vec![Span::styled(format!("{total} thread{}", if total == 1 { "" } else { "s" }), FAINT)];
            if open > 0 {
                right.extend([Span::styled(" · ", FAINT), Span::styled(format!("{open} unanswered"), YELLOW)]);
            }
            rows.push(group_header(short_org(&i.org), TEAL, right, w));
        }
        header_of.push(header);
        row_of.push(rows.len());
        let note_color = if i.note.contains("solved") {
            GREEN
        } else if unanswered(i) {
            YELLOW
        } else {
            DIM
        };
        rows.push(ListItem::new(Line::from(vec![
            marker(sel == Some(n), focused),
            Span::raw("  "),
            Span::styled(fit(&i.title, title_w), FG),
            Span::styled(format!("  {:>note_w$}", i.note), note_color),
            Span::styled(format!("  {}", local(i.when.unwrap_or(0), "%d %b").to_lowercase()), DIM),
        ]))
        .style(row_style(app, i, sel == Some(n), focused)));
    }
    app.views[2].select(sel.map(|s| row_of[s]));
    keep_header(&mut app.views[2], sel.map(|s| header_of[s]));
    f.render_stateful_widget(List::new(rows), inner, &mut app.views[2]);
    if focused && let Some(s) = sel {
        app.rects.row = row_rect(inner, row_of[s], app.views[2].offset());
    }
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
                Span::styled(rel, Style::new().fg(if i.pending { urgency(d) } else { DIM }).bold()),
            ]);
        }
        if i.email_done() {
            meta.extend([sep(), Span::styled("done, found in email", GREEN)]);
        } else if !i.pending {
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

/// Rows the due list needs: items, a header per bucket, and gaps between buckets.
fn due_rows(app: &App, compact: bool) -> u16 {
    let due = &app.data.sections[0];
    let groups = due.iter().map(bucket).collect::<std::collections::BTreeSet<_>>().len() as u16;
    let gaps = if compact { 0 } else { groups.saturating_sub(1) };
    (due.len() as u16 + groups + gaps).max(1)
}

pub fn draw(f: &mut Frame, app: &mut App) {
    f.render_widget(Block::new().style(Style::new().bg(BG).fg(FG)), f.area());
    let area = f.area().inner(Margin::new(2, 1));
    let [top, sub, class, _, body, _, bottom] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    header(f, app, top, sub);
    if let Some(line) = week::next_line(app) {
        f.render_widget(line, class);
    }
    footer(f, bottom, app.calendar);
    app.rects = fx::Rects::default();

    if app.calendar {
        let [grid, det] = Layout::vertical([Constraint::Fill(1), Constraint::Length(5)]).areas(body);
        week::draw(f, app, grid);
        week::details(f, app, det);
        return fx::apply(f, app);
    }

    let wide = body.width >= WIDE;
    let details_h = if body.height >= if wide { 20 } else { 32 } { 5 } else { 0 };
    let [main, det] = Layout::vertical([Constraint::Fill(1), Constraint::Length(details_h)]).areas(body);
    if details_h > 0 {
        details(f, app, det);
    }

    if wide {
        let [left, right] = Layout::horizontal([Constraint::Percentage(55), Constraint::Fill(1)]).spacing(1).areas(main);
        let news_h = (app.data.sections[1].len() as u16 + 2).clamp(3, right.height / 2);
        let [news, forum] = Layout::vertical([Constraint::Length(news_h), Constraint::Fill(1)]).areas(right);
        app.rects.panels = [left, news, forum];
        due_panel(f, app, left, false);
        news_panel(f, app, news);
        forum_panel(f, app, forum);
    } else {
        // Stacked: due gets what it needs (items + group headers), feeds share the rest.
        let news_h = (app.data.sections[1].len() as u16 + 2).clamp(3, 6);
        let due_h = (due_rows(app, true) + 2).min(main.height.saturating_sub(news_h + 4));
        let [d, n, fo] = Layout::vertical([Constraint::Length(due_h), Constraint::Length(news_h), Constraint::Fill(1)]).areas(main);
        app.rects.panels = [d, n, fo];
        due_panel(f, app, d, true);
        news_panel(f, app, n);
        forum_panel(f, app, fo);
    }
    fx::apply(f, app);
}
