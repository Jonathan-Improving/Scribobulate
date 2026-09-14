//! A document whose buffer has become the only copy of it: its file was deleted, or
//! emptied, on disk (TDD 3.4, 3.5, 3.6, 15.22, 22.18).
//!
//! When a loss begins or ends is decided elsewhere — the monitor's `Deleted` arm, and
//! `winstate::external_change_action` for everything read back from disk. This module
//! applies the answer, so both losses reach every guard through one route: Save's
//! enablement, the ⚠ badge, the close prompt (`TabState::needs_close_prompt`) and the
//! crash-recovery snapshot all re-derive here.

use super::*;
use crate::winstate::BackingLoss;

/// Record that `tab`'s buffer is now the only copy of its document, and announce it.
///
/// A repeat of the loss already recorded changes nothing and announces nothing, so a
/// backend that reports one deletion twice does not stack a second notice.
pub(crate) fn mark_backing_lost(tab: &Rc<TabState>, loss: BackingLoss) {
    // A recorded loss supersedes a pending "is it still blank?" re-read.
    super::reload::cancel_truncation_settle(tab);
    if tab.backing_loss.replace(Some(loss)) == Some(loss) {
        return;
    }
    log::info!(
        "tab {}: backing file lost ({loss:?}): {}",
        tab.id,
        path_for_log(tab)
    );
    refresh_guards(tab);
    // Pushed through the chrome, so both its timer and an early retraction take it down
    // from the window that showed it even if this tab is moved or closed first
    // (`WindowChrome::push_timed_notice`). A change of reason replaces the notice.
    let notice = tab
        .chrome()
        .push_timed_notice(loss.notice(), crate::winstate::ERROR_NOTICE_TIME);
    if let Some(previous) = tab.backing_notice.replace(Some(notice)) {
        previous.retract();
    }
}

/// Retire `tab`'s loss — the file is back with the content last loaded or saved, or a
/// save or an explicit reload made the buffer and the file agree — and take down the
/// notice that announced it, which must not go on saying so after it stops being true.
pub(crate) fn clear_backing_loss(tab: &Rc<TabState>) {
    let Some(loss) = tab.backing_loss.take() else {
        return;
    };
    if let Some(notice) = tab.backing_notice.take() {
        notice.retract();
    }
    log::info!(
        "tab {}: backing file restored after {loss:?}: {}",
        tab.id,
        path_for_log(tab)
    );
    refresh_guards(tab);
}

/// Re-derive everything that reads the loss. Tab-scoped where the guard is the tab's
/// (badge, snapshot) and window-scoped where it is the window's (Save), resolving the
/// window the tab lives in NOW, since a background tab can be moved between windows.
fn refresh_guards(tab: &Rc<TabState>) {
    if let Some(window) = window_of_content_box(&tab.content_box) {
        update_save_action_state(&window);
    }
    badge_tab_label(tab);
    sync_tab_swap(tab);
}

fn path_for_log(tab: &TabState) -> String {
    tab.path
        .borrow()
        .as_ref()
        .map_or_else(|| "<untitled>".into(), |p| p.display().to_string())
}
