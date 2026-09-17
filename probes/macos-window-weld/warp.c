#include <ApplicationServices/ApplicationServices.h>
#include <stdlib.h>
#include <stdio.h>
int main(int argc, char **argv) {
    if (argc < 3) { fprintf(stderr, "usage: warp X Y\n"); return 2; }
    CGPoint p = CGPointMake(atof(argv[1]), atof(argv[2]));
    printf("warp: hardware-warping cursor to (%.0f,%.0f), no motion event posted\n", p.x, p.y);
    CGWarpMouseCursorPosition(p);
    CGAssociateMouseAndMouseCursorPosition(true);
    CGEventRef e = CGEventCreate(NULL);
    CGPoint q = CGEventGetLocation(e);
    printf("cursor now %.0f,%.0f\n", q.x, q.y);
    CFRelease(e);
    return 0;
}
