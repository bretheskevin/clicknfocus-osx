use accessibility_sys::{AXIsProcessTrustedWithOptions, kAXTrustedCheckOptionPrompt};
use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;

/// Check if this process has Accessibility permissions.
/// If `prompt` is true, macOS will show the system dialog asking the user to grant access.
/// Returns true if permission is already granted.
pub fn check_accessibility_permission(prompt: bool) -> bool {
    let prompt_value = if prompt {
        CFBoolean::true_value()
    } else {
        CFBoolean::false_value()
    };

    // SAFETY: kAXTrustedCheckOptionPrompt is a static CFStringRef from the
    // Accessibility framework (get rule -- we do not own it, so wrap_under_get_rule).
    let key = unsafe { CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt) };
    let dict = CFDictionary::from_CFType_pairs(&[(key, prompt_value)]);

    // SAFETY: dict is a valid CFDictionary with the expected key-value pair.
    unsafe { AXIsProcessTrustedWithOptions(dict.as_concrete_TypeRef()) }
}
