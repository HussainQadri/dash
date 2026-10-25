use crate::app::App;
use crate::fx::lerp;
use crate::model::{CalEvent, Item, days_from_today, local, now};
use crate::theme::*;
use crate::ui::{day_label, fit, hm, icon, panel, short_org, span_text, urgency};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

const GUTTER: u16 = 6;
const GRID: Color = Color::Rgb(0x1e, 0x21, 0x23);
const BLUE: Color = Color::Rgb(0x8f, 0xb4, 0xf0);
const PURPLE: Color = Color::Rgb(0xc6, 0xa0, 0xf6);
const ORANGE: Color = Color::Rgb(0xf0, 0xa8, 0x6c);
const SKY: Color = Color::Rgb(0x9a, 0xd0, 0xe6);

/// Personal (non-timetable) events are pink; each module gets its own colour,
/// assigned in code order so they don't collide.
fn color(e: &CalEvent, modules: &[&str]) -> Color {
    const PALETTE: [Color; 7] = [TEAL, YELLOW, GREEN, BLUE, PURPLE, ORANGE, SKY];
    match modules.iter().position(|m| *m == e.org) {
        Some(i) if !e.kind.is_empty() => PALETTE[i % PALETTE.len()],
        _ => PINK,
    }
}

fn modules(app: &App) -> Vec<&str> {
    let mut m: Vec<&str> = app.data.events.iter().filter(|e| !e.kind.is_empty()).map(|e| e.org.as_str()).collect();
    m.sort_unstable();
    m.dedup();
    m
}

/// "Bush House Lecture Theatre 1 BH(S)1.01" -> "BH(S)1.01", "STRAND BLDG STR LG206" -> "STR LG206".
pub fn room(location: &str) -> String {
    let words: Vec<&str> = location.split_whitespace().filter(|w| *w != "(Lab)").collect();
    if let Some(i) = words.iter().position(|w| w.starts_with("BH")) {
        return words[i..].concat();
    }
    if location.starts_with("Bush House") {
        return format!("BH{}", words.get(2..).map(|w| w.concat()).unwrap_or_default());
    }
    if location == location.to_uppercase() && words.len() > 2 {
        return words[words.len() - 2..].join(" ");
    }
    location.to_string()
}

fn label(e: &CalEvent) -> String {
    if e.kind.is_empty() { e.title.clone() } else { format!("{} {}", short_org(&e.org), e.kind) }
}

/// Full-screen week: a column per day, hours down the side, events as blocks.
pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    let (start, events) = app.data.week(app.week);
    let days: Vec<i64> = (0..7).map(|d| crate::model::days_start(start, d)).collect();
    // Weekends only when something is on.
    let shown = if events.iter().any(|e| e.start >= days[5]) { 7 } else { 5 };
    let title = format!(
        "{}  ·  {} – {}",
        match app.week {
            -1 => "last week".to_string(),
            0 => "this week".to_string(),
            1 => "next week".to_string(),
            _ => local(start, "week %V"),
        },
        local(start, "%d %b").to_lowercase(),
        local(days[shown - 1], "%d %b").to_lowercase()
    );
    let block = panel(icon::CAL, &title, events.len(), true, None)
        .title_bottom(Line::styled(" ‹ h   l › ", FAINT).right_aligned());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let col_w = inner.width.saturating_sub(GUTTER) / shown as u16;
    let x_of = |d: usize| inner.x + GUTTER + d as u16 * col_w;
    let pick = app.pick.clone();
    let modules = modules(app);
    let mut picked = None;

    // Day names, then a strip of deadlines and all-day events per day.
    let due: Vec<&Item> = app.data.sections[0].iter().filter(|i| i.due.is_some_and(|d| d >= start && d < days[6] + 86400)).collect();
    let mut strip: Vec<Vec<Line>> = vec![Vec::new(); shown];
    for (d, &day) in days.iter().take(shown).enumerate() {
        let today = days_from_today(day) == 0;
        let name = Line::from(vec![
            Span::styled(local(day, "%a ").to_lowercase(), if today { Style::new().fg(TEAL).bold() } else { Style::new().fg(DIM) }),
            Span::styled(local(day, "%d"), if today { Style::new().fg(TEAL).bold() } else { Style::new().fg(FG) }),
        ]);
        f.render_widget(name, Rect { x: x_of(d) + 1, y: inner.y, width: col_w.saturating_sub(1), height: 1 });
        for i in due.iter().filter(|i| days_from_today(i.due.unwrap_or(0)) == days_from_today(day)) {
            let when = i.due.unwrap_or(0);
            let color = if i.pending { urgency(when) } else { DIM };
            let text = format!("◆ {} {} {}", hm(when), short_org(&i.org), i.tag);
            strip[d].push(Line::styled(fit(&text, col_w.saturating_sub(2) as usize), color));
        }
        for e in events.iter().filter(|e| e.all_day && days_from_today(e.start) == days_from_today(day)) {
            strip[d].push(Line::styled(fit(&format!("▪ {}", label(e)), col_w.saturating_sub(2) as usize), color(e, &modules)));
        }
    }
    let strip_h = strip.iter().map(Vec::len).max().unwrap_or(0).min(3) as u16;
    for (d, lines) in strip.into_iter().enumerate() {
        let r = Rect { x: x_of(d) + 1, y: inner.y + 1, width: col_w.saturating_sub(1), height: strip_h };
        f.render_widget(Paragraph::new(lines), r);
    }

    // Hours: 09–18 at least, stretched to fit the week's earliest and latest events.
    let timed: Vec<&CalEvent> = events.iter().copied().filter(|e| !e.all_day).collect();
    let hour = |ts: i64| local(ts, "%H").parse::<i64>().unwrap_or(0);
    let h0 = timed.iter().map(|e| hour(e.start)).min().unwrap_or(9).min(9);
    let h1 = timed.iter().map(|e| hour(e.end - 1) + 1).max().unwrap_or(18).max(18);
    let grid_y = inner.y + 2 + strip_h;
    let grid_h = inner.bottom().saturating_sub(grid_y);
    let per_hour = (grid_h as i64 / (h1 - h0).max(1)).clamp(1, 4);
    let y_of = |ts: i64, day: i64| {
        let mins = (ts - day) / 60 - h0 * 60;
        grid_y as i64 + mins * per_hour / 60
    };

    for h in h0..h1 {
        let y = grid_y as i64 + (h - h0) * per_hour;
        if y >= inner.bottom() as i64 {
            break;
        }
        let y = y as u16;
        f.render_widget(Span::styled(format!("{h:02}:00"), FAINT), Rect { x: inner.x, y, width: 5, height: 1 });
        let rule = "╌".repeat((col_w * shown as u16).saturating_sub(1) as usize);
        f.render_widget(Span::styled(rule, GRID), Rect { x: x_of(0) + 1, y, width: col_w * shown as u16, height: 1 });
    }

    // Blocks; events that overlap share the column in lanes.
    for (d, &day) in days.iter().take(shown).enumerate() {
        let mut todays: Vec<&CalEvent> = timed.iter().copied().filter(|e| e.start >= day && e.start < day + 86400 + 3600).collect();
        todays.retain(|e| days_from_today(e.start) == days_from_today(day));
        let lanes = lanes(&todays);
        for (e, (lane, of)) in todays.iter().zip(lanes) {
            let w = (col_w.saturating_sub(1)) / of as u16;
            let (y0, y1) = (y_of(e.start, day), y_of(e.end, day));
            let y0 = y0.clamp(grid_y as i64, inner.bottom() as i64 - 1) as u16;
            let y1 = y1.clamp(y0 as i64 + 1, inner.bottom() as i64) as u16;
            let r = Rect { x: x_of(d) + 1 + lane as u16 * w, y: y0, width: w.saturating_sub(1).max(1), height: y1 - y0 };
            let key = format!("cal:{}:{}", e.uid, e.start);
            let selected = pick.as_deref() == Some(key.as_str());
            event_block(f, app, e, r, selected, &key, color(e, &modules));
            if selected {
                picked = Some(r);
            }
        }
    }

    // A red line at the current time across today's column.
    let n = now();
    if let Some(d) = days.iter().take(shown).position(|&day| days_from_today(day) == 0) {
        let y = y_of(n, days[d]);
        if y >= grid_y as i64 && y < inner.bottom() as i64 {
            let y = y as u16;
            f.render_widget(Span::styled(hm(n), Style::new().fg(RED).bold()), Rect { x: inner.x, y, width: 5, height: 1 });
            let line = format!("●{}", "━".repeat(col_w.saturating_sub(2) as usize));
            f.render_widget(Span::styled(line, RED), Rect { x: x_of(d), y, width: col_w, height: 1 });
        }
    }
    app.rects.row = picked;
}

/// (lane, lanes in its overlap cluster) for events sorted by start.
fn lanes(events: &[&CalEvent]) -> Vec<(usize, usize)> {
    let mut out = vec![(0, 1); events.len()];
    let mut i = 0;
    while i < events.len() {
        let (mut j, mut end, mut ends) = (i, events[i].end, Vec::<i64>::new());
        while j < events.len() && (j == i || events[j].start < end) {
            let lane = ends.iter().position(|&e| e <= events[j].start).unwrap_or(ends.len());
            if lane == ends.len() {
                ends.push(events[j].end);
            } else {
                ends[lane] = events[j].end;
            }
            out[j].0 = lane;
            end = end.max(events[j].end);
            j += 1;
        }
        for o in &mut out[i..j] {
            o.1 = ends.len();
        }
        i = j;
    }
    out
}

fn event_block(f: &mut Frame, app: &App, e: &CalEvent, r: Rect, selected: bool, key: &str, c: Color) {
    let n = now();
    let (on, past) = (e.start <= n && n < e.end, e.end <= n);
    let mut bg = lerp(BG, c, if selected { 0.42 } else if on { 0.32 } else if past { 0.08 } else { 0.18 });
    if let Some(g) = app.glows.get(key) {
        bg = g.over(bg);
    }
    let text = if past && !selected { DIM } else { FG };
    let w = r.width.saturating_sub(1) as usize;
    let mut lines = vec![Line::from(vec![
        Span::styled(fit(&label(e), w), Style::new().fg(if past && !selected { DIM } else { c }).bold()),
    ])];
    let mut more = vec![format!("{}–{}", hm(e.start), hm(e.end))];
    if !e.location.is_empty() {
        more.insert(0, room(&e.location));
    }
    if !e.note.is_empty() {
        more.push(e.note.clone());
    }
    lines.extend(more.into_iter().map(|t| Line::styled(fit(&t, w), text)));
    let bar = border::Set { vertical_left: "▌", ..border::PLAIN };
    let block = Block::new()
        .borders(Borders::LEFT)
        .border_set(bar)
        .border_style(Style::new().fg(if selected { FG } else { c }).bg(bg))
        .style(Style::new().bg(bg));
    f.render_widget(Paragraph::new(lines).block(block), r);
}

/// The picked event, full text: title, module, time, room, staff.
pub fn details(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::bordered().border_type(ratatui::widgets::BorderType::Rounded).border_style(FAINT).padding(ratatui::widgets::Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let (_, events) = app.data.week(app.week);
    let Some(e) = events.iter().find(|e| app.pick.as_deref() == Some(format!("cal:{}:{}", e.uid, e.start).as_str())) else {
        let hint = if events.is_empty() { "nothing this week" } else { "j/k to pick an event" };
        return f.render_widget(Line::styled(hint, FAINT), inner);
    };
    let sep = || Span::styled("  ·  ", FAINT);
    let n = now();
    let rel = match () {
        _ if e.start <= n && n < e.end => Span::styled("on now", Style::new().fg(TEAL).bold()),
        _ if e.end <= n => Span::styled("finished", DIM),
        _ => Span::styled(format!("in {}", span_text(e.start - n)), Style::new().fg(TEAL).bold()),
    };
    let time = if e.all_day { "all day".to_string() } else { format!("{}–{}", hm(e.start), hm(e.end)) };
    let mut meta = vec![Span::styled(e.org.clone(), color(e, &modules(app)))];
    if !e.kind.is_empty() {
        meta.extend([sep(), Span::styled(e.kind.clone(), FG)]);
    }
    meta.extend([sep(), Span::styled(format!("{} {time}  ", day_label(e.start)), FG), rel, sep(), Span::styled(format!("{} {}", icon::CAL, e.source), DIM)]);
    let mut last = Vec::new();
    if !e.location.is_empty() {
        last.push(Span::styled(e.location.clone(), YELLOW));
    }
    if !e.note.is_empty() {
        if !last.is_empty() {
            last.push(sep());
        }
        last.push(Span::styled(e.note.clone(), DIM));
    }
    let lines = vec![Line::styled(e.title.clone(), Style::new().fg(FG).bold()), Line::from(meta), Line::from(last)];
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

/// One line for the main page: the class on now, else the next one.
pub fn next_line(app: &App) -> Option<Line<'static>> {
    let n = now();
    let e = app.data.events.iter().filter(|e| !e.all_day && e.end > n).min_by_key(|e| e.start)?;
    let on = e.start <= n;
    let mut spans = vec![
        Span::styled(format!("{} ", icon::CAL), if on { TEAL } else { DIM }),
        Span::styled(if on { "on now  " } else { "next  " }, if on { Style::new().fg(TEAL).bold() } else { Style::new().fg(FAINT) }),
        Span::styled(label(e), Style::new().fg(color(e, &modules(app))).bold()),
    ];
    if !e.location.is_empty() {
        spans.extend([Span::styled("  ·  ", FAINT), Span::styled(room(&e.location), FG)]);
    }
    let when = if on {
        format!("until {}", hm(e.end))
    } else if days_from_today(e.start) == 0 {
        hm(e.start)
    } else {
        format!("{} {}", day_label(e.start), hm(e.start))
    };
    spans.extend([Span::styled("  ·  ", FAINT), Span::styled(when, DIM)]);
    if !on {
        spans.extend([Span::styled("  ·  ", FAINT), Span::styled(format!("in {}", span_text(e.start - n)), Style::new().fg(TEAL).bold())]);
    }
    Some(Line::from(spans))
}
