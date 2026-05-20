mod app;
mod args;
mod browser;
mod cdp;
mod doctor;
mod output;
mod papers;
mod queue;
mod scinet;

use std::process;

fn main() {
    if let Err(error) = app::run() {
        eprintln!("snq: {error}");
        process::exit(1);
    }
}
