#include <ApplicationServices/ApplicationServices.h>
#include <stdlib.h>
#include <stdio.h>
#include <unistd.h>
// Post real mouseMoved events along a path from current position to (x,y).
int main(int argc, char **argv) {
    if (argc < 3) { fprintf(stderr, "usage: mmove X Y [steps]\n"); return 2; }
    double tx = atof(argv[1]), ty = atof(argv[2]);
    int steps = argc > 3 ? atoi(argv[3]) : 20;
    CGEventRef cur = CGEventCreate(NULL);
    CGPoint p0 = CGEventGetLocation(cur); CFRelease(cur);
    printf("mmove: from (%.0f,%.0f) to (%.0f,%.0f) steps=%d\n", p0.x, p0.y, tx, ty, steps);
    for (int i = 1; i <= steps; i++) {
        double t = (double)i / steps;
        CGPoint p = CGPointMake(p0.x + (tx-p0.x)*t, p0.y + (ty-p0.y)*t);
        CGEventRef e = CGEventCreateMouseEvent(NULL, kCGEventMouseMoved, p, kCGMouseButtonLeft);
        CGEventPost(kCGHIDEventTap, e);
        CFRelease(e);
        usleep(15000);
    }
    CGEventRef c2 = CGEventCreate(NULL);
    CGPoint q = CGEventGetLocation(c2); CFRelease(c2);
    printf("cursor now %.0f,%.0f\n", q.x, q.y);
    return 0;
}
