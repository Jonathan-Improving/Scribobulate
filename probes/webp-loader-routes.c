/* webp-loader-routes.c — which gdk-pixbuf entry points leak on an animated WebP?
 *
 * WHY THIS EXISTS
 * PLAN.memory-gates.md's route table was measured once, on assets/splash.webp.
 * The fix is "use the flat route", so this probe is the thing that names the
 * decode-side route the application may take. It prints footprint after each
 * iteration of one named arm; a climbing arm is a leak, a plateau is usable.
 *
 * ARM is selected by argv[1]; the path is argv[2]. Prints its own configuration
 * at startup (probes/README.md: a probe that cannot name its own arm is not an
 * instrument).
 *
 * ARMS
 *   file_info          gdk_pixbuf_get_file_info
 *   from_file          gdk_texture_new_from_file          (GdkPixbufLoader incremental)
 *   both               file_info then from_file           (load_texture today)
 *   anim_wh            gdk_pixbuf_animation_new_from_file, read w/h, drop
 *   anim_static        animation_new_from_file + get_static_image + texture_new_for_pixbuf
 *   from_bytes         g_file_load_contents + gdk_texture_new_from_bytes
 *
 * EXIT
 *   0  ran; the numbers are the result, not the status
 *   2  usage / could not open the file / GTK init failed
 */

#include <gtk/gtk.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static unsigned long vmrss_kb(void) {
  FILE *f = fopen("/proc/self/status", "r");
  if (!f) return 0;
  char line[256];
  unsigned long kb = 0;
  while (fgets(line, sizeof line, f)) {
    if (sscanf(line, "VmRSS: %lu kB", &kb) == 1) break;
  }
  fclose(f);
  return kb;
}

static void die(const char *msg) {
  fprintf(stderr, "webp-loader-routes: %s\n", msg);
  exit(2);
}

static GdkTexture *arm_from_file(const char *path) {
  GError *err = NULL;
  GFile *file = g_file_new_for_path(path);
  GdkTexture *t = gdk_texture_new_from_file(file, &err);
  g_object_unref(file);
  if (!t) {
    fprintf(stderr, "from_file: %s\n", err ? err->message : "unknown");
    g_clear_error(&err);
  }
  return t;
}

static GdkTexture *arm_anim_static(const char *path) {
  GError *err = NULL;
  GdkPixbufAnimation *anim = gdk_pixbuf_animation_new_from_file(path, &err);
  if (!anim) {
    fprintf(stderr, "anim: %s\n", err ? err->message : "unknown");
    g_clear_error(&err);
    return NULL;
  }
  GdkPixbuf *pb = gdk_pixbuf_animation_get_static_image(anim);
  GdkTexture *t = NULL;
  if (pb) t = gdk_texture_new_for_pixbuf(pb);
  g_object_unref(anim);
  return t;
}

static GdkTexture *arm_from_bytes(const char *path) {
  GError *err = NULL;
  gchar *bytes = NULL;
  gsize len = 0;
  if (!g_file_get_contents(path, &bytes, &len, &err)) {
    fprintf(stderr, "read: %s\n", err ? err->message : "unknown");
    g_clear_error(&err);
    return NULL;
  }
  GBytes *gbytes = g_bytes_new_take(bytes, len);
  GdkTexture *t = gdk_texture_new_from_bytes(gbytes, &err);
  g_bytes_unref(gbytes);
  if (!t) {
    fprintf(stderr, "from_bytes: %s\n", err ? err->message : "unknown");
    g_clear_error(&err);
  }
  return t;
}

int main(int argc, char **argv) {
  if (argc < 3) {
    fprintf(stderr, "usage: %s <arm> <image> [n=8]\n", argv[0]);
    return 2;
  }
  const char *arm = argv[1];
  const char *path = argv[2];
  int n = argc > 3 ? atoi(argv[3]) : 8;
  if (n < 2) n = 2;

  if (!gtk_init_check()) die("gtk_init failed (need a display)");

  printf("arm=%s path=%s n=%d pid=%d\n", arm, path, n, (int)getpid());

  unsigned long prev = vmrss_kb();
  printf("  start  footprint=%lu kB\n", prev);

  for (int i = 0; i < n; i++) {
    if (strcmp(arm, "file_info") == 0) {
      gint w = 0, h = 0;
      gdk_pixbuf_get_file_info(path, &w, &h);
    } else if (strcmp(arm, "from_file") == 0) {
      GdkTexture *t = arm_from_file(path);
      g_clear_object(&t);
    } else if (strcmp(arm, "both") == 0) {
      gint w = 0, h = 0;
      gdk_pixbuf_get_file_info(path, &w, &h);
      GdkTexture *t = arm_from_file(path);
      g_clear_object(&t);
    } else if (strcmp(arm, "anim_wh") == 0) {
      GError *err = NULL;
      GdkPixbufAnimation *anim = gdk_pixbuf_animation_new_from_file(path, &err);
      if (anim) {
        int w = gdk_pixbuf_animation_get_width(anim);
        int h = gdk_pixbuf_animation_get_height(anim);
        (void)w;
        (void)h;
        g_object_unref(anim);
      } else {
        fprintf(stderr, "anim_wh: %s\n", err ? err->message : "unknown");
        g_clear_error(&err);
        return 2;
      }
    } else if (strcmp(arm, "anim_static") == 0) {
      GdkTexture *t = arm_anim_static(path);
      g_clear_object(&t);
    } else if (strcmp(arm, "from_bytes") == 0) {
      GdkTexture *t = arm_from_bytes(path);
      g_clear_object(&t);
    } else if (strcmp(arm, "anim_pixbuf") == 0) {
      /* static_image only — no GdkTexture wrap, so we can tell which object leaks. */
      GError *err = NULL;
      GdkPixbufAnimation *anim = gdk_pixbuf_animation_new_from_file(path, &err);
      if (!anim) {
        fprintf(stderr, "anim_pixbuf: %s\n", err ? err->message : "unknown");
        g_clear_error(&err);
        return 2;
      }
      GdkPixbuf *pb = gdk_pixbuf_animation_get_static_image(anim);
      (void)pb;
      g_object_unref(anim);
    } else if (strcmp(arm, "new_from_file") == 0) {
      GError *err = NULL;
      GdkPixbuf *pb = gdk_pixbuf_new_from_file(path, &err);
      if (pb) {
        GdkTexture *t = gdk_texture_new_for_pixbuf(pb);
        g_object_unref(pb);
        g_clear_object(&t);
      } else {
        fprintf(stderr, "new_from_file: %s\n", err ? err->message : "unknown");
        g_clear_error(&err);
      }
    } else if (strcmp(arm, "loader_oneshot") == 0) {
      GError *err = NULL;
      gchar *bytes = NULL;
      gsize len = 0;
      if (!g_file_get_contents(path, &bytes, &len, &err)) {
        fprintf(stderr, "read: %s\n", err ? err->message : "unknown");
        g_clear_error(&err);
        return 2;
      }
      GdkPixbufLoader *loader = gdk_pixbuf_loader_new();
      gboolean ok = gdk_pixbuf_loader_write(loader, (const guchar *)bytes, len, &err)
                    && gdk_pixbuf_loader_close(loader, err ? NULL : &err);
      g_free(bytes);
      if (!ok) {
        fprintf(stderr, "loader: %s\n", err ? err->message : "unknown");
        g_clear_error(&err);
        g_object_unref(loader);
        return 2;
      }
      GdkPixbuf *pb = gdk_pixbuf_loader_get_pixbuf(loader);
      GdkTexture *t = pb ? gdk_texture_new_for_pixbuf(pb) : NULL;
      g_object_unref(loader);
      g_clear_object(&t);
    } else {
      die("unknown arm");
    }
    unsigned long now = vmrss_kb();
    printf("  i=%d  footprint=%lu kB  delta=%ld kB\n", i, now, (long)now - (long)prev);
    prev = now;
  }
  return 0;
}
