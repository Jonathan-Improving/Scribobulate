//! The decisions behind a restore, over plain numbers and strings so they test without a
//! display. Every GDK and X11 call lives in `lib.rs` and `x11.rs`; nothing here touches
//! either.

#[cfg(any(test, all(unix, not(target_os = "macos"))))] // X11 only; tested everywhere.
/// How much of the window has to stay on its monitor, in logical pixels, when the
/// monitor it is restored onto is now smaller than the place it was saved at. Enough to
/// grab the title bar and drag the window back.
pub(crate) const KEEP_REACHABLE: i32 = 48;

/// A rectangle in GDK logical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[cfg(any(test, all(unix, not(target_os = "macos"))))] // X11 only; tested everywhere.
/// Where a window's frame goes on `monitor`, given the offset from the monitor's origin
/// it was saved at and the window's width.
///
/// A position that still leaves the title bar reachable is kept exactly, even if part of
/// the window hangs off the monitor: the reader may have put it there. Only a position
/// that would lose the title bar is pulled back, by the least distance that makes
/// [`KEEP_REACHABLE`] pixels of it reachable again.
pub(crate) fn frame_origin(monitor: Rect, offset: (i32, i32), width: i32) -> (i32, i32) {
    let (dx, dy) = offset;
    let keep_x = KEEP_REACHABLE.min(width.max(1));
    let min_x = -(width - keep_x).max(0);
    let max_x = (monitor.width - keep_x).max(0);
    let max_y = (monitor.height - KEEP_REACHABLE).max(0);
    (
        monitor.x + dx.clamp(min_x.min(max_x), max_x),
        monitor.y + dy.clamp(0, max_y),
    )
}

/// The size to open at: the saved size, but never larger than the monitor it opens on.
/// A narrower request is still only a request; GTK grows it back to the content's
/// minimum, so this never makes a window smaller than it can be drawn.
pub(crate) fn fit_size(size: (i32, i32), monitor: Option<Rect>) -> (i32, i32) {
    let (w, h) = size;
    match monitor {
        Some(m) if m.width > 0 && m.height > 0 => (w.min(m.width), h.min(m.height)),
        _ => (w, h),
    }
}

#[cfg(any(test, all(unix, not(target_os = "macos"))))] // X11 only; tested everywhere.
/// The nil UUID, which KWin reads as "every activity".
pub(crate) const ALL_ACTIVITIES: &str = "00000000-0000-0000-0000-000000000000";

#[cfg(any(test, all(unix, not(target_os = "macos"))))] // X11 only; tested everywhere.
/// The activities to write for a restored window: the saved ones that still exist.
///
/// `running` is the activity manager's current list, or `None` when it could not be
/// asked. Unasked, the saved list is written as it is and KWin drops the ones it does
/// not know. The nil UUID is always kept. An empty result means "write nothing", so the
/// window lands on the current activity like a new window; KWin, given a list that
/// matches nothing, would put it on every activity instead.
pub(crate) fn surviving_activities(saved: &[String], running: Option<&[String]>) -> Vec<String> {
    saved
        .iter()
        .filter(|a| !a.is_empty())
        .filter(|a| {
            a.as_str() == ALL_ACTIVITIES
                || running.is_none_or(|r| r.iter().any(|known| known == *a))
        })
        .cloned()
        .collect()
}

#[cfg(any(test, all(unix, not(target_os = "macos"))))] // X11 only; tested everywhere.
/// The activities as KWin's property spells them: comma-separated UUIDs.
pub(crate) fn split_activities(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MON: Rect = Rect {
        x: 1920,
        y: 0,
        width: 2560,
        height: 1440,
    };

    #[test]
    fn a_position_with_the_title_bar_on_screen_is_kept_exactly() {
        assert_eq!(frame_origin(MON, (100, 200), 800), (2020, 200));
        // Hanging off the right and left edges, title bar still reachable.
        assert_eq!(frame_origin(MON, (2400, 10), 800), (4320, 10));
        assert_eq!(frame_origin(MON, (-500, 10), 800), (1420, 10));
    }

    #[test]
    fn a_monitor_that_shrank_pulls_the_window_back_until_its_title_bar_is_reachable() {
        let small = Rect {
            width: 1280,
            height: 720,
            ..MON
        };
        // Saved at the far corner of the big monitor.
        assert_eq!(
            frame_origin(small, (2400, 1300), 800),
            (1920 + 1280 - KEEP_REACHABLE, 720 - KEEP_REACHABLE)
        );
        // Above the top edge, where the title bar is.
        assert_eq!(frame_origin(small, (10, -40), 800), (1930, 0));
        // Further off the left than the window is wide.
        assert_eq!(
            frame_origin(small, (-5000, 10), 800),
            (1920 - (800 - KEEP_REACHABLE), 10)
        );
    }

    #[test]
    fn a_window_narrower_than_the_keep_margin_stays_wholly_on_screen() {
        assert_eq!(frame_origin(MON, (-100, 0), 20), (1920, 0));
    }

    #[test]
    fn size_is_fitted_to_the_monitor_only_when_one_is_known() {
        assert_eq!(fit_size((3000, 2000), Some(MON)), (2560, 1440));
        assert_eq!(fit_size((800, 600), Some(MON)), (800, 600));
        assert_eq!(fit_size((3000, 2000), None), (3000, 2000));
    }

    #[test]
    fn an_activity_that_no_longer_exists_is_dropped_and_the_rest_kept() {
        let saved = vec!["a".to_owned(), "gone".to_owned(), "b".to_owned()];
        let running = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
        assert_eq!(
            surviving_activities(&saved, Some(&running)),
            vec!["a".to_owned(), "b".to_owned()]
        );
    }

    #[test]
    fn when_none_survive_nothing_is_written() {
        let saved = vec!["gone".to_owned()];
        assert!(surviving_activities(&saved, Some(&["a".to_owned()])).is_empty());
    }

    #[test]
    fn all_activities_survives_and_an_unreachable_manager_keeps_the_list() {
        let saved = vec![ALL_ACTIVITIES.to_owned()];
        assert_eq!(surviving_activities(&saved, Some(&[])), saved);
        let saved = vec!["x".to_owned(), "y".to_owned()];
        assert_eq!(surviving_activities(&saved, None), saved);
    }

    #[test]
    fn the_property_splits_on_commas_and_skips_empties() {
        assert_eq!(split_activities("a,b"), vec!["a", "b"]);
        assert!(split_activities("").is_empty());
        assert_eq!(split_activities("a,,b,"), vec!["a", "b"]);
    }
}
