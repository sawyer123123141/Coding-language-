#include <stdio.h>
#include <stdint.h>
int main(void) {
    volatile int vs = 2000; int size = vs, max_iter = 200, inside = 0; int64_t total_iter = 0;
    for (int py = 0; py < size; py++) for (int px = 0; px < size; px++) {
        float cx = (float)px / (float)size * 3.0f - 2.0f, cy = (float)py / (float)size * 3.0f - 1.5f;
        float x = 0.0f, y = 0.0f; int i = 0;
        while (i < max_iter && x * x + y * y <= 4.0f) { float xt = x * x - y * y + cx; y = 2.0f * x * y + cy; x = xt; i++; }
        total_iter += i; if (i == max_iter) inside++;
    }
    printf("%d %lld\n", inside, (long long)total_iter); return 0;
}
