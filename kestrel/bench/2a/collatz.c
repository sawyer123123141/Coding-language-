#include <stdio.h>
#include <stdint.h>
static int collatz_len(int64_t n) { int steps = 0; while (n != 1) { if (n % 2 == 0) n /= 2; else n = 3 * n + 1; steps++; } return steps; }
int main(void) {
    volatile int64_t vl = 1000000; int64_t limit = vl, best_start = 0; int best = 0;
    for (int64_t s = 1; s < limit; s++) { int len = collatz_len(s); if (len > best) { best = len; best_start = s; } }
    printf("%lld %d\n", (long long)best_start, best); return 0;
}
