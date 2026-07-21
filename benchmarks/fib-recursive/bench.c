#include <stdio.h>
#include <stdint.h>

static int64_t fib(int64_t n) {
    if (n < 2) {
        return n;
    } else {
        return fib(n - 1) + fib(n - 2);
    }
}

int main(void) {
    printf("%lld\n", (long long)fib(38));
    return 0;
}
