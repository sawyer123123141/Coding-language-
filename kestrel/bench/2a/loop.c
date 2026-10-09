#include <stdio.h>
#include <stdint.h>
int main(void) {
    volatile int64_t vn = 200000000; int64_t n = vn, total = 0;
    for (int64_t i = 0; i < n; i++) total = (total + i * i) % 1000000007;
    printf("%lld\n", (long long)total); return 0;
}
