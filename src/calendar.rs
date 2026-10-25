use crate::model::*;
use chrono::TimeZone;
use jiff::{civil, tz::TimeZone as JiffTz};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::process::Command;

/// Window kept for browsing weeks: one back, the rest of the academic year ahead.
const BEHIND_DAYS: i64 = 7;
const AHEAD_DAYS: i64 = 240;

/// One VEVENT as raw properties: name -> [(params, value)].
type Props = BTreeMap<String, Vec<(String, String)>>;

/// Fetch every feed in ~/.config/dash/calendars ("name url" per line) and write
/// the events around now. A feed that fails keeps its last events.
pub fn job() -> Result<(), String> {
    let config = fs::read_to_string(calendars_config()).map_err(|_| "no ~/.config/dash/calendars".to_string())?;
    let feeds: Vec<(&str, &str)> = config
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once(char::is_whitespace))
        .map(|(n, u)| (n, u.trim()))
        .collect();
    if feeds.is_empty() {
        return Err("no feeds in ~/.config/dash/calendars".into());
    }

    let old: Vec<CalEvent> = read_json(&calendar_file());
    let (from, to) = (days_start(now(), -BEHIND_DAYS), now() + AHEAD_DAYS * 86400);
    let (mut events, mut uids, mut errors) = (Vec::new(), HashSet::new(), Vec::new());
    for (name, url) in feeds {
        let parsed = match fetch(url) {
            Ok(text) => parse(&text, name, from, to),
            Err(e) => {
                errors.push(format!("{name}: {e}"));
                old.iter().filter(|e| e.source == name).cloned().collect()
            }
        };
        // The same event can come through two feeds (e.g. a timetable also
        // subscribed in Google); the first feed listed wins.
        let feed_uids: HashSet<String> = parsed.iter().map(|e| e.uid.clone()).collect();
        events.extend(parsed.into_iter().filter(|e| !uids.contains(&e.uid)));
        uids.extend(feed_uids);
    }
    events.sort_by_key(|e| (e.start, e.end));
    write_json(&calendar_file(), &events).map_err(|e| e.to_string())?;
    if errors.is_empty() { Ok(()) } else { Err(errors.join(", ")) }
}

fn fetch(url: &str) -> Result<String, String> {
    let out = Command::new("curl")
        .args(["-fsSL", "--max-time", "30", url])
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("fetch failed ({})", out.status.code().unwrap_or(-1)));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn parse(text: &str, source: &str, from: i64, to: i64) -> Vec<CalEvent> {
    let vevents = vevents(text);
    // Edited or cancelled instances of a recurring event replace the generated one.
    let mut moved: HashMap<String, HashSet<i64>> = HashMap::new();
    for p in &vevents {
        if let (Some(uid), Some((ts, _))) = (value(p, "UID"), p.get("RECURRENCE-ID").and_then(|v| ical_time(&v[0]))) {
            moved.entry(uid.to_string()).or_default().insert(ts);
        }
    }

    let mut out = Vec::new();
    for p in &vevents {
        if value(p, "STATUS") == Some("CANCELLED") {
            continue;
        }
        let Some((start, all_day)) = p.get("DTSTART").and_then(|v| ical_time(&v[0])) else { continue };
        let end = p.get("DTEND").and_then(|v| ical_time(&v[0])).map_or(start + if all_day { 86400 } else { 3600 }, |t| t.0);
        let starts = match p.get("RRULE").filter(|_| !p.contains_key("RECURRENCE-ID")) {
            Some(_) => {
                let skip = value(p, "UID").and_then(|u| moved.get(u));
                occurrences(p, from - (end - start), to).into_iter().filter(|t| !skip.is_some_and(|s| s.contains(t))).collect()
            }
            None => vec![start],
        };
        for s in starts.into_iter().filter(|&s| s + (end - start) >= from && s <= to) {
            out.push(event(p, source, s, s + (end - start), all_day));
        }
    }
    out
}

/// Unfold lines and group each VEVENT's properties.
fn vevents(text: &str) -> Vec<Props> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.lines() {
        match (raw.strip_prefix([' ', '\t']), lines.last_mut()) {
            (Some(cont), Some(last)) => last.push_str(cont),
            _ => lines.push(raw.trim_end_matches('\r').to_string()),
        }
    }
    let (mut all, mut cur): (Vec<Props>, Option<Props>) = (Vec::new(), None);
    for line in lines {
        match line.as_str() {
            "BEGIN:VEVENT" => cur = Some(Props::new()),
            "END:VEVENT" => all.extend(cur.take()),
            _ => {
                if let (Some(p), Some((head, val))) = (cur.as_mut(), line.split_once(':')) {
                    let (name, params) = head.split_once(';').unwrap_or((head, ""));
                    p.entry(name.to_string()).or_default().push((params.to_string(), val.to_string()));
                }
            }
        }
    }
    all
}

fn value<'a>(p: &'a Props, name: &str) -> Option<&'a str> {
    p.get(name).map(|v| v[0].1.as_str())
}

fn unescape(s: &str) -> String {
    s.replace("\\n", "\n").replace("\\N", "\n").replace("\\,", ",").replace("\\;", ";").replace("\\\\", "\\")
}

/// DTSTART-style value -> (unix seconds, all-day). UTC, TZID or floating (local).
fn ical_time((params, value): &(String, String)) -> Option<(i64, bool)> {
    let tz = params
        .split(';')
        .find_map(|p| p.strip_prefix("TZID="))
        .and_then(|id| JiffTz::get(id.trim_matches('"')).ok())
        .unwrap_or_else(JiffTz::system);
    if params.contains("VALUE=DATE") || value.len() == 8 {
        let d = civil::Date::strptime("%Y%m%d", value).ok()?;
        return Some((d.at(0, 0, 0, 0).to_zoned(tz).ok()?.timestamp().as_second(), true));
    }
    let (v, utc) = value.strip_suffix('Z').map_or((value.as_str(), false), |v| (v, true));
    let dt = civil::DateTime::strptime("%Y%m%dT%H%M%S", v).ok()?;
    let tz = if utc { JiffTz::UTC } else { tz };
    Some((dt.to_zoned(tz).ok()?.timestamp().as_second(), false))
}

/// Start times of a recurring event between `from` and `to`.
fn occurrences(p: &Props, from: i64, to: i64) -> Vec<i64> {
    let mut spec = String::new();
    for name in ["DTSTART", "RRULE", "EXDATE"] {
        for (params, val) in p.get(name).into_iter().flatten() {
            // rrule wants date-times; an all-day series starts at local midnight.
            let (params, val) = match params.contains("VALUE=DATE") || (name != "RRULE" && val.len() == 8) {
                true => (String::new(), format!("{val}T000000")),
                false => (params.clone(), val.clone()),
            };
            let sep = if params.is_empty() { "" } else { ";" };
            spec.push_str(&format!("{name}{sep}{params}:{val}\n"));
        }
    }
    let Ok(set) = spec.trim_end().parse::<rrule::RRuleSet>() else { return Vec::new() };
    let at = |s: i64| rrule::Tz::UTC.timestamp_opt(s, 0).single();
    let (Some(a), Some(b)) = (at(from), at(to)) else { return Vec::new() };
    set.after(a).before(b).all(500).dates.iter().map(|d| d.timestamp()).collect()
}

/// Timetable feeds (Scientia) put "Event type: …\nDescription: …\nStaff: …" in
/// DESCRIPTION and a module code first in SUMMARY; anything else is used as is.
fn event(p: &Props, source: &str, start: i64, end: i64, all_day: bool) -> CalEvent {
    let summary = unescape(value(p, "SUMMARY").unwrap_or("(no title)"));
    let desc = strip_html(&unescape(value(p, "DESCRIPTION").unwrap_or("")));
    let field = |key: &str| desc.lines().find_map(|l| l.strip_prefix(key)).map(|v| v.trim().to_string());
    let timetable = field("Event type:").is_some();
    let code = summary.split([' ', '/']).next().filter(|c| is_module_code(c));
    CalEvent {
        source: source.to_string(),
        uid: value(p, "UID").unwrap_or_default().to_string(),
        org: code.unwrap_or(source).to_string(),
        kind: field("Event type:").unwrap_or_default(),
        title: field("Description:").unwrap_or(summary),
        location: unescape(value(p, "LOCATION").unwrap_or("")).trim().to_string(),
        note: match timetable {
            true => field("Staff:").unwrap_or_default(),
            false => desc.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_string(),
        },
        start,
        end,
        all_day,
    }
}

/// Google descriptions can be HTML: keep line breaks, drop tags.
fn strip_html(s: &str) -> String {
    let s = s.replace("<br>", "\n").replace("<br />", "\n").replace("<br/>", "\n");
    let mut out = String::new();
    let mut tag = false;
    for c in s.chars() {
        match c {
            '<' => tag = true,
            '>' if tag => tag = false,
            _ if !tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&nbsp;", " ")
}

/// "5CCS2PEP"-style: digits, capitals, digits, then the module letters.
fn is_module_code(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 6 && b[0].is_ascii_digit() && b.iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}
