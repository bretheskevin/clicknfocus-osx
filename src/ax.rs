use accessibility_sys::*;
use core_foundation::base::{CFRelease, CFRetain, CFTypeRef, TCFType};
use core_foundation::string::CFString;
use std::ffi::c_void;
use std::ptr;

use crate::bundle::bundle_id_for_pid;
use crate::focus::{FocusResolver, WindowInfo};

/// Production implementation of FocusResolver using macOS Accessibility API.
pub struct AxFocusResolver {
    system_wide: AXUIElementRef,
}

impl AxFocusResolver {
    pub fn new() -> Self {
        // SAFETY: AXUIElementCreateSystemWide has no preconditions and always
        // returns a valid AXUIElementRef that we own (create rule).
        let system_wide = unsafe { AXUIElementCreateSystemWide() };
        Self { system_wide }
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

/// Maximum depth for walking up the AX element hierarchy.
/// Prevents stack overflow from cyclic or excessively deep accessibility trees.
const AX_MAX_PARENT_WALK_DEPTH: u32 = 32;

/// Helper: walk up the element hierarchy to find the window element.
/// Returns the window AXUIElement (caller must release), or None.
///
/// # Safety
/// `element` must be a valid, non-null AXUIElementRef.
unsafe fn ax_find_window(element: AXUIElementRef) -> Option<AXUIElementRef> {
    // SAFETY: Delegates to the depth-limited inner function.
    unsafe { ax_find_window_inner(element, AX_MAX_PARENT_WALK_DEPTH) }
}

/// Depth-limited recursive helper for `ax_find_window`.
///
/// # Safety
/// `element` must be a valid, non-null AXUIElementRef.
unsafe fn ax_find_window_inner(
    element: AXUIElementRef,
    remaining_depth: u32,
) -> Option<AXUIElementRef> {
    unsafe {
        // First check if the element itself is a window
        if let Some(role) = ax_get_string_attribute(element, kAXRoleAttribute)
            && role == "AXWindow"
        {
            // SAFETY: Retain because the caller is responsible for releasing.
            CFRetain(element as *const c_void);
            return Some(element);
        }

        // Try to get the AXWindow attribute
        let attr = CFString::new(kAXWindowAttribute);
        let mut value: CFTypeRef = ptr::null();
        let err = AXUIElementCopyAttributeValue(element, attr.as_concrete_TypeRef(), &mut value);
        if err == kAXErrorSuccess && !value.is_null() {
            return Some(value as AXUIElementRef);
        }

        // Stop recursion if we've reached the depth limit
        if remaining_depth == 0 {
            log::debug!("ax_find_window: max depth reached, giving up");
            return None;
        }

        // Walk up via AXParent
        let parent_attr = CFString::new(kAXParentAttribute);
        let mut parent: CFTypeRef = ptr::null();
        let err =
            AXUIElementCopyAttributeValue(element, parent_attr.as_concrete_TypeRef(), &mut parent);
        if err == kAXErrorSuccess && !parent.is_null() {
            let result = ax_find_window_inner(parent as AXUIElementRef, remaining_depth - 1);
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

            // Find the window
            let window = ax_find_window(element);
            CFRelease(element as *const c_void);

            let window = match window {
                Some(w) => w,
                None => {
                    log::debug!("No window found at ({}, {})", x, y);
                    return None;
                }
            };

            // Get PID
            let pid = match ax_get_pid(window) {
                Some(p) => p,
                None => {
                    CFRelease(window as *const c_void);
                    log::debug!("Could not get PID for window at ({}, {})", x, y);
                    return None;
                }
            };

            // Get bundle ID from PID
            let bundle_id = bundle_id_for_pid(pid);

            // Use the window pointer as a unique ID
            let window_id = window as u64;

            CFRelease(window as *const c_void);

            Some(WindowInfo {
                pid,
                bundle_id,
                role,
                window_id,
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

            // Set app as frontmost
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

            // Query the focused window once for both raise and set-main operations.
            let focused_attr = CFString::new(kAXFocusedWindowAttribute);
            let mut win_ref: CFTypeRef = ptr::null();
            let err = AXUIElementCopyAttributeValue(
                app_element,
                focused_attr.as_concrete_TypeRef(),
                &mut win_ref,
            );
            if err == kAXErrorSuccess && !win_ref.is_null() {
                // Raise the window if requested
                if raise {
                    let raise_action = CFString::new(kAXRaiseAction);
                    let err = AXUIElementPerformAction(
                        win_ref as AXUIElementRef,
                        raise_action.as_concrete_TypeRef(),
                    );
                    if err != kAXErrorSuccess {
                        log::debug!("AXRaise failed: {}", error_string(err));
                    }
                }

                // Set the window as main
                let main_attr = CFString::new(kAXMainAttribute);
                let true_value = core_foundation::boolean::CFBoolean::true_value();
                let _ = AXUIElementSetAttributeValue(
                    win_ref as AXUIElementRef,
                    main_attr.as_concrete_TypeRef(),
                    true_value.as_CFTypeRef(),
                );

                CFRelease(win_ref);
            }

            CFRelease(app_element as *const c_void);

            log::info!(
                "Focused app pid={} bundle={:?} raise={}",
                info.pid,
                info.bundle_id,
                raise
            );
        }
    }
}
