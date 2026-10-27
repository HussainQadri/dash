use crate::model::{calendars_config, data_home, keats_token, now};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};
use std::{env, process, thread};

const LAUNCH: &str = "https://keats.kcl.ac.uk/admin/tool/mobile/launch.php?service=moodle_mobile_app&urlscheme=moodledl&passport=";
/// On macOS Firefox isn't on PATH but inside its app bundle.
const FIREFOX: &[&str] = &["firefox", "/Applications/Firefox.app/Contents/MacOS/firefox"];
const SCHEME: &str = "x-scheme-handler/moodledl";
const DESKTOP: &str = "dash-login.desktop";

/// `hud login` asks for calendar links if there are none yet, then signs in to
/// KEATS through the mobile app launch flow in a fresh browser session (a fresh
/// login is what makes KEATS include the private token). The moodledl://
/// redirect comes back as `hud login <url>` via the scheme handler.
pub fn run(callback: Option<&str>) -> io::Result<()> {
    match callback {
        Some(url) => save(url),
        None => login(),
    }
}

fn login() -> io::Result<()> {
    setup_calendars()?;
    install_handler()?;
    let before = modified();
    let url = format!("{LAUNCH}{}", now());
    let mut chromium = None;
    if let Some(ff) = FIREFOX.iter().find_map(|b| crate::timetable::find(b)) {
        Command::new(ff)
            .args(["--private-window", &url])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?;
        println!("Sign in to KEATS in the private window, and let Firefox open the link with hud login.");
    } else if let Some(browser) = crate::timetable::chromium() {
        // A throwaway profile is as fresh as a private window.
        let profile = env::temp_dir().join(format!("hud-keats-{}", process::id()));
        let _ = fs::remove_dir_all(&profile);
        let child = Command::new(browser)
            .arg(format!("--user-data-dir={}", profile.display()))
            .args(["--no-first-run", "--no-default-browser-check", &url])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        chromium = Some((child, profile));
        println!("Sign in to KEATS in the window that opens, and let it open the link with hud login.");
    } else {
        println!("Open this in a private window, sign in to KEATS, and let the browser open the link with hud login:\n\n{url}\n");
    }
    let result = wait_for_token(before);
    if let Some((mut child, profile)) = chromium {
        let _ = child.kill();
        let _ = child.wait();
        let _ = fs::remove_dir_all(profile);
    }
    result
}

fn wait_for_token(before: Option<SystemTime>) -> io::Result<()> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(600) {
        thread::sleep(Duration::from_millis(500));
        if modified() != before {
            if !has_private_token(&fs::read_to_string(keats_token())?) {
                return Err(io::Error::other("token saved, but KEATS left out the private token"));
            }
            println!("Logged in. The forum session now renews itself.");
            return Ok(());
        }
    }
    Err(io::Error::other("timed out waiting for the login"))
}

fn save(url: &str) -> io::Result<()> {
    if !url.starts_with("moodledl://token=") {
        return Err(io::Error::other("not a KEATS token link"));
    }
    write_private(&keats_token(), url)
}

/// Secret links and tokens: readable by the owner only.
fn write_private(path: &Path, contents: &str) -> io::Result<()> {
    fs::create_dir_all(path.parent().unwrap())?;
    let tmp = path.with_extension("tmp");
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?
        .write_all(contents.as_bytes())?;
    fs::rename(tmp, path)
}

/// First run only: fetch the KCL timetable link by signing in to MyTimetable
/// (falling back to asking for it), ask for the Google one, check both load.
fn setup_calendars() -> io::Result<()> {
    let path = calendars_config();
    if path.exists() {
        return Ok(());
    }
    let mut lines = vec!["# name url (secret links: keep out of git)".to_string()];
    println!("KCL timetable: sign in to MyTimetable in the window that opens.");
    let kcl = match crate::timetable::feed_url().and_then(|url| check(&url).map(|n| (url, n))) {
        Ok((url, n)) => {
            println!("  ok, {n} events");
            Some(url)
        }
        Err(e) => {
            println!("  couldn't get it automatically ({e})");
            ask("kcl", "mytimetable.kcl.ac.uk → Subscribe → Manual subscription → Copy")?
        }
    };
    let google = ask(
        "google",
        "Google Calendar (on a computer): Settings → your calendar → Integrate calendar → Secret address in iCal format",
    )?;
    lines.extend(kcl.map(|u| format!("kcl {u}")));
    lines.extend(google.map(|u| format!("google {u}")));
    if lines.len() > 1 {
        write_private(&path, &(lines.join("\n") + "\n"))?;
        println!("Saved to {}\n", path.display());
    } else {
        println!("Skipped; run hud login again to add them.\n");
    }
    Ok(())
}

/// Prompt for a feed link until one loads; Enter skips it.
fn ask(name: &str, hint: &str) -> io::Result<Option<String>> {
    println!("\n{hint} (Enter skips)");
    loop {
        print!("{name} link: ");
        io::stdout().flush()?;
        let mut url = String::new();
        io::stdin().read_line(&mut url)?;
        let url = url.trim();
        if url.is_empty() {
            return Ok(None);
        }
        match check(url) {
            Ok(n) => {
                println!("  ok, {n} events");
                return Ok(Some(url.to_string()));
            }
            Err(e) => println!("  {e}, try again"),
        }
    }
}

/// Number of events if `url` serves an iCal feed.
fn check(url: &str) -> Result<usize, String> {
    let text = crate::calendar::fetch(url)?;
    if !text.contains("BEGIN:VCALENDAR") {
        return Err("that link isn't a calendar feed".into());
    }
    Ok(text.matches("BEGIN:VEVENT").count())
}

fn install_handler() -> io::Result<()> {
    let dir = data_home().join("applications");
    fs::create_dir_all(&dir)?;
    fs::write(
        dir.join(DESKTOP),
        format!(
            "[Desktop Entry]\nType=Application\nName=hud login\nExec={} login %u\nMimeType={SCHEME};\nNoDisplay=true\n",
            env::current_exe()?.display()
        ),
    )?;
    Command::new("xdg-mime").args(["default", DESKTOP, SCHEME]).status()?;
    Ok(())
}

fn modified() -> Option<SystemTime> {
    fs::metadata(keats_token()).and_then(|m| m.modified()).ok()
}

/// The token link decodes to signature:::token[:::privatetoken].
fn has_private_token(url: &str) -> bool {
    let b64: String = url
        .split_once("token=")
        .map(|(_, t)| t.chars().take_while(|c| c.is_ascii_alphanumeric() || "+/=".contains(*c)).collect())
        .unwrap_or_default();
    STANDARD
        .decode(b64)
        .is_ok_and(|d| String::from_utf8_lossy(&d).split(":::").count() >= 3)
}
