// macOS only. Names the CURRENT GLOBAL mouse cursor — `arrow`, `text`, `pointer`, … —
// by matching its bitmap against the standard NSCursor singletons. Cross-process:
// +[NSCursor currentSystemCursor] reads the window server's cursor, so this answers
// "what is the user actually looking at" about ANY application, not about itself.
//
// It exists because "did the cursor change?" is a question two obvious instruments
// answer badly, and both were tried first:
//
//   - A SCREENSHOT is legible but easy to over-read. Classifying arrow-from-I-beam
//     means correlating a cursor bitmap over whatever application content happens to
//     sit behind it, by eye, once per trial. A hover-cursor defect on this platform
//     was carried as "intermittent, not reliably reproducible" for a long time on
//     screenshot evidence; re-measured with this reader it was deterministic and
//     split cleanly on a single variable, 12 trials to 0.
//   - CGSCurrentCursorSeed is worse than useless HERE: `screencapture -C` increments
//     the seed itself, measured by running the identical sampling loop over blank
//     space with no hover target and seeing the same climb. An instrument inside its
//     own measurement, and the artefact it produced happened to match the hypothesis
//     under test.
//
// PROVE THE INSTRUMENT BEFORE TRUSTING A RESULT FROM IT. Three checks, each a
// different claim, all runnable in under a minute:
//   1. `cliclick m:600,10` (the menu bar) -> `arrow`. It reads the system cursor.
//   2. Hover a Terminal window's text area -> `unknown(9x18:81412)`, Terminal's own
//      custom thin I-beam. It reads a cursor set by ANOTHER process, and an `unknown`
//      is a real reading — plenty of applications ship cursors that are not the
//      singletons, and the arrow/non-arrow distinction still holds.
//   3. Hover a GtkEntry in `gtk4-widget-factory` -> `text`, and its GtkLinkButton ->
//      `pointer`. It reads what GTK sets, on a window known to be healthy.
// Check 3 is the one that matters: without a positive control on a healthy GTK
// window, a run of `arrow` readings is equally consistent with a broken application
// and a broken probe.
//
//   clang -fobjc-arc -framework Cocoa -o /tmp/cursorid probes/quartz-cursor-identity.m
#import <Cocoa/Cocoa.h>

static NSString *sig(NSCursor *c) {
  if (!c) return nil;
  NSImage *img = [c image];
  if (!img) return nil;
  NSData *tiff = [img TIFFRepresentation];
  if (!tiff) return nil;
  // Identity is (image size, TIFF byte length). NOT the hotspot: the window server's
  // copy of a cursor reports a hotspot a fraction off the singleton's (arrow reads
  // 4.5,4.0 globally against 4.0,2.0 locally), so a hotspot comparison never matches.
  NSSize z = [img size];
  return [NSString stringWithFormat:@"%.0fx%.0f:%lu", z.width, z.height,
          (unsigned long)[tiff length]];
}

int main(int argc, char **argv) {
  @autoreleasepool {
    // NSApplication must exist before the cursor singletons resolve; a bare tool
    // otherwise gets nil back from some of them.
    [NSApplication sharedApplication];
    NSMutableDictionary *known = [NSMutableDictionary dictionary];
    #define ADD(n, sel) do { NSCursor *c_ = [NSCursor sel]; if (c_) known[n] = c_; } while (0)
    ADD(@"arrow",       arrowCursor);
    ADD(@"text",        IBeamCursor);
    ADD(@"pointer",     pointingHandCursor);
    ADD(@"crosshair",   crosshairCursor);
    ADD(@"openhand",    openHandCursor);
    ADD(@"closedhand",  closedHandCursor);
    ADD(@"resize-lr",   resizeLeftRightCursor);
    ADD(@"resize-ud",   resizeUpDownCursor);
    ADD(@"text-vert",   IBeamCursorForVerticalLayout);
    ADD(@"not-allowed", operationNotAllowedCursor);
    ADD(@"contextmenu", contextualMenuCursor);
    ADD(@"dragcopy",    dragCopyCursor);
    ADD(@"draglink",    dragLinkCursor);
    ADD(@"disappear",   disappearingItemCursor);
    #undef ADD
    NSCursor *cur = [NSCursor currentSystemCursor];
    NSString *s = sig(cur);
    if (!s) { printf("unreadable\n"); return 2; }
    for (NSString *name in known) {
      if ([sig(known[name]) isEqualToString:s]) { printf("%s\n", [name UTF8String]); return 0; }
    }
    printf("unknown(%s)\n", [s UTF8String]);
    return 0;
  }
}
