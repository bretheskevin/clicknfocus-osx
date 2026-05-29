use clap::Parser;

/// Eager click-to-focus for macOS.
///
/// Intercepts mouse-down events and activates the window under the
/// cursor synchronously before the event reaches the target app,
/// so a single click both focuses and acts.
#[derive(Parser, Debug)]
#[command(name = "clicknfocus-osx", version, about)]
pub struct Args {
    /// Also raise the focused window to the front
    #[arg(long, default_value_t = false)]
    pub raise: bool,

    /// Bundle IDs to ignore (repeatable, e.g. --ignore com.apple.dock)
    #[arg(long = "ignore", action = clap::ArgAction::Append)]
    pub ignore: Vec<String>,

    /// Enable verbose logging
    #[arg(long, default_value_t = false)]
    pub verbose: bool,
}
