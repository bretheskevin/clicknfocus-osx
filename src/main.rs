#[macro_use]
extern crate objc;

mod ax;
mod bundle;
mod cli;
mod event_tap;
mod focus;
mod permissions;

use clap::Parser;
use cli::Args;
use focus::FocusConfig;

fn main() {
    let args = Args::parse();

    let log_level = if args.verbose {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };
    env_logger::Builder::from_default_env()
        .filter_level(log_level)
        .init();

    log::info!(
        "clicknfocus-osx starting (raise={}, ignore={:?})",
        args.raise,
        args.ignore
    );

    // Check accessibility permission (will prompt the user if not granted)
    if !permissions::check_accessibility_permission(true) {
        eprintln!(
            "error: Accessibility permission is required.\n\
             Grant access in: System Settings > Privacy & Security > Accessibility\n\
             Then restart clicknfocus-osx."
        );
        std::process::exit(1);
    }
    log::info!("Accessibility permission granted");

    // Use libc::getpid() directly instead of std::process::id() as i32
    // to avoid a theoretical truncation on platforms where u32 > i32::MAX.
    let own_pid = unsafe { libc::getpid() };

    let config = FocusConfig {
        raise: args.raise,
        ignore_bundle_ids: args.ignore,
        own_pid,
    };

    let resolver = ax::AxFocusResolver::new();

    // This blocks forever
    event_tap::run_event_loop(resolver, config);
}
