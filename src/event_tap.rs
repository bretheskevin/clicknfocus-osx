use core_graphics::event::{
    CGEvent, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    CGMouseButton, CallbackResult, EventField,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;

use crate::focus::{FocusConfig, FocusResolver, should_focus};

/// Magic value written to EVENT_SOURCE_USER_DATA on synthetic re-posted clicks.
/// Used to detect and pass through our own events, preventing infinite loops.
/// Spells "clnf" in ASCII: 0x636C6E66.
const SYNTHETIC_EVENT_MAGIC: i64 = 0x636C6E66;

/// Shared state accessible from the event tap callback.
struct TapState<R: FocusResolver> {
    resolver: R,
    config: FocusConfig,
    /// Window ID of the last focused window (for dedup).
    last_focused_window_id: Option<u64>,
}

/// Map a CGEventType mouse-down variant to the corresponding CGMouseButton.
fn mouse_button_for_event_type(etype: CGEventType) -> CGMouseButton {
    match etype {
        CGEventType::RightMouseDown => CGMouseButton::Right,
        CGEventType::OtherMouseDown => CGMouseButton::Center,
        // LeftMouseDown and anything else default to Left
        _ => CGMouseButton::Left,
    }
}

/// Synthesize a mouse-down event of the same type/button at the given position,
/// tagged with our magic user-data so the tap callback can recognise it.
/// Posts it to HID immediately (no delay).
fn synthesize_and_post_click(etype: CGEventType, point: CGPoint) {
    let source = match CGEventSource::new(CGEventSourceStateID::HIDSystemState) {
        Ok(s) => s,
        Err(()) => {
            log::warn!("Failed to create CGEventSource for synthetic click");
            return;
        }
    };

    let button = mouse_button_for_event_type(etype);
    let event = match CGEvent::new_mouse_event(source, etype, point, button) {
        Ok(e) => e,
        Err(()) => {
            log::warn!("Failed to create synthetic mouse event");
            return;
        }
    };

    // Tag so our callback can recognise and pass through this event.
    event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, SYNTHETIC_EVENT_MAGIC);

    // Post immediately — no delay between activate() and re-post.
    // NOTE: Timing is the fragile part of this approach. If some apps still
    // swallow the re-posted click, a small delay (e.g. 5-10 ms) can be
    // inserted here between activate() and this post. For now we go immediate
    // because the user chose the immediate strategy.
    event.post(CGEventTapLocation::HID);
}

/// Run the event tap loop. This function blocks forever (runs the CFRunLoop).
///
/// Uses an **active** (Default) event tap to intercept mouse-down events.
/// When a click targets a background (non-frontmost) window:
///   1. The original click is **dropped** (swallowed).
///   2. The target app is activated via the Accessibility API.
///   3. A **synthetic** mouse-down at the same position is immediately re-posted
///      with a magic tag in EVENT_SOURCE_USER_DATA so the callback can
///      recognise and pass it through on the next invocation.
///
/// This ensures the click both focuses the window AND actuates the control
/// under the cursor, defeating macOS's `acceptsFirstMouse:` consumption.
///
/// Mouse-up events are NOT intercepted — the hardware mouse-up naturally
/// pairs with the synthetic mouse-down.
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
        // Active tap so the callback can Drop events.
        CGEventTapOptions::Default,
        vec![
            CGEventType::LeftMouseDown,
            CGEventType::RightMouseDown,
            CGEventType::OtherMouseDown,
            // Listen for tap-disabled notifications so we can re-enable.
            CGEventType::TapDisabledByTimeout,
            CGEventType::TapDisabledByUserInput,
        ],
        move |_proxy, etype, event| {
            // --- Handle tap-disabled notifications ---
            // If the tap is disabled (e.g. because the callback took too long),
            // the system sends these sentinel event types. The tap's `enable()`
            // method is not accessible from inside the callback, so we cannot
            // re-enable here. The CGEventTap wrapper calls `enable()` during
            // setup; if the tap is disabled, macOS will deliver a
            // TapDisabledByTimeout event. Unfortunately core-graphics 0.25
            // does not expose the mach port inside the callback closure, so we
            // cannot call CGEventTapEnable ourselves. We log the situation;
            // in practice the tap rarely times out because our callback is fast.
            // TODO: If this becomes a real issue, hold a clone of the CFMachPort
            // in TapState and call CGEventTapEnable from here.
            match etype {
                CGEventType::TapDisabledByTimeout => {
                    log::warn!(
                        "Event tap disabled by timeout — tap will remain disabled \
                         until the process is restarted. Consider filing a bug."
                    );
                    return CallbackResult::Keep;
                }
                CGEventType::TapDisabledByUserInput => {
                    log::warn!("Event tap disabled by user input");
                    return CallbackResult::Keep;
                }
                _ => {}
            }

            // --- Ignore our own synthetic re-posted events ---
            // If the event carries our magic tag, it is a click we synthesised
            // after activation. Let it pass through untouched.
            let user_data = event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA);
            if user_data == SYNTHETIC_EVENT_MAGIC {
                return CallbackResult::Keep;
            }

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

                // Decide whether to focus (skip own process, Dock, menubar, etc.)
                if !should_focus(&window_info, &s.config, s.last_focused_window_id) {
                    return CallbackResult::Keep;
                }

                // If the target app is already frontmost, the click is a normal
                // in-app click — let it pass through without redirect.
                if let Some(front_pid) = s.resolver.frontmost_pid()
                    && window_info.pid == front_pid
                {
                    log::debug!(
                        "Target app pid={} is already frontmost, passing click through",
                        window_info.pid
                    );
                    return CallbackResult::Keep;
                }

                // --- Redirect: swallow original click, activate, re-post ---
                log::debug!(
                    "Redirecting click: pid={} bundle={:?} window_id={}",
                    window_info.pid,
                    window_info.bundle_id,
                    window_info.window_id
                );

                // (a) Activate the target app synchronously via Accessibility API.
                s.resolver.activate(&window_info, s.config.raise);
                s.last_focused_window_id = Some(window_info.window_id);

                // (b) Immediately synthesize and post a new mouse-down at the
                //     same position, tagged with our magic constant.
                synthesize_and_post_click(etype, CGPoint::new(x, y));

                // (c) Drop the original event so it doesn't also arrive at the
                //     (now-focused) app, which would cause a double-click effect.
                CallbackResult::Drop
            }
        },
        || {
            log::info!("Event tap active (active mode). Listening for mouse-down events...");
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
