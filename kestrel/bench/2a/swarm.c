#include <stdio.h>
#include <stdlib.h>

typedef struct { float x, y, vx, vy; } Particle;

int main(void) {
    int n = 1000000, len = 0, cap = 4;
    Particle *ps = malloc(cap * sizeof(Particle));
    for (int i = 0; i < n; i++) {
        if (len == cap) { cap *= 2; ps = realloc(ps, cap * sizeof(Particle)); }
        float f = (float)(i % 1000);
        ps[len++] = (Particle){ f, f * 0.5f, 1.0f - f * 0.002f, 0.0f };
    }
    for (int s = 0; s < 100; s++) {
        for (int k = 0; k < len; k++) {
            Particle *p = &ps[k];
            p->vy -= 0.01f;
            p->x += p->vx * 0.01f;
            p->y += p->vy * 0.01f;
            if (p->y < 0.0f) { p->y = -p->y; p->vy = -p->vy * 0.9f; }
        }
    }
    float sum = 0.0f;
    for (int k = 0; k < len; k++) sum += ps[k].x + ps[k].y;
    printf("%d\n", (int)sum);
    return 0;
}
