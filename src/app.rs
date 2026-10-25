use crate::fx::{self, Cue, DISMISS_MS, Glow, Rects};
use crate::model::*;
use crate::{sync, ui};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::widgets::ListState;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};
use std::{env, fs, io, thread};
use tachyonfx::EffectManager;

const STALE_SECS: i64 = 3600;

pub struct App {
    pub data: Data,
    pub focus: usize,
    pub tables: [ListState; 3],
    pub views: [ListState; 3],
    /// The full-screen week view instead of the panels.
    pub calendar: bool,
    /// Weeks from this one shown in the calendar.
    pub week: i64,
    /// Selected calendar event as "cal:uid:start".
    pub pick: Option<String>,
    pub fx: EffectManager<&'static str>,
    pub rects: Rects,
    pub cues: Vec<Cue>,
    pub last_frame: Instant,
    started: Instant,
    last_check: Instant,
    /// Item being dismissed: it dissolves first, then gets marked done.
    leaving: Option<(String, Instant)>,
    pub glows: HashMap<String, Glow>,
    /// Due items' urgency (0 fine, 1 under 24h, 2 overdue), to catch them crossing over.
    levels: HashMap<String, u8>,
    state: State,
    undo: Vec<String>,
    child: Option<Child>,
    external_sync: bool,
    mtimes: Vec<Option<SystemTime>>,
    quit: bool,
}

fn mtimes() -> Vec<Option<SystemTime>> {
    [keats_items(), gmail_file(), coursework_file(), calendar_file(), status_file()]
        .iter()
        .map(|p| fs::metadata(p).and_then(|m| m.modified()).ok())
        .collect()
}

impl App {
    fn new() -> App {
        let state: State = read_json(&state_file());
        let mut app = App {
            data: Data::load(&state.done),
            focus: 0,
            tables: Default::default(),
            views: Default::default(),
            calendar: false,
            week: 0,
            pick: None,
            fx: EffectManager::default(),
            rects: Rects::default(),
            cues: Vec::new(),
            last_frame: Instant::now(),
            started: Instant::now(),
            last_check: Instant::now(),
            leaving: None,
            glows: HashMap::new(),
            levels: HashMap::new(),
            state,
            undo: Vec::new(),
            child: None,
            external_sync: sync::lock_held(),
            mtimes: mtimes(),
            quit: false,
        };
        app.clamp();
        app.mark_new();
        app.check_deadlines();
        app
    }

    /// Spinner frame, by time rather than loop count (frames run faster while animating).
    pub fn tick(&self) -> usize {
        (self.started.elapsed().as_millis() / 80) as usize
    }

    /// Effects and glows run at ~60fps; the rest (breathing border, pulses) is slow.
    fn animating(&self) -> bool {
        self.fx.is_running()
            || self.leaving.is_some()
            || !self.cues.is_empty()
            || !self.glows.is_empty()
    }

    /// Light up items dash hasn't shown before (not on the very first run).
    fn mark_new(&mut self) {
        let first = self.state.seen.is_empty();
        // Calendar events arrived later than the rest; don't light up the whole week the first time.
        let first_cal = !self.state.seen.iter().any(|k| k.starts_with("cal:"));
        let mut changed = false;
        for key in &self.data.keys {
            if self.state.seen.insert(key.clone()) {
                changed = true;
                if !first && !(first_cal && key.starts_with("cal:")) {
                    self.glows.insert(key.clone(), Glow::new(fx::NEW, 4000, 1));
                }
            }
        }
        if changed {
            let _ = fs::create_dir_all(data_dir());
            let _ = write_json(&state_file(), &self.state);
        }
    }

    /// Pulse a row red when it drops under 24h or goes overdue while dash is open.
    fn check_deadlines(&mut self) {
        let now = now();
        for i in self.data.sections[0].iter().filter(|i| i.pending) {
            let Some(d) = i.due else { continue };
            let level = match d - now {
                s if s < 0 => 2,
                s if s <= 86400 => 1,
                _ => 0,
            };
            if self.levels.insert(i.key(), level).is_some_and(|p| level > p) {
                self.glows.insert(i.key(), Glow::new(fx::ALARM, 2400, 3));
            }
        }
    }

    pub fn syncing(&self) -> bool {
        self.child.is_some() || self.external_sync
    }

    fn reload(&mut self) {
        self.data = Data::load(&self.state.done);
        self.mtimes = mtimes();
        self.clamp();
        self.mark_new();
    }

    fn clamp(&mut self) {
        for (t, items) in self.tables.iter_mut().zip(&self.data.sections) {
            t.select(match items.len() {
                0 => None,
                n => Some(t.selected().unwrap_or(0).min(n - 1)),
            });
        }
    }

    pub fn selected(&self) -> Option<&Item> {
        self.data.sections[self.focus].get(self.tables[self.focus].selected()?)
    }

    fn stale(&self) -> bool {
        let s = &self.data.status;
        now() - s.keats.at.min(s.gmail.at) > STALE_SECS
    }

    fn start_sync(&mut self) {
        if self.syncing() {
            return;
        }
        let Ok(exe) = env::current_exe() else { return };
        // Own process group so closing the terminal mid-sync doesn't kill it.
        self.child = Command::new(exe)
            .arg("sync")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .ok();
    }

    fn open(&self) {
        let Some(item) = self.selected().filter(|i| !i.url.is_empty()) else { return };
        let child = Command::new("xdg-open")
            .arg(&item.url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn();
        if let Ok(mut c) = child {
            thread::spawn(move || c.wait());
        }
    }

    fn set_done(&mut self, key: String, done: bool) {
        if done {
            self.state.done.insert(key);
        } else {
            self.state.done.remove(&key);
        }
        let _ = fs::create_dir_all(data_dir());
        let _ = write_json(&state_file(), &self.state);
        self.reload();
    }

    fn move_by(&mut self, delta: isize) {
        let n = self.data.sections[self.focus].len();
        if n == 0 {
            return;
        }
        let t = &mut self.tables[self.focus];
        let before = t.selected();
        let i = t.selected().unwrap_or(0) as isize + delta;
        t.select(Some(i.clamp(0, n as isize - 1) as usize));
        if t.selected() != before {
            self.cues.push(Cue::Select);
        }
    }

    fn focus_by(&mut self, step: usize) {
        self.focus = (self.focus + step) % 3;
        self.cues.extend([Cue::Focus, Cue::Select]);
    }

    fn key(&mut self, k: KeyEvent) {
        if self.leaving.is_some() && !matches!(k.code, KeyCode::Char('q') | KeyCode::Esc) {
            return;
        }
        if self.calendar {
            return self.calendar_key(k);
        }
        match k.code {
            KeyCode::Char('c') => {
                self.calendar = true;
                self.go_week(self.current_week());
            }
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char('j') | KeyCode::Down => self.move_by(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_by(-1),
            KeyCode::Char('g') | KeyCode::Home => self.move_by(-9999),
            KeyCode::Char('G') | KeyCode::End => self.move_by(9999),
            KeyCode::Tab => self.focus_by(1),
            KeyCode::BackTab => self.focus_by(2),
            KeyCode::Enter | KeyCode::Char('o') => self.open(),
            KeyCode::Char('r') => self.start_sync(),
            KeyCode::Char('d') => {
                if let Some(key) = self.selected().map(Item::key) {
                    self.leaving = Some((key, Instant::now()));
                    self.cues.push(Cue::Dismiss);
                }
            }
            KeyCode::Char('u') => {
                if let Some(key) = self.undo.pop() {
                    self.set_done(key, false);
                }
            }
            _ => {}
        }
    }

    fn calendar_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('c') | KeyCode::Esc => self.calendar = false,
            KeyCode::Char('h') | KeyCode::Left => self.go_week(self.week - 1),
            KeyCode::Char('l') | KeyCode::Right => self.go_week(self.week + 1),
            KeyCode::Char('t') => self.go_week(self.current_week()),
            KeyCode::Char('j') | KeyCode::Down => self.pick_by(1),
            KeyCode::Char('k') | KeyCode::Up => self.pick_by(-1),
            KeyCode::Char('r') => self.start_sync(),
            _ => {}
        }
    }

    /// This week, or next week once this one has nothing left (e.g. at the weekend).
    fn current_week(&self) -> i64 {
        let n = now();
        if self.data.week(0).1.iter().any(|e| e.end > n) { 0 } else { 1 }
    }

    fn go_week(&mut self, week: i64) {
        self.week = week;
        self.pick_default();
    }

    fn week_keys(&self) -> Vec<String> {
        self.data.week(self.week).1.iter().map(|e| format!("cal:{}:{}", e.uid, e.start)).collect()
    }

    /// The class on now or next up this week; otherwise the week's first event.
    fn pick_default(&mut self) {
        let (_, events) = self.data.week(self.week);
        let n = now();
        self.pick = events
            .iter()
            .find(|e| e.end > n)
            .or(events.first())
            .map(|e| format!("cal:{}:{}", e.uid, e.start));
        self.cues.push(Cue::Select);
    }

    fn pick_by(&mut self, delta: isize) {
        let keys = self.week_keys();
        let Some(i) = self.pick.as_ref().and_then(|p| keys.iter().position(|k| k == p)) else {
            return self.pick_default();
        };
        let j = (i as isize + delta).clamp(0, keys.len() as isize - 1) as usize;
        if j != i {
            self.pick = Some(keys[j].clone());
            self.cues.push(Cue::Select);
        }
    }

    fn on_tick(&mut self) {
        if let Some((_, at)) = &self.leaving
            && at.elapsed() >= Duration::from_millis(DISMISS_MS as u64)
        {
            let (key, _) = self.leaving.take().unwrap();
            self.undo.push(key.clone());
            self.set_done(key, true);
            self.cues.push(Cue::Select);
        }
        if let Some(c) = &mut self.child
            && !matches!(c.try_wait(), Ok(None))
        {
            self.child = None;
        }
        if self.last_check.elapsed() >= Duration::from_secs(1) {
            self.last_check = Instant::now();
            self.external_sync = self.child.is_none() && sync::lock_held();
            if mtimes() != self.mtimes {
                self.reload();
            }
            self.check_deadlines();
        }
        self.glows.retain(|_, g| g.live());
    }
}

pub fn run() -> io::Result<()> {
    let mut app = App::new();
    if app.stale() {
        app.start_sync();
    }
    let mut terminal = ratatui::init();
    let result = (|| {
        while !app.quit {
            terminal.draw(|f| ui::draw(f, &mut app))?;
            let wait = if app.animating() { 16 } else { 50 };
            if event::poll(Duration::from_millis(wait))?
                && let Event::Key(k) = event::read()?
                && k.kind == KeyEventKind::Press
            {
                app.key(k);
            }
            app.on_tick();
        }
        Ok(())
    })();
    ratatui::restore();
    result
}
