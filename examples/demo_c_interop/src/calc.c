#include <stdint.h>

#if defined(_WIN32)
#define LUME_EXPORT __declspec(dllexport)
#else
#define LUME_EXPORT
#endif

typedef int64_t (*lume_callback_fn)(int64_t);

LUME_EXPORT int64_t c_matmul(int64_t n, void *callback)
{
    lume_callback_fn fn = (lume_callback_fn)callback;
    return fn ? fn(n) : n * 2;
}
