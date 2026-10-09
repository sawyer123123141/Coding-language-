#include <stdio.h>
#include <stdlib.h>

int main(void) {
    int n = 50000000;
    char *composite = calloc(n, 1);
    int count = 0;
    for (int i = 2; i < n; i++) {
        if (!composite[i]) {
            count++;
            for (int j = i * 2; j < n; j += i) composite[j] = 1;
        }
    }
    printf("%d\n", count);
    return 0;
}
