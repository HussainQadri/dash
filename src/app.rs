use crate::model::*;
use crate::{sync, ui};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::widgets::ListState;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime};
use std::{env, fs, io, thread};

const STALE_SECS: i64 = 3600;

pub struct App {
    pub data: Data,
    pub focus: usize,
    pub tables: [ListState; 3],
    pub views: [ListState; 3],
    pub tick: usize,
    state: State,
    undo: Vec<String>,
    child: Option<Child>,
    external_sync: bool,
    mtimes: Vec<Option<SystemTime>>,
    quit: bool,
}

fn mtimes() -> Vec<Option<SystemTime>> {
    [keats_items(), gmail_file(), status_file()]
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
            tick: 0,
            state,
            undo: Vec::new(),
            child: None,
            external_sync: sync::lock_held(),
            mtimes: mtimes(),
            quit: false,
        };
        app.clamp();
        app
    }

    pub fn syncing(&self) -> bool {
        self.child.is_some() || self.external_sync
    }

    fn reload(&mut self) {
        self.data = Data::load(&self.state.done);
        self.mtimes = mtimes();
        self.clamp();
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
        let i = t.selected().unwrap_or(0) as isize + delta;
        t.select(Some(i.clamp(0, n as isize - 1) as usize));
    }

    fn key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char('j') | KeyCode::Down => self.move_by(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_by(-1),
            KeyCode::Char('g') | KeyCode::Home => self.move_by(-9999),
            KeyCode::Char('G') | KeyCode::End => self.move_by(9999),
            KeyCode::Tab => self.focus = (self.focus + 1) % 3,
            KeyCode::BackTab => self.focus = (self.focus + 2) % 3,
            KeyCode::Enter | KeyCode::Char('o') => self.open(),
            KeyCode::Char('r') => self.start_sync(),
            KeyCode::Char('d') => {
                if let Some(key) = self.selected().map(Item::key) {
                    self.undo.push(key.clone());
                    self.set_done(key, true);
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

    fn on_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        if let Some(c) = &mut self.child
            && !matches!(c.try_wait(), Ok(None))
        {
            self.child = None;
        }
        if self.tick.is_multiple_of(4) {
            self.external_sync = self.child.is_none() && sync::lock_held();
            if mtimes() != self.mtimes {
                self.reload();
            }
        }
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
            if event::poll(Duration::from_millis(250))?
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
