/// Information about a window resolved from a screen position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    /// The process ID owning the window.
    pub pid: i32,
    /// The bundle identifier of the owning app (e.g. "com.apple.Safari"), if available.
    pub bundle_id: Option<String>,
    /// The AX role of the resolved element (e.g. "AXWindow", "AXMenuBar").
    pub role: Option<String>,
    /// An opaque identifier for the window, used to detect "same window" across calls.
    /// In production this is the stringified pointer of the AXUIElement window.
    pub window_id: u64,
}

/// Trait abstracting macOS Accessibility API calls.
/// Implement with real AX calls for production; mock for tests.
pub trait FocusResolver {
    /// Resolve the window/app at the given screen coordinates.
    /// Returns None if no valid window is found at that position.
    fn window_at_position(&self, x: f64, y: f64) -> Option<WindowInfo>;

    /// Make the app with the given PID frontmost and focus its window.
    /// If `raise` is true, also raise the window above other windows.
    fn activate(&self, info: &WindowInfo, raise: bool);
}

/// Configuration for the focus decision engine.
pub struct FocusConfig {
    /// Whether to also raise windows when focusing.
    pub raise: bool,
    /// Bundle IDs to ignore (never focus these apps).
    pub ignore_bundle_ids: Vec<String>,
    /// Our own process ID (never focus ourselves).
    pub own_pid: i32,
}

/// Roles that should never be focused (system UI elements).
const SKIP_ROLES: &[&str] = &["AXMenuBar", "AXMenu", "AXMenuItem"];

/// Bundle IDs that should always be skipped (Dock, etc.).
const ALWAYS_SKIP_BUNDLES: &[&str] = &["com.apple.dock"];

/// Decides whether the given window should be focused,
/// considering the current state and configuration.
/// Returns true if activation should proceed.
pub fn should_focus(
    info: &WindowInfo,
    config: &FocusConfig,
    last_focused_window_id: Option<u64>,
) -> bool {
    // Skip if it's our own process
    if info.pid == config.own_pid {
        log::debug!("Skipping: own process (pid={})", info.pid);
        return false;
    }

    // Skip system UI roles
    if let Some(ref role) = info.role
        && SKIP_ROLES.iter().any(|r| r == role)
    {
        log::debug!("Skipping: system role {:?}", role);
        return false;
    }

    // Skip always-ignored bundles (Dock)
    if let Some(ref bundle) = info.bundle_id
        && ALWAYS_SKIP_BUNDLES.iter().any(|b| b == bundle)
    {
        log::debug!("Skipping: always-skip bundle {:?}", bundle);
        return false;
    }

    // Skip user-configured ignore list
    if let Some(ref bundle) = info.bundle_id
        && config.ignore_bundle_ids.iter().any(|b| b == bundle)
    {
        log::debug!("Skipping: user-ignored bundle {:?}", bundle);
        return false;
    }

    // Skip if same window is already focused
    if let Some(last_id) = last_focused_window_id
        && info.window_id == last_id
    {
        log::debug!("Skipping: same window (id={})", info.window_id);
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn make_config() -> FocusConfig {
        FocusConfig {
            raise: false,
            ignore_bundle_ids: vec!["com.example.ignored".to_string()],
            own_pid: 999,
        }
    }

    fn make_window(pid: i32, bundle_id: &str, role: &str, window_id: u64) -> WindowInfo {
        WindowInfo {
            pid,
            bundle_id: Some(bundle_id.to_string()),
            role: Some(role.to_string()),
            window_id,
        }
    }

    #[test]
    fn test_should_focus_normal_window() {
        let config = make_config();
        let info = make_window(100, "com.example.app", "AXWindow", 1);
        assert!(should_focus(&info, &config, None));
    }

    #[test]
    fn test_skip_own_process() {
        let config = make_config();
        let info = make_window(999, "com.example.self", "AXWindow", 1);
        assert!(!should_focus(&info, &config, None));
    }

    #[test]
    fn test_skip_menu_bar() {
        let config = make_config();
        let info = make_window(100, "com.apple.systemuiserver", "AXMenuBar", 2);
        assert!(!should_focus(&info, &config, None));
    }

    #[test]
    fn test_skip_menu_item() {
        let config = make_config();
        let info = make_window(100, "com.apple.systemuiserver", "AXMenuItem", 3);
        assert!(!should_focus(&info, &config, None));
    }

    #[test]
    fn test_skip_dock() {
        let config = make_config();
        let info = make_window(200, "com.apple.dock", "AXWindow", 4);
        assert!(!should_focus(&info, &config, None));
    }

    #[test]
    fn test_skip_user_ignored_bundle() {
        let config = make_config();
        let info = make_window(300, "com.example.ignored", "AXWindow", 5);
        assert!(!should_focus(&info, &config, None));
    }

    #[test]
    fn test_skip_same_window() {
        let config = make_config();
        let info = make_window(100, "com.example.app", "AXWindow", 42);
        assert!(!should_focus(&info, &config, Some(42)));
    }

    #[test]
    fn test_focus_different_window_same_app() {
        let config = make_config();
        let info = make_window(100, "com.example.app", "AXWindow", 43);
        // Different window_id from the last focused one -- should focus
        assert!(should_focus(&info, &config, Some(42)));
    }

    #[test]
    fn test_focus_window_no_bundle_id() {
        let config = make_config();
        let info = WindowInfo {
            pid: 100,
            bundle_id: None,
            role: Some("AXWindow".to_string()),
            window_id: 10,
        };
        assert!(should_focus(&info, &config, None));
    }

    #[test]
    fn test_focus_window_no_role() {
        let config = make_config();
        let info = WindowInfo {
            pid: 100,
            bundle_id: Some("com.example.app".to_string()),
            role: None,
            window_id: 11,
        };
        assert!(should_focus(&info, &config, None));
    }

    #[test]
    fn test_skip_menu_role() {
        let config = make_config();
        let info = make_window(100, "com.apple.systemuiserver", "AXMenu", 12);
        assert!(!should_focus(&info, &config, None));
    }

    // --- Mock-based integration tests ---

    /// Mock FocusResolver for testing the full flow.
    struct MockResolver {
        windows: Vec<(f64, f64, WindowInfo)>,
        activated: RefCell<Vec<(WindowInfo, bool)>>,
    }

    impl MockResolver {
        fn new() -> Self {
            Self {
                windows: Vec::new(),
                activated: RefCell::new(Vec::new()),
            }
        }

        fn add_window(&mut self, x: f64, y: f64, info: WindowInfo) {
            self.windows.push((x, y, info));
        }
    }

    impl FocusResolver for MockResolver {
        fn window_at_position(&self, x: f64, y: f64) -> Option<WindowInfo> {
            // Find the first window whose position matches (within 1.0 tolerance)
            self.windows
                .iter()
                .find(|(wx, wy, _)| (wx - x).abs() < 1.0 && (wy - y).abs() < 1.0)
                .map(|(_, _, info)| info.clone())
        }

        fn activate(&self, info: &WindowInfo, raise: bool) {
            self.activated.borrow_mut().push((info.clone(), raise));
        }
    }

    #[test]
    fn test_mock_resolver_finds_window() {
        let mut resolver = MockResolver::new();
        resolver.add_window(100.0, 200.0, make_window(1, "com.test.app", "AXWindow", 1));

        let result = resolver.window_at_position(100.0, 200.0);
        assert!(result.is_some());
        assert_eq!(result.unwrap().pid, 1);
    }

    #[test]
    fn test_mock_resolver_no_window() {
        let resolver = MockResolver::new();
        let result = resolver.window_at_position(500.0, 500.0);
        assert!(result.is_none());
    }

    #[test]
    fn test_mock_resolver_activate_records() {
        let resolver = MockResolver::new();
        let info = make_window(1, "com.test.app", "AXWindow", 1);
        resolver.activate(&info, false);
        resolver.activate(&info, true);

        let activated = resolver.activated.borrow();
        assert_eq!(activated.len(), 2);
        assert!(!activated[0].1); // first call: raise=false
        assert!(activated[1].1); // second call: raise=true
    }

    #[test]
    fn test_full_focus_flow_with_mock() {
        let mut resolver = MockResolver::new();
        let window = make_window(100, "com.test.safari", "AXWindow", 42);
        resolver.add_window(500.0, 300.0, window);
        let config = make_config();

        // Resolve window at position
        let info = resolver.window_at_position(500.0, 300.0).unwrap();

        // Should focus: new window, different app, not ignored
        assert!(should_focus(&info, &config, None));

        // Activate
        resolver.activate(&info, config.raise);

        let activated = resolver.activated.borrow();
        assert_eq!(activated.len(), 1);
        assert_eq!(activated[0].0.pid, 100);
        assert!(!activated[0].1); // raise=false (default config)
    }

    #[test]
    fn test_full_focus_flow_skip_same_window() {
        let mut resolver = MockResolver::new();
        let window = make_window(100, "com.test.safari", "AXWindow", 42);
        resolver.add_window(500.0, 300.0, window);
        let config = make_config();

        let info = resolver.window_at_position(500.0, 300.0).unwrap();

        // Should NOT focus: same window already focused
        assert!(!should_focus(&info, &config, Some(42)));

        // No activation should happen
        let activated = resolver.activated.borrow();
        assert_eq!(activated.len(), 0);
    }
}
