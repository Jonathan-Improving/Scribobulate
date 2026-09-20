//! How much of GLib's shared I/O thread pool this application may hold, and who holds it.
//!
//! One table, one sum, one compile-time check. The pool's size and the cliff at its
//! base size are argued in [`super::pool`]'s module comment — that evidence is not
//! repeated here; this module owns only the ALLOCATION of it.
//!
//! # Why a table rather than a constant per consumer
//!
//! There were three admission gates over one pool — document operations, animation
//! decodes, status-bar counts — each declaring its own cap, and the combined budget was
//! stated **in prose, in a doc comment, in the file that does not own the cliff**:
//!
//! > *"Two more from animation decodes brings the combined worst case to six, still
//! > four short of the measured cliff at the tenth blocked task."*
//!
//! It was seven and three by the time anyone read it again. The sentence went false the
//! moment a third consumer appeared, and nothing could notice: no code computed the
//! sum, so there was no place for the discrepancy to show up. A budget narrated in a
//! comment is not a budget.
//!
//! So the caps live here, each consumer asks for its own, and the sum is asserted
//! against the headroom at compile time. Adding a fourth consumer means adding a
//! variant — which does not compile until its cap is declared, and does not link until
//! the sum still fits.

/// Declare the pool's consumers once: the enum, the exhaustive list, and each cap.
///
/// **A macro because the three must not be able to disagree.** They were three separate
/// declarations, and the failure that leaves is silent: a fourth consumer added to the
/// enum and to `cap` — which the exhaustive match forces — but left out of the list
/// under-counts `TOTAL`, so the compile-time budget assertion passes on a number that is
/// not the budget. The exhaustive match cannot help, because it is satisfied; nothing in
/// the language ties a list to a set of variants. Generating all three from one place is
/// what ties them.
macro_rules! consumers {
    ($( $(#[$meta:meta])* $name:ident => $cap:expr ),+ $(,)?) => {
        /// Everything in this application that occupies a thread of GLib's shared I/O
        /// pool.
        ///
        /// Exhaustive on purpose. A consumer that dispatches to that pool without a
        /// variant here is spending a budget nobody accounted for, and the pool it is
        /// spending from is shared with the crash-recovery snapshot writer. The
        /// `clippy.toml` ban on `gio::spawn_blocking` is what stops one appearing.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum Consumer {
            $( $(#[$meta])* $name, )+
        }

        /// Every consumer, generated alongside the enum, so it cannot omit one.
        pub(crate) const EVERY: &[Consumer] = &[ $( Consumer::$name, )+ ];

        /// How many pool threads `who` may hold at once.
        pub(crate) const fn cap(who: Consumer) -> usize {
            match who {
                $( Consumer::$name => $cap, )+
            }
        }
    };
}

consumers! {
    /// Every read and write of a user document, through [`super::pool`].
    ///
    /// Four: the live-reload monitor fires one read per affected tab, and one checkout
    /// across a documentation tree rewrites every open document at once.
    Document => 4,
    /// Animation frame decodes, through `animation::worker`.
    ///
    /// Two: a decode is CPU-bound and short, and more buys nothing a reader can see.
    AnimationDecode => 2,
    /// The status bar's word and line-ending counts, through `window::statusbar`.
    ///
    /// One. The count is a single whole-document pass, and a second concurrent one
    /// would be counting a document nobody is looking at; the status bar queues the
    /// rest one deep PER TAB rather than holding more threads.
    WordCount => 1,
}

/// GLib's base I/O pool size — `G_TASK_POOL_SIZE`, and the cliff.
///
/// Not a tuning knob: it is a constant of the toolkit, measured at 0.2ms for nine
/// blocked tasks and 206ms for ten. [`super::pool`]'s module comment carries the table.
pub(crate) const POOL_THREADS: usize = 10;

/// Threads that must stay free for the crash-recovery snapshot writer.
///
/// The writer shares this pool and has no way to get its own — there is no public API
/// for a second pool, and `io_priority` does not help, because GLib sorts its queue by
/// a flag set only for tasks queued from inside a pool thread. So headroom is the only
/// lever, and a late snapshot is unsaved work left unprotected for exactly that long.
pub(crate) const RESERVED_FOR_SNAPSHOT: usize = 3;

/// The application's worst case: every gate simultaneously full.
///
/// Summed from [`EVERY`] rather than written out, because a hand-written sum is the
/// prose failure this module exists to retire, moved one level in: a fourth consumer
/// added to the enum and to `cap` — which the exhaustive match forces — but left out of
/// an arithmetic expression here would compile, link and satisfy the assertion below on
/// an under-count.
const TOTAL: usize = {
    let mut total = 0;
    let mut i = 0;
    while i < EVERY.len() {
        total += cap(EVERY[i]);
        i += 1;
    }
    total
};

/// The check the prose could not perform.
///
/// A compile error rather than a test, because the failure it guards is a *sum* that no
/// single consumer can see — the file that raises a cap is never the file that would
/// have noticed, which is exactly how the last one went false unremarked.
const _: () = assert!(
    TOTAL + RESERVED_FOR_SNAPSHOT <= POOL_THREADS,
    "the admission gates' combined worst case, plus the snapshot writer's reserve, \
     exceeds GLib's base I/O pool — raising a cap here means the crash-recovery \
     snapshot can cross the cliff and arrive hundreds of milliseconds late. Lower a \
     cap, or argue the reserve down deliberately; do not raise POOL_THREADS, which is \
     the toolkit's number and not ours."
);

#[cfg(test)]
mod tests {
    use super::{cap, EVERY, POOL_THREADS, RESERVED_FOR_SNAPSHOT, TOTAL};

    /// The arithmetic, stated as a number a reader can check by eye.
    ///
    /// `EVERY` no longer needs a completeness test: it is generated from the same
    /// declaration as the enum, so a variant that is not in it does not exist. The
    /// previous version of this test counted `EVERY.len()` against a hand-written
    /// literal, which was the same omission one level further out — adding a variant
    /// without touching either left both at three and passed.
    #[test]
    fn the_budget_leaves_the_snapshot_writer_its_reserve() {
        let summed: usize = EVERY.iter().copied().map(cap).sum();
        assert_eq!(summed, TOTAL, "TOTAL is not the sum of EVERY's caps");
        assert!(
            summed + RESERVED_FOR_SNAPSHOT <= POOL_THREADS,
            "{summed} held + {RESERVED_FOR_SNAPSHOT} reserved exceeds {POOL_THREADS}"
        );
    }

    /// Every consumer has a positive cap. A zero would deadlock its gate silently —
    /// callers would queue behind a slot that is never granted.
    #[test]
    fn every_consumer_may_hold_at_least_one_thread() {
        for who in EVERY.iter().copied() {
            assert!(cap(who) > 0, "{who:?} may never be admitted");
        }
    }
}
