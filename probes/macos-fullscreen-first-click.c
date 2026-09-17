// Does entering fullscreen on GTK4/Quartz eat the first click on an
// in-window toolbar button? Zero application code: one GtkApplicationWindow,
// one toolbar row with three plain GtkButtons driven by set_action_name,
// nothing else. Same toolbar-as-first-row shape as Scribobulate's macOS
// chrome (menubar.bar is None there; src/window/chrome.rs), but nothing
// about the chrome shape or this project's own code is exercised here.
//
// Logs every press on a toolbar button (GtkGestureClick, capture phase, on
// the button itself) with widget-space/root-space coords, surface size,
// scale, fullscreen and active state, so a press that never reaches the
// button is visible as a MISSING line rather than inferred from a
// screenshot.
//
// Three ways to enter fullscreen, deliberately: this project has no
// fullscreen command of its own (checked — reachable only through macOS's
// own affordances), so a real GTK app and this probe both enter fullscreen
// exactly the same three ways:
//
//   1. Click the native zoom button (the green traffic light).
//   2. F7 in this window: calls gtk_window_fullscreen() directly, GTK's own
//      path, no mouse, no accessibility API in the loop at all.
//   3. A real physical Ctrl+Cmd+F keypress at the keyboard, if you have one —
//      synthetic input cannot post this specific combo (see the answer
//      below); it is wired to app.toggle-fullscreen for a human to try.
//
// Shift+F7 leaves fullscreen via gtk_window_unfullscreen(), same reason.

#include <gtk/gtk.h>
#include <stdio.h>
#include <time.h>

static double mono_ms(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec * 1000.0 + ts.tv_nsec / 1e6;
}

static void log_click(GtkGestureClick *g, int n_press, double x, double y, GtkWidget *btn) {
    GtkWidget *win = gtk_widget_get_ancestor(btn, GTK_TYPE_WINDOW);
    GdkSurface *surf = win ? gtk_native_get_surface(GTK_NATIVE(win)) : NULL;
    graphene_point_t root;
    gboolean ok = gtk_widget_compute_point(btn, GTK_WIDGET(win), &GRAPHENE_POINT_INIT(x, y), &root);
    gboolean fullscreen = win && (gtk_window_is_fullscreen(GTK_WINDOW(win)));
    gboolean active = win && gtk_window_is_active(GTK_WINDOW(win));
    int sw = surf ? gdk_surface_get_width(surf) : -1;
    int sh = surf ? gdk_surface_get_height(surf) : -1;
    double scale = surf ? gdk_surface_get_scale_factor(surf) : -1.0;
    printf("[%.1f] CLICK n=%d widget=%s widget-space=(%.1f,%.1f) root-space=(%.1f,%.1f)%s "
           "surface=%dx%d scale=%.1f fullscreen=%d active=%d\n",
           mono_ms(), n_press, gtk_widget_get_name(btn), x, y,
           ok ? root.x : -1, ok ? root.y : -1, ok ? "" : " [compute_point FAILED]",
           sw, sh, scale, fullscreen, active);
    fflush(stdout);
}

static void toggle_fullscreen_action(GSimpleAction *a, GVariant *p, gpointer win) {
    GtkWindow *w = GTK_WINDOW(win);
    if (gtk_window_is_fullscreen(w)) {
        printf("[%.1f] app.toggle-fullscreen -> LEAVING\n", mono_ms());
        gtk_window_unfullscreen(w);
    } else {
        printf("[%.1f] app.toggle-fullscreen -> ENTERING\n", mono_ms());
        gtk_window_fullscreen(w);
    }
    fflush(stdout);
}

// F7 -> gtk_window_fullscreen(), Shift+F7 -> unfullscreen(). Direct calls,
// no accelerator/action machinery between the keypress and the GtkWindow
// method — this is entry mechanism #2, GTK's own path with nothing macOS
// or accessibility-shaped in front of it.
static gboolean on_key(GtkEventControllerKey *c, guint keyval, guint keycode,
                        GdkModifierType state, gpointer win) {
    if (keyval == GDK_KEY_F7) {
        GtkWindow *w = GTK_WINDOW(win);
        if (state & GDK_SHIFT_MASK) {
            printf("[%.1f] F7(shift) -> gtk_window_unfullscreen()\n", mono_ms());
            gtk_window_unfullscreen(w);
        } else {
            printf("[%.1f] F7 -> gtk_window_fullscreen()\n", mono_ms());
            gtk_window_fullscreen(w);
        }
        fflush(stdout);
        return TRUE;
    }
    return FALSE;
}

static GtkWidget *make_button(const char *name, const char *label) {
    GtkWidget *b = gtk_button_new_with_label(label);
    gtk_widget_set_name(b, name);
    GtkGesture *g = gtk_gesture_click_new();
    gtk_event_controller_set_propagation_phase(GTK_EVENT_CONTROLLER(g), GTK_PHASE_CAPTURE);
    g_signal_connect(g, "pressed", G_CALLBACK(log_click), b);
    gtk_widget_add_controller(b, GTK_EVENT_CONTROLLER(g));
    return b;
}

static void activate(GtkApplication *app, gpointer _) {
    GtkWidget *win = gtk_application_window_new(app);
    gtk_window_set_title(GTK_WINDOW(win), "fsclick");
    gtk_window_set_default_size(GTK_WINDOW(win), 640, 420);

    GtkWidget *toolbar = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 6);
    gtk_widget_set_margin_top(toolbar, 6);
    gtk_widget_set_margin_bottom(toolbar, 6);
    gtk_widget_set_margin_start(toolbar, 6);
    gtk_widget_set_margin_end(toolbar, 6);
    gtk_box_append(GTK_BOX(toolbar), make_button("tb-bold", "Bold"));
    gtk_box_append(GTK_BOX(toolbar), make_button("tb-italic", "Italic"));
    gtk_box_append(GTK_BOX(toolbar), make_button("tb-link", "Link"));

    GtkWidget *body = gtk_label_new(
        "fullscreen-first-click probe -- F7 fullscreens, Shift+F7 restores,\n"
        "Ctrl+Cmd+F tries the real OS shortcut (won't work from synthetic input)");
    gtk_widget_set_vexpand(body, TRUE);

    GtkWidget *outer = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
    gtk_box_append(GTK_BOX(outer), toolbar);
    gtk_box_append(GTK_BOX(outer), gtk_separator_new(GTK_ORIENTATION_HORIZONTAL));
    gtk_box_append(GTK_BOX(outer), body);
    gtk_window_set_child(GTK_WINDOW(win), outer);

    GtkEventController *keys = gtk_event_controller_key_new();
    g_signal_connect(keys, "key-pressed", G_CALLBACK(on_key), win);
    gtk_widget_add_controller(win, keys);

    GSimpleAction *fs = g_simple_action_new("toggle-fullscreen", NULL);
    g_signal_connect(fs, "activate", G_CALLBACK(toggle_fullscreen_action), win);
    g_action_map_add_action(G_ACTION_MAP(app), G_ACTION(fs));
    const char *accels[] = { "<Primary><Control>f", NULL };
    gtk_application_set_accels_for_action(app, "app.toggle-fullscreen", accels);

    gtk_window_present(GTK_WINDOW(win));
    printf("fsclick: 3-button toolbar, no menu bar, no switches. F7/Shift+F7 "
           "drive fullscreen directly; Ctrl+Cmd+F is wired for a human at a "
           "real keyboard.\n");
    fflush(stdout);
}

int main(int argc, char **argv) {
    GtkApplication *app = gtk_application_new("org.scribobulate.probe.fsclick",
                                               G_APPLICATION_DEFAULT_FLAGS);
    g_signal_connect(app, "activate", G_CALLBACK(activate), NULL);
    return g_application_run(G_APPLICATION(app), argc, argv);
}
