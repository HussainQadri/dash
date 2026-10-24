use crate::model::{data_home, keats_token, now};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};
use std::{env, thread};

const LAUNCH: &str = "https://keats.kcl.ac.uk/admin/tool/mobile/launch.php?service=moodle_mobile_app&urlscheme=moodledl&passport=";
const SCHEME: &str = "x-scheme-handler/moodledl";
const DESKTOP: &str = "dash-login.desktop";

/// `dash login` signs in through the mobile app launch flow in a private window
/// (a fresh login is what makes KEATS include the private token). The
/// moodledl:// redirect comes back as `dash login <url>` via the scheme handler.
pub fn run(callback: Option<&str>) -> io::Result<()> {
    match callback {
        Some(url) => save(url),
        None => login(),
    }
}

fn login() -> io::Result<()> {
    install_handler()?;
    let before = modified();
    Command::new("firefox")
        .args(["--private-window", &format!("{LAUNCH}{}", now())])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    println!("Sign in to KEATS in the private window, and let Firefox open the link with dash login.");

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
    let path = keats_token();
    fs::create_dir_all(path.parent().unwrap())?;
    let tmp = path.with_extension("tmp");
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?
        .write_all(url.as_bytes())?;
    fs::rename(tmp, path)
}

fn install_handler() -> io::Result<()> {
    let dir = data_home().join("applications");
    fs::create_dir_all(&dir)?;
    fs::write(
        dir.join(DESKTOP),
        format!(
            "[Desktop Entry]\nType=Application\nName=dash login\nExec={} login %u\nMimeType={SCHEME};\nNoDisplay=true\n",
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
