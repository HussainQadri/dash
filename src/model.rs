use jiff::{Timestamp, civil, tz::TimeZone};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::{env, fs, io};

pub const RECENT_DAYS: i64 = 14;

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
}

#[derive(Serialize, Deserialize, Default)]
pub struct State {
    #[serde(default)]
    pub done: BTreeSet<String>,
}

#[derive(Default)]
pub struct Data {
    pub sections: [Vec<Item>; 3],
    pub forum_note: Option<String>,
    pub status: Status,
}

impl Data {
    pub fn load(done: &BTreeSet<String>) -> Data {
        let now = now();
        let keats: KeatsFile = read_json(&keats_items());
        let gmail: Vec<GmailItem> = read_json(&gmail_file());
        let coursework: Coursework = read_json(&coursework_file());
        let items = keats
            .items
            .into_iter()
            .chain(coursework.items.iter().map(CourseworkItem::to_item))
            .chain(gmail.iter().filter(|g| g.status == "pending").map(GmailItem::to_item));

        let mut sections: [Vec<Item>; 3] = Default::default();
        for item in items.filter(|i| !done.contains(&i.key())) {
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
        sections[2].sort_by_key(|i| std::cmp::Reverse(i.when));

        Data {
            sections,
            forum_note: keats.forum_note,
            status: read_json(&status_file()),
        }
    }
}
