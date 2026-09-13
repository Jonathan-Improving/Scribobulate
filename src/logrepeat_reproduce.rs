//! Reproduction and end-to-end regression guard for the log-repeat collapse
//! (POLICY § Logging, TDD 21.13): a flood of identical GLib warnings must not
//! bury a test run, driven THROUGH GLib's real logging pipeline — not by
//! calling the writer function directly — so this also proves `glib::g_warning!`
//! really does reach [`gtk_log_harness::writer`] the way GTK's own diagnostics
//! do.
//!
//! # Why a third, narrow crate root rather than `tests/*.rs`
//!
//! An ordinary `tests/*.rs` target links this crate *externally* and sees only
//! `pub` items (`src/lib.rs`'s own doc comment states why the tree stays
//! `pub(crate)`), and this file needs `crate::gtk_log_harness::install_once`,
//! which is `pub(crate)`. `src/gtk_suite.rs` solves the identical problem for
//! the whole GTK suite by being a second crate root beside `lib.rs`, but
//! mirrors `lib.rs`'s *entire* module list because it runs bodies scattered
//! across every module. This file needs exactly two leaf modules
//! (`logrepeat`, `gtk_log_harness`), so it declares just those rather than
//! paying — and risking drifting — a second copy of that full inventory.
//!
//! # Why redirect a real fd rather than a pipe
//!
//! `gtk_suite.rs`'s own per-case timeout comment explains the hazard directly:
//! intercepting output through a pipe needs a reader draining it concurrently,
//! and a body that out-writes an undrained pipe *blocks* — turning a
//! would-be-noisy pass into a hang, the opposite of what a flood-safety test
//! should risk. `glib::log_writer_default` (and this file's own `eprintln!`
//! summaries) write to fd 2 through ordinary buffered stdio; redirecting fd 2
//! to a temp FILE for the duration has none of that hazard — a file never
//! blocks a writer, and nothing needs to be draining it while the burst runs.
//! The output is read back only after the redirect is undone.
//!
//! # Why the writer func is exercised here, not shared with `gtk_suite.rs`
//!
//! `glib::log_set_writer_func` panics on a second call in one process
//! (`gtk_log_harness`'s own doc comment), so this target's exclusive control
//! over the process's ONE writer func is exactly why it needs its own process —
//! a `[[test]]` target with `harness = false` already runs as one.
//!
//! Run: `cargo test --features gtk-integration-tests --test logrepeat_reproduce`
//!
//! # Why the crate-level `allow`
//!
//! Cargo builds this target with `--cfg test` but **not** `--test` (`gtk_suite.rs`
//! measured and documents this), so `logrepeat.rs`'s own `#[cfg(test)] mod tests`
//! is compiled here — its `--cfg test` gate is satisfied — but every `#[test]`
//! item inside it is stripped, orphaning the small helpers only those tests call
//! (`TestLevel`, `key`) into `never used`. Silencing that is correct in *this*
//! file, which ships nothing; the same code is linted normally by `cargo test
//! --lib`, where it is not orphaned.
#![allow(dead_code, unused_imports)]

mod gtk_log_harness;
mod logrepeat;

use std::io::Write;
use std::os::unix::io::AsRawFd;

/// Repeats of the identical flood message. Large enough to be a real stress
/// case (the recorded hang was ~99 million) while keeping this target's own
/// runtime short — the collapse core is `O(1)` per record, so 200,000 vs 99
/// million proves the same property; see `logrepeat`'s own
/// `the_milestone_scheme_bounds…` reasoning for why the bound does not depend
/// on the repeat count.
const REPEATS: u64 = 200_000;

const FLOOD_MESSAGE: &str = "identical flood message 7f3a1c";
const BREAK_MESSAGE: &str = "the flood is over 7f3a1c";
const DOMAIN: &str = "LogRepeatRepro";

fn main() {
    let tmp = tempfile::NamedTempFile::new().expect("temp file for the redirected stderr");
    let saved_stderr = dup_fd(2);
    redirect_fd(tmp.as_raw_fd(), 2);

    gtk_log_harness::install_once();

    for _ in 0..REPEATS {
        // THROUGH GLib's real logging pipeline, not a direct call into the
        // writer function — `g_log` (what this macro expands to) is what GTK's
        // own diagnostics use, and it is what routes into
        // `g_log_structured_array`, which is what invokes the registered
        // writer func. Calling `gtk_log_harness::writer` directly would prove
        // only that the Rust function does the right thing with an input we
        // constructed by hand, not that GLib actually delivers one.
        glib::g_warning!(DOMAIN, "{}", FLOOD_MESSAGE);
    }
    // Break the run: one distinct message, so the flood's TRUE final count
    // (200,000 — not a power of ten) is still reported via the "closed"
    // summary, rather than silently left at whatever the last milestone said.
    glib::g_warning!(DOMAIN, "{}", BREAK_MESSAGE);

    let _ = std::io::stderr().flush();
    restore_fd(saved_stderr, 2);
    close_fd(saved_stderr);

    let text = std::fs::read_to_string(tmp.path()).expect("read the redirected output back");
    let lines: Vec<&str> = text.lines().collect();
    // An assertion message is itself diagnostic OUTPUT — dumping the whole
    // capture into it would defeat the point of this file the moment collapse
    // regresses and a failure is what needs reading (the exact flood this
    // change exists to bound). Bounded to a small prefix either way.
    let preview = || preview(&lines);

    // Unbounded, `REPEATS` identical warnings would be roughly `2 * REPEATS`
    // lines (GLib's default writer prints a blank line plus the record per
    // call — confirmed against a standalone probe of `glib::g_warning!`'s
    // actual stderr shape). Bounded: one first occurrence (2 lines), the five
    // milestones a 200,000-repeat run crosses — 10, 100, 1_000, 10_000,
    // 100_000 — at one `eprintln!` line each, one closing summary, and the
    // breaking message's own first occurrence (2 lines): comfortably under 100
    // regardless of `REPEATS`.
    assert!(
        lines.len() < 100,
        "expected a small, BOUNDED number of lines for a {REPEATS}-repeat flood \
         (unbounded would be roughly {}), got {} — the collapse did not engage. \
         First lines:\n{}",
        REPEATS * 2,
        lines.len(),
        preview()
    );

    // The first occurrence must be GLib's own default format, VERBATIM — the
    // whole reason `gtk_log_harness::writer` delegates rather than reformats
    // (both GTK harnesses grep this literal shape; `logging::forward`'s bridge
    // deliberately does NOT preserve it, and is a different code path).
    assert!(
        text.contains(&format!("{DOMAIN}-WARNING **:")),
        "the first occurrence must carry GLib's own domain-WARNING marker. First \
         lines:\n{}",
        preview()
    );
    assert!(
        text.contains(FLOOD_MESSAGE),
        "the first occurrence's message text must be verbatim. First lines:\n{}",
        preview()
    );

    // GLib's own writer (the "(process:PID):" block header it alone prints) is
    // invoked exactly twice: once for the flood's genuine first occurrence,
    // once for the breaking message's genuine first occurrence — never once
    // per repeat, and never for a milestone/closing summary (those are this
    // module's own `eprintln!` lines, with no such header).
    let native_glib_blocks = text.matches("(process:").count();
    assert_eq!(
        native_glib_blocks,
        2,
        "GLib's own writer must run only for the two genuine first occurrences, not once \
         per repeat. First lines:\n{}",
        preview()
    );

    // Bounded growth was visible WHILE the flood was in progress, not only
    // once it ended.
    assert!(
        text.contains("repeated 10 times"),
        "the first milestone (10 repeats) must be visible. First lines:\n{}",
        preview()
    );
    assert!(
        text.contains("repeated 100000 times"),
        "the largest milestone a 200,000-repeat run crosses (10^5) must be visible. First \
         lines:\n{}",
        preview()
    );

    // The run's TRUE final count — not a milestone value — is reported once it
    // breaks (TDD 21.13).
    assert!(
        text.contains(&format!("repeated {REPEATS} times")),
        "the broken run's true final count must be reported once it ends. First lines:\n{}",
        preview()
    );

    println!(
        "ok — {} lines for a {REPEATS}-repeat flood (unbounded would be roughly {})",
        lines.len(),
        REPEATS * 2
    );
}

/// The first 20 lines (plus a truncation note past that), for use in an
/// assertion message — never the whole capture. An assertion failure here means
/// collapse itself regressed, so the raw capture is exactly the unbounded flood
/// this file exists to prevent; echoing all of it back would reproduce the
/// defect inside the report of the defect.
fn preview(lines: &[&str]) -> String {
    const MAX: usize = 20;
    if lines.len() <= MAX {
        lines.join("\n")
    } else {
        format!(
            "{}\n… ({} more lines omitted)",
            lines[..MAX].join("\n"),
            lines.len() - MAX
        )
    }
}

/// `dup(2)` a file descriptor so it can be restored later.
///
/// SAFETY: `dup` is async-signal-safe and side-effect-free beyond allocating a
/// new fd; the argument is a small fixed constant (`2`, stderr) at every call
/// site in this file, always valid for the process's lifetime.
fn dup_fd(fd: i32) -> i32 {
    let dup = unsafe { libc::dup(fd) };
    assert!(
        dup >= 0,
        "dup({fd}) failed: {}",
        std::io::Error::last_os_error()
    );
    dup
}

/// Make `to` a copy of `from` (`dup2`), closing whatever `to` previously named.
///
/// SAFETY: `dup2` is async-signal-safe; both arguments are valid, already-open
/// descriptors at every call site in this file.
fn redirect_fd(from: i32, to: i32) {
    let result = unsafe { libc::dup2(from, to) };
    assert!(
        result >= 0,
        "dup2({from}, {to}) failed: {}",
        std::io::Error::last_os_error()
    );
}

/// Alias of [`redirect_fd`] used at the restore call site purely for readability
/// — `dup2(saved, target)` reads the same either way round, so the two names
/// exist to make "redirect into the temp file" and "restore the original" read
/// as different operations at their call sites.
fn restore_fd(from: i32, to: i32) {
    redirect_fd(from, to);
}

/// SAFETY: `close` is async-signal-safe; `fd` is the process's own saved
/// duplicate from [`dup_fd`], not shared with anything else.
fn close_fd(fd: i32) {
    unsafe {
        libc::close(fd);
    }
}
