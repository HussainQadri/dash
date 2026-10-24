use crate::model::*;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{self, File, TryLockError};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use std::io::{Read, Write};
use std::{env, thread};

const KEATS_SCRIPT: &str = include_str!("../keats/keats.py");
const GMAIL_PROMPT: &str = include_str!("../prompts/gmail.md");
const GMAIL_SCHEMA: &str = r#"{"type":"object","required":["items"],"properties":{"items":{"type":"array","items":{"type":"object","required":["thread_id","company","kind","title","status"],"properties":{"thread_id":{"type":"string"},"company":{"type":"string"},"kind":{"enum":["OA","assessment","interview"]},"title":{"type":"string"},"deadline":{"type":["string","null"]},"url":{"type":["string","null"]},"status":{"enum":["pending","done","rejected"]}}}}}}"#;
const COURSEWORK_PROMPT: &str = include_str!("../prompts/coursework.md");
const COURSEWORK_SCHEMA: &str = r#"{"type":"object","required":["items"],"properties":{"items":{"type":"array","items":{"type":"object","required":["module","title"],"properties":{"module":{"type":"string"},"title":{"type":"string"},"weight":{"type":["string","null"]},"due":{"type":["string","null"]},"note":{"type":["string","null"]}}}}}}"#;
const CLAUDE_TIMEOUT: Duration = Duration::from_secs(300);

pub fn lock_held() -> bool {
    let Ok(f) = File::options().create(true).truncate(false).write(true).open(lock_file()) else {
        return false;
    };
    matches!(f.try_lock(), Err(TryLockError::WouldBlock))
}

pub fn run() -> Result<(), String> {
    fs::create_dir_all(data_dir()).map_err(|e| e.to_string())?;
    let lock = File::create(lock_file()).map_err(|e| e.to_string())?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Err("sync already running".into()),
        Err(TryLockError::Error(e)) => return Err(e.to_string()),
    }

    let status = Mutex::new(read_json::<Status>(&status_file()));
    let gmail_since = {
        let mut s = status.lock().unwrap();
        s.keats.state = "running".into();
        s.gmail.state = "running".into();
        let _ = write_json(&status_file(), &*s);
        s.gmail.last_ok
    };

    if let Err(e) = wait_for_network() {
        finish(&status, |s| &mut s.keats, Err(e.clone()));
        finish(&status, |s| &mut s.gmail, Err(e.clone()));
        return Err(e);
    }

    thread::scope(|s| {
        s.spawn(|| finish(&status, |s| &mut s.keats, keats_job().and_then(|()| coursework_job())));
        s.spawn(|| finish(&status, |s| &mut s.gmail, gmail_job(gmail_since)));
    });
    Ok(())
}

fn finish(status: &Mutex<Status>, pick: fn(&mut Status) -> &mut SourceStatus, result: Result<(), String>) {
    let mut s = status.lock().unwrap();
    let src = pick(&mut s);
    src.at = now();
    match result {
        Ok(()) => {
            src.state = "ok".into();
            src.last_ok = src.at;
            src.error.clear();
        }
        Err(e) => {
            src.state = "error".into();
            src.error = e;
        }
    }
    let _ = write_json(&status_file(), &*s);
}

fn wait_for_network() -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let up = ("keats.kcl.ac.uk", 443)
            .to_socket_addrs()
            .ok()
            .and_then(|mut a| a.next())
            .is_some_and(|a| TcpStream::connect_timeout(&a, Duration::from_secs(3)).is_ok());
        if up {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err("offline".into());
        }
        thread::sleep(Duration::from_secs(3));
    }
}

fn last_line(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let line = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("failed");
    line.trim().chars().take(160).collect()
}

fn keats_job() -> Result<(), String> {
    // keats.py puts its output next to itself, so run it from the data dir.
    if fs::read_to_string(keats_script()).ok().as_deref() != Some(KEATS_SCRIPT) {
        fs::write(keats_script(), KEATS_SCRIPT).map_err(|e| format!("keats.py: {e}"))?;
    }
    let out = Command::new("python3")
        .arg(keats_script())
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("python3: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let err = last_line(&out.stderr);
    Err(if err.contains("invalidtoken") {
        "token invalid".into()
    } else {
        // "RuntimeError: msg" -> "msg"
        err.split_once("Error: ").map_or(err.clone(), |(_, m)| m.to_string())
    })
}

/// Coursework deadlines live in free text on each module page, phrased
/// differently per module, so Claude reads them. Only re-run when the text
/// keats.py gathered has changed since the last extraction.
fn coursework_job() -> Result<(), String> {
    let input = fs::read_to_string(keats_assessments()).map_err(|e| format!("assessments.json: {e}"))?;
    let mut saved: Coursework = read_json(&coursework_file());
    if saved.input == input {
        return Ok(());
    }
    let modules: BTreeMap<String, Value> = serde_json::from_str(&input).map_err(|e| e.to_string())?;
    let text: String = modules
        .iter()
        .map(|(code, m)| format!("# {code}\n{}\n", m["text"].as_str().unwrap_or("")))
        .collect();
    let now = now();
    let year: i32 = local(now, "%Y").parse().unwrap_or(2026);
    let start = if local(now, "%m").parse::<u32>().unwrap_or(9) >= 9 { year } else { year - 1 };
    let prompt = COURSEWORK_PROMPT
        .replace("{today}", &local(now, "%Y-%m-%d"))
        .replace("{y1}", &start.to_string())
        .replace("{y2}", &(start + 1).to_string())
        .replace("{text}", &text);

    let reply = claude(&prompt, COURSEWORK_SCHEMA, &[])?;
    let mut items: Vec<CourseworkItem> = serde_json::from_value(reply["items"].clone())
        .map_err(|e| format!("deadlines: bad output: {e}"))?;
    for i in &mut items {
        i.url = modules.get(&i.module).and_then(|m| m["url"].as_str()).unwrap_or_default().to_string();
    }
    saved = Coursework { input, items };
    write_json(&coursework_file(), &saved).map_err(|e| e.to_string())
}

fn drain(mut r: impl Read + Send + 'static) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = r.read_to_end(&mut buf);
        buf
    })
}

fn claude_bin() -> PathBuf {
    env::var_os("PATH")
        .into_iter()
        .flat_map(|p| env::split_paths(&p).collect::<Vec<_>>())
        .map(|d| d.join("claude"))
        .find(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(".local/bin/claude"))
}

/// Runs headless Claude (Sonnet) on `prompt` and returns its schema-checked
/// `structured_output`. `tools` empty means no tools and no MCP servers at all.
fn claude(prompt: &str, schema: &str, tools: &[&str]) -> Result<Value, String> {
    let mut cmd = Command::new(claude_bin());
    cmd.args(["-p", "--model", "sonnet", "--no-session-persistence"])
        .args(["--output-format", "json", "--json-schema", schema]);
    if tools.is_empty() {
        cmd.args(["--strict-mcp-config", "--tools", ""]);
    } else {
        cmd.args(["--allowedTools", &tools.join(",")]);
    }
    let mut child = cmd
        .current_dir(data_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("claude: {e}"))?;
    child.stdin.take().unwrap().write_all(prompt.as_bytes()).map_err(|e| e.to_string())?;

    // Drain pipes on threads so a large reply can't block the child while we poll.
    let stdout = drain(child.stdout.take().unwrap());
    let stderr = drain(child.stderr.take().unwrap());
    let started = Instant::now();
    let exit = loop {
        if let Some(code) = child.try_wait().map_err(|e| e.to_string())? {
            break code;
        }
        if started.elapsed() > CLAUDE_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err("claude timed out".into());
        }
        thread::sleep(Duration::from_millis(500));
    };
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();

    let reply: Value = serde_json::from_slice(&stdout).map_err(|_| {
        if exit.success() { "claude: unreadable reply".into() } else { last_line(&stderr) }
    })?;
    if reply["is_error"].as_bool() == Some(true) {
        return Err(reply["result"].as_str().unwrap_or("claude error").chars().take(160).collect());
    }
    Ok(reply["structured_output"].clone())
}

fn gmail_job(last_ok: i64) -> Result<(), String> {
    let now = now();
    let since = if last_ok > 0 { last_ok - 86400 } else { now - 30 * 86400 };
    let mut items: Vec<GmailItem> = read_json(&gmail_file());
    let open: Vec<&GmailItem> = items.iter().filter(|g| g.status == "pending").collect();
    let prompt = GMAIL_PROMPT
        .replace("{since}", &local(since, "%Y/%m/%d"))
        .replace("{open}", &serde_json::to_string(&open).unwrap_or_default());

    let reply = claude(&prompt, GMAIL_SCHEMA, &["mcp__claude_ai_Gmail__search_threads", "mcp__claude_ai_Gmail__get_thread"])?;
    let found: Vec<GmailItem> = serde_json::from_value(reply["items"].clone())
        .map_err(|e| format!("claude: bad output: {e}"))?;

    for g in found {
        match items.iter_mut().find(|i| i.thread_id == g.thread_id) {
            Some(i) => *i = GmailItem { seen: i.seen, ..g },
            None => items.push(GmailItem { seen: now, ..g }),
        }
    }
    items.retain(|g| {
        let due = g.deadline.as_deref().and_then(parse_deadline);
        match g.status.as_str() {
            "pending" => due.is_none_or(|d| d >= now - RECENT_DAYS * 86400),
            _ => g.seen >= now - 30 * 86400,
        }
    });
    write_json(&gmail_file(), &items).map_err(|e| e.to_string())
}
