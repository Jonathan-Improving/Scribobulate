// Two GTK4 toplevels on Quartz, each with a tooltip-bearing button, plus a
// periodic dump of every NSWindow's AppKit parent/child relationships.
//
// Purpose: find out whether hovering tooltips in two independent toplevels
// welds them into one AppKit ordering group (shared child window), and whether
// a destroyed tooltip leaves a zombie NSWindow in its old parent's childWindows.
//
// Mirrors Scribobulate's pointercrossing.rs input-region repair, because without
// it GDK's 0x0 NSTrackingArea means crossing events -- and therefore tooltips --
// never fire on Quartz at all.

#import <AppKit/AppKit.h>
#include <gtk/gtk.h>
#include <gdk/macos/gdkmacos.h>

static const char *state_of(NSWindow *w) {
  static char buf[128];
  snprintf(buf, sizeof buf, "%svisible %smini",
           [w isVisible] ? "" : "!", [w isMiniaturized] ? "" : "!");
  return buf;
}

static void dump(const char *tag) {
  printf("\n===== DUMP %s =====\n", tag);
  for (NSWindow *w in [NSApp windows]) {
    NSWindow *p = [w parentWindow];
    printf("  win#%ld %-22s title=%-28s %s parent=%s\n",
           (long)[w windowNumber],
           [NSStringFromClass([w class]) UTF8String],
           [[w title] UTF8String] ?: "",
           state_of(w),
           p ? [[NSString stringWithFormat:@"#%ld", (long)[p windowNumber]] UTF8String] : "nil");
    for (NSWindow *c in [w childWindows]) {
      NSWindow *cp = [c parentWindow];
      printf("      child#%ld %-20s %s  child's parent=%s%s\n",
             (long)[c windowNumber],
             [NSStringFromClass([c class]) UTF8String],
             state_of(c),
             cp ? [[NSString stringWithFormat:@"#%ld", (long)[cp windowNumber]] UTF8String] : "nil",
             (cp != w) ? "   <<< MISMATCH: listed as child here but parented elsewhere" : "");
    }
  }
  fflush(stdout);
}

static gboolean tick(gpointer _) { dump("periodic"); return G_SOURCE_CONTINUE; }

// ---- pointercrossing.rs's repair, minimal restatement -----------------------
static void set_full_input_region(GdkSurface *surface) {
  int w = gdk_surface_get_width(surface), h = gdk_surface_get_height(surface);
  cairo_rectangle_int_t r = { 0, 0, w, h };
  cairo_region_t *region = cairo_region_create_rectangle(&r);
  gdk_surface_set_input_region(surface, region);
  cairo_region_destroy(region);
}
static void on_layout(GdkSurface *s, int w, int h, gpointer _) { set_full_input_region(s); }
static void on_realize(GtkWidget *win, gpointer _) {
  GdkSurface *s = gtk_native_get_surface(GTK_NATIVE(win));
  set_full_input_region(s);
  g_signal_connect(s, "layout", G_CALLBACK(on_layout), NULL);
}

static GtkWidget *make_window(GtkApplication *app, const char *title, int x_hint) {
  GtkWidget *win = gtk_application_window_new(app);
  gtk_window_set_title(GTK_WINDOW(win), title);
  gtk_window_set_default_size(GTK_WINDOW(win), 520, 380);

  GtkWidget *box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 12);
  gtk_widget_set_margin_top(box, 40);
  gtk_widget_set_margin_start(box, 40);
  for (int i = 0; i < 3; i++) {
    char label[64], tip[64];
    snprintf(label, sizeof label, "%s button %d", title, i);
    snprintf(tip, sizeof tip, "tooltip for %s button %d", title, i);
    GtkWidget *b = gtk_button_new_with_label(label);
    gtk_widget_set_tooltip_text(b, tip);
    gtk_box_append(GTK_BOX(box), b);
  }
  gtk_window_set_child(GTK_WINDOW(win), box);

  g_signal_connect(win, "realize", G_CALLBACK(on_realize), NULL);
  gtk_window_present(GTK_WINDOW(win));
  return win;
}

static void activate(GtkApplication *app, gpointer _) {
  make_window(app, "ALPHA", 0);
  make_window(app, "BETA", 1);
  g_timeout_add_seconds(3, tick, NULL);
  dump("startup");
}

int main(int argc, char **argv) {
  printf("twotop: two GTK4 toplevels (ALPHA, BETA), each with tooltip-bearing "
         "buttons; dumping AppKit parent/child every 3s. No switches — one arm.\n");
  fflush(stdout);
  GtkApplication *app = gtk_application_new("org.scribobulate.probe.twotop",
                                            G_APPLICATION_DEFAULT_FLAGS);
  g_signal_connect(app, "activate", G_CALLBACK(activate), NULL);
  return g_application_run(G_APPLICATION(app), argc, argv);
}
