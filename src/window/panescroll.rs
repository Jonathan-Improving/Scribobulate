//! `panescroll`: the one way a command scrolls a text pane on purpose.
//!
//! In split mode the two panes are tied by the scroll sync (`window::scrollsync`), which
//! projects the LEADING pane onto the other every frame. A command that scrolls the
//! pane that is not leading has its scroll cancelled on the next frame: the sync puts
//! the leader's unchanged position straight back. Nothing warns; the command simply
//! appears to do nothing. That happened to Find, then again to Undo/Redo (ScrAP-361),
//! each time because one call site scrolled without first making its pane lead.
//!
//! So a navigation names the pane it moves, and this module makes that pane lead
//! before scrolling it:
//!
//! - [`editor_to_mark`] is the only route for scrolling the editor to a mark. The raw
//!   `farscroll::scroll_to_mark_when_ready` is banned in `clippy.toml`; this module's
//!   call carries the sanctioned `#[expect]`.
//! - [`lead`] is for a navigation whose scroll goes through another seam (the preview's
//!   heading, fragment and line scrolls), which takes the pane and has no mark.
//!
//! Outside split mode the lead has no effect: only the split sync reads it.
//!
//! What this module does NOT cover: restoring a reading position after a re-render or
//! a mode switch. That is preserving a place, not moving to a new one, and it holds the
//! sync off by other means (Reading-Position Preservation CAM).

use gtk::prelude::*;

use crate::codeview::CodePreviewView;
use crate::preview::scrib_render_data;
use crate::readingpos::{self, DocPosition};
use crate::winstate::{ScrollDriver, TabState};

/// Make `pane` the one the split scroll sync follows, so a scroll the caller is about
/// to issue on it sticks and the other pane follows it.
pub(crate) fn lead(st: &TabState, pane: ScrollDriver) {
    st.scroll.driver.set(pane);
}

/// How [`editor_to_mark`] places the mark: the same four arguments as
/// `gtk_text_view_scroll_to_mark`, named.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Placement {
    pub(crate) within_margin: f64,
    pub(crate) use_align: bool,
    pub(crate) xalign: f64,
    pub(crate) yalign: f64,
}

impl Placement {
    /// The least movement that brings the mark on screen; nothing moves when it
    /// already is.
    pub(crate) const MINIMAL: Self = Self {
        within_margin: 0.0,
        use_align: false,
        xalign: 0.0,
        yalign: 0.0,
    };
    /// The mark's line at the top of the viewport.
    pub(crate) const TOP: Self = Self {
        within_margin: 0.0,
        use_align: true,
        xalign: 0.0,
        yalign: 0.0,
    };
    /// The mark's line at the middle of the viewport.
    pub(crate) const MIDDLE: Self = Self {
        within_margin: 0.0,
        use_align: true,
        xalign: 0.0,
        yalign: 0.5,
    };
}

/// Scroll the editor so `mark` is placed as `placement` says, with the editor leading
/// the split sync. Reaches marks far into a document still being laid out
/// (`farscroll`, GTK4Rs/AP-260).
pub(crate) fn editor_to_mark(st: &TabState, mark: &gtk::TextMark, placement: Placement) {
    lead(st, ScrollDriver::Editor);
    #[expect(
        clippy::disallowed_methods,
        reason = "the sanctioned route: the editor was made to lead the split sync on the line above (ScrAP-361)"
    )]
    crate::farscroll::scroll_to_mark_when_ready(
        st.editor.upcast_ref(),
        mark,
        placement.within_margin,
        placement.use_align,
        placement.xalign,
        placement.yalign,
    );
}

/// The preview pane's view, when the tab has one.
fn preview_view(st: &TabState) -> Option<CodePreviewView> {
    st.split
        .preview_scroller()?
        .child()?
        .downcast::<CodePreviewView>()
        .ok()
}

/// The first and last preview buffer lines on screen, read before a change re-renders
/// the preview so [`preview_reveal`] can tell whether the change was already in view.
/// `None` when the preview is not on screen.
pub(crate) fn preview_lines_on_screen(st: &TabState) -> Option<(i32, i32)> {
    let view = preview_view(st)?;
    // An unmapped view answers `line_at_y` with the buffer's last line (GTK4Rs/AP-263).
    if !view.is_mapped() {
        return None;
    }
    let rect = view.visible_rect();
    let (top, _) = view.line_at_y(rect.y());
    let (bottom, _) = view.line_at_y(rect.y() + rect.height() - 1);
    Some((top.line(), bottom.line()))
}

/// Bring `pos` into view in the preview, with the preview leading the split sync,
/// unless its line was already on screen in `before` (from [`preview_lines_on_screen`]).
///
/// The line lands at the top of the viewport. The preview's own coalesced scroll is
/// used, so this supersedes a re-render's pending restore of the old reading position
/// when it is issued after that re-render. `before` describes the buffer as it was
/// BEFORE the re-render, so the "already on screen" test is approximate by however
/// many lines the change itself added or removed above the viewport.
pub(crate) fn preview_reveal(st: &TabState, pos: DocPosition, before: Option<(i32, i32)>) {
    let Some(view) = preview_view(st) else { return };
    let Some(rd) = scrib_render_data(&view) else {
        return;
    };
    let buf_char = {
        let rd = rd.borrow();
        readingpos::reading_to_preview_char(&rd.source_map_inv, &rd.shifts, &rd.original_owned, pos)
    };
    let line = view.buffer().iter_at_offset(buf_char).line();
    if before.is_some_and(|(top, bottom)| (top..=bottom).contains(&line)) {
        return;
    }
    lead(st, ScrollDriver::Preview);
    view.scroll_to_buffer_offset(buf_char);
}
