#![recursion_limit = "1024"]

#[macro_use]
extern crate log;

#[macro_use]
extern crate error_chain;

extern crate clap;
extern crate env_logger;
extern crate iron;
extern crate mount;
extern crate network_manager;
extern crate nix;
extern crate params;
extern crate persistent;
extern crate router;
extern crate serde_json;
extern crate staticfile;
extern crate futures;
extern crate tokio_core;
extern crate tokio_ping;

mod errors;
mod config;
mod network;
mod server;
mod logger;
mod exit;

use std::path;
use std::thread;
use std::sync::mpsc::channel;
use std::io::Write;
use std::process;

use errors::*;
use config::get_config;
use network::{init_networking, process_network_commands};
use exit::block_exit_signals;

fn main() {
    // Run the program
    if let Err(ref e) = run() {

        // Handle errors
        let stderr = &mut ::std::io::stderr();
        let errmsg = "Error writing to stderr";

        writeln!(stderr, "\x1B[1;31mError: {}\x1B[0m", e).expect(errmsg);

        for inner in e.iter().skip(1) {
            writeln!(stderr, "  caused by: {}", inner).expect(errmsg);
        }

        // Exit with error
        process::exit(exit_code(e));
    }
}

fn run() -> Result<()> {
    // Stops signal handlers from working
    block_exit_signals()?;

    // Initialize logger
    logger::init();

    // Load config information
    let config = get_config();

    // Start NetworkManager, if necessary, and clear existing Wifi configurations
    init_networking()?;

    let (exit_tx, exit_rx) = channel();

    // Spawn a thread that takes ownership of config and exit_tx
    thread::spawn(move || {
        process_network_commands(&config, &exit_tx);
    });

    // Sleep until an exit signal is received, indicating that the thread has terminated, either intentionally or due to an error
    match exit_rx.recv() {
        Ok(result) => if let Err(reason) = result {
            return Err(reason);
        },
        Err(e) => {
            return Err(e.into());
        },
    }

    Ok(())
}
