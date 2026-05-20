mod app;
mod args;
mod browser;
mod cdp;
mod doctor;
mod locks;
mod output;
mod page;
mod papers;
mod queue;
mod scinet;

use std::{env, process};

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let wants_json = app::args_want_json(&args);

    if let Err(error) = app::run(args) {
        if wants_json {
            let value = serde_json::json!({ "error": error });
            eprintln!("{value}");
        } else {
            eprintln!("snq: {error}");
        }
        process::exit(1);
    }
}
