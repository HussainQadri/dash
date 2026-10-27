use serde_json::Value;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use std::{env, fs, process, thread};

/// MyTimetable (Scientia) signs in through sciauth, which hands over to KCL's
/// Microsoft login and comes back with the token in the address bar.
const AUTH: &str = "https://sciauth.scientia.com/issue/oauth?response_type=token&client_id=KCL_68013085592130&redirect_uri=https%3A%2F%2Fmytimetable.kcl.ac.uk&scope=BF66331C-04F5-409E-A3FE-F19DC2B23008";
const API: &str = "https://scientia-eu-v4-api-d4-02.azurewebsites.net/";
const RETURN: &str = "https://mytimetable.kcl.ac.uk";

/// Without JavaScript MyTimetable can't clear the token from its address bar
/// before we read it; the Microsoft sign-in pages are unaffected.
const PREFS: &str = r#"{"profile":{"content_settings":{"exceptions":{"javascript":{"https://mytimetable.kcl.ac.uk:443,*":{"setting":2}}}}}}"#;

const BROWSERS: &[&str] = &[
    "chromium",
    "chromium-browser",
    "google-chrome-stable",
    "google-chrome",
    "brave",
    "brave-browser",
    "microsoft-edge-stable",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
];

/// The KCL timetable's iCal link: sign in once in a throwaway Chromium window,
/// then read the user's ids from their MyTimetable profile.
pub fn feed_url() -> Result<String, String> {
    let token = sign_in(AUTH)?;
    let profile = api("api/UserProfile", &token)?;
    let field = |k: &str| profile.as_object()?.iter().find(|(key, _)| key.eq_ignore_ascii_case(k)).map(|(_, v)| v.clone());
    let institution = field("InstitutionIdentity").and_then(|v| v.as_str().map(String::from));
    let resource = field("ResourceIdentities").and_then(|v| v.get(0)?.as_str().map(String::from));
    match (institution, resource) {
        // Same shape MyTimetable builds for its Subscribe button.
        (Some(i), Some(r)) => Ok(format!("{API}/api/ical/{i}/{r}/timetable.ics")),
        _ => Err("MyTimetable has no personal timetable for this account".into()),
    }
}

fn sign_in(url: &str) -> Result<String, String> {
    let browser = BROWSERS
        .iter()
        .find_map(|b| find(b))
        .ok_or("needs Chromium, Chrome, Brave or Edge")?;
    let profile = env::temp_dir().join(format!("hud-mytimetable-{}", process::id()));
    let _ = fs::remove_dir_all(&profile);
    fs::create_dir_all(profile.join("Default")).map_err(|e| e.to_string())?;
    fs::write(profile.join("Default/Preferences"), PREFS).map_err(|e| e.to_string())?;
    let port = TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map_err(|e| e.to_string())?
        .port();
    let mut child = Command::new(&browser)
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(format!("--remote-debugging-port={port}"))
        .args(["--no-first-run", "--no-default-browser-check"])
        .arg(format!("--app={url}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{browser}: {e}"))?;
    let token = wait_for_token(port, &mut child);
    let _ = child.kill();
    let _ = child.wait();
    let _ = fs::remove_dir_all(&profile);
    token
}

fn find(name: &str) -> Option<String> {
    if name.starts_with('/') {
        return Path::new(name).exists().then(|| name.to_string());
    }
    env::split_paths(&env::var_os("PATH")?)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
        .map(|p| p.display().to_string())
}

/// Watch the window's address (via the local debugging port) until it lands
/// back on MyTimetable with `#access_token=…`.
fn wait_for_token(port: u16, child: &mut Child) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(600);
    while Instant::now() < deadline {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return Err("the sign-in window was closed".into());
        }
        for url in tabs(port) {
            let Some(fragment) = url.strip_prefix(RETURN).and_then(|u| u.split_once('#')).map(|(_, f)| f) else { continue };
            if let Some(token) = fragment.split('&').find_map(|kv| kv.strip_prefix("access_token=")) {
                return Ok(token.to_string());
            }
        }
        thread::sleep(Duration::from_millis(300));
    }
    Err("timed out waiting for the sign-in".into())
}

fn tabs(port: u16) -> Vec<String> {
    let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) else { return Vec::new() };
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    // Chromium ignores HTTP/1.0 here; 1.1 with Connection: close reads to the end.
    let _ = s.write_all(b"GET /json/list HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    let mut resp = String::new();
    let _ = s.read_to_string(&mut resp);
    let body = resp.split_once("\r\n\r\n").map_or("", |(_, b)| b);
    serde_json::from_str::<Vec<Value>>(body)
        .unwrap_or_default()
        .iter()
        .filter_map(|t| t.get("url")?.as_str().map(String::from))
        .collect()
}

/// GET from the Scientia API; the token goes in via stdin, not the command line.
/// The API is shared between universities and tells them apart by Referer.
fn api(path: &str, token: &str) -> Result<Value, String> {
    let mut child = Command::new("curl")
        .args(["-fsS", "--max-time", "30", "-e", &format!("{RETURN}/"), "-K", "-", &format!("{API}{path}")])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl: {e}"))?;
    let header = format!("header = \"Authorization: Bearer {token}\"\n");
    child.stdin.take().map(|mut i| i.write_all(header.as_bytes()));
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("MyTimetable API: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("MyTimetable API: {e}"))
}
