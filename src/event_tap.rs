use core_graphics::event::{
    CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    CallbackResult,
};

use crate::focus::{FocusConfig, FocusResolver, should_focus};

/// Shared state accessible from the event tap callback.
struct TapState<R: FocusResolver> {
    resolver: R,
    config: FocusConfig,
    /// Window ID of the last focused window (for dedup).
    last_focused_window_id: Option<u64>,
}

/// Run the event tap loop. This function blocks forever (runs the CFRunLoop).
///
/// Listens for mouse-down events (left, right, and other) and synchronously
/// activates the window under the click position before the event reaches
/// the target application.
pub fn run_event_loop<R: FocusResolver + 'static>(resolver: R, config: FocusConfig) {
    use core_foundation::runloop::CFRunLoop;

    let state = Box::new(std::cell::RefCell::new(TapState {
        resolver,
        config,
        last_focused_window_id: None,
    }));
    let state_ptr = Box::into_raw(state);

    let result = CGEventTap::with_enabled(
        CGEventTapLocation::HID,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::ListenOnly,
        vec![
            CGEventType::LeftMouseDown,
            CGEventType::RightMouseDown,
            CGEventType::OtherMouseDown,
        ],
        move |_proxy, _etype, event| {
            let location = event.location();
            let (x, y) = (location.x, location.y);

            // SAFETY: state_ptr points to a heap-allocated RefCell created by
            // Box::into_raw above. It remains valid for the lifetime of the run loop.
            // The callback runs on the single CFRunLoop thread, so RefCell borrow
            // cannot alias.
            unsafe {
                let state = &*state_ptr;
                let mut s = state.borrow_mut();

                // Resolve the window at the click position
                let window_info = match s.resolver.window_at_position(x, y) {
                    Some(info) => info,
                    None => {
                        log::debug!("No window at ({}, {})", x, y);
                        return CallbackResult::Keep;
                    }
                };

                // Decide whether to focus
                if !should_focus(&window_info, &s.config, s.last_focused_window_id) {
                    return CallbackResult::Keep;
                }

                // Activate synchronously before the event propagates
                log::debug!(
                    "Focusing: pid={} bundle={:?} window_id={}",
                    window_info.pid,
                    window_info.bundle_id,
                    window_info.window_id
                );
                s.resolver.activate(&window_info, s.config.raise);
                s.last_focused_window_id = Some(window_info.window_id);
            }

            CallbackResult::Keep
        },
        || {
            log::info!("Event tap active. Listening for mouse-down events...");
            CFRunLoop::run_current();
        },
    );

    match result {
        Ok(_) => {
            log::info!("Event loop ended");
        }
        Err(()) => {
            eprintln!(
                "error: Failed to create CGEventTap.\n\
                 This usually means Accessibility permission is not granted.\n\
                 Grant access in: System Settings > Privacy & Security > Accessibility"
            );
            std::process::exit(1);
        }
    }

    // Cleanup (unreachable in practice since CFRunLoop::run_current blocks forever,
    // but included for correctness if the run loop is ever stopped externally).
    // SAFETY: state_ptr was created by Box::into_raw and has not been freed.
    unsafe {
        let _ = Box::from_raw(state_ptr);
    }
}
