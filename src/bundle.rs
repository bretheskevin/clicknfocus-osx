/// Returns the bundle identifier (e.g. "com.apple.Safari") for a given process ID,
/// or None if the process is not found or has no bundle identifier.
pub fn bundle_id_for_pid(pid: i32) -> Option<String> {
    use std::ffi::CStr;

    // SAFETY: All Objective-C message sends target known Apple framework classes
    // (NSRunningApplication, NSString). Null checks guard every pointer before use.
    // The CStr::from_ptr call is safe because UTF8String returns a null-terminated
    // C string whose lifetime is tied to the NSString (valid for this scope).
    unsafe {
        // Get NSRunningApplication class
        let cls = objc::runtime::Class::get("NSRunningApplication")?;
        // Call +[NSRunningApplication runningApplicationWithProcessIdentifier:]
        let app: *mut objc::runtime::Object =
            msg_send![cls, runningApplicationWithProcessIdentifier: pid];
        if app.is_null() {
            return None;
        }
        // Call -[NSRunningApplication bundleIdentifier]
        let bundle_id: *mut objc::runtime::Object = msg_send![app, bundleIdentifier];
        if bundle_id.is_null() {
            return None;
        }
        // Convert NSString to &str
        let cstr: *const std::ffi::c_char = msg_send![bundle_id, UTF8String];
        if cstr.is_null() {
            return None;
        }
        Some(CStr::from_ptr(cstr).to_string_lossy().into_owned())
    }
}
