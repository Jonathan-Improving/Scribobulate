/* macOS only. MEASURED on GTK 4.22.4 (Homebrew) / Quartz, 2026-09-09.
 *
 * On the GDK macOS backend, a cursor set with gtk_widget_set_cursor_from_name() is
 * NEVER APPLIED, for the whole life of a window that MAPPED while the pointer was
 * outside the rectangle it came up in. Park the pointer inside that rectangle instead
 * and every cursor applies, always. The split is on that one variable and it is
 * clean — 4/4 here, and 6/6 each way against the application.
 *
 * WHY THIS FILE IS IN C AND IS THIS SMALL. The defect was first seen through an
 * application's own hover handler and read as a defect in it: intermittent, confined
 * to some hovered elements and not others. It is none of those things. Forty lines
 * with ONE GtkApplicationWindow, ONE GtkLabel and ONE set_cursor_from_name call
 * reproduce it exactly, which is what moves it out of any application and into the
 * toolkit. `gtk4-widget-factory` reproduces it too, if you would rather not build
 * anything.
 *
 * NOTHING UN-LATCHES IT EXCEPT A MOUSE-DOWN INSIDE THE WINDOW, after which cursors
 * work permanently. Each of these was tried on a fresh broken window and did NOT
 * un-latch it: activating and reactivating the application, leaving the window and
 * re-entering it, the scroll wheel, a keystroke, opening a native menu. Nor does
 * layout churn — an application that re-renders and re-lays-out continuously stays
 * broken. Setting the cursor on the TOPLEVEL instead of the child does not help
 * either (CURSOR_ON=window below), which is what places the break below GTK's widget
 * layer rather than in the walk up from the hovered widget.
 *
 * TWO CONTROLS WORTH KNOWING BEFORE YOU RE-RUN THIS, because each kills a plausible
 * wrong diagnosis:
 *   - AppKit's OWN cursors on the same broken window still work: hover the window's
 *     frame edges and the resize cursors appear normally. So neither the window
 *     server nor a synthetic-pointer-move harness is at fault; only GDK's path is.
 *   - Motion events are delivered and handled in the broken state — hover affordances
 *     light up and repaint under the pointer while the cursor stays an arrow. So this
 *     is not "the app never learns the pointer is there".
 *
 *   clang -o /tmp/cursor-map probes/macos-cursor-map-latch.c $(pkg-config --cflags --libs gtk4)
 *
 *   # BROKEN: park the pointer away from where the window will come up, then run it
 *   # and hover the label WITHOUT clicking -> arrow, and it stays arrow.
 *   # HEALTHY: park the pointer where the window will come up, run it again
 *   # -> the label shows the pointing hand immediately, and keeps it.
 *   # CURSOR_ON=window puts the cursor on the toplevel instead: no difference.
 *
 * EVERY RUN PRINTS ITS OWN CONFIGURATION FIRST, per probes/README.md's "a probe that
 * cannot name its own arm is not an instrument":
 *
 *   CONFIG cursor_on=label CURSOR_ON=<unset>
 *
 * Both the resolved behaviour and the raw variable, because neither determines the
 * other. `<unset>` and `""` print differently on purpose: whether an empty value
 * disarms a switch depends on how that switch is TESTED, which the caller cannot see
 * from the command line. Here CURSOR_ON is compared by content, so `CURSOR_ON=` is
 * disarmed; a presence-tested switch would be ARMED by the same keystroke. Read the
 * CONFIG line rather than the command you typed.
 *
 * READ THE CURSOR BY NAME, not off a screenshot — probes/quartz-cursor-identity.m
 * does that, and judging this by eye is precisely how the defect got recorded as
 * intermittent when it is deterministic.
 */
#include <gtk/gtk.h>

/* Render an environment switch for the CONFIG line. THREE outcomes, not two, because
 * `g_getenv` reports PRESENCE and the empty string is present: `<unset>` is a disarmed
 * arm, `""` is a LIVE arm carrying no value, and they are the same keystroke apart.
 * A run that cannot tell you which one it had is the failure probes/README.md's
 * "cannot name its own arm" section exists to stop. */
static const char *envshow(const char *name) {
  const char *v = g_getenv(name);
  if (!v)
    return "<unset>";
  if (!*v)
    return "\"\"";
  return v;
}

static void on_activate(GtkApplication *app, gpointer user_data) {
  GtkWidget *win = gtk_application_window_new(app);
  gtk_window_set_title(GTK_WINDOW(win), "cursor-map");
  gtk_window_set_default_size(GTK_WINDOW(win), 600, 400);

  GtkWidget *label = gtk_label_new("HOVER ME — this label asks for the 'pointer' cursor");
  gtk_widget_set_hexpand(label, TRUE);
  gtk_widget_set_vexpand(label, TRUE);
  /* The whole test: one cursor, set once, never touched again.
   * CURSOR_ON=window puts it on the toplevel instead of the label, to ask whether
   * the toplevel is reachable when a descendant is not. */
  const char *on = g_getenv("CURSOR_ON");
  gboolean on_window = on && g_str_equal(on, "window");
  if (on_window)
    gtk_widget_set_cursor_from_name(win, "pointer");
  else
    gtk_widget_set_cursor_from_name(label, "pointer");

  /* The one arm this probe has, named on stdout before it can mislead anyone. BOTH
   * the resolved behaviour and the raw variable, because they do not determine each
   * other: CURSOR_ON is tested by CONTENT above (g_str_equal), so `CURSOR_ON=` is
   * disarmed and reads `cursor_on=label` — whereas a PRESENCE-tested switch
   * (`g_getenv(x) ? ...`) would be ARMED by that same empty value. The caller cannot
   * see which discipline a switch uses, so the program has to say. */
  g_print("CONFIG cursor_on=%s CURSOR_ON=%s\n", on_window ? "window" : "label",
          envshow("CURSOR_ON"));

  gtk_window_set_child(GTK_WINDOW(win), label);
  gtk_window_present(GTK_WINDOW(win));
}

int main(int argc, char **argv) {
  GtkApplication *app =
      gtk_application_new("org.example.cursormap", G_APPLICATION_NON_UNIQUE);
  g_signal_connect(app, "activate", G_CALLBACK(on_activate), NULL);
  int r = g_application_run(G_APPLICATION(app), argc, argv);
  g_object_unref(app);
  return r;
}
