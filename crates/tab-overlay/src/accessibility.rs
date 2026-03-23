use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::string::CFString;
use std::ffi::c_void;
use std::ptr;

// AXUIElement raw FFI bindings (ApplicationServices framework)
#[allow(non_camel_case_types)]
type AXUIElementRef = *const c_void;
#[allow(non_camel_case_types)]
type AXError = i32;

const K_AX_ERROR_SUCCESS: AXError = 0;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: core_foundation::string::CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
}

/// Check if this process has accessibility permissions.
pub fn is_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// Get the frontmost application's PID.
pub fn frontmost_app_pid() -> Option<i32> {
    use objc2_app_kit::NSWorkspace;

    let workspace = NSWorkspace::sharedWorkspace();
    let app = workspace.frontmostApplication()?;
    Some(app.processIdentifier())
}

/// Window position and size in screen coordinates.
#[derive(Debug, Clone, Copy)]
pub struct WindowFrame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Get the focused window's frame for a given PID using Accessibility API.
pub fn get_focused_window_frame(pid: i32) -> Option<WindowFrame> {
    if !is_trusted() {
        tracing::warn!("accessibility not trusted - cannot get window frame");
        return None;
    }

    unsafe {
        let app_element = AXUIElementCreateApplication(pid);
        if app_element.is_null() {
            return None;
        }

        // Get focused window
        let attr = CFString::new("AXFocusedWindow");
        let mut window_ref: CFTypeRef = ptr::null();
        let err = AXUIElementCopyAttributeValue(
            app_element,
            attr.as_concrete_TypeRef(),
            &mut window_ref,
        );
        CFRelease(app_element as CFTypeRef);

        if err != K_AX_ERROR_SUCCESS || window_ref.is_null() {
            return None;
        }

        let window_element = window_ref as AXUIElementRef;

        // Get position
        let pos_attr = CFString::new("AXPosition");
        let mut pos_value: CFTypeRef = ptr::null();
        let err = AXUIElementCopyAttributeValue(
            window_element,
            pos_attr.as_concrete_TypeRef(),
            &mut pos_value,
        );

        let (x, y) = if err == K_AX_ERROR_SUCCESS && !pos_value.is_null() {
            let point = ax_value_to_point(pos_value);
            CFRelease(pos_value);
            point
        } else {
            CFRelease(window_ref);
            return None;
        };

        // Get size
        let size_attr = CFString::new("AXSize");
        let mut size_value: CFTypeRef = ptr::null();
        let err = AXUIElementCopyAttributeValue(
            window_element,
            size_attr.as_concrete_TypeRef(),
            &mut size_value,
        );

        let (width, height) = if err == K_AX_ERROR_SUCCESS && !size_value.is_null() {
            let size = ax_value_to_size(size_value);
            CFRelease(size_value);
            size
        } else {
            CFRelease(window_ref);
            return None;
        };

        CFRelease(window_ref);

        Some(WindowFrame {
            x,
            y,
            width,
            height,
        })
    }
}

/// Calculate the screen position of the cursor based on terminal window frame
/// and shell-reported cursor position.
pub fn calculate_cursor_screen_position(
    window: &WindowFrame,
    cursor_row: u32,
    cursor_col: u32,
    terminal_cols: u32,
    terminal_rows: u32,
    title_bar_height: f64,
) -> (f64, f64) {
    let content_width = window.width;
    let content_height = window.height - title_bar_height;

    let cell_width = content_width / terminal_cols as f64;
    let cell_height = content_height / terminal_rows as f64;

    let screen_x = window.x + (cursor_col as f64 * cell_width);
    // macOS AX coordinates: origin top-left, y goes down
    let screen_y = window.y + title_bar_height + ((cursor_row as f64 + 1.0) * cell_height);

    (screen_x, screen_y)
}

// ── AXValue helpers ──

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn AXValueGetValue(
        value: CFTypeRef,
        value_type: u32,
        value_ptr: *mut c_void,
    ) -> bool;
}

const K_AX_VALUE_CGPOINT: u32 = 1;
const K_AX_VALUE_CGSIZE: u32 = 2;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}

unsafe fn ax_value_to_point(value: CFTypeRef) -> (f64, f64) {
    let mut point = CGPoint { x: 0.0, y: 0.0 };
    AXValueGetValue(
        value,
        K_AX_VALUE_CGPOINT,
        &mut point as *mut CGPoint as *mut c_void,
    );
    (point.x, point.y)
}

unsafe fn ax_value_to_size(value: CFTypeRef) -> (f64, f64) {
    let mut size = CGSize {
        width: 0.0,
        height: 0.0,
    };
    AXValueGetValue(
        value,
        K_AX_VALUE_CGSIZE,
        &mut size as *mut CGSize as *mut c_void,
    );
    (size.width, size.height)
}
