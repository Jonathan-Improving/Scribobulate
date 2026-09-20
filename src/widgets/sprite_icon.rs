//! `SpriteIcon` — a theme sprite drawn at a fixed size, as a widget.
//!
//! Two sprite shapes take it: the disclosure indicator, and an ANIMATED heading marker.
//! Each would otherwise be a single texture resampled once, which can only ever show
//! one frame. This draws in `snapshot` through `animation::sprites::Frames` instead, so
//! an animated sprite plays under the same rules as every other one (TDD 27.9), and a
//! still one paints the exact texture `sprite::scaled` produces.
//!
//! A heading marker takes this only when its sprite is animated. A still marker stays a
//! buffer paintable, and an animated one cannot be: GTK caches a text line's render node
//! and re-snapshots a buffer paintable only after its `invalidate-contents`, which
//! re-wraps and revalidates the line — per frame, restarting the text view's validation
//! idles that reading-position restore waits on (researcher-verified, GTK 4.6.9 and
//! 4.22.4). An anchored child repaints alone.
//!
//! It owns its own `SpriteTable` rather than borrowing the preview's, so every frame's
//! repaint is queued on the widget that draws it and every pass is bracketed inside this
//! widget's own `snapshot` — nothing depends on where GTK orders an anchored child's
//! snapshot relative to the text view's layers.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::ObjectSubclassIsExt;

mod imp {
    use super::*;
    use gtk::graphene;
    use gtk::subclass::prelude::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    pub(crate) struct SpriteIcon {
        pub(super) sprite: RefCell<Option<crate::sprite::SpriteRef>>,
        pub(super) width: Cell<i32>,
        pub(super) height: Cell<i32>,
        pub(super) sprites: crate::animation::sprites::SpriteTable,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SpriteIcon {
        const NAME: &'static str = "ScribSpriteIcon";
        type Type = super::SpriteIcon;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for SpriteIcon {
        /// Release the sprite's playback (tick, decoder, watches) while the widget can
        /// still be disconnected from — `dispose` runs before this struct's `Drop`.
        fn dispose(&self) {
            self.sprites.release();
        }
    }

    impl WidgetImpl for SpriteIcon {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let s = match orientation {
                gtk::Orientation::Horizontal => self.width.get(),
                _ => self.height.get(),
            }
            .max(0);
            (s, s, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let sprite = self.sprite.borrow();
            let Some(sprite) = sprite.as_ref() else {
                return;
            };
            let obj = self.obj();
            let rect = graphene::Rect::new(0.0, 0.0, obj.width() as f32, obj.height() as f32);
            self.sprites.begin_pass();
            crate::widgets::draw_sprite_into(
                snapshot,
                &rect,
                sprite,
                self.sprites.frames(obj.upcast_ref()),
            );
            self.sprites.end_pass();
        }
    }
}

glib::wrapper! {
    pub(crate) struct SpriteIcon(ObjectSubclass<imp::SpriteIcon>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl SpriteIcon {
    /// `sprite`, drawn `size × size`, announced as an image — the disclosure indicator.
    /// The caller decides the sprite is usable (it resamples) before choosing this over
    /// the next rung of the decoration.
    pub(crate) fn new(sprite: crate::sprite::SpriteRef, size: i32) -> Self {
        Self::sized(sprite, size, size, gtk::AccessibleRole::Img)
    }

    /// `sprite`, drawn `width × height`, announced with `role` — `Presentation` for pure
    /// decoration such as a heading marker, which stands for no content.
    pub(crate) fn sized(
        sprite: crate::sprite::SpriteRef,
        width: i32,
        height: i32,
        role: gtk::AccessibleRole,
    ) -> Self {
        let obj: Self = glib::Object::new();
        obj.set_accessible_role(role);
        obj.imp().sprite.replace(Some(sprite));
        obj.imp().width.set(width);
        obj.imp().height.set(height);
        obj
    }

    /// The table this icon plays its sprite from — for tests asking the driver.
    #[cfg(all(test, feature = "gtk-integration-tests"))]
    pub(crate) fn sprites(&self) -> &crate::animation::sprites::SpriteTable {
        &self.imp().sprites
    }
}

#[cfg(all(test, feature = "gtk-integration-tests"))]
mod gtk_tests {
    use super::*;
    use crate::animation::sprites::testkit;

    /// An icon presented in its own application window, mapped and allocated at exactly
    /// its requested size.
    ///
    /// Pinned to the top-left corner, because a `GtkWindow` gives its sole child the whole
    /// content area: on Windows the native frame will not shrink to 16 px, so an unpinned
    /// icon was allocated about 144×33 and drew the sprite resampled to THAT — measured by
    /// the Windows seat as a render 18.6× the area of the box being asserted about.
    fn presented(
        r: &crate::sprite::SpriteRef,
        size: i32,
        suffix: &str,
    ) -> (SpriteIcon, gtk::ApplicationWindow) {
        let app = crate::window::testkit::test_app_suffixed(&format!("spriteicon.{suffix}"));
        let icon = SpriteIcon::new(r.clone(), size);
        icon.set_halign(gtk::Align::Start);
        icon.set_valign(gtk::Align::Start);
        let win = gtk::ApplicationWindow::new(&app);
        win.set_child(Some(&icon));
        win.present();
        crate::testpump::until(
            crate::testpump::Clock::Idle,
            "the icon to be allocated",
            || icon.width() > 0,
        );
        assert_eq!(
            (icon.width(), icon.height()),
            (size, size),
            "precondition: the icon is allocated exactly its requested size"
        );
        (icon, win)
    }

    /// TDD 27.9 — an animated disclosure indicator PLAYS: the icon's own pixels change
    /// under the real frame clock, driven by its own sprite table.
    #[gtktest::test]
    fn an_animated_sprite_icon_plays() {
        let _enable = crate::animation::policy::EnableAnimationsGuard::set(true);
        crate::sprite::clear_cache();
        let (_dir, r) = testkit::animated_fixture();
        let (icon, win) = presented(&r, 64, "anim");
        let paint = |s: &gtk::Snapshot| gtk::subclass::prelude::WidgetImpl::snapshot(icon.imp(), s);

        let _ = testkit::rendered(paint);
        assert!(
            icon.sprites()
                .with_anim(&r, |anim| anim.is_ticking())
                .unwrap_or(false),
            "a mapped icon with an animated sprite must be ticking"
        );
        let played = testkit::plays(paint);
        win.destroy();
        assert!(played, "the animated indicator never changed across 10s");
        crate::sprite::clear_cache();
    }

    /// TDD 18.2 — a STILL sprite paints exactly the texture the `GtkPicture` this widget
    /// replaced showed: `sprite::scaled` at the icon's size, drawn once into its box.
    #[gtktest::test]
    fn a_still_sprite_icon_paints_the_still_resample() {
        let _enable = crate::animation::policy::EnableAnimationsGuard::set(true);
        crate::sprite::clear_cache();
        let (_dir, r) = testkit::still_fixture();
        let size = 16;
        let (icon, win) = presented(&r, size, "still");
        let got =
            testkit::rendered(|s| gtk::subclass::prelude::WidgetImpl::snapshot(icon.imp(), s));
        let want = testkit::rendered(|s| {
            let tex = crate::sprite::scaled(&r, size, size).expect("the fixture resamples");
            s.append_texture(
                &tex,
                &gtk::graphene::Rect::new(0.0, 0.0, size as f32, size as f32),
            );
        });
        win.destroy();
        assert_eq!(
            got, want,
            "a still icon must paint the still resample, unchanged"
        );
        assert!(
            icon.sprites().with_anim(&r, |_| ()).is_none(),
            "a still sprite must never gain a driver entry"
        );
        crate::sprite::clear_cache();
    }

    /// TDD 27.3 / 27.9 — an icon first painted while OUT of view starts playing once it
    /// is scrolled INTO view. That first pass must refuse to play, and nothing makes GTK
    /// paint the icon a second time on its own, so the table's visibility watch has to
    /// ask. MEASURED in the running app before the fix: a disclosure indicator below the
    /// viewport at its first paint never animated after the window grew to show it.
    #[gtktest::test]
    fn an_icon_first_painted_out_of_view_plays_once_scrolled_into_view() {
        let _enable = crate::animation::policy::EnableAnimationsGuard::set(true);
        crate::sprite::clear_cache();
        let (_dir, r) = testkit::animated_fixture();
        let app = crate::window::testkit::test_app_suffixed("spriteicon.scrolled");
        let icon = SpriteIcon::new(r.clone(), 64);
        let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        spacer.set_size_request(-1, 1000);
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&spacer);
        column.append(&icon);
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_child(Some(&column));
        let win = gtk::ApplicationWindow::new(&app);
        win.set_default_size(200, 200);
        win.set_child(Some(&scroller));
        win.present();
        crate::testpump::until(
            crate::testpump::Clock::Frame,
            "the icon to be allocated below the viewport",
            || icon.width() >= 64,
        );
        crate::testpump::drain_for(
            crate::testpump::Clock::Frame,
            std::time::Duration::from_millis(200),
        );
        assert!(
            icon.sprites().with_anim(&r, |_| ()).is_none(),
            "precondition: below the viewport, the icon must not be playing"
        );

        let vadj = scroller.vadjustment();
        crate::saferizer::scrollpos::jump(&vadj, vadj.upper() - vadj.page_size());
        crate::testpump::until(
            crate::testpump::Clock::Frame,
            "the icon to start playing once scrolled into view",
            || {
                icon.sprites()
                    .with_anim(&r, |anim| anim.is_ticking())
                    .unwrap_or(false)
            },
        );
        win.destroy();
        crate::sprite::clear_cache();
    }
}
