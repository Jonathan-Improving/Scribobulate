#include <ApplicationServices/ApplicationServices.h>
#include <stdlib.h>
#include <stdio.h>
#include <unistd.h>
int main(int argc, char **argv) {
    if (argc < 3) { fprintf(stderr, "usage: click X Y\n"); return 2; }
    CGPoint p = CGPointMake(atof(argv[1]), atof(argv[2]));
    printf("click: synthetic left click at (%.0f,%.0f)\n", p.x, p.y);
    CGEventRef mv = CGEventCreateMouseEvent(NULL, kCGEventMouseMoved, p, kCGMouseButtonLeft);
    CGEventPost(kCGHIDEventTap, mv); CFRelease(mv); usleep(80000);
    CGEventRef dn = CGEventCreateMouseEvent(NULL, kCGEventLeftMouseDown, p, kCGMouseButtonLeft);
    CGEventPost(kCGHIDEventTap, dn); CFRelease(dn); usleep(60000);
    CGEventRef up = CGEventCreateMouseEvent(NULL, kCGEventLeftMouseUp, p, kCGMouseButtonLeft);
    CGEventPost(kCGHIDEventTap, up); CFRelease(up);
    return 0;
}
