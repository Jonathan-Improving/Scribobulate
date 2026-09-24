//! Put text on the macOS general pasteboard **now**, as data, rather than as a promise.
//!
//! # Why GDK's own write is not used here
//!
//! GTK 4.22's Quartz backend never writes clipboard data eagerly: `gdk_clipboard_set_text`
//! registers a lazy promise (`setDataProvider:forTypes:`), and the data is produced only
//! when some reader asks for it. Its answer to that request,
//! `-[GdkMacosPasteboard pasteboard:item:provideDataForType:]`, waits for the data by
//! looping `g_main_context_iteration` on the default context. When the request arrives
//! inside GDK's own event-source `prepare()` — which it does when GDK drains its
//! autorelease pool there, the drain frees an `NSWindow`, and that window's input-method
//! teardown runs a nested `CFRunLoop` that services the pasteboard — GLib refuses the
//! nested prepare and check, the data never arrives, and the process spins forever
//! logging `g_main_context_prepare() called recursively`.
//!
//! Every macOS clipboard reader that reads eagerly makes this reachable: clipboard-history
//! apps and clipboard-sharing tools read within about a second of every copy. Fixed
//! upstream in GTK 4.24.0 (MR !10216); not in any 4.22 release. With the data written
//! eagerly there is no promise, so there is no fulfil request and no nested wait.
//!
//! # What this covers: text copies, the application's own and GTK's built-in ones
//!
//! This covers the writes the application makes itself, all of which are plain text and
//! go through `crate::clipboard::set_text`, which calls GDK's `set_text` FIRST — so GDK
//! holds the new text as its local content and in-application pastes read it — and
//! this straight after, in the same main-thread block, so the promise GDK registered is
//! replaced before any run-loop turn could deliver a request for it. GTK's own built-in
//! Copy on a text field or selectable label is covered by `crate::clipboard`'s
//! after-handlers, which call this once GTK's handler has run.

use std::ffi::{c_char, c_void, CString};

// Declared by hand rather than by taking `objc2`, as `fullscreen.rs` does: a handful
// of public, ABI-stable symbols.
type Id = *mut c_void;
type Sel = *const c_void;

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Sel;
    /// Untyped on purpose; see `fullscreen.rs`. Each use transmutes it to the exact
    /// signature of the selector it sends, which aarch64 requires.
    fn objc_msgSend();
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    /// `NSPasteboardTypeString` (`public.utf8-plain-text`), an `NSString*` constant.
    static NSPasteboardTypeString: Id;
}

/// `NSUTF8StringEncoding`, `Foundation/NSString.h`.
const NS_UTF8_STRING_ENCODING: usize = 4;

fn class(name: &str) -> Option<Id> {
    let name = CString::new(name).ok()?;
    // SAFETY: `name` is a valid NUL-terminated string for the duration of the call.
    let class = unsafe { objc_getClass(name.as_ptr()) };
    (!class.is_null()).then_some(class)
}

fn selector(name: &str) -> Option<Sel> {
    let name = CString::new(name).ok()?;
    // SAFETY: as above; selectors are interned by the runtime and never freed.
    let sel = unsafe { sel_registerName(name.as_ptr()) };
    (!sel.is_null()).then_some(sel)
}

/// Replace the general pasteboard's contents with `text`, eagerly. Called by
/// `crate::clipboard::set_text` straight after GDK's own `set_text`, to replace the
/// promise GDK just registered. Returns `false` if any step failed, in which case the
/// promise stays — the unpatched behaviour, not a lost copy.
pub(crate) fn write_text(text: &str) -> bool {
    let (
        Some(pasteboard_class),
        Some(string_class),
        Some(general),
        Some(clear),
        Some(set_string),
        Some(alloc),
        Some(init_bytes),
        Some(release),
    ) = (
        class("NSPasteboard"),
        class("NSString"),
        selector("generalPasteboard"),
        selector("clearContents"),
        selector("setString:forType:"),
        selector("alloc"),
        selector("initWithBytes:length:encoding:"),
        selector("release"),
    )
    else {
        return false;
    };

    // SAFETY: every `objc_msgSend` below is called through a pointer carrying the exact
    // signature of the selector it sends — `+[NSPasteboard generalPasteboard]` and
    // `+[NSString alloc]` (→ id), `-[NSPasteboard clearContents]` (→ NSInteger),
    // `-[NSString initWithBytes:length:encoding:]` (const void*, NSUInteger, NSUInteger
    // → id), `-[NSPasteboard setString:forType:]` (id, id → BOOL) and `-[NSObject
    // release]` (→ void). The string is +1 from alloc/init and released exactly once
    // here; the pasteboard copies what it keeps. `text`'s bytes are only read for the
    // duration of the init call. The pool is pushed and popped around the whole
    // exchange so anything AppKit autoreleases is freed here, not deferred into GDK's
    // pool — whose drain is where the defect this module avoids begins.
    unsafe {
        let msg_id: extern "C" fn(Id, Sel) -> Id = std::mem::transmute(objc_msgSend as *const ());
        let msg_isize: extern "C" fn(Id, Sel) -> isize =
            std::mem::transmute(objc_msgSend as *const ());
        let msg_init: extern "C" fn(Id, Sel, *const c_void, usize, usize) -> Id =
            std::mem::transmute(objc_msgSend as *const ());
        let msg_set: extern "C" fn(Id, Sel, Id, Id) -> bool =
            std::mem::transmute(objc_msgSend as *const ());
        let msg_void: extern "C" fn(Id, Sel) = std::mem::transmute(objc_msgSend as *const ());

        let pool = objc_autoreleasePoolPush();
        let written = 'write: {
            let pasteboard = msg_id(pasteboard_class, general);
            if pasteboard.is_null() {
                break 'write false;
            }
            let string = msg_init(
                msg_id(string_class, alloc),
                init_bytes,
                text.as_ptr().cast(),
                text.len(),
                NS_UTF8_STRING_ENCODING,
            );
            if string.is_null() {
                break 'write false;
            }
            msg_isize(pasteboard, clear);
            let ok = msg_set(pasteboard, set_string, string, NSPasteboardTypeString);
            msg_void(string, release);
            ok
        };
        objc_autoreleasePoolPop(pool);
        written
    }
}
