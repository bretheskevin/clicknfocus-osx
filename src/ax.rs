use accessibility_sys::*;
use core_foundation::base::{CFGetTypeID, CFRelease, CFRetain, CFTypeRef, TCFType};
use core_foundation::string::CFString;
use core_foundation_sys::array::{
    CFArrayGetCount, CFArrayGetTypeID, CFArrayGetValueAtIndex, CFArrayRef,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr;
use std::time::{Duration, Instant};

use crate::bundle::bundle_id_for_pid;
use crate::focus::{FocusResolver, WindowInfo};

/// Messaging timeout (seconds) for AX IPC calls. Bounds how long a synchronous
/// AX call can block the event-tap callback when a target app is unresponsive —
/// without it, an unresponsive app could stall mouse-down delivery and trip the
/// tap's watchdog (see the tap-disabled handling in `event_tap.rs`).
const AX_MESSAGING_TIMEOUT_SECS: f32 = 0.25;

/// Wall-clock budget for the whole parent-walk in `ax_find_window`. Each AX call
/// is individually bounded by `AX_MESSAGING_TIMEOUT_SECS`, but a deep walk could
/// chain `AX_MAX_PARENT_WALK_DEPTH` of them and stall the event-tap callback long
/// enough to trip macOS's tap watchdog. This caps the cumulative time instead.
const AX_WALK_BUDGET: Duration = Duration::from_millis(500);

/// Upper bound on `bundle_cache` entries. The daemon runs indefinitely under
/// launchd, so pids accumulate and are never removed; clearing on overflow keeps
/// memory bounded. The cache is only a latency optimisation, so an occasional
/// flush is harmless.
const BUNDLE_CACHE_MAX: usize = 1024;

// Private (but long-stable) Accessibility API mapping a window AXUIElement to
// its CoreGraphics window id. Gives a *stable* window identity across calls —
// unlike the AXUIElementRef pointer, which AX re-allocates on every copy.
unsafe extern "C" {
    fn _AXUIElementGetWindow(element: AXUIElementRef, out: *mut u32) -> AXError;
}

/// Production implementation of FocusResolver using macOS Accessibility API.
pub struct AxFocusResolver {
    system_wide: AXUIElementRef,
    /// Cache of bundle id per pid. Avoids an Obj-C lookup on every click.
    /// NOTE: pids can be recycled by the OS; a stale entry could mis-identify a
    /// reused pid until the process restarts. Acceptable for this tool — worst
    /// case is one click wrongly (un)ignored.
    bundle_cache: RefCell<HashMap<i32, Option<String>>>,
}

impl AxFocusResolver {
    pub fn new() -> Self {
        // SAFETY: AXUIElementCreateSystemWide has no preconditions and always
        // returns a valid AXUIElementRef that we own (create rule).
        let system_wide = unsafe { AXUIElementCreateSystemWide() };
        // Bound AX IPC latency so an unresponsive app can't stall the tap.
        // SAFETY: system_wide is a valid AXUIElementRef just created above.
        unsafe {
            AXUIElementSetMessagingTimeout(system_wide, AX_MESSAGING_TIMEOUT_SECS);
        }
        Self {
            system_wide,
            bundle_cache: RefCell::new(HashMap::new()),
        }
    }

    /// Bundle id for a pid, memoised. See `bundle_cache` for the staleness note.
    fn cached_bundle_id(&self, pid: i32) -> Option<String> {
        if let Some(cached) = self.bundle_cache.borrow().get(&pid) {
            return cached.clone();
        }
        let bundle = bundle_id_for_pid(pid);
        let mut cache = self.bundle_cache.borrow_mut();
        // Bound memory over long uptime — see BUNDLE_CACHE_MAX.
        if cache.len() >= BUNDLE_CACHE_MAX {
            cache.clear();
        }
        cache.insert(pid, bundle.clone());
        bundle
    }
}

impl Drop for AxFocusResolver {
    fn drop(&mut self) {
        // SAFETY: self.system_wide was created by AXUIElementCreateSystemWide
        // (create rule) in new() and has not been released elsewhere.
        unsafe {
            CFRelease(self.system_wide as *const c_void);
        }
    }
}

/// Helper: convert an AXError code to a human-readable string.
#[allow(non_upper_case_globals)]
fn error_string(err: AXError) -> &'static str {
    match err {
        kAXErrorSuccess => "success",
        kAXErrorFailure => "failure",
        kAXErrorIllegalArgument => "illegal argument",
        kAXErrorInvalidUIElement => "invalid UI element",
        kAXErrorInvalidUIElementObserver => "invalid UI element observer",
        kAXErrorCannotComplete => "cannot complete",
        kAXErrorAttributeUnsupported => "attribute unsupported",
        kAXErrorActionUnsupported => "action unsupported",
        kAXErrorNotificationUnsupported => "notification unsupported",
        kAXErrorNotImplemented => "not implemented",
        kAXErrorNotificationAlreadyRegistered => "notification already registered",
        kAXErrorNotificationNotRegistered => "notification not registered",
        kAXErrorAPIDisabled => "API disabled",
        kAXErrorNoValue => "no value",
        kAXErrorParameterizedAttributeUnsupported => "parameterized attribute unsupported",
        kAXErrorNotEnoughPrecision => "not enough precision",
        _ => "unknown error",
    }
}

/// Helper: get a string attribute from an AXUIElement.
///
/// # Safety
/// `element` must be a valid, non-null AXUIElementRef.
unsafe fn ax_get_string_attribute(element: AXUIElementRef, attribute: &str) -> Option<String> {
    unsafe {
        let attr = CFString::new(attribute);
        let mut value: CFTypeRef = ptr::null();
        let err = AXUIElementCopyAttributeValue(element, attr.as_concrete_TypeRef(), &mut value);
        if err != kAXErrorSuccess || value.is_null() {
            return None;
        }
        // Verify the returned value is actually a CFString before wrapping.
        // AXUIElementCopyAttributeValue can return any CF type; wrapping a
        // non-CFString as CFString would be undefined behavior.
        let string_type_id = core_foundation::string::CFString::type_id();
        let actual_type_id = core_foundation::base::CFGetTypeID(value);
        if actual_type_id != string_type_id {
            CFRelease(value);
            return None;
        }
        // SAFETY: We verified the type ID matches CFString above.
        let cf_string = CFString::wrap_under_create_rule(value as _);
        Some(cf_string.to_string())
    }
}

/// Helper: get the PID from an AXUIElement.
///
/// # Safety
/// `element` must be a valid, non-null AXUIElementRef.
unsafe fn ax_get_pid(element: AXUIElementRef) -> Option<i32> {
    // SAFETY: element validity is a precondition; AXUIElementGetPid writes to
    // our stack-local `pid` and returns an error code if the element is invalid.
    unsafe {
        let mut pid: pid_t = 0;
        let err = AXUIElementGetPid(element, &mut pid);
        if err == kAXErrorSuccess {
            Some(pid)
        } else {
            None
        }
    }
}

/// Helper: get the CoreGraphics window id for a window AXUIElement.
///
/// # Safety
/// `element` must be a valid, non-null AXUIElementRef referring to a window.
unsafe fn ax_window_cgid(element: AXUIElementRef) -> Option<u32> {
    // SAFETY: element validity is a precondition; _AXUIElementGetWindow writes
    // the window id to our stack-local and returns an error code otherwise.
    unsafe {
        let mut window_id: u32 = 0;
        let err = _AXUIElementGetWindow(element, &mut window_id);
        if err == kAXErrorSuccess && window_id != 0 {
            Some(window_id)
        } else {
            None
        }
    }
}

/// Helper: find the application's window whose CoreGraphics window id matches
/// `target_cgid`. Returns an owned AXUIElementRef (caller must release), or None.
///
/// # Safety
/// `app_element` must be a valid, non-null application AXUIElementRef.
unsafe fn ax_find_window_by_cgid(
    app_element: AXUIElementRef,
    target_cgid: u32,
    deadline: Instant,
) -> Option<AXUIElementRef> {
    unsafe {
        let attr = CFString::new(kAXWindowsAttribute);
        let mut value: CFTypeRef = ptr::null();
        let err =
            AXUIElementCopyAttributeValue(app_element, attr.as_concrete_TypeRef(), &mut value);
        if err != kAXErrorSuccess || value.is_null() {
            return None;
        }
        // Verify it really is a CFArray before treating it as one.
        if CFGetTypeID(value) != CFArrayGetTypeID() {
            CFRelease(value);
            return None;
        }
        let array = value as CFArrayRef;
        let count = CFArrayGetCount(array);
        let mut found: Option<AXUIElementRef> = None;
        for i in 0..count {
            // Each ax_window_cgid is an AX IPC; an app with many windows could
            // chain enough to stall the tap callback. Stop once the budget is
            // blown and fall back to the focused window (see AX_WALK_BUDGET).
            if Instant::now() >= deadline {
                log::debug!("ax_find_window_by_cgid: time budget exceeded, giving up");
                break;
            }
            let el = CFArrayGetValueAtIndex(array, i) as AXUIElementRef;
            if el.is_null() {
                continue;
            }
            if ax_window_cgid(el) == Some(target_cgid) {
                // Retain: the array owns its elements and we release it below.
                CFRetain(el as *const c_void);
                found = Some(el);
                break;
            }
        }
        CFRelease(value);
        found
    }
}

/// Helper: copy the application's focused window. Returns an owned
/// AXUIElementRef (caller must release), or None.
///
/// # Safety
/// `app_element` must be a valid, non-null application AXUIElementRef.
unsafe fn ax_copy_focused_window(app_element: AXUIElementRef) -> Option<AXUIElementRef> {
    unsafe {
        let attr = CFString::new(kAXFocusedWindowAttribute);
        let mut value: CFTypeRef = ptr::null();
        let err =
            AXUIElementCopyAttributeValue(app_element, attr.as_concrete_TypeRef(), &mut value);
        if err == kAXErrorSuccess && !value.is_null() {
            // Owned via the copy rule.
            Some(value as AXUIElementRef)
        } else {
            None
        }
    }
}

/// Maximum depth for walking up the AX element hierarchy.
/// Prevents stack overflow from cyclic or excessively deep accessibility trees.
const AX_MAX_PARENT_WALK_DEPTH: u32 = 32;

/// Helper: walk up the element hierarchy to find the window element.
/// Returns the window AXUIElement (caller must release), or None.
///
/// # Safety
/// `element` must be a valid, non-null AXUIElementRef.
unsafe fn ax_find_window(element: AXUIElementRef) -> Option<AXUIElementRef> {
    // SAFETY: Delegates to the depth- and time-limited inner function.
    let deadline = Instant::now() + AX_WALK_BUDGET;
    unsafe { ax_find_window_inner(element, AX_MAX_PARENT_WALK_DEPTH, deadline) }
}

/// Depth- and time-limited recursive helper for `ax_find_window`.
///
/// # Safety
/// `element` must be a valid, non-null AXUIElementRef.
unsafe fn ax_find_window_inner(
    element: AXUIElementRef,
    remaining_depth: u32,
    deadline: Instant,
) -> Option<AXUIElementRef> {
    unsafe {
        // Bail before issuing any AX IPC at this level if the cumulative walk has
        // blown its time budget — a deep, slow AX tree must not stall the
        // event-tap callback (see AX_WALK_BUDGET). Checked first so the budget
        // can't be overshot by this level's role/window lookups.
        if Instant::now() >= deadline {
            log::debug!("ax_find_window: time budget exceeded, giving up");
            return None;
        }

        if let Some(role) = ax_get_string_attribute(element, kAXRoleAttribute)
            && role == "AXWindow"
        {
            // SAFETY: Retain because the caller is responsible for releasing.
            CFRetain(element as *const c_void);
            return Some(element);
        }

        let attr = CFString::new(kAXWindowAttribute);
        let mut value: CFTypeRef = ptr::null();
        let err = AXUIElementCopyAttributeValue(element, attr.as_concrete_TypeRef(), &mut value);
        if err == kAXErrorSuccess && !value.is_null() {
            return Some(value as AXUIElementRef);
        }

        if remaining_depth == 0 {
            log::debug!("ax_find_window: max depth reached, giving up");
            return None;
        }

        let parent_attr = CFString::new(kAXParentAttribute);
        let mut parent: CFTypeRef = ptr::null();
        let err =
            AXUIElementCopyAttributeValue(element, parent_attr.as_concrete_TypeRef(), &mut parent);
        if err == kAXErrorSuccess && !parent.is_null() {
            let result =
                ax_find_window_inner(parent as AXUIElementRef, remaining_depth - 1, deadline);
            CFRelease(parent);
            return result;
        }

        None
    }
}

impl FocusResolver for AxFocusResolver {
    fn frontmost_pid(&self) -> Option<i32> {
        // SAFETY: self.system_wide is a valid AXUIElementRef.
        // We query kAXFocusedApplicationAttribute to get the frontmost app,
        // then extract its PID via AXUIElementGetPid.
        unsafe {
            let attr = CFString::new(kAXFocusedApplicationAttribute);
            let mut value: CFTypeRef = ptr::null();
            let err = AXUIElementCopyAttributeValue(
                self.system_wide,
                attr.as_concrete_TypeRef(),
                &mut value,
            );
            if err != kAXErrorSuccess || value.is_null() {
                return None;
            }
            let app_element = value as AXUIElementRef;
            let pid = ax_get_pid(app_element);
            CFRelease(value);
            pid
        }
    }

    fn window_at_position(&self, x: f64, y: f64) -> Option<WindowInfo> {
        // SAFETY: self.system_wide is a valid AXUIElementRef created in new().
        // All CF objects returned by Copy functions are released on every path.
        unsafe {
            let mut element: AXUIElementRef = ptr::null_mut();
            let err = AXUIElementCopyElementAtPosition(
                self.system_wide,
                x as f32,
                y as f32,
                &mut element,
            );

            if err != kAXErrorSuccess || element.is_null() {
                log::debug!(
                    "AXUIElementCopyElementAtPosition failed at ({}, {}): {}",
                    x,
                    y,
                    error_string(err)
                );
                return None;
            }

            // Get the role of the element directly under cursor (for skip logic)
            let role = ax_get_string_attribute(element, kAXRoleAttribute);

            let window = ax_find_window(element);
            CFRelease(element as *const c_void);

            let window = match window {
                Some(w) => w,
                None => {
                    log::debug!("No window found at ({}, {})", x, y);
                    return None;
                }
            };

            let pid = match ax_get_pid(window) {
                Some(p) => p,
                None => {
                    CFRelease(window as *const c_void);
                    log::debug!("Could not get PID for window at ({}, {})", x, y);
                    return None;
                }
            };

            let bundle_id = self.cached_bundle_id(pid);

            // Stable window identity via the CoreGraphics window id. The dedup
            // id (`window_id`) falls back to the element pointer when the CG id
            // is unavailable — the pointer differs per copy, so it merely
            // disables dedup for this window instead of risking a false "same
            // window" match. `cg_window_id` stays `None` in that case so
            // activation knows it cannot match the exact window.
            let cg_window_id = ax_window_cgid(window);
            let window_id = cg_window_id.map(u64::from).unwrap_or(window as u64);

            CFRelease(window as *const c_void);

            Some(WindowInfo {
                pid,
                bundle_id,
                role,
                window_id,
                cg_window_id,
            })
        }
    }

    fn activate(&self, info: &WindowInfo, raise: bool) {
        // SAFETY: AXUIElementCreateApplication returns a new element (create rule).
        // All CF objects are released on every path. AX setter/action calls are
        // safe to call even if they fail (they return error codes).
        unsafe {
            let app_element = AXUIElementCreateApplication(info.pid);
            if app_element.is_null() {
                log::warn!("Could not create AXUIElement for pid {}", info.pid);
                return;
            }
            // Bound AX IPC so an unresponsive target can't stall the tap callback.
            AXUIElementSetMessagingTimeout(app_element, AX_MESSAGING_TIMEOUT_SECS);

            let frontmost_attr = CFString::new(kAXFrontmostAttribute);
            let true_value = core_foundation::boolean::CFBoolean::true_value();
            let err = AXUIElementSetAttributeValue(
                app_element,
                frontmost_attr.as_concrete_TypeRef(),
                true_value.as_CFTypeRef(),
            );
            if err != kAXErrorSuccess {
                log::warn!(
                    "Failed to set frontmost for pid {}: {}",
                    info.pid,
                    error_string(err)
                );
            }

            // Raise/focus the *clicked* window, matched by its stable CG window
            // id. Falls back to the app's focused window if the CG id is
            // unknown or the exact window can't be located, so multi-window
            // apps still behave reasonably.
            // Bound the whole window-resolution step so a slow/unresponsive
            // target can't stall the event-tap callback (mirrors ax_find_window).
            let deadline = Instant::now() + AX_WALK_BUDGET;
            let target_window = info
                .cg_window_id
                .and_then(|cgid| ax_find_window_by_cgid(app_element, cgid, deadline))
                .or_else(|| ax_copy_focused_window(app_element));

            if let Some(win) = target_window {
                if raise {
                    let raise_action = CFString::new(kAXRaiseAction);
                    let err = AXUIElementPerformAction(win, raise_action.as_concrete_TypeRef());
                    if err != kAXErrorSuccess {
                        log::debug!("AXRaise failed: {}", error_string(err));
                    }
                }

                let main_attr = CFString::new(kAXMainAttribute);
                let true_value = core_foundation::boolean::CFBoolean::true_value();
                let _ = AXUIElementSetAttributeValue(
                    win,
                    main_attr.as_concrete_TypeRef(),
                    true_value.as_CFTypeRef(),
                );

                CFRelease(win as *const c_void);
            }

            CFRelease(app_element as *const c_void);

            // Logged at debug to keep the default log quiet (one line per
            // redirected click would otherwise grow the log file unbounded).
            log::debug!(
                "Focused app pid={} bundle={:?} raise={}",
                info.pid,
                info.bundle_id,
                raise
            );
        }
    }
}
