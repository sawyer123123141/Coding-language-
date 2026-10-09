#include <stdio.h>
typedef struct { float x, y, vx, vy; } Particle;
static void step(Particle *p, float dt) {
    p->vy -= 9.8f * dt; p->x += p->vx * dt; p->y += p->vy * dt;
    if (p->y < 0.0f) { p->y = -p->y; p->vy = -p->vy * 0.9f; }
}
int main(void) {
    volatile int vn = 100000000; volatile float vdt = 0.001f; int n = vn; float dt = vdt;
    Particle p = {0.0f, 10.0f, 1.0f, 0.0f}; int bounces = 0;
    for (int i = 0; i < n; i++) { float before = p.vy; step(&p, dt); if (before < 0.0f && p.vy > 0.0f) bounces++; }
    printf("%d %d %d\n", (int)(p.x * 1000.0f), (int)(p.y * 1000.0f), bounces); return 0;
}
