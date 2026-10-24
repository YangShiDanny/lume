#ifndef LUME_RUNTIME_H
#define LUME_RUNTIME_H

#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <stdbool.h>
#include <string.h>
#include <stdarg.h>
#include <math.h>
#include <time.h>
#include <ctype.h>

static inline void lume_log_flush_thread(void);
static inline void lume_log_shutdown(void);

#ifdef _WIN32
  #include <windows.h>
  typedef HANDLE LumeThread;
  #define LUME_WORKER_RET     DWORD WINAPI
    #define LUME_WORKER_RETURN  do { lume_log_flush_thread(); return 0; } while (0)
  #define LUME_THREAD_CREATE(thread, fn, arg) \
      do { (thread) = CreateThread(NULL, 0, (LPTHREAD_START_ROUTINE)(fn), (arg), 0, NULL); } while (0)
  #define LUME_THREAD_JOIN(thread) \
      do { WaitForSingleObject((thread), INFINITE); CloseHandle((thread)); } while (0)
  typedef CONDITION_VARIABLE LumeCond;
  #define LUME_COND_INIT CONDITION_VARIABLE_INIT
  #define LUME_COND_WAIT(cond, mutex) SleepConditionVariableCS(&(cond), &(mutex).cs, INFINITE)
  #define LUME_COND_SIGNAL(cond) WakeConditionVariable(&(cond))
  #define LUME_COND_BROADCAST(cond) WakeAllConditionVariable(&(cond))
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
    #define LUME_WORKER_RETURN  do { lume_log_flush_thread(); return NULL; } while (0)
  #define LUME_THREAD_CREATE(thread, fn, arg) \
      do { pthread_create(&(thread), NULL, (fn), (arg)); } while (0)
  #define LUME_THREAD_JOIN(thread) \
      do { pthread_join((thread), NULL); } while (0)
  typedef pthread_cond_t LumeCond;
  #define LUME_COND_INIT PTHREAD_COND_INITIALIZER
  #define LUME_COND_WAIT(cond, mutex) pthread_cond_wait(&(cond), &(mutex))
  #define LUME_COND_SIGNAL(cond) pthread_cond_signal(&(cond))
  #define LUME_COND_BROADCAST(cond) pthread_cond_broadcast(&(cond))
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

/* say/write append to thread-local memory; a single background thread owns stdout. */
#define LUME_LOG_CHUNK_SIZE (64u * 1024u)
#ifdef _WIN32
  #define LUME_THREAD_LOCAL __declspec(thread)
#else
  #define LUME_THREAD_LOCAL _Thread_local
#endif

typedef struct LumeLogChunk {
        char* data;
        size_t length;
        uint64_t sequence;
        struct LumeLogChunk* next;
} LumeLogChunk;

static LUME_MUTEX_T lume_log_mutex_ = LUME_MUTEX_INIT;
static LumeCond lume_log_cond_ = LUME_COND_INIT;
static LumeLogChunk* lume_log_head_;
static LumeLogChunk* lume_log_tail_;
static uint64_t lume_log_submitted_;
static uint64_t lume_log_completed_;
static int lume_log_stopping_;
static int lume_log_started_;
static LumeThread lume_log_writer_thread_;

static LUME_THREAD_LOCAL char* lume_log_buffer_;
static LUME_THREAD_LOCAL size_t lume_log_length_;
static LUME_THREAD_LOCAL size_t lume_log_capacity_;

static inline int lume_log_reserve_(size_t extra) {
    if (extra <= lume_log_capacity_ - lume_log_length_) return 1;
    size_t needed = lume_log_length_ + extra;
    size_t capacity = lume_log_capacity_ ? lume_log_capacity_ : 1024;
    while (capacity < needed) {
        if (capacity > SIZE_MAX / 2) {
            capacity = needed;
            break;
        }
        capacity *= 2;
    }
    char* grown = (char*)realloc(lume_log_buffer_, capacity);
    if (grown == NULL) return 0;
    lume_log_buffer_ = grown;
    lume_log_capacity_ = capacity;
    return 1;
}

static inline uint64_t lume_log_submit_thread(void) {
    if (lume_log_length_ == 0) return 0;
    LumeLogChunk* chunk = (LumeLogChunk*)malloc(sizeof(LumeLogChunk));
    if (chunk == NULL) abort();

    chunk->data = lume_log_buffer_;
    chunk->length = lume_log_length_;
    chunk->next = NULL;
    lume_log_buffer_ = NULL;
    lume_log_length_ = 0;
    lume_log_capacity_ = 0;

    LUME_MUTEX_LOCK(lume_log_mutex_);
    uint64_t sequence = ++lume_log_submitted_;
    chunk->sequence = sequence;
    if (lume_log_tail_ == NULL) {
        lume_log_head_ = chunk;
    } else {
        lume_log_tail_->next = chunk;
    }
    lume_log_tail_ = chunk;
    LUME_COND_SIGNAL(lume_log_cond_);
    LUME_MUTEX_UNLOCK(lume_log_mutex_);
    return sequence;
}

static LUME_WORKER_RET lume_log_writer(void* unused) {
    (void)unused;
    for (;;) {
        LUME_MUTEX_LOCK(lume_log_mutex_);
        while (lume_log_head_ == NULL && !lume_log_stopping_) {
            LUME_COND_WAIT(lume_log_cond_, lume_log_mutex_);
        }
        if (lume_log_head_ == NULL && lume_log_stopping_) {
            LUME_MUTEX_UNLOCK(lume_log_mutex_);
            break;
        }
        LumeLogChunk* chunk = lume_log_head_;
        lume_log_head_ = chunk->next;
        if (lume_log_head_ == NULL) lume_log_tail_ = NULL;
        LUME_MUTEX_UNLOCK(lume_log_mutex_);

        fwrite(chunk->data, 1, chunk->length, stdout);
        fflush(stdout);
        free(chunk->data);

        LUME_MUTEX_LOCK(lume_log_mutex_);
        lume_log_completed_ = chunk->sequence;
        free(chunk);
        LUME_COND_BROADCAST(lume_log_cond_);
        LUME_MUTEX_UNLOCK(lume_log_mutex_);
    }
#ifdef _WIN32
    return 0;
#else
    return NULL;
#endif
}

static inline void lume_log_flush_thread(void) {
    (void)lume_log_submit_thread();
}

static inline void lume_log_write_literal(const char* text, size_t length) {
    if (length == 0) return;
    if (!lume_log_reserve_(length)) abort();
    memcpy(lume_log_buffer_ + lume_log_length_, text, length);
    lume_log_length_ += length;
    if (lume_log_length_ >= LUME_LOG_CHUNK_SIZE) {
        (void)lume_log_submit_thread();
    }
}

static inline void lume_log_write_v_(const char* format, va_list args) {
    char small[256];
    va_list small_args;
    va_copy(small_args, args);
    int needed = vsnprintf(small, sizeof(small), format, small_args);
    va_end(small_args);
    if (needed < 0) return;

    size_t length = (size_t)needed;
    if (!lume_log_reserve_(length + 1)) abort();
    if (length < sizeof(small)) {
        memcpy(lume_log_buffer_ + lume_log_length_, small, length);
    } else {
        vsnprintf(lume_log_buffer_ + lume_log_length_, length + 1, format, args);
    }
    lume_log_length_ += length;
    if (lume_log_length_ >= LUME_LOG_CHUNK_SIZE) {
        (void)lume_log_submit_thread();
    }
}

static inline void lume_logf(const char* format, ...) {
    va_list args;
    va_start(args, format);
    lume_log_write_v_(format, args);
    va_end(args);
}

static inline void lume_log_init_main(void) {
    if (lume_log_started_) return;
#ifdef _WIN32
    lume_log_writer_thread_ = CreateThread(
        NULL, 0, (LPTHREAD_START_ROUTINE)lume_log_writer, NULL, 0, NULL
    );
    if (lume_log_writer_thread_ == NULL) abort();
#else
    if (pthread_create(&lume_log_writer_thread_, NULL, lume_log_writer, NULL) != 0) {
        abort();
    }
#endif
    lume_log_started_ = 1;
    atexit(lume_log_shutdown);
}

static inline void lume_log_shutdown(void) {
    (void)lume_log_submit_thread();
    LUME_MUTEX_LOCK(lume_log_mutex_);
    lume_log_stopping_ = 1;
    LUME_COND_SIGNAL(lume_log_cond_);
    LUME_MUTEX_UNLOCK(lume_log_mutex_);
#ifdef _WIN32
    WaitForSingleObject(lume_log_writer_thread_, INFINITE);
    CloseHandle(lume_log_writer_thread_);
#else
    pthread_join(lume_log_writer_thread_, NULL);
#endif
}

static inline void lume_log_flush(void) {
    uint64_t target = lume_log_submit_thread();
    LUME_MUTEX_LOCK(lume_log_mutex_);
    if (target == 0) target = lume_log_submitted_;
    while (lume_log_completed_ < target) {
        LUME_COND_WAIT(lume_log_cond_, lume_log_mutex_);
    }
    LUME_MUTEX_UNLOCK(lume_log_mutex_);
}

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

/* ── 切片边界检查 ── */
/* a[i..j] 生成 (a + off, len) 两个值。越界在 C 里是静默的越界读写，
   编译期能算出来的（都是字面量）checker 直接报错，这里兜运行期变量。
   和除零检查同一个思路：崩得明明白白好过静默算错。
   注意 off 和 len 必须一致，所以两个函数共用同一条校验：
   0 <= i <= j <= 原长度。 */
static inline void lume_slice_check(
    int64_t start, int64_t end, int64_t base_len
) {
    if (start < 0 || end < start || end > base_len) {
        fprintf(stderr,
                "lume: runtime error: slice [%lld..%lld] out of range "
                "(len=%lld)\n",
                (long long)start, (long long)end,
                (long long)base_len);
        abort();
    }
}

static inline int64_t lume_slice_len(
    const void* base, int64_t start, int64_t end, int64_t base_len
) {
    (void)base;
    lume_slice_check(start, end, base_len);
    return end - start;
}

static inline int64_t lume_slice_off(
    const void* base, int64_t start, int64_t end, int64_t base_len
) {
    lume_slice_check(start, end, base_len);
    return start;
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

/* ── 内部：强度削弱辅助 ── */

static inline int64_t lume_shl(int64_t x, int64_t n) {
    return x << n;
}

static inline int64_t lume_bitand(int64_t x, int64_t m) {
    return x & m;
}

/* ── 内部：有符号除以 2^n（向零舍入）── */

static inline int64_t lume_sdiv_pow2(int64_t x, int64_t n) {
    int64_t mask = (1LL << n) - 1;
    return (x + ((x >> 63) & mask)) >> n;
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

int main(void);

int main(void) {
    lume_log_init_main();
    int64_t start = lume_now_ns();
    #if defined(__clang__)
    #pragma clang loop vectorize(enable) interleave(enable)
    #endif
    for (int64_t x = 1LL; x <= 100LL; x++) {
        lume_logf("%lld\n", (long long)(x));
    }
    lume_log_flush();
    int64_t elapsed = (lume_now_ns() - start);
    lume_logf("elapsed_ns=%lld\n", (long long)(elapsed));
    return 0;
}

