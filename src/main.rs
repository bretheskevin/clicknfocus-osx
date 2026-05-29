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

    let log_level = if args.verbose { log::LevelFilter::Debug } else { log::LevelFilter::Info };
    env_logger::Builder::from_default_env().filter_level(log_level).init();

    log::info!("clicknfocus-osx starting (raise={}, ignore={:?})", args.raise, args.ignore);

    // Check accessibility permission (prompts the user once if not granted).
    //
    // macOS caches the TCC result per-process: a process that launched
    // *untrusted* generally never observes a grant made while it's running, so
    // polling in-process forever can spin indefinitely. Instead we poll only
    // briefly — long enough to catch the cases where it does update live — then
    // exit. launchd's KeepAlive relaunches a fresh process, and a fresh process
    // *does* see the current grant. The grace period before exiting keeps
    // respawns slow (well above launchd's 10 s throttle) rather than a tight
    // crash loop, and avoids exit(1) churn.
    if !permissions::check_accessibility_permission(true) {
        log::warn!(
            "Accessibility permission not granted. Waiting for it to be enabled in \
             System Settings > Privacy & Security > Accessibility..."
        );
        // Poll without re-prompting (the system dialog was already shown above).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !permissions::check_accessibility_permission(false) {
            if std::time::Instant::now() >= deadline {
                log::warn!(
                    "Permission still not granted after waiting; exiting so launchd \
                     relaunches a fresh process that can pick it up."
                );
                std::process::exit(0);
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    }
    log::info!("Accessibility permission granted");

    // Use libc::getpid() directly instead of std::process::id() as i32
    // to avoid a theoretical truncation on platforms where u32 > i32::MAX.
    let own_pid = unsafe { libc::getpid() };

    let config = FocusConfig { raise: args.raise, ignore_bundle_ids: args.ignore, own_pid };

    let resolver = ax::AxFocusResolver::new();

    // This blocks forever
    event_tap::run_event_loop(resolver, config);
}
