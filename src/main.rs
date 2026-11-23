mod app;
mod ask;
mod calendar;
mod fx;
mod login;
mod model;
mod sync;
mod theme;
mod timetable;
mod ui;
mod week;

fn main() {
    match std::env::args().nth(1).as_deref() {
        None => {
            if let Err(e) = app::run() {
                eprintln!("dash: {e}");
                std::process::exit(1);
            }
        }
        Some("sync") => {
            if let Err(e) = sync::run() {
                eprintln!("dash: {e}");
                std::process::exit(1);
            }
        }
        Some("login") => {
            if let Err(e) = login::run(std::env::args().nth(2).as_deref()) {
                eprintln!("dash: {e}");
                std::process::exit(1);
            }
        }
        Some("ask") => {
            let question = std::env::args().skip(2).collect::<Vec<_>>().join(" ");
            if let Err(e) = ask::cli(&question) {
                eprintln!("dash: {e}");
                std::process::exit(1);
            }
        }
        Some(_) => {
            eprintln!("usage: hud [sync|login|ask [question]]");
            std::process::exit(2);
        }
    }
}
