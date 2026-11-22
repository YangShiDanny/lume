#include <stdint.h>

int64_t c_matmul(int64_t n, uint8_t *buf)
{
    (void)buf;
    return n * 2;
}
