//! Keeping the box awake while a Host serves, as `caffeinate -i` does: on macOS, a power assertion
//! that stops idle sleep, held for as long as it's kept (and released when the process ends,
//! however it ends). A closed lid still sleeps a Mac. Elsewhere, nothing to do.

/// Keeps this box from idle-sleeping until it's dropped.
pub struct Awake(#[allow(dead_code)] Inner); // held only for its Drop

impl Awake {
    /// Keep the box awake, saying `why` to anyone who asks the system (`pmset -g assertions`).
    /// None where there's nothing to hold, or the system refused.
    pub fn hold(why: &str) -> Option<Awake> {
        Inner::hold(why).map(Awake)
    }
}

#[cfg(target_os = "macos")]
use mac::Inner;

#[cfg(not(target_os = "macos"))]
struct Inner;

#[cfg(not(target_os = "macos"))]
impl Inner {
    fn hold(_why: &str) -> Option<Inner> {
        None
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{CString, c_char, c_void};

    type CFStringRef = *const c_void;
    const UTF8: u32 = 0x0800_0100; // kCFStringEncodingUTF8
    const ON: u32 = 255; // kIOPMAssertionLevelOn

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            text: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFRelease(cf: *const c_void);
    }

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOPMAssertionCreateWithName(
            kind: CFStringRef,
            level: u32,
            name: CFStringRef,
            id: *mut u32,
        ) -> i32;
        fn IOPMAssertionRelease(id: u32) -> i32;
    }

    /// A CFString of `text`, released when dropped.
    struct CfString(CFStringRef);

    impl CfString {
        fn new(text: &str) -> Option<CfString> {
            let text = CString::new(text).ok()?;
            // SAFETY: `text` is NUL-terminated and outlives the call, which copies it; the default
            // allocator is null. A null result is checked.
            let s = unsafe { CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), UTF8) };
            (!s.is_null()).then_some(CfString(s))
        }
    }

    impl Drop for CfString {
        fn drop(&mut self) {
            // SAFETY: a non-null CFString this owns, created above and released only here.
            unsafe { CFRelease(self.0) }
        }
    }

    /// An IOKit power assertion against idle sleep; released when dropped.
    pub struct Inner(u32);

    impl Inner {
        pub fn hold(why: &str) -> Option<Inner> {
            let kind = CfString::new("PreventUserIdleSystemSleep")?;
            let name = CfString::new(why)?;
            let mut id = 0;
            // SAFETY: both CFStrings live past the call, which retains what it keeps; `id` is a
            // valid place for the assertion's id.
            let made = unsafe { IOPMAssertionCreateWithName(kind.0, ON, name.0, &mut id) };
            (made == 0).then_some(Inner(id))
        }
    }

    impl Drop for Inner {
        fn drop(&mut self) {
            // SAFETY: an assertion this process made, released only here.
            unsafe { IOPMAssertionRelease(self.0) };
        }
    }
}
