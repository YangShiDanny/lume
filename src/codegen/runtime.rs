pub const RUNTIME_HEADER: &str = r#"#ifndef LUME_RUNTIME_H
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

/* ── par for 常驻线程池 ──
   以前每个 par for 都 pthread_create/join 一轮，固定成本约 100 µs：
   10 万元素以下的循环开线程净亏（实测 1 万元素慢 60 倍）。
   现在第一次 par for 建 N-1 个常驻 worker，主线程也一起抢任务；
   之后每次只做一次代际广播 + 等待。

   唤醒几个 worker 由调用方按元素数算：元素数 / LUME_PAR_CHUNK_ELEMS + 1。
   小循环只叫 1~2 个，大循环才全叫醒 —— 每个 worker 都用**自己的**
   条件变量，唤醒成本只和真正参与的数量成正比。 */

#define LUME_POOL_MAX_WORKERS 63
#define LUME_PAR_CHUNK_ELEMS 32768

typedef LUME_WORKER_RET (*LumeWorkerFn)(void*);

#ifdef _WIN32
#  define LUME_COND_INIT_ONE(c) InitializeConditionVariable(&(c))
#else
#  define LUME_COND_INIT_ONE(c) pthread_cond_init(&(c), NULL)
#endif

static LUME_MUTEX_T lume_pool_mutex_ = LUME_MUTEX_INIT;
static LumeCond lume_pool_done_ = LUME_COND_INIT;
static LumeCond lume_pool_conds_[LUME_POOL_MAX_WORKERS];
static LumeThread lume_pool_threads_[LUME_POOL_MAX_WORKERS];
static int lume_pool_nworkers_ = 0;
static int lume_pool_started_ = 0;
static int lume_pool_stopping_ = 0;
static uint64_t lume_pool_generation_ = 0;
static LumeWorkerFn lume_pool_fn_ = NULL;
static char* lume_pool_args_ = NULL;
static size_t lume_pool_stride_ = 0;
static int lume_pool_ntasks_ = 0;
static int lume_pool_active_ = 0;
static long lume_pool_next_ = 0;
static int lume_pool_busy_ = 0;
/* 当前线程是否正在执行 par 任务。par for 里再调 par for（含跨函数）
   会把池状态冲掉，这种递归并行直接退化成串行。 */
static LUME_THREAD_LOCAL int lume_pool_depth_ = 0;

static inline void lume_pool_call_(LumeWorkerFn fn, void* arg) {
    lume_pool_depth_++;
    fn(arg);
    lume_pool_depth_--;
}

static void lume_par_shutdown(void);
static LUME_WORKER_RET lume_pool_worker_(void* arg);

static void lume_par_init_locked(void) {
    if (lume_pool_started_) return;
    int total = lume_cpu_count();
    /* 运行时旋钮：LUME_PAR_THREADS=4 表示总共用 4 个线程（含主线程）。
       默认用全部逻辑核；内存带宽受限的循环往往少开几个更快。 */
    const char* env = getenv("LUME_PAR_THREADS");
    if (env != NULL && env[0] != 0) {
        int v = atoi(env);
        if (v > 0) total = v;
    }
    int n = total - 1;  /* 主线程也抢任务 */
    if (n < 0) n = 0;
    if (n > LUME_POOL_MAX_WORKERS) n = LUME_POOL_MAX_WORKERS;
    lume_pool_nworkers_ = n;
    lume_pool_started_ = 1;
    for (int i = 0; i < n; i++) {
        LUME_COND_INIT_ONE(lume_pool_conds_[i]);
        LUME_THREAD_CREATE(
            lume_pool_threads_[i], lume_pool_worker_, (void*)(size_t)i
        );
    }
    atexit(lume_par_shutdown);
}

static LUME_WORKER_RET lume_pool_worker_(void* arg) {
    int idx = (int)(size_t)arg;
    uint64_t seen = 0;
    for (;;) {
        LUME_MUTEX_LOCK(lume_pool_mutex_);
        while (!lume_pool_stopping_ && lume_pool_generation_ == seen) {
            LUME_COND_WAIT(lume_pool_conds_[idx], lume_pool_mutex_);
        }
        if (lume_pool_stopping_) {
            LUME_MUTEX_UNLOCK(lume_pool_mutex_);
            break;
        }
        uint64_t gen = lume_pool_generation_;
        if (idx >= lume_pool_active_) {
            /* 本次没叫我（或信号丢失后我晚到了）—— 标记已读，继续睡 */
            seen = gen;
            LUME_MUTEX_UNLOCK(lume_pool_mutex_);
            continue;
        }
        seen = gen;
        LumeWorkerFn fn = lume_pool_fn_;
        char* base = lume_pool_args_;
        size_t stride = lume_pool_stride_;
        int ntasks = lume_pool_ntasks_;
        LUME_MUTEX_UNLOCK(lume_pool_mutex_);

        int ran = 0;
        for (;;) {
            int t = (int)LUME_NEXT_SLOT(&lume_pool_next_);
            if (t >= ntasks) break;
            lume_pool_call_(fn, base + (size_t)t * stride);
            ran = 1;
        }
        if (ran) lume_log_flush_thread();

        LUME_MUTEX_LOCK(lume_pool_mutex_);
        lume_pool_busy_--;
        if (lume_pool_busy_ == 0) LUME_COND_BROADCAST(lume_pool_done_);
        LUME_MUTEX_UNLOCK(lume_pool_mutex_);
    }
    LUME_WORKER_RETURN;
}

static void lume_par_shutdown(void) {
    if (!lume_pool_started_) return;
    LUME_MUTEX_LOCK(lume_pool_mutex_);
    lume_pool_stopping_ = 1;
    for (int i = 0; i < lume_pool_nworkers_; i++) {
        LUME_COND_SIGNAL(lume_pool_conds_[i]);
    }
    LUME_MUTEX_UNLOCK(lume_pool_mutex_);
    for (int i = 0; i < lume_pool_nworkers_; i++) {
        LUME_THREAD_JOIN(lume_pool_threads_[i]);
    }
    lume_pool_started_ = 0;
    lume_pool_nworkers_ = 0;
    lume_pool_stopping_ = 0;
}

/* 提交一批任务：args 是结构体数组，stride 是 sizeof(结构体)。
   nactive 是本次参与的线程总数（含主线程），1 表示主线程串行跑完。 */
static inline void lume_par_run(
    LumeWorkerFn fn, void* args, size_t stride, int ntasks, int nactive
) {
    if (ntasks <= 0) return;

    /* 嵌套并行（par for 里再 par for，或经由函数间接调用）→ 串行 */
    if (lume_pool_depth_ > 0 || nactive <= 1) {
        for (int t = 0; t < ntasks; t++) {
            lume_pool_call_(fn, (char*)args + (size_t)t * stride);
        }
        lume_log_flush_thread();
        return;
    }

    LUME_MUTEX_LOCK(lume_pool_mutex_);
    if (!lume_pool_started_) lume_par_init_locked();
    int max_parts = lume_pool_nworkers_ + 1;
    if (nactive > max_parts) nactive = max_parts;
    if (nactive <= 1) {
        LUME_MUTEX_UNLOCK(lume_pool_mutex_);
        for (int t = 0; t < ntasks; t++) {
            lume_pool_call_(fn, (char*)args + (size_t)t * stride);
        }
        lume_log_flush_thread();
        return;
    }
    int active = nactive - 1;  /* 唤醒的 worker 数，主线程也算一个 */

    lume_pool_fn_ = fn;
    lume_pool_args_ = (char*)args;
    lume_pool_stride_ = stride;
    lume_pool_ntasks_ = ntasks;
    lume_pool_next_ = 0;
    lume_pool_active_ = active;
    lume_pool_busy_ = active;
    lume_pool_generation_++;
    for (int i = 0; i < active; i++) {
        LUME_COND_SIGNAL(lume_pool_conds_[i]);
    }
    LUME_MUTEX_UNLOCK(lume_pool_mutex_);

    /* 主线程也抢任务 */
    for (;;) {
        int t = (int)LUME_NEXT_SLOT(&lume_pool_next_);
        if (t >= ntasks) break;
        lume_pool_call_(fn, (char*)args + (size_t)t * stride);
    }
    lume_log_flush_thread();

    LUME_MUTEX_LOCK(lume_pool_mutex_);
    while (lume_pool_busy_ > 0) {
        LUME_COND_WAIT(lume_pool_done_, lume_pool_mutex_);
    }
    LUME_MUTEX_UNLOCK(lume_pool_mutex_);
}

/* ── 优化提示 ── */

/* 少于这个元素数，par for 直接串行跑、不进线程池。
   线程池把固定成本从 ~100 µs 降到几 µs，但一次广播 + 唤醒
   仍然要和几十个元素的加法比。实测（8 核）：
     1 万元素求和 串行 1.3 µs，线程池 ~8 µs；10 万元素 14 µs vs ~14 µs。
   阈值取 32768：轻循环到这里才开始不亏，重循环（sqrt 等）也够长。 */
#define LUME_PAR_SERIAL_THRESHOLD 32768

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

/* ── 数组下标检查 ── */
/* a[i] 在 C 里是裸的指针运算：越界读拿到垃圾值、越界写静默踩内存，
   两边都不报错。和切片/除零一样，崩得明明白白好过静默算错。
   返回 i 本身，所以能直接塞进下标位置，读写两条路径都覆盖
   （赋值语句也是走 gen_expr(target) 生成的）。
   长度只在编译期能确定时才插检查：局部字面量、数组参数、切片。
   函数返回值/struct 字段数组拿不到长度，仍然不检查。
   热点循环里 clang 通常能证明 i < len 从而消掉这个分支；
   消不掉又不想付代价的，用 @unchecked 或 --unchecked。 */
static inline int64_t lume_idx_check(int64_t i, int64_t len) {
    /* 一次比较同时挡负数：转成无符号后负数必然 >= len */
    if ((uint64_t)i >= (uint64_t)len) {
        fprintf(stderr,
                "lume: runtime error: index %lld out of range (len=%lld)\n",
                (long long)i, (long long)len);
        abort();
    }
    return i;
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

/* ── 字符串 arena ──
   Lume 的 str 就是 const char*：零开销，能直接交给 printf / libc。

   难点在「谁负责释放」。Lume 现在没有所有权检查也没有 GC，
   逐个 free 不可能做对 —— 字符串会被塞进结构体、数组、跨函数
   传递、当成 map 的键，精确跟踪所有权的成本远超收益。

   所以用 arena：所有派生字符串（拼接、upper、trim、切片……）
   都从一个只增不还的池子里切，整体在 lume_str_arena_reset() 时
   一次释放。这不是偷懒，是编译器的标准做法 —— clang 的
   ASTContext、rustc 的 TypedArena 都是这个模型：token、标识符、
   AST 里的字符串本来就活到编译结束，逐个回收既慢又容易出错。

   顺带解决两个老限制：
   - 过去 str_* 的返回值放在 8 个静态槽里轮转，连续调用 9 次
     就开始互相覆盖（词法器会被咬死）。现在任意多个字符串可以
     同时存活。
   - 过去结果上限 4095 字节，超了静默截断。现在只受内存限制。

   性能：一次 take 就是「加一下 used」，比 malloc 快一个量级。 */

#define LUME_ARENA_CHUNK ((size_t)1 << 20)  /* 1 MiB：够放一个小项目的全部 token */

typedef struct LumeArenaChunk {
    struct LumeArenaChunk* next;
    size_t cap;
    size_t used;
} LumeArenaChunk;

static LumeArenaChunk* lume_arena_cur = NULL;
static LUME_MUTEX_T lume_arena_mu = LUME_MUTEX_INIT;

/* 取一块 n 字节、16 字节对齐的空间。加锁是为了 par for 里的 worker
   线程也能安全派生字符串（锁只在 take 期间持有，粒度很小）。 */
static void* lume_arena_take(size_t n) {
    n = (n + 15) & ~(size_t)15;
    LUME_MUTEX_LOCK(lume_arena_mu);
    LumeArenaChunk* c = lume_arena_cur;
    if (!c || c->cap - c->used < n) {
        size_t cap = n > LUME_ARENA_CHUNK ? n : LUME_ARENA_CHUNK;
        LumeArenaChunk* nc =
            (LumeArenaChunk*)malloc(sizeof(LumeArenaChunk) + cap);
        if (!nc) {
            LUME_MUTEX_UNLOCK(lume_arena_mu);
            fflush(stdout);
            fputs("lume: 内存不足（字符串 arena）\n", stderr);
            abort();
        }
        nc->next = c;
        nc->cap = cap;
        nc->used = 0;
        lume_arena_cur = nc;
        c = nc;
    }
    char* p = (char*)c + sizeof(LumeArenaChunk) + c->used;
    c->used += n;
    LUME_MUTEX_UNLOCK(lume_arena_mu);
    return p;
}

/* 一次性释放整个 arena。适合「一段临时字符串用完就丢」的场景
   （比如解析完一个文件）。注意：释放后所有 str 变成悬垂指针。 */
static inline void lume_str_arena_reset(void) {
    LUME_MUTEX_LOCK(lume_arena_mu);
    LumeArenaChunk* c = lume_arena_cur;
    while (c) {
        LumeArenaChunk* nx = c->next;
        free(c);
        c = nx;
    }
    lume_arena_cur = NULL;
    LUME_MUTEX_UNLOCK(lume_arena_mu);
}

/* 申请 n 字节可写缓冲（内部多留 1 字节给 '\0'，不自动置零） */
static inline char* lume_str_buf(int64_t n) {
    if (n < 0) {
        fflush(stdout);
        fputs("lume: 字符串长度为负\n", stderr);
        abort();
    }
    return (char*)lume_arena_take((size_t)n + 1);
}

/* ── std.str ── */

static inline int64_t lume_str_len(const char* s) {
    return (int64_t)strlen(s);
}

static inline const char* lume_str_upper(const char* s) {
    size_t n = strlen(s);
    char* buf = lume_str_buf((int64_t)n);
    for (size_t i = 0; i < n; i++) buf[i] = (char)toupper((unsigned char)s[i]);
    buf[n] = 0;
    return buf;
}

static inline const char* lume_str_lower(const char* s) {
    size_t n = strlen(s);
    char* buf = lume_str_buf((int64_t)n);
    for (size_t i = 0; i < n; i++) buf[i] = (char)tolower((unsigned char)s[i]);
    buf[n] = 0;
    return buf;
}

static inline const char* lume_str_trim(const char* s) {
    size_t n = strlen(s);
    size_t start = 0;
    while (start < n && isspace((unsigned char)s[start])) start++;
    size_t end = n;
    while (end > start && isspace((unsigned char)s[end - 1])) end--;
    size_t len = end - start;
    char* buf = lume_str_buf((int64_t)len);
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

/* replace：先算精确长度再分配，不再有 4095 截断。 */
static inline const char* lume_str_replace(
    const char* s, const char* old_s, const char* new_s
) {
    size_t old_len = strlen(old_s);
    size_t new_len = strlen(new_s);
    size_t src_len = strlen(s);
    if (old_len == 0) {
        /* 空 old 会让匹配位置永不前进 → 死循环，原样复制一份返回 */
        char* same = lume_str_buf((int64_t)src_len);
        memcpy(same, s, src_len + 1);
        return same;
    }
    /* 第一趟只数命中次数。hits * old_len 恒 <= src_len，不会下溢 */
    size_t hits = 0;
    for (const char* p = s; (p = strstr(p, old_s)) != 0; p += old_len) hits++;
    size_t total = src_len - hits * old_len + hits * new_len;
    char* buf = lume_str_buf((int64_t)total);
    /* 第二趟真正拷贝 */
    size_t pos = 0;
    const char* p = s;
    for (;;) {
        const char* hit = strstr(p, old_s);
        size_t seg = hit ? (size_t)(hit - p) : strlen(p);
        memcpy(buf + pos, p, seg);
        pos += seg;
        if (!hit) break;
        memcpy(buf + pos, new_s, new_len);
        pos += new_len;
        p = hit + old_len;
    }
    buf[pos] = 0;
    return buf;
}

/* ── 拼接与派生（`a + b` 的 codegen 目标）──
   两边都是 str 时 `Add` 落到这里。绝不能让 C 的 `+` 接住 ——
   那是指针算术，会得到一个偏移后的地址，静默错得离谱。 */
static inline const char* lume_str_concat(const char* a, const char* b) {
    size_t la = strlen(a), lb = strlen(b);
    char* buf = lume_str_buf((int64_t)(la + lb));
    memcpy(buf, a, la);
    memcpy(buf + la, b, lb + 1);
    return buf;
}

/* 切片 [start, end)。越界自动夹到 [0, len]，不 abort ——
   词法器经常按「可能偏一点」的下标取，夹边比崩溃有用。 */
static inline const char* lume_str_slice(
    const char* s, int64_t start, int64_t end
) {
    int64_t n = (int64_t)strlen(s);
    if (start < 0) start = 0;
    if (start > n) start = n;
    if (end < start) end = start;
    if (end > n) end = n;
    size_t len = (size_t)(end - start);
    char* buf = lume_str_buf((int64_t)len);
    memcpy(buf, s + start, len);
    buf[len] = 0;
    return buf;
}

/* 子串首次出现的位置，找不到返回 -1。按字节算（str 是 UTF-8 字节
   序列，和 str_len / 切片的语义一致）。 */
static inline int64_t lume_str_index_of(const char* s, const char* sub) {
    const char* hit = strstr(s, sub);
    return hit ? (int64_t)(hit - s) : -1;
}

/* 重复 times 次。生成缩进、对齐填充都用它。 */
static inline const char* lume_str_repeat(const char* s, int64_t times) {
    size_t n = strlen(s);
    if (times < 0) times = 0;
    size_t total = n * (size_t)times;
    char* buf = lume_str_buf((int64_t)total);
    for (int64_t i = 0; i < times; i++) memcpy(buf + n * (size_t)i, s, n);
    buf[total] = 0;
    return buf;
}

/* 整数 → 字符串。自举时生成 C 代码全靠它（满屏的下标和常量）。 */
static inline const char* lume_int_str(int64_t v) {
    char tmp[32];
    int i = 31;
    tmp[i] = 0;
    int neg = v < 0;
    /* 用无符号取绝对值，避开 INT64_MIN 的 `-v` 溢出 UB */
    uint64_t u = neg ? (uint64_t)0 - (uint64_t)v : (uint64_t)v;
    if (u == 0) tmp[--i] = '0';
    while (u > 0) { tmp[--i] = (char)('0' + (int)(u % 10)); u /= 10; }
    if (neg) tmp[--i] = '-';
    size_t len = (size_t)(31 - i);
    char* buf = lume_str_buf((int64_t)len);
    memcpy(buf, tmp + i, len + 1);
    return buf;
}

/* 浮点 → 字符串。用 %g（6 位有效数字），和 say 的默认观感一致。
   负数、科学计数法都交给 libc，别自己写。 */
static inline const char* lume_float_str(double v) {
    char tmp[64];
    int n = snprintf(tmp, sizeof(tmp), "%g", v);
    if (n < 0) n = 0;
    if (n > (int)sizeof(tmp) - 1) n = (int)sizeof(tmp) - 1;
    char* buf = lume_str_buf((int64_t)n);
    memcpy(buf, tmp, (size_t)n + 1);
    return buf;
}

/* split：返回字符串数组（段指针）+ 段数出参。
   段内容顺序放在一个 4096 的缓冲里，指针指进去；
   整套（指针数组 + 字符缓冲）按槽位轮转，连续调用不互相覆盖。 */
/* split：按 sep 切成若干段，段数通过 out_len 带出。
   段一律拷进 arena —— 和原文脱钩，原文被释放后分割结果仍然有效。
   段数不再有 64 的上限。 */
static inline const char** lume_str_split(
    const char* s, const char* sep, int64_t* out_len
) {
    size_t src_len = strlen(s);
    size_t sep_len = strlen(sep);
    size_t step = sep_len == 0 ? 1 : sep_len;

    /* 第一趟：数段数。空串算一段（一个空段）。 */
    int64_t count = 1;
    if (sep_len == 0) {
        count = src_len == 0 ? 1 : (int64_t)src_len;
    } else {
        const char* p = s;
        while ((p = strstr(p, sep)) != 0) { count++; p += step; }
    }

    const char** out =
        (const char**)lume_arena_take(sizeof(const char*) * (size_t)count);
    /* 池子 = 所有段字符 + 每段一个 NUL，再多留 1 字节余量 */
    char* pool = (char*)lume_arena_take(src_len + (size_t)count + 1);

    int64_t k = 0;
    size_t pos = 0;
    if (sep_len == 0) {
        /* 空分隔符 = 按字节切成单字符段 */
        for (size_t i = 0; i < src_len; i++) {
            pool[pos] = s[i];
            pool[pos + 1] = 0;
            out[k++] = pool + pos;
            pos += 2;
        }
        if (k == 0) { pool[0] = 0; out[k++] = pool; }
    } else {
        const char* p = s;
        for (;;) {
            const char* hit = strstr(p, sep);
            size_t seg = hit ? (size_t)(hit - p) : strlen(p);
            memcpy(pool + pos, p, seg);
            pool[pos + seg] = 0;
            out[k++] = pool + pos;
            pos += seg + 1;
            if (!hit) break;
            p = hit + step;
        }
    }
    *out_len = k;
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
"#;