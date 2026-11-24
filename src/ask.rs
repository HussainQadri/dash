//! The ask pane: questions about the synced data, answered by headless Claude
//! reading `data_dir()` with read-only tools. Follow-ups resume the session.

use crate::model::*;
use crate::theme::*;
use crate::ui::{SPINNER, icon, panel, short_org};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Padding, Paragraph};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};
use std::{env, thread};

const PROMPT: &str = include_str!("../prompts/ask.md");
const CODE: Color = Color::Rgb(0xc9, 0xd1, 0xd9);

/// Read-only: file tools are confined to the data dir by --restricted, and the
/// only shell commands allowed are for looking inside zips and PDFs.
const TOOLS: &str = "Read,Grep,Glob,Bash";
const ALLOWED: &str = "Read,Grep,Glob,Bash(unzip -l:*),Bash(pdftotext:*)";

pub const MODELS: [&str; 2] = ["sonnet", "opus"];

/// $HUD_ASK_MODEL, else the model last picked in the pane, else sonnet.
pub fn default_model() -> String {
    env::var("HUD_ASK_MODEL")
        .ok()
        .or_else(|| read_json::<State>(&state_file()).ask_model)
        .unwrap_or_else(|| MODELS[0].into())
}

/// `claude` set up to read the data dir; the caller adds -p etc.
fn claude(model: &str) -> Command {
    let mut cmd = Command::new(crate::sync::claude_bin());
    cmd.current_dir(data_dir())
        .args(["--restricted", "--strict-mcp-config", "--model", model])
        .args(["--tools", TOOLS, "--allowedTools", ALLOWED])
        .args(["--append-system-prompt", PROMPT]);
    cmd
}

pub enum Event {
    Session(String),
    /// A new assistant message began: text so far was a preamble to tool calls.
    Message,
    Text(String),
    Tool(String),
    Done(Result<String, String>),
}

pub struct Run {
    child: Child,
    rx: Receiver<Event>,
}

impl Run {
    pub fn recv(&self) -> Option<Event> {
        self.rx.recv().ok()
    }

    fn try_recv(&self) -> Result<Event, TryRecvError> {
        self.rx.try_recv()
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Starts headless Claude on `message`, streaming events back over a channel.
pub fn start(message: &str, session: Option<&str>, model: &str) -> Result<Run, String> {
    let mut cmd = claude(model);
    cmd.args(["-p", "--output-format", "stream-json", "--verbose", "--include-partial-messages"]);
    if let Some(id) = session {
        cmd.args(["--resume", id]);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("claude: {e}"))?;
    child.stdin.take().unwrap().write_all(message.as_bytes()).map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let errors = thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut finished = false;
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            for e in events(&v) {
                finished |= matches!(e, Event::Done(_));
                if tx.send(e).is_err() {
                    return;
                }
            }
        }
        if !finished {
            let err = errors.join().unwrap_or_default();
            let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("claude stopped");
            let _ = tx.send(Event::Done(Err(last.trim().chars().take(200).collect())));
        }
    });
    Ok(Run { child, rx })
}

fn events(v: &Value) -> Vec<Event> {
    let s = |v: &Value| v.as_str().unwrap_or_default().to_string();
    match v["type"].as_str() {
        Some("system") if v["subtype"] == "init" => vec![Event::Session(s(&v["session_id"]))],
        Some("stream_event") => {
            let e = &v["event"];
            match e["type"].as_str() {
                Some("message_start") => vec![Event::Message],
                Some("content_block_delta") if e["delta"]["type"] == "text_delta" => vec![Event::Text(s(&e["delta"]["text"]))],
                _ => vec![],
            }
        }
        Some("assistant") => v["message"]["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| c["type"] == "tool_use")
            .map(|c| Event::Tool(activity(&s(&c["name"]), &c["input"])))
            .collect(),
        Some("result") => {
            let mut out = vec![Event::Session(s(&v["session_id"]))];
            out.push(Event::Done(if v["is_error"].as_bool() == Some(true) {
                Err(v["result"].as_str().or(v["subtype"].as_str()).unwrap_or("claude error").chars().take(200).collect())
            } else {
                Ok(s(&v["result"]))
            }));
            out
        }
        _ => vec![],
    }
}

/// "reading out/forums/…", for the spinner line while Claude looks things up.
fn activity(tool: &str, input: &Value) -> String {
    let dir = data_dir();
    let rel = |p: &Value| {
        let p = p.as_str().unwrap_or_default();
        Path::new(p).strip_prefix(&dir).map(|r| r.display().to_string()).unwrap_or_else(|_| p.to_string())
    };
    match tool {
        "Read" => format!("reading {}", rel(&input["file_path"])),
        "Grep" => format!("searching for “{}”", input["pattern"].as_str().unwrap_or_default()),
        "Glob" => format!("looking for {}", input["pattern"].as_str().unwrap_or_default()),
        "Bash" => format!("running {}", input["command"].as_str().unwrap_or_default()),
        t => t.to_lowercase(),
    }
}

fn sync_line(name: &str, s: &SourceStatus) -> String {
    match s.last_ok {
        0 => format!("{name} never"),
        t => format!("{name} {}", local(t, "%a %d %b %H:%M")),
    }
}

/// What Claude gets each turn: the time and sync freshness (the system prompt is
/// recorded once per conversation), the dashboard selection, then the question.
fn message(question: &str, context: Option<&str>) -> String {
    let s: Status = read_json(&status_file());
    let mut m = format!(
        "Now: {}.\nLast successful sync: {}, {}, {}.\n",
        local(now(), "%A %d %B %Y %H:%M %Z"),
        sync_line("keats", &s.keats),
        sync_line("gmail", &s.gmail),
        sync_line("calendar", &s.calendar),
    );
    if let Some(c) = context {
        m += &format!("Selected in the dashboard when the question was asked:\n{c}\n");
    }
    m + &format!("\nQuestion: {question}")
}

/// `hud ask [question]`: answer one question on stdout, or with no question,
/// open interactive Claude Code set up the same way.
pub fn cli(question: &str) -> Result<(), String> {
    if question.trim().is_empty() {
        use std::os::unix::process::CommandExt;
        return Err(claude(&default_model()).exec().to_string());
    }
    let run = start(&message(question, None), None, &default_model())?;
    let (mut out, mut wrote) = (std::io::stdout(), false);
    while let Some(e) = run.recv() {
        match e {
            Event::Message if wrote => {
                let _ = writeln!(out, "\n");
                wrote = false;
            }
            Event::Text(t) => {
                let _ = write!(out, "{t}");
                let _ = out.flush();
                wrote = true;
            }
            Event::Tool(a) => eprintln!("\x1b[2m· {a}\x1b[0m"),
            Event::Done(Ok(_)) => {
                let _ = writeln!(out);
                return Ok(());
            }
            Event::Done(Err(e)) => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

pub enum Copy {
    Answer,
    All,
    Code,
}

/// The contents of every ``` block, without the fences.
fn code_blocks(text: &str) -> String {
    let (mut out, mut fence) = (Vec::new(), false);
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fence = !fence;
            if !fence {
                out.push(String::new());
            }
        } else if fence {
            out.push(line.to_string());
        }
    }
    out.join("\n").trim_end().to_string()
}

/// Puts `text` on the system clipboard with whichever tool the session has,
/// falling back to OSC 52, which asks the terminal (or tmux) to do it.
fn clipboard(text: &str) -> Result<(), String> {
    let tools: &[&[&str]] = if cfg!(target_os = "macos") {
        &[&["pbcopy"]]
    } else if env::var_os("WAYLAND_DISPLAY").is_some() {
        &[&["wl-copy"], &["xclip", "-selection", "clipboard"], &["xsel", "-ib"]]
    } else {
        &[&["xclip", "-selection", "clipboard"], &["xsel", "-ib"]]
    };
    for tool in tools {
        let Ok(mut child) = Command::new(tool[0])
            .args(&tool[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };
        let wrote = child.stdin.take().map(|mut i| i.write_all(text.as_bytes()));
        if matches!(wrote, Some(Ok(()))) && child.wait().is_ok_and(|s| s.success()) {
            return Ok(());
        }
    }
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(text);
    let mut out = std::io::stdout();
    write!(out, "\x1b]52;c;{b64}\x07").and_then(|()| out.flush()).map_err(|e| e.to_string())
}

pub struct Turn {
    pub question: String,
    pub answer: String,
    pub activity: String,
    pub error: Option<String>,
}

pub struct Ask {
    pub input: String,
    /// Used from the next question on; a follow-up can switch.
    pub model: String,
    pub editing: bool,
    pub turns: Vec<Turn>,
    /// Short label of the dashboard item the conversation is about.
    pub about: Option<String>,
    context: Option<String>,
    session: Option<String>,
    run: Option<Run>,
    /// Lines scrolled from the top; None follows the bottom.
    pub scroll: Option<u16>,
    /// Bottom-most scroll position at the last draw, and the pane height.
    max_scroll: u16,
    page: u16,
    /// Brief confirmation on the pane border, e.g. "copied answer".
    notice: Option<(String, Instant)>,
}

impl Ask {
    pub fn new(item: Option<&Item>, model: String) -> Ask {
        Ask {
            input: String::new(),
            model,
            editing: true,
            turns: Vec::new(),
            about: item.map(|i| format!("{} · {}", short_org(&i.org), i.title)),
            context: item.map(describe),
            session: None,
            run: None,
            scroll: None,
            max_scroll: 0,
            page: 10,
            notice: None,
        }
    }

    pub fn running(&self) -> bool {
        self.run.is_some()
    }

    pub fn submit(&mut self) {
        let q = self.input.trim().to_string();
        if q.is_empty() || self.running() {
            return;
        }
        self.input.clear();
        self.scroll = None;
        // The selection only matters for the first question; later ones build on it.
        let context = if self.turns.is_empty() { self.context.as_deref() } else { None };
        let mut turn = Turn { question: q.clone(), answer: String::new(), activity: "thinking".into(), error: None };
        match start(&message(&q, context), self.session.as_deref(), &self.model) {
            Ok(run) => self.run = Some(run),
            Err(e) => turn.error = Some(e),
        }
        self.turns.push(turn);
    }

    pub fn cancel(&mut self) {
        if let Some(mut run) = self.run.take() {
            run.kill();
            if let Some(t) = self.turns.last_mut() {
                t.error = Some("stopped".into());
            }
        }
    }

    /// Drain streamed events; true if anything changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Some(run) = &self.run {
            let e = match run.try_recv() {
                Ok(e) => e,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => Event::Done(Err("claude stopped".into())),
            };
            changed = true;
            let Some(turn) = self.turns.last_mut() else { break };
            match e {
                Event::Session(id) if !id.is_empty() => self.session = Some(id),
                Event::Message => turn.answer.clear(),
                Event::Text(t) => turn.answer += &t,
                Event::Tool(a) => turn.activity = a,
                Event::Done(r) => {
                    match r {
                        Ok(text) if !text.trim().is_empty() => turn.answer = text,
                        Ok(_) => {}
                        Err(e) => turn.error = Some(e),
                    }
                    if let Some(mut run) = self.run.take() {
                        let _ = run.child.wait();
                    }
                }
                _ => {}
            }
        }
        changed
    }

    pub fn next_model(&mut self) {
        let i = MODELS.iter().position(|m| *m == self.model).map_or(0, |i| i + 1);
        self.model = MODELS[i % MODELS.len()].into();
    }

    pub fn scroll_by(&mut self, delta: i32) {
        let at = self.scroll.unwrap_or(self.max_scroll) as i32;
        let to = (at + delta).clamp(0, self.max_scroll as i32) as u16;
        self.scroll = if to >= self.max_scroll { None } else { Some(to) };
    }

    pub fn page(&self) -> i32 {
        (self.page / 2).max(1) as i32
    }

    fn latest(&self) -> Option<&Turn> {
        self.turns.iter().rev().find(|t| !t.answer.is_empty())
    }

    /// Copy the latest answer (`Answer`), the whole conversation, or just the
    /// code blocks in the latest answer, and say so on the border.
    pub fn copy(&mut self, what: Copy) {
        let text = match what {
            Copy::Answer => self.latest().map(|t| t.answer.clone()),
            Copy::All => Some(
                self.turns
                    .iter()
                    .filter(|t| !t.answer.is_empty())
                    .map(|t| format!("> {}\n\n{}", t.question, t.answer))
                    .collect::<Vec<_>>()
                    .join("\n\n---\n\n"),
            )
            .filter(|s| !s.is_empty()),
            Copy::Code => self.latest().map(|t| code_blocks(&t.answer)).filter(|s| !s.is_empty()),
        };
        let what = match what {
            Copy::Answer => "answer",
            Copy::All => "conversation",
            Copy::Code => "code",
        };
        let msg = match text {
            None if what == "code" => "no code in the answer".to_string(),
            None => "nothing to copy yet".to_string(),
            Some(t) => match clipboard(&t) {
                Ok(()) => format!("{} copied {what}", icon::COPY),
                Err(e) => format!("copy failed: {e}"),
            },
        };
        self.notice = Some((msg, Instant::now()));
    }

    /// The last link in the latest answer, for ↵.
    pub fn last_link(&self) -> Option<String> {
        let t = self.turns.iter().rev().find(|t| !t.answer.is_empty())?;
        t.answer
            .split(|c: char| c.is_whitespace() || "()<>[]\"'`*".contains(c))
            .rev()
            .find(|w| w.starts_with("https://") || w.starts_with("http://"))
            .map(|w| w.trim_end_matches(['.', ',', ';', ':']).to_string())
    }
}

impl Drop for Ask {
    fn drop(&mut self) {
        if let Some(mut run) = self.run.take() {
            run.kill();
        }
    }
}

fn describe(i: &Item) -> String {
    let mut s = format!("- {} item: {}\n- module: {}\n", i.section, i.title, i.org);
    if !i.tag.is_empty() {
        s += &format!("- kind: {}\n", i.tag);
    }
    if let Some(d) = i.due {
        s += &format!("- due: {}\n", local(d, "%a %d %b %Y %H:%M"));
    }
    if let Some(w) = i.when {
        s += &format!("- posted: {}\n", local(w, "%a %d %b %Y %H:%M"));
    }
    if !i.note.is_empty() {
        s += &format!("- note: {}\n", i.note);
    }
    if !i.url.is_empty() {
        s += &format!("- link: {}\n", i.url);
    }
    s
}

// ---- drawing ----

pub fn draw(f: &mut Frame, ask: &mut Ask, tick: usize, area: Rect) {
    let [body, input] = Layout::vertical([Constraint::Fill(1), Constraint::Length(3)]).areas(area);

    let mut block = panel(icon::ASK, "ask", 0, !ask.editing, None);
    if let Some(a) = &ask.about {
        block = block.title_top(Line::styled(format!(" {a} "), DIM).right_aligned());
    }
    let model = Line::from(vec![Span::styled(" tab ", FAINT), Span::styled(format!("{} ", ask.model), if ask.model == "opus" { PINK } else { DIM })]);
    block = block.title_bottom(model.right_aligned());
    let inner = block.inner(body);
    f.render_widget(block, body);
    let w = inner.width.max(10) as usize;

    if ask.turns.is_empty() {
        let hint = [
            Line::from(vec![Span::styled(format!("{}  ", icon::ASK), TEAL), Span::styled("ask about your modules", Style::new().fg(FG).bold())]),
            Line::raw(""),
            Line::styled("specs, templates, forum answers, deadlines, timetable, emails", DIM),
            Line::raw(""),
            Line::styled("“how do I submit the FC2 coursework?”", FAINT),
            Line::styled("“any unanswered PEP questions about part 4?”", FAINT),
            Line::styled("“what's due before friday?”", FAINT),
        ];
        let y = inner.y + inner.height.saturating_sub(hint.len() as u16) / 2;
        let r = Rect { y, height: (hint.len() as u16).min(inner.height), ..inner };
        f.render_widget(Paragraph::new(hint.to_vec()).centered(), r);
    } else {
        let lines = transcript(ask, tick, w);
        ask.page = inner.height;
        ask.max_scroll = (lines.len() as u16).saturating_sub(inner.height);
        let top = ask.scroll.unwrap_or(ask.max_scroll).min(ask.max_scroll);
        f.render_widget(Paragraph::new(lines).scroll((top, 0)), inner);
        if top < ask.max_scroll {
            // Left end of the bottom border; the model label sits at the right.
            let more = Line::styled(format!(" ↓ {} more ", ask.max_scroll - top), DIM);
            f.render_widget(more, Rect { y: body.y + body.height - 1, height: 1, x: body.x + 2, width: body.width.saturating_sub(16) });
        }
    }

    ask.notice = ask.notice.take().filter(|(_, at)| at.elapsed() < Duration::from_millis(2500));
    if let Some((msg, _)) = &ask.notice {
        let color = if msg.starts_with(icon::COPY) { GREEN } else { YELLOW };
        let r = Rect { y: body.y + body.height - 1, height: 1, x: body.x + 2, width: body.width.saturating_sub(16) };
        f.render_widget(Line::styled(format!(" {msg} "), color), r);
    }

    let border = if ask.editing { TEAL } else { FAINT };
    let block = Block::bordered().border_type(BorderType::Rounded).border_style(border).padding(Padding::horizontal(1));
    let iw = block.inner(input).width.saturating_sub(3) as usize;
    let line = if ask.editing {
        let chars: Vec<char> = ask.input.chars().collect();
        let shown: String = chars[chars.len().saturating_sub(iw)..].iter().collect();
        let mut spans = vec![Span::styled("› ", TEAL), Span::styled(shown, FG), Span::styled("▏", TEAL)];
        if ask.input.is_empty() {
            let text = if ask.running() { "answering… type your next question" } else if ask.turns.is_empty() { "ask a question" } else { "ask a follow-up" };
            spans.push(Span::styled(text, FAINT));
        }
        Line::from(spans)
    } else {
        Line::from(vec![Span::styled("› ", FAINT), Span::styled("press a to type", FAINT)])
    };
    f.render_widget(Paragraph::new(line).block(block), input);
}

fn transcript(ask: &Ask, tick: usize, w: usize) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let last = ask.turns.len() - 1;
    for (n, t) in ask.turns.iter().enumerate() {
        if n > 0 {
            out.push(Line::raw(""));
            out.push(Line::styled("─".repeat(w.min(40)), FAINT));
            out.push(Line::raw(""));
        }
        let q = Line::from(vec![Span::styled("› ", TEAL), Span::styled(t.question.clone(), Style::new().fg(TEAL).bold())]);
        out.extend(wrap(q, w, 2));
        out.push(Line::raw(""));
        for (line, hang) in markdown(&t.answer) {
            out.extend(wrap(line, w, hang));
        }
        if n == last && ask.running() {
            if !t.answer.is_empty() {
                out.push(Line::raw(""));
            }
            let spin = Span::styled(format!("{} ", SPINNER[tick % SPINNER.len()]), YELLOW);
            out.extend(wrap(Line::from(vec![spin, Span::styled(t.activity.clone(), DIM)]), w, 2));
        }
        if let Some(e) = &t.error {
            out.push(Line::raw(""));
            out.extend(wrap(Line::from(vec![Span::styled(format!("{} ", icon::ALERT), RED), Span::styled(e.clone(), RED)]), w, 2));
        }
    }
    out
}

/// Light markdown: headings, bullets, quotes, code fences, **bold**, `code`, links.
/// Each line comes with the hanging indent its wrapped continuation should use.
fn markdown(text: &str) -> Vec<(Line<'static>, usize)> {
    let mut out = Vec::new();
    let mut fence = false;
    for raw in text.lines() {
        let trimmed = raw.trim_start();
        if trimmed.starts_with("```") {
            fence = !fence;
            continue;
        }
        if fence {
            out.push((Line::from(Span::styled(format!("  {raw}"), CODE)), 4));
            continue;
        }
        let indent = raw.len() - trimmed.len();
        let pad = " ".repeat(indent);
        if let Some(h) = trimmed.strip_prefix("#").map(|h| h.trim_start_matches('#').trim()) {
            let spans = inline(h, Style::new().fg(FG).bold());
            out.push((Line::from(spans), 0));
        } else if let Some(rest) = trimmed.strip_prefix("- ").or(trimmed.strip_prefix("* ")).or(trimmed.strip_prefix("• ")) {
            let mut spans = vec![Span::raw(pad), Span::styled("• ", TEAL)];
            spans.extend(inline(rest, Style::new().fg(FG)));
            out.push((Line::from(spans), indent + 2));
        } else if let Some(rest) = trimmed.strip_prefix("> ").or(trimmed.strip_prefix(">")) {
            let mut spans = vec![Span::styled("│ ", FAINT)];
            spans.extend(inline(rest, Style::new().fg(DIM)));
            out.push((Line::from(spans), 2));
        } else if let Some(dot) = numbered(trimmed) {
            let mut spans = vec![Span::raw(pad), Span::styled(trimmed[..dot].to_string(), TEAL)];
            spans.extend(inline(&trimmed[dot..], Style::new().fg(FG)));
            out.push((Line::from(spans), indent + dot));
        } else {
            let mut spans = vec![Span::raw(pad)];
            spans.extend(inline(trimmed, Style::new().fg(FG)));
            out.push((Line::from(spans), indent));
        }
    }
    out
}

/// "12. text" -> Some(4), the length of "12. ".
fn numbered(s: &str) -> Option<usize> {
    let digits = s.chars().take_while(char::is_ascii_digit).count();
    (digits > 0 && s[digits..].starts_with(". ")).then_some(digits + 2)
}

fn inline(s: &str, base: Style) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let (mut bold, mut code, mut buf) = (false, false, String::new());
    let style = |bold: bool, code: bool| {
        if code {
            Style::new().fg(YELLOW)
        } else if bold {
            base.bold()
        } else {
            base
        }
    };
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '`' {
            push_text(&mut out, std::mem::take(&mut buf), style(bold, code), code);
            code = !code;
        } else if !code && c == '*' && chars.peek() == Some(&'*') {
            chars.next();
            push_text(&mut out, std::mem::take(&mut buf), style(bold, code), code);
            bold = !bold;
        } else {
            buf.push(c);
        }
    }
    push_text(&mut out, buf, style(bold, code), code);
    out
}

/// Text with any URLs in it picked out.
fn push_text(out: &mut Vec<Span<'static>>, text: String, style: Style, code: bool) {
    if text.is_empty() {
        return;
    }
    if code {
        return out.push(Span::styled(text, style));
    }
    let mut rest = text.as_str();
    while let Some(i) = rest.find("https://").or_else(|| rest.find("http://")) {
        let end = rest[i..].find(|c: char| c.is_whitespace() || "()<>[]\"'".contains(c)).map_or(rest.len(), |e| i + e);
        if i > 0 {
            out.push(Span::styled(rest[..i].to_string(), style));
        }
        out.push(Span::styled(rest[i..end].to_string(), Style::new().fg(TEAL).underlined()));
        rest = &rest[end..];
    }
    if !rest.is_empty() {
        out.push(Span::styled(rest.to_string(), style));
    }
}

/// Word-wrap a styled line to `w` cells; continuation lines are indented by `hang`.
fn wrap(line: Line<'static>, w: usize, hang: usize) -> Vec<Line<'static>> {
    let hang = if hang * 2 > w { 0 } else { hang };
    let mut tokens: Vec<(String, Style)> = Vec::new();
    for span in line.spans {
        let mut word = String::new();
        let mut space = None;
        for c in span.content.chars() {
            let is_space = c == ' ';
            if space.is_some_and(|s| s != is_space) && !word.is_empty() {
                tokens.push((std::mem::take(&mut word), span.style));
            }
            space = Some(is_space);
            word.push(c);
        }
        if !word.is_empty() {
            tokens.push((word, span.style));
        }
    }
    let width = |s: &str| Span::raw(s).width();
    let (mut lines, mut cur, mut used, mut start) = (Vec::new(), Vec::<Span<'static>>::new(), 0, 0);
    let newline = |lines: &mut Vec<Line<'static>>, cur: &mut Vec<Span<'static>>| {
        lines.push(Line::from(std::mem::take(cur)));
        cur.push(Span::raw(" ".repeat(hang)));
    };
    for (mut tok, style) in tokens {
        let tw = width(&tok);
        if tok.starts_with(' ') {
            // Spaces that fall on a break are dropped.
            if used + tw <= w {
                cur.push(Span::styled(tok, style));
                used += tw;
            } else {
                newline(&mut lines, &mut cur);
                (used, start) = (hang, hang);
            }
            continue;
        }
        if used + tw > w && used > start {
            newline(&mut lines, &mut cur);
            (used, start) = (hang, hang);
        }
        // A word longer than a whole line gets split.
        while used + width(&tok) > w {
            let room = w.saturating_sub(used).max(1);
            cur.push(Span::styled(tok.chars().take(room).collect::<String>(), style));
            tok = tok.chars().skip(room).collect();
            newline(&mut lines, &mut cur);
            (used, start) = (hang, hang);
        }
        if !tok.is_empty() {
            used += width(&tok);
            cur.push(Span::styled(tok, style));
        }
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(Line::from(cur));
    }
    lines
}
