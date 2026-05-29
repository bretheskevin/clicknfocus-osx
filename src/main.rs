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

    // Check accessibility permission (prompts the user once if not granted).
    // Rather than exit(1) — which under launchd's KeepAlive would respawn in a
    // tight loop and re-prompt — we wait in-process until the user grants it.
    if !permissions::check_accessibility_permission(true) {
        log::warn!(
            "Accessibility permission not granted. Waiting for it to be enabled in \
             System Settings > Privacy & Security > Accessibility..."
        );
        // Poll without re-prompting (the system dialog was already shown above).
        while !permissions::check_accessibility_permission(false) {
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
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
