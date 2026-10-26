use jiff::{Timestamp, civil, tz::TimeZone};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::{env, fs, io};

pub const RECENT_DAYS: i64 = 14;
/// Newest Q&A threads shown per module; the group header still counts them all.
pub const FORUM_PER_MODULE: usize = 5;

fn home() -> PathBuf {
    PathBuf::from(env::var_os("HOME").unwrap_or_default())
}

pub fn data_home() -> PathBuf {
    env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/share"))
}

pub fn data_dir() -> PathBuf {
    data_home().join("dash")
}

pub fn keats_script() -> PathBuf {
    data_dir().join("keats.py")
}

pub fn keats_token() -> PathBuf {
    home().join(".config/keats/token")
}

pub fn keats_items() -> PathBuf {
    data_dir().join("out/items.json")
}

pub fn keats_assessments() -> PathBuf {
    data_dir().join("out/assessments.json")
}

pub fn calendars_config() -> PathBuf {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
        .join("dash/calendars")
}

pub fn calendar_file() -> PathBuf {
    data_dir().join("calendar.json")
}

pub fn coursework_file() -> PathBuf {
    data_dir().join("coursework.json")
}

pub fn gmail_file() -> PathBuf {
    data_dir().join("gmail.json")
}

pub fn status_file() -> PathBuf {
    data_dir().join("status.json")
}

pub fn state_file() -> PathBuf {
    data_dir().join("state.json")
}

pub fn lock_file() -> PathBuf {
    data_dir().join("sync.lock")
}

pub fn now() -> i64 {
    Timestamp::now().as_second()
}

pub fn local(ts: i64, fmt: &str) -> String {
    Timestamp::from_second(ts)
        .map(|t| t.to_zoned(TimeZone::system()).strftime(fmt).to_string())
        .unwrap_or_default()
}

/// Calendar days between today and `ts` in local time (0 = today, 1 = tomorrow).
pub fn days_from_today(ts: i64) -> i64 {
    let tz = TimeZone::system();
    let day = |t: Timestamp| t.to_zoned(tz.clone()).date();
    Timestamp::from_second(ts)
        .ok()
        .and_then(|t| day(t).since(day(Timestamp::now())).ok())
        .map_or(0, |s| s.get_days() as i64)
}

/// Local midnight `days` days after the day containing `ts`.
pub fn days_start(ts: i64, days: i64) -> i64 {
    Timestamp::from_second(ts)
        .ok()
        .and_then(|t| t.to_zoned(TimeZone::system()).date().checked_add(jiff::Span::new().days(days)).ok())
        .and_then(|d| d.to_zoned(TimeZone::system()).ok())
        .map_or(ts, |z| z.timestamp().as_second())
}

/// "2026-10-25", "2026-10-25T14:00" (local) or a full RFC 3339 timestamp.
pub fn parse_deadline(s: &str) -> Option<i64> {
    if let Ok(t) = s.parse::<Timestamp>() {
        return Some(t.as_second());
    }
    // A bare date means "by end of that day", not midnight at its start.
    let dt = if s.len() == 10 {
        s.parse::<civil::Date>().ok()?.at(23, 59, 0, 0)
    } else {
        s.parse::<civil::DateTime>().ok()?
    };
    dt.to_zoned(TimeZone::system()).ok().map(|z| z.timestamp().as_second())
}

pub fn read_json<T: DeserializeOwned + Default>(path: &Path) -> T {
    fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    fs::rename(tmp, path)
}

#[derive(Deserialize, Clone, Default)]
pub struct Item {
    pub id: String,
    pub section: String,
    #[serde(default)]
    pub tag: String,
    pub org: String,
    pub title: String,
    #[serde(default)]
    pub due: Option<i64>,
    #[serde(default)]
    pub when: Option<i64>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub note: String,
    #[serde(default = "yes")]
    pub pending: bool,
    #[serde(skip)]
    pub gmail: bool,
}

fn yes() -> bool {
    true
}

impl Item {
    pub fn key(&self) -> String {
        format!("{}:{}", if self.gmail { "gmail" } else { "keats" }, self.id)
    }
}

#[derive(Deserialize, Default)]
struct KeatsFile {
    #[serde(default)]
    forum_note: Option<String>,
    #[serde(default)]
    items: Vec<Item>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct GmailItem {
    pub thread_id: String,
    #[serde(default)]
    pub company: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub deadline: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub seen: i64,
    /// When a sync first found this tracked task done.
    #[serde(default)]
    pub closed: i64,
}

impl GmailItem {
    fn to_item(&self) -> Item {
        Item {
            id: self.thread_id.clone(),
            section: "due".into(),
            tag: self.kind.clone(),
            org: self.company.clone(),
            title: self.title.clone(),
            due: self.deadline.as_deref().and_then(parse_deadline),
            url: self.url.clone().filter(|u| !u.is_empty()).unwrap_or_else(|| {
                format!("https://mail.google.com/mail/u/0/#all/{}", self.thread_id)
            }),
            pending: true,
            gmail: true,
            ..Default::default()
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct CourseworkItem {
    pub module: String,
    pub title: String,
    #[serde(default)]
    pub weight: Option<String>,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub url: String,
}

impl CourseworkItem {
    fn to_item(&self) -> Item {
        let title = match &self.weight {
            Some(w) => format!("{} ({w})", self.title),
            None => self.title.clone(),
        };
        Item {
            id: format!("cw:{}:{}", self.module, self.title),
            section: "due".into(),
            tag: "coursework".into(),
            org: self.module.clone(),
            title,
            due: self.due.as_deref().and_then(parse_deadline),
            url: self.url.clone(),
            note: self.note.clone().unwrap_or_default(),
            pending: true,
            ..Default::default()
        }
    }
}

/// An event from a calendar feed, already expanded if it recurs.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct CalEvent {
    /// Feed name from the config ("kcl", "google").
    pub source: String,
    pub uid: String,
    /// Module code for timetable events, otherwise the feed name.
    pub org: String,
    pub kind: String,
    pub title: String,
    pub location: String,
    pub note: String,
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
}

/// Extracted deadlines plus the exact text they came from (to skip re-extraction).
#[derive(Serialize, Deserialize, Default)]
pub struct Coursework {
    #[serde(default)]
    pub input: String,
    #[serde(default)]
    pub items: Vec<CourseworkItem>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct SourceStatus {
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub at: i64,
    #[serde(default)]
    pub last_ok: i64,
    #[serde(default)]
    pub error: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Status {
    #[serde(default)]
    pub keats: SourceStatus,
    #[serde(default)]
    pub gmail: SourceStatus,
    #[serde(default)]
    pub calendar: SourceStatus,
}

#[derive(Serialize, Deserialize, Default)]
pub struct State {
    #[serde(default)]
    pub done: BTreeSet<String>,
    /// Every item key dash has shown, so new ones can be lit up.
    #[serde(default)]
    pub seen: BTreeSet<String>,
}

#[derive(Default)]
pub struct Data {
    /// due, announcements, forums.
    pub sections: [Vec<Item>; 3],
    pub events: Vec<CalEvent>,
    pub forum_note: Option<String>,
    /// Per module: (threads, unanswered), before trimming to FORUM_PER_MODULE.
    pub forum_counts: HashMap<String, (usize, usize)>,
    /// Every loaded item, including ones trimmed or too old to show.
    pub keys: BTreeSet<String>,
    pub status: Status,
}

impl Data {
    /// Monday 00:00 of the week `offset` weeks from this one, and its events.
    pub fn week(&self, offset: i64) -> (i64, Vec<&CalEvent>) {
        let n = now();
        let weekday = Timestamp::from_second(n)
            .map(|t| t.to_zoned(TimeZone::system()).weekday().to_monday_zero_offset() as i64)
            .unwrap_or(0);
        let start = days_start(n, offset * 7 - weekday);
        let end = days_start(start, 7);
        (start, self.events.iter().filter(|e| e.start < end && e.end > start).collect())
    }

    pub fn load(done: &BTreeSet<String>) -> Data {
        let now = now();
        let keats: KeatsFile = read_json(&keats_items());
        let gmail: Vec<GmailItem> = read_json(&gmail_file());
        let coursework: Coursework = read_json(&coursework_file());
        let mut events: Vec<CalEvent> = read_json(&calendar_file());
        events.sort_by_key(|e| (e.start, e.end));
        let items = keats
            .items
            .into_iter()
            .chain(coursework.items.iter().map(CourseworkItem::to_item))
            .chain(gmail.iter().filter(|g| g.status == "pending").map(GmailItem::to_item));

        let mut sections: [Vec<Item>; 3] = Default::default();
        let mut keys: BTreeSet<String> = events.iter().map(|e| format!("cal:{}:{}", e.uid, e.start)).collect();
        for item in items.filter(|i| !done.contains(&i.key())) {
            keys.insert(item.key());
            let idx = match item.section.as_str() {
                "due" if item.due.is_none_or(|d| d >= now - 86400) => 0,
                "news" if item.when.unwrap_or(0) >= now - RECENT_DAYS * 86400 => 1,
                "forum" if item.when.unwrap_or(0) >= now - RECENT_DAYS * 86400 => 2,
                _ => continue,
            };
            sections[idx].push(item);
        }
        sections[0].sort_by_key(|i| (i.due.is_none(), i.due, !i.pending, i.title.clone()));
        sections[1].sort_by_key(|i| std::cmp::Reverse(i.when));
        // Forum threads grouped by module, most recently active module first.
        let mut latest = HashMap::new();
        for i in &sections[2] {
            let t = latest.entry(i.org.clone()).or_insert(0);
            *t = (*t).max(i.when.unwrap_or(0));
        }
        sections[2].sort_by_key(|i| (std::cmp::Reverse(latest[&i.org]), i.org.clone(), std::cmp::Reverse(i.when)));
        let mut forum_counts: HashMap<String, (usize, usize)> = HashMap::new();
        for i in &sections[2] {
            let c = forum_counts.entry(i.org.clone()).or_default();
            c.0 += 1;
            c.1 += i.note.starts_with("0 ") as usize;
        }
        let mut shown: HashMap<String, usize> = HashMap::new();
        sections[2].retain(|i| {
            let n = shown.entry(i.org.clone()).or_default();
            *n += 1;
            *n <= FORUM_PER_MODULE
        });

        Data {
            sections,
            forum_note: keats.forum_note,
            forum_counts,
            keys,
            events,
            status: read_json(&status_file()),
        }
    }
}
