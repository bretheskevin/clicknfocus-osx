use core_foundation::base::TCFType;
use core_foundation::mach_port::CFMachPortRef;
use core_graphics::event::{
    CGEvent, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    CGMouseButton, CallbackResult, EventField,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;
use std::ptr;

use crate::focus::{FocusConfig, FocusResolver, WindowInfo, should_focus};

// Re-enables a disabled event tap. Not exposed by core-graphics 0.25's public
// API, so we declare it ourselves; it takes the tap's CFMachPort.
unsafe extern "C" {
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
}

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
    /// Mach port of our own event tap, so the callback can re-enable the tap
    /// if macOS disables it (timeout / user input). Null until set after the
    /// tap is created.
    tap_port: CFMachPortRef,
    /// Reused event source for synthetic clicks (avoids re-creating one per
    /// click). `None` if creation failed at startup — we fall back per click.
    event_source: Option<CGEventSource>,
}

/// Pure-ish decision: given the resolver, decide whether a mouse-down at
/// `(x, y)` should be redirected (drop original → activate target → re-post),
/// returning the target [`WindowInfo`], or `None` to let the click pass through.
///
/// Extracted from the tap callback so the decision can be unit-tested with a
/// mock resolver (see tests below).
fn resolve_redirect_target<R: FocusResolver>(
    resolver: &R,
    config: &FocusConfig,
    last_focused_window_id: Option<u64>,
    x: f64,
    y: f64,
) -> Option<WindowInfo> {
    let window_info = resolver.window_at_position(x, y)?;

    // Skip own process, Dock, menubar, ignored bundles, already-focused window.
    if !should_focus(&window_info, config, last_focused_window_id) {
        return None;
    }

    // If the target app is already frontmost, the click is a normal in-app
    // click — let it pass through without redirect.
    if let Some(front_pid) = resolver.frontmost_pid()
        && window_info.pid == front_pid
    {
        log::debug!(
            "Target app pid={} is already frontmost, passing click through",
            window_info.pid
        );
        return None;
    }

    Some(window_info)
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

/// Synthesize a mouse-down event matching the original at the given position,
/// tagged with our magic user-data so the tap callback can recognise it.
/// Posts it to HID immediately (no delay).
///
/// `cached_source` is reused when present to avoid recreating a CGEventSource
/// on every click; the original event is used to preserve modifier flags,
/// click state (double-clicks), and the precise button number.
fn synthesize_and_post_click(
    cached_source: Option<&CGEventSource>,
    etype: CGEventType,
    original: &CGEvent,
    point: CGPoint,
) {
    let source = match cached_source {
        Some(s) => s.clone(),
        None => match CGEventSource::new(CGEventSourceStateID::HIDSystemState) {
            Ok(s) => s,
            Err(()) => {
                log::warn!("Failed to create CGEventSource for synthetic click");
                return;
            }
        },
    };

    let button = mouse_button_for_event_type(etype);
    let event = match CGEvent::new_mouse_event(source, etype, point, button) {
        Ok(e) => e,
        Err(()) => {
            log::warn!("Failed to create synthetic mouse event");
            return;
        }
    };

    // Preserve original event state so the synthetic click behaves like the
    // real one: modifier flags (cmd/shift/ctrl-click), click state (double
    // clicks), and the exact button number (mouse buttons 3/4/5, not just the
    // Left/Right/Center bucket that new_mouse_event sets).
    event.set_flags(original.get_flags());
    let click_state = original.get_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE);
    event.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, click_state);
    let button_number = original.get_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER);
    event.set_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER, button_number);

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
    use core_foundation::runloop::{CFRunLoop, kCFRunLoopCommonModes};

    // Pre-create one event source to reuse for every synthetic click.
    let event_source = CGEventSource::new(CGEventSourceStateID::HIDSystemState).ok();
    if event_source.is_none() {
        log::warn!("Could not pre-create CGEventSource; will create one per click");
    }

    let state = Box::new(std::cell::RefCell::new(TapState {
        resolver,
        config,
        last_focused_window_id: None,
        tap_port: ptr::null_mut(),
        event_source,
    }));
    let state_ptr = Box::into_raw(state);

    let callback = move |_proxy, etype, event: &CGEvent| {
        // --- Handle tap-disabled notifications ---
        // macOS disables the tap if the callback takes too long (timeout) or on
        // certain user input. Re-enable it using the mach port stored after
        // setup, so the tool recovers instead of silently dying.
        match etype {
            CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput => {
                log::warn!("Event tap disabled by macOS; re-enabling");
                // SAFETY: state_ptr is valid for the run loop's lifetime; the
                // callback runs single-threaded so the borrow cannot alias.
                unsafe {
                    let s = (*state_ptr).borrow();
                    if !s.tap_port.is_null() {
                        CGEventTapEnable(s.tap_port, true);
                    }
                }
                return CallbackResult::Keep;
            }
            _ => {}
        }

        // --- Ignore our own synthetic re-posted events ---
        // Checked before borrowing state so a (theoretical) reentrant delivery
        // of our own posted event cannot double-borrow the RefCell.
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
            let mut s = (*state_ptr).borrow_mut();

            let window_info = match resolve_redirect_target(
                &s.resolver,
                &s.config,
                s.last_focused_window_id,
                x,
                y,
            ) {
                Some(info) => info,
                None => return CallbackResult::Keep,
            };

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

            // (b) Immediately synthesize and post a new mouse-down at the same
            //     position, preserving the original event's state and tagged
            //     with our magic constant.
            synthesize_and_post_click(s.event_source.as_ref(), etype, event, CGPoint::new(x, y));

            // (c) Drop the original event so it doesn't also arrive at the
            //     (now-focused) app, which would cause a double-click effect.
            CallbackResult::Drop
        }
    };

    // Build the tap with new_unchecked so we keep the CGEventTap (and its mach
    // port) — needed to register the run-loop source, enable the tap, and
    // re-enable it from the callback if macOS disables it.
    // SAFETY: the callback only captures `state_ptr` and is invoked solely on
    // this thread's run loop; the tap is dropped after the loop returns.
    let tap = match unsafe {
        CGEventTap::new_unchecked(
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
            callback,
        )
    } {
        Ok(t) => t,
        Err(()) => {
            eprintln!(
                "error: Failed to create CGEventTap.\n\
                 This usually means Accessibility permission is not granted.\n\
                 Grant access in: System Settings > Privacy & Security > Accessibility"
            );
            // SAFETY: state_ptr was created by Box::into_raw and not yet freed.
            unsafe {
                let _ = Box::from_raw(state_ptr);
            }
            std::process::exit(1);
        }
    };

    // Record the mach port so the callback can re-enable the tap on disable.
    // SAFETY: state_ptr is valid and no other borrow is active (loop not started).
    unsafe {
        (*state_ptr).borrow_mut().tap_port = tap.mach_port().as_concrete_TypeRef();
    }

    let loop_source = match tap.mach_port().create_runloop_source(0) {
        Ok(s) => s,
        Err(()) => {
            eprintln!("error: Failed to create the event-tap run-loop source.");
            // SAFETY: state_ptr was created by Box::into_raw and not yet freed;
            // `tap` is dropped normally as it goes out of scope on exit.
            unsafe {
                let _ = Box::from_raw(state_ptr);
            }
            std::process::exit(1);
        }
    };
    // SAFETY: kCFRunLoopCommonModes is a static CFStringRef from CoreFoundation.
    CFRunLoop::get_current().add_source(&loop_source, unsafe { kCFRunLoopCommonModes });
    tap.enable();
    log::info!("Event tap active (active mode). Listening for mouse-down events...");

    // Blocks forever (until the run loop is stopped externally).
    CFRunLoop::run_current();

    log::info!("Event loop ended");

    // Cleanup (unreachable in practice since run_current blocks forever). Keep
    // `tap` alive until here so the tap stays installed for the whole run.
    drop(tap);
    // SAFETY: state_ptr was created by Box::into_raw and has not been freed.
    unsafe {
        let _ = Box::from_raw(state_ptr);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::focus::{FocusConfig, WindowInfo};

    /// Minimal mock resolver for exercising `resolve_redirect_target` (which
    /// never calls `activate`, so the mock doesn't record activations).
    struct MockResolver {
        window: Option<WindowInfo>,
        frontmost: Option<i32>,
    }

    impl MockResolver {
        fn new(window: Option<WindowInfo>, frontmost: Option<i32>) -> Self {
            Self { window, frontmost }
        }
    }

    impl FocusResolver for MockResolver {
        fn window_at_position(&self, _x: f64, _y: f64) -> Option<WindowInfo> {
            self.window.clone()
        }
        fn activate(&self, _info: &WindowInfo, _raise: bool) {}
        fn frontmost_pid(&self) -> Option<i32> {
            self.frontmost
        }
    }

    fn config() -> FocusConfig {
        FocusConfig {
            raise: false,
            ignore_bundle_ids: vec!["com.example.ignored".to_string()],
            own_pid: 999,
        }
    }

    fn window(pid: i32, bundle: &str, role: &str, id: u64) -> WindowInfo {
        WindowInfo {
            pid,
            bundle_id: Some(bundle.to_string()),
            role: Some(role.to_string()),
            window_id: id,
            cg_window_id: Some(id as u32),
        }
    }

    #[test]
    fn redirects_background_window() {
        let win = window(100, "com.test.app", "AXWindow", 7);
        let resolver = MockResolver::new(Some(win.clone()), Some(42)); // other app frontmost
        let target = resolve_redirect_target(&resolver, &config(), None, 10.0, 20.0);
        assert_eq!(target, Some(win));
    }

    #[test]
    fn no_redirect_when_target_already_frontmost() {
        let win = window(100, "com.test.app", "AXWindow", 7);
        let resolver = MockResolver::new(Some(win), Some(100)); // target IS frontmost
        assert!(resolve_redirect_target(&resolver, &config(), None, 10.0, 20.0).is_none());
    }

    #[test]
    fn no_redirect_when_no_window() {
        let resolver = MockResolver::new(None, Some(42));
        assert!(resolve_redirect_target(&resolver, &config(), None, 10.0, 20.0).is_none());
    }

    #[test]
    fn no_redirect_for_own_process() {
        let win = window(999, "com.test.self", "AXWindow", 7); // own_pid == 999
        let resolver = MockResolver::new(Some(win), Some(42));
        assert!(resolve_redirect_target(&resolver, &config(), None, 10.0, 20.0).is_none());
    }

    #[test]
    fn no_redirect_for_ignored_bundle() {
        let win = window(100, "com.example.ignored", "AXWindow", 7);
        let resolver = MockResolver::new(Some(win), Some(42));
        assert!(resolve_redirect_target(&resolver, &config(), None, 10.0, 20.0).is_none());
    }

    #[test]
    fn no_redirect_when_same_window_already_focused() {
        let win = window(100, "com.test.app", "AXWindow", 7);
        let resolver = MockResolver::new(Some(win), Some(42));
        // last_focused_window_id == 7 → dedup skips it.
        assert!(resolve_redirect_target(&resolver, &config(), Some(7), 10.0, 20.0).is_none());
    }
}
