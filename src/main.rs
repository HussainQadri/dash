mod model;
mod sync;

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("sync") => {
            if let Err(e) = sync::run() {
                eprintln!("dash: {e}");
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!("usage: dash sync");
            std::process::exit(2);
        }
    }
}
