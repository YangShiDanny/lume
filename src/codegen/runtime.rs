pub const RUNTIME_HEADER: &str = r#"#ifndef LUME_RUNTIME_H
#define LUME_RUNTIME_H

#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <stdbool.h>
#include <string.h>
#include <math.h>
#include <time.h>
#include <ctype.h>

#ifdef _WIN32
  #include <windows.h>
  typedef HANDLE LumeThread;
  #define LUME_WORKER_RET     DWORD WINAPI
  #define LUME_WORKER_RETURN  return 0
  #define LUME_THREAD_CREATE(thread, fn, arg) \
      do { (thread) = CreateThread(NULL, 0, (LPTHREAD_START_ROUTINE)(fn), (arg), 0, NULL); } while (0)
  #define LUME_THREAD_JOIN(thread) \
      do { WaitForSingleObject((thread), INFINITE); CloseHandle((thread)); } while (0)
  /* 互斥：用 InitOnceExecuteOnce 做一次性初始化，避免静态初始化顺序问题 */
  #define LUME_MUTEX_T        struct { CRITICAL_SECTION cs; INIT_ONCE once; }
  #define LUME_MUTEX_INIT     { 0 }
  static inline BOOL CALLBACK lume_init_cs_(PINIT_ONCE _o, PVOID _p, PVOID* _c) {
      (void)_o; (void)_c;
      InitializeCriticalSection((CRITICAL_SECTION*)_p);
      return TRUE;
  }
  #define LUME_MUTEX_LOCK(m)   do { InitOnceExecuteOnce(&(m).once, lume_init_cs_, (PVOID)&(m).cs, NULL); EnterCriticalSection(&(m).cs); } while (0)
  #define LUME_MUTEX_UNLOCK(m) LeaveCriticalSection(&(m).cs)
  #define LUME_NEXT_SLOT(p)    ((unsigned long)(InterlockedIncrement((long*)(p)) - 1))
  /* 结果缓存下来：par for 每次都要问一遍核数 */
  static inline int lume_cpu_count(void) {
      static int cached = 0;
      if (cached > 0) return cached;
      SYSTEM_INFO si;
      GetSystemInfo(&si);
      int n = (int)si.dwNumberOfProcessors;
      cached = n < 1 ? 1 : n;
      return cached;
  }
  static inline int64_t lume_now_ns(void) {
      LARGE_INTEGER freq, counter;
      QueryPerformanceFrequency(&freq);
      QueryPerformanceCounter(&counter);
      return (int64_t)(counter.QuadPart * 1000000000LL / freq.QuadPart);
  }
#else
  #include <pthread.h>
  #include <unistd.h>
  typedef pthread_t LumeThread;
  #define LUME_WORKER_RET     void*
  #define LUME_WORKER_RETURN  return NULL
  #define LUME_THREAD_CREATE(thread, fn, arg) \
      do { pthread_create(&(thread), NULL, (fn), (arg)); } while (0)
  #define LUME_THREAD_JOIN(thread) \
      do { pthread_join((thread), NULL); } while (0)
  /* 互斥 */
  #define LUME_MUTEX_T        pthread_mutex_t
  #define LUME_MUTEX_INIT     PTHREAD_MUTEX_INITIALIZER
  #define LUME_MUTEX_LOCK(m)  pthread_mutex_lock(&(m))
  #define LUME_MUTEX_UNLOCK(m) pthread_mutex_unlock(&(m))
  #define LUME_NEXT_SLOT(p)   ((unsigned long)__sync_fetch_and_add((long*)(p), 1))
  /* 结果缓存下来：par for 每次都要问一遍核数，sysconf 是次系统调用 */
  static inline int lume_cpu_count(void) {
      static int cached = 0;
      if (cached > 0) return cached;
      long n = sysconf(_SC_NPROCESSORS_ONLN);
      if (n < 1) n = 1;
      cached = (int)n;
      return cached;
  }
  static inline int64_t lume_now_ns(void) {
      struct timespec ts;
      clock_gettime(CLOCK_MONOTONIC, &ts);
      return (int64_t)ts.tv_sec * 1000000000LL + (int64_t)ts.tv_nsec;
  }
#endif

/* ── 优化提示 ── */

/* 少于这个元素数，par for 直接串行跑、不开线程。
   一次 pthread_create / CreateThread 就要几十 µs，而几千个元素的循环
   是微秒级——不设阈值的话小数据上开线程纯属净亏。
   阈值改这一个地方就行。 */
#define LUME_PAR_SERIAL_THRESHOLD 1024

/* Lume 的数组是值语义，两个数组参数不可能互相别名。
   给数组参数加 restrict，clang 才敢把循环向量化（这是打平 Fortran 的关键）。 */
#define LUME_RESTRICT __restrict

/* __attribute__((const))：函数只看参数、只返回值，不读不写全局状态。
   clang 因此可以消除重复调用、把调用提到循环外。
   只有编译器证明过纯度的函数才会被标上（见 purity.rs）。 */
#if defined(__GNUC__) || defined(__clang__)
#  define LUME_PURE __attribute__((const))
#else
#  define LUME_PURE
#endif

/* ── 除零 / 溢出检查 ── */
/* 整数除零、INT64_MIN / -1 在 C 里都是 UB（后者直接 SIGFPE）。
   Lume 双保险：编译期常量 0 直接报错，运行期变量兜底 ——
   错误信息说明白，然后 abort。崩得明明白白好过静默算错。
   double 除法是 IEEE 语义（inf 有定义），不包这层，零开销。 */
static inline int64_t lume_checked_div_i64(int64_t a, int64_t b) {
    if (b == 0) {
        fprintf(stderr, "lume: runtime error: division by zero\n");
        abort();
    }
    if (a == INT64_MIN && b == -1) {
        fprintf(stderr, "lume: runtime error: INT64_MIN / -1 overflow\n");
        abort();
    }
    return a / b;
}

static inline int64_t lume_checked_mod_i64(int64_t a, int64_t b) {
    if (b == 0) {
        fprintf(stderr, "lume: runtime error: modulo by zero\n");
        abort();
    }
    if (a == INT64_MIN && b == -1) {
        /* C99 下 INT64_MIN % -1 是 UB，数学余数是 0 */
        return 0;
    }
    return a % b;
}

/* ── std.str ── */
/* 字符串函数返回内部缓冲区。用轮转槽位代替单一 static buf，
   这样 say("{str_upper(a)} {str_upper(b)}") 不会拿到同一个指针。 */

#define LUME_STR_SLOTS 8

static inline int64_t lume_str_len(const char* s) {
    return (int64_t)strlen(s);
}

static inline const char* lume_str_upper(const char* s) {
    static char bufs[LUME_STR_SLOTS][4096];
    static long slot = 0;
    char* buf = bufs[LUME_NEXT_SLOT(&slot) % LUME_STR_SLOTS];
    size_t n = strlen(s);
    if (n > 4095) n = 4095;
    for (size_t i = 0; i < n; i++) buf[i] = (char)toupper((unsigned char)s[i]);
    buf[n] = 0;
    return buf;
}

static inline const char* lume_str_lower(const char* s) {
    static char bufs[LUME_STR_SLOTS][4096];
    static long slot = 0;
    char* buf = bufs[LUME_NEXT_SLOT(&slot) % LUME_STR_SLOTS];
    size_t n = strlen(s);
    if (n > 4095) n = 4095;
    for (size_t i = 0; i < n; i++) buf[i] = (char)tolower((unsigned char)s[i]);
    buf[n] = 0;
    return buf;
}

static inline const char* lume_str_trim(const char* s) {
    static char bufs[LUME_STR_SLOTS][4096];
    static long slot = 0;
    char* buf = bufs[LUME_NEXT_SLOT(&slot) % LUME_STR_SLOTS];
    size_t n = strlen(s);
    if (n > 4095) n = 4095;
    size_t start = 0;
    while (start < n && isspace((unsigned char)s[start])) start++;
    size_t end = n;
    while (end > start && isspace((unsigned char)s[end - 1])) end--;
    size_t len = end - start;
    memcpy(buf, s + start, len);
    buf[len] = 0;
    return buf;
}

static inline int64_t lume_str_to_int(const char* s) {
    return (int64_t)atoll(s);
}

/* contains / starts_with / ends_with：都返回 0/1。
   不自己写匹配循环，直接交给 strstr / strncmp —— 它们是 libc 里
   少数几个用 SIMD 优化过的函数，手写循环只会更慢。 */

static inline int64_t lume_str_contains(const char* s, const char* sub) {
    return strstr(s, sub) != NULL ? 1 : 0;
}

static inline int64_t lume_str_starts_with(const char* s, const char* pre) {
    size_t n = strlen(pre);
    return strncmp(s, pre, n) == 0 ? 1 : 0;
}

static inline int64_t lume_str_ends_with(const char* s, const char* suf) {
    size_t ls = strlen(s), lf = strlen(suf);
    if (lf > ls) return 0;
    return strcmp(s + (ls - lf), suf) == 0 ? 1 : 0;
}

/* replace：结果可能比原文长，用轮转缓冲区装。
   上限 4095，超了截断 —— 和其他 str_* 的行为一致。 */
static inline const char* lume_str_replace(
    const char* s, const char* old_s, const char* new_s
) {
    static char bufs[LUME_STR_SLOTS][4096];
    static long slot = 0;
    char* buf = bufs[LUME_NEXT_SLOT(&slot) % LUME_STR_SLOTS];
    size_t old_len = strlen(old_s);
    size_t new_len = strlen(new_s);
    if (old_len == 0) {
        /* 空 old 会让匹配位置永不前进 → 死循环，直接返回原文 */
        strncpy(buf, s, 4095);
        buf[4095] = 0;
        return buf;
    }
    size_t pos = 0;
    const char* p = s;
    while (*p != 0 && pos < 4095) {
        const char* hit = strstr(p, old_s);
        size_t seg = hit ? (size_t)(hit - p) : strlen(p);
        if (pos + seg > 4095) seg = 4095 - pos;
        memcpy(buf + pos, p, seg);
        pos += seg;
        if (hit == 0) break;
        size_t nl = new_len;
        if (pos + nl > 4095) nl = 4095 - pos;
        memcpy(buf + pos, new_s, nl);
        pos += nl;
        p = hit + old_len;
    }
    buf[pos] = 0;
    return buf;
}

/* split：返回字符串数组（段指针）+ 段数出参。
   段内容顺序放在一个 4096 的缓冲里，指针指进去；
   整套（指针数组 + 字符缓冲）按槽位轮转，连续调用不互相覆盖。 */
#define LUME_STR_SPLIT_MAX 64

static inline const char** lume_str_split(
    const char* s, const char* sep, int64_t* out_len
) {
    static char bufs[LUME_STR_SLOTS][4096];
    static const char* ptrs[LUME_STR_SLOTS][LUME_STR_SPLIT_MAX];
    static long slot = 0;
    long idx = LUME_NEXT_SLOT(&slot) % LUME_STR_SLOTS;
    char* buf = bufs[idx];
    const char** out = ptrs[idx];
    size_t sep_len = strlen(sep);
    if (sep_len == 0) sep_len = 1;  /* 空分隔符：退化成按字符切，避免死循环 */
    int64_t count = 0;
    size_t pos = 0;
    const char* p = s;
    while (*p != 0 && count < LUME_STR_SPLIT_MAX - 1) {
        const char* hit = strstr(p, sep);
        size_t seg = hit ? (size_t)(hit - p) : strlen(p);
        if (pos + seg + 1 > 4096) seg = 4096 - pos - 1;
        memcpy(buf + pos, p, seg);
        buf[pos + seg] = 0;
        out[count++] = buf + pos;
        pos += seg + 1;
        if (hit == 0) break;
        p = hit + sep_len;
    }
    *out_len = count;
    return out;
}

static inline double lume_str_to_float(const char* s) {
    return atof(s);
}

/* ── std.int ── */

static inline int64_t lume_int_abs(int64_t x) {
    return x < 0 ? -x : x;
}

static inline int64_t lume_int_min(int64_t a, int64_t b) {
    return a < b ? a : b;
}

static inline int64_t lume_int_max(int64_t a, int64_t b) {
    return a > b ? a : b;
}

/* ── std.float ── */

static inline double lume_float_sqrt(double x) { return sqrt(x); }
static inline double lume_float_abs(double x) { return fabs(x); }
static inline double lume_float_pow(double x, double y) { return pow(x, y); }
static inline double lume_float_sin(double x) { return sin(x); }
static inline double lume_float_cos(double x) { return cos(x); }
static inline double lume_float_tan(double x) { return tan(x); }
static inline double lume_float_floor(double x) { return floor(x); }
static inline double lume_float_ceil(double x) { return ceil(x); }
static inline double lume_float_round(double x) { return round(x); }

/* ── 128 位整数 ──
   tcc 不支持 __int128，用 __SIZEOF_INT128__ 守卫：tcc 不定义这个宏，
   预处理阶段直接跳过整段，普通程序（不用 i128）的编译不受影响；
   只有用到 i128/u128 且编译器支持（gcc/clang 64 位）时才包含。 */
#if defined(__SIZEOF_INT128__)
static char _lume_i128_buf[8][48];
static int _lume_i128_slot = 0;
static const char* lume_i128_str(__int128 v) {
    char* b = _lume_i128_buf[_lume_i128_slot];
    _lume_i128_slot = (_lume_i128_slot + 1) & 7;
    if (v == 0) { b[0] = '0'; b[1] = 0; return b; }
    int neg = v < 0;
    /* 用无符号算术取绝对值，避开 INT128_MIN 的符号溢出 UB */
    unsigned __int128 u = neg ? ((unsigned __int128)0 - (unsigned __int128)v)
                              : (unsigned __int128)v;
    int i = 47;
    while (u > 0) { b[i--] = (char)('0' + (int)(u % 10ULL)); u /= 10ULL; }
    if (neg) b[i--] = '-';
    return b + i + 1;
}
static char _lume_u128_buf[8][48];
static int _lume_u128_slot = 0;
static const char* lume_u128_str(unsigned __int128 v) {
    char* b = _lume_u128_buf[_lume_u128_slot];
    _lume_u128_slot = (_lume_u128_slot + 1) & 7;
    if (v == 0) { b[0] = '0'; b[1] = 0; return b; }
    int i = 47;
    while (v > 0) { b[i--] = (char)('0' + (int)(v % 10ULL)); v /= 10ULL; }
    return b + i + 1;
}
#endif

#endif
"#;