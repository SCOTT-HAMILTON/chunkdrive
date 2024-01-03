use global::{AsyncGlobal, BlockingGlobal, Global};
use serde_yaml::from_reader;
use std::env::{args, var};
use std::path::Path;
use std::sync::Arc;

/* #region Modules */
mod blocks;
mod bucket;
mod encryption;
mod global;
mod inodes;
mod s3;
mod services;
mod shell;
mod sources;
mod stored;

#[cfg(test)]
mod tests; // this is only included when running tests
           /* #endregion */

// these will be checked if CD_CONFIG_PATH is not set
const CONFIG_PATHS: [&str; 1] = ["./config.yml"];

fn main() {
    // Find config file path
    let mut config_path = None;
    if let Ok(path) = var("CD_CONFIG_PATH") {
        config_path = Some(path);
    } else {
        for path in CONFIG_PATHS.iter() {
            if Path::new(path).exists() {
                config_path = Some(path.to_string());
                break;
            }
        }
    }

    // Load config file
    let file = std::fs::File::open(config_path.unwrap_or_else(||
        panic!("Could not find config file. Please set CD_CONFIG_PATH or place config.yml in one of the following locations: {:?}", CONFIG_PATHS))
    ).unwrap();
    let global: Global = from_reader(file).unwrap();

    // Setup a handler for Ctrl+C (SIGINT)
    let main_thread_handle = std::thread::current();
    ctrlc::set_handler(move || {
        println!("Ctrl+C received...");
        main_thread_handle.unpark();
    })
    .expect("Error setting Ctrl-C handler");

    // if run with --shell, start the shell
    if args().any(|arg| arg == "--shell") {
        let bglobal = Arc::new(BlockingGlobal::new(global));
        shell::shell(bglobal);
    } else {
        // otherwise, start services
        let aglobal = Arc::new(AsyncGlobal::new(global));
        global::run_services(aglobal);
        std::thread::park();
    }
}
