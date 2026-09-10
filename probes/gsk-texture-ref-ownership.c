/* gsk-texture-ref-ownership.c — does anything inside GSK hold a strong reference
 * to a GdkTexture after it has been rendered?
 *
 * WHY THIS EXISTS
 * A per-render memory leak (an animated image re-decoded and retained on every
 * preview re-render) is to be gated by a finalization test: weak-ref the
 * per-render decoded texture, drive N re-renders, assert generation N-1
 * finalized. That test is only honest if nothing inside GSK is keeping the old
 * generation alive. The worry was GSK's uploaded-texture cache: if the cache
 * held a ref, finalization would wait on cache eviction, eviction is
 * frame-driven, and an unmapped test window has no ticking frame clock — so the
 * gate would go red on healthy code. A mandatory gate that reds on healthy code
 * gets disabled, and then it protects nothing.
 *
 * The source says the worry is unfounded, in both versions, and that the
 * ownership runs the other way: 4.6.9 gsk/gl/gskgldriver.c:806-811 stores a bare
 * pointer alongside gdk_texture_set_render_data, and 4.22.4
 * gsk/gpu/gskgpucache.c uses g_object_weak_ref — neither takes a reference. The
 * association is torn down BY the texture dying. This probe is the measurement
 * that claim is worth, because "confirmed against the C source" is evidence and
 * not proof.
 *
 * WHAT IT MEASURES
 * Phase A — ownership. Render a texture through each renderer that will realize,
 *   then drop only the references this probe holds, with ZERO main-loop
 *   iterations. If the texture finalizes, nothing in GSK held it.
 *   Two negative controls run first: the texture must NOT finalize while the
 *   probe still holds a reference, and must NOT finalize when only the render
 *   node has been released. A probe that cannot fail cannot confirm.
 *
 * Phase B — the planted leak. Retain the render node tree and drop the probe's
 *   own texture reference. GskTextureNode DOES take a strong reference
 *   (gsk/gsktexturenode.c:280 @ 4.22.4), so this must NOT finalize — not after
 *   idle iterations, and not after outlasting 4.22.4's 15-second wall-clock
 *   cache GC (gsk/gpu/gskgpudevice.c CACHE_TIMEOUT). Releasing the node must
 *   then finalize it immediately. This is the arm that proves the eventual gate
 *   reddens on the real defect shape, and reddens for the RIGHT reason: the
 *   timing hypothesis is explicitly excluded rather than merely unmentioned.
 *
 * CROSS-VERSION
 * Builds and runs at the project's 4.6 floor as well as at 4.22.4.
 * gsk_renderer_realize_for_display is 4.14+, so the realize call is guarded;
 * everything else here (gdk_memory_texture_new, gsk_texture_node_new,
 * gsk_renderer_render_texture, gsk_render_node_unref, GWeakRef) exists at 4.6.
 * Reads no environment switches — there are no arms to name.
 *
 * EXIT CODES
 *   0  every assertion held
 *   1  no renderer would realize (measured nothing — NOT a pass)
 *   2  an assertion failed; the finalization gate is not safe as designed
 */

#include <gtk/gtk.h>
/* GTK 4.6 declares gsk_gl_renderer_new here and does not reach it from gtk.h, so without
 * this the probe will not build on the floor. Guarded because at 4.14+ the same header is
 * only a deprecation shim whose body is a #warning (the declaration moved to
 * gsk/gpu/gskglrenderer.h and arrives via gtk.h) -- measured firing on macOS 4.22.4. */
#if !GTK_CHECK_VERSION(4, 14, 0)
# include <gsk/gl/gskglrenderer.h>
#endif

#define TEX_DIM 128
#define CACHE_TIMEOUT_OUTLAST_SECS 17   /* 4.22.4 CACHE_TIMEOUT is 15s */
#define IDLE_SPINS 500

static int failures = 0;

static void fail(const char *fmt, ...) {
  va_list ap; va_start(ap, fmt);
  char *m = g_strdup_vprintf(fmt, ap); va_end(ap);
  g_print("    FAIL: %s\n", m); g_free(m); failures++;
}

typedef struct { gboolean finalized; } Watch;
static void on_finalize(gpointer data, GObject *where_the_object_was) {
  ((Watch *)data)->finalized = TRUE;
}

static GdkTexture *make_texture(void) {
  gsize n = (gsize)TEX_DIM * TEX_DIM * 4;
  guchar *p = g_malloc(n);
  /* Non-trivial content: a solid fill could in principle be interned. */
  for (gsize i = 0; i < n; i++) p[i] = (guchar)(i * 7 + 3);
  GBytes *b = g_bytes_new_take(p, n);
  GdkTexture *t = gdk_memory_texture_new(TEX_DIM, TEX_DIM,
                                         GDK_MEMORY_R8G8B8A8, b, TEX_DIM * 4);
  g_bytes_unref(b);
  return t;
}

static void spin_idle(int n) {
  for (int i = 0; i < n; i++) g_main_context_iteration(NULL, FALSE);
}

/* Realize whichever way this GTK supports. Returns FALSE with *error set. */
static gboolean realize_offscreen(GskRenderer *r, GError **error) {
#if GTK_CHECK_VERSION(4, 14, 0)
  return gsk_renderer_realize_for_display(r, gdk_display_get_default(), error);
#else
  return gsk_renderer_realize(r, NULL, error);
#endif
}

/* Phase A: after a render, does anything in GSK still hold the texture? */
static void phase_a(const char *label, GskRenderer *r) {
  g_print("  [A/%s] ownership after render\n", label);
  Watch w = { FALSE };
  GdkTexture *tex = make_texture();
  g_object_weak_ref(G_OBJECT(tex), on_finalize, &w);

  graphene_rect_t rect = GRAPHENE_RECT_INIT(0, 0, TEX_DIM, TEX_DIM);
  GskRenderNode *node = gsk_texture_node_new(tex, &rect);
  GdkTexture *out = gsk_renderer_render_texture(r, node, NULL);
  if (out == NULL) { fail("[A/%s] render_texture returned NULL", label); }
  g_clear_object(&out);

  /* Negative control 1: we still hold tex, so it must be alive. */
  if (w.finalized) {
    fail("[A/%s] finalized while the probe still held a reference", label);
    gsk_render_node_unref(node);
    return;
  }
  g_print("    control: alive while probe holds a ref ....... ok\n");

  /* Negative control 2: node released, we still hold tex. Still alive. */
  gsk_render_node_unref(node);
  if (w.finalized) {
    fail("[A/%s] finalized on node release while probe held a reference", label);
    return;
  }
  g_print("    control: alive after node release ............ ok\n");

  /* The measurement: drop our last ref, spin NOTHING. */
  g_object_unref(tex);
  if (w.finalized) {
    g_print("    RESULT:  finalized with 0 loop iterations .... no GSK strong ref\n");
  } else {
    fail("[A/%s] NOT finalized after the probe's last ref was dropped -- "
         "something in GSK holds it; finalization gate is unsafe as designed", label);
    spin_idle(IDLE_SPINS);
    g_print("    after %d idle iterations: finalized=%s\n",
            IDLE_SPINS, w.finalized ? "YES (needs an idle pump)" : "still NO");
  }
}

/* Phase B: a deliberately planted leak must be detected, and not by luck. */
static void phase_b(const char *label, GskRenderer *r) {
  g_print("  [B/%s] planted leak: render node tree retained\n", label);
  Watch w = { FALSE };
  GdkTexture *tex = make_texture();
  g_object_weak_ref(G_OBJECT(tex), on_finalize, &w);

  graphene_rect_t rect = GRAPHENE_RECT_INIT(0, 0, TEX_DIM, TEX_DIM);
  GskRenderNode *node = gsk_texture_node_new(tex, &rect);
  GdkTexture *out = gsk_renderer_render_texture(r, node, NULL);
  g_clear_object(&out);

  g_object_unref(tex);            /* drop OURS; the node still holds one */

  if (w.finalized) {
    fail("[B/%s] finalized while the render node still held it -- "
         "GskTextureNode is expected to hold a strong reference", label);
    return;
  }
  g_print("    leak still live immediately ................... ok\n");

  spin_idle(IDLE_SPINS);
  if (w.finalized) { fail("[B/%s] leak vanished after idle spin", label); return; }
  g_print("    leak still live after %d idle iterations ..... ok\n", IDLE_SPINS);

  /* Exclude the timing hypothesis: outlast the wall-clock cache GC. */
  g_print("    sleeping %ds to outlast the cache GC ...\n", CACHE_TIMEOUT_OUTLAST_SECS);
  g_usleep((gulong)CACHE_TIMEOUT_OUTLAST_SECS * G_USEC_PER_SEC);
  spin_idle(IDLE_SPINS);
  if (w.finalized) {
    fail("[B/%s] leak vanished after %ds -- retention is TIME-dependent, "
         "so a finalization gate here would be timing-sensitive",
         label, CACHE_TIMEOUT_OUTLAST_SECS);
    return;
  }
  g_print("    leak still live after %ds + spin ............. ok\n", CACHE_TIMEOUT_OUTLAST_SECS);

  gsk_render_node_unref(node);
  if (w.finalized) {
    g_print("    RESULT:  finalized the instant the node was released\n");
  } else {
    fail("[B/%s] NOT finalized even after releasing the node tree", label);
  }
}

static void run(const char *label, GskRenderer *r) {
  GError *e = NULL;
  if (!realize_offscreen(r, &e)) {
    g_print("  [%s] would not realize: %s (skipped)\n", label,
            e ? e->message : "no error set");
    g_clear_error(&e);
    return;
  }
  g_print("%s renderer -- %s\n", label, G_OBJECT_TYPE_NAME(r));
  phase_a(label, r);
  phase_b(label, r);
  gsk_renderer_unrealize(r);
  g_print("\n");
}

int main(void) {
  gtk_init();

  g_print("gsk-texture-ref-ownership\n");
  g_print("  compiled against GTK %d.%d.%d, running on %d.%d.%d\n",
          GTK_MAJOR_VERSION, GTK_MINOR_VERSION, GTK_MICRO_VERSION,
          gtk_get_major_version(), gtk_get_minor_version(), gtk_get_micro_version());
  g_print("  display backend: %s\n",
          gdk_display_get_default() ? G_OBJECT_TYPE_NAME(gdk_display_get_default())
                                    : "(none)");
  g_print("  realize path: %s\n\n",
#if GTK_CHECK_VERSION(4, 14, 0)
          "gsk_renderer_realize_for_display (4.14+)");
#else
          "gsk_renderer_realize(NULL surface) (pre-4.14)");
#endif

  int realized = 0;
  struct { const char *label; GskRenderer *(*ctor)(void); } arms[] = {
    { "gl",    gsk_gl_renderer_new },
    { "cairo", gsk_cairo_renderer_new },
  };
  for (gsize i = 0; i < G_N_ELEMENTS(arms); i++) {
    GskRenderer *r = arms[i].ctor();
    GError *e = NULL;
    gboolean ok = realize_offscreen(r, &e);
    if (ok) { gsk_renderer_unrealize(r); realized++; }
    g_clear_error(&e);
    g_object_unref(r);
    r = arms[i].ctor();
    run(arms[i].label, r);
    g_object_unref(r);
  }

  if (realized == 0) {
    g_print("VERDICT: no renderer realized -- this run measured NOTHING.\n");
    return 1;
  }
  if (failures > 0) {
    g_print("VERDICT: %d assertion(s) failed. Do NOT make the finalization "
            "half mandatory on this version.\n", failures);
    return 2;
  }
  g_print("VERDICT: no GSK strong reference; a planted leak is detected and not "
          "by timing. Finalization gate is sound on this version.\n");
  return 0;
}
