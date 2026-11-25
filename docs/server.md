# 小型服务器：Socket 最小闭环

Lume 的 socket 路径先走已完成的动态 FFI：C ABI 垫片负责平台差异，Lume 侧用
manifest 的 `[ffi.extern]` 声明调用，解释器通过 libffi 做动态分派。这样不需要
在 JIT 的 `i64` 值 ABI 里塞入 `sockaddr_in`，也不绕过 buffer 的生命周期契约。

## 运行 echo 示例

```bash
cargo build -p lumec_core
target/debug/lume-bootstrap check examples/server_echo/manifest.toml
target/debug/lume-bootstrap header examples/server_echo/manifest.toml
target/debug/lume-bootstrap run examples/server_echo/manifest.toml
bash examples/server_echo/test_echo.sh
```

服务默认监听 `0.0.0.0:18080`。`test_echo.sh` 会启动服务，先跑单客户端
`hello lume` 回环，再跑多客户端并发用例（两个连接同时打开，验证第二个客户端
能在第一个仍存活时被服务），最后等待服务在空闲超时后退出。

## 文件

* `examples/server_echo/src/net.c`：跨平台 C ABI 垫片。
* `examples/server_echo/src/main.lume`：Lume echo server 与客户端处理。
* `examples/server_echo/manifest.toml`：C 模块、extern 签名与导出符号。
* `examples/server_echo/test_echo.c`：单客户端回环验证。
* `examples/server_echo/test_echo_multi.c`：多客户端并发回环验证。
* `examples/server_echo/test_echo.sh`：编译并运行上述两个客户端。

## C ABI

所有 fd 和长度统一用 `i64`，避免 Windows 上 `SOCKET` 句柄被截断。

| 函数 | 签名 | 说明 |
| --- | --- | --- |
| `lume_net_listen` | `i64(i64)` | 创建并监听 IPv4 端口，返回 fd 或 `-1` |
| `lume_net_accept` | `i64(i64, i64)` | 带超时接受连接，返回 fd、`-1` 错误、`-2` 超时 |
| `lume_net_recv` | `i64(i64, pointer, i64, i64)` | 从连接读取，返回字节数、`0` 关闭、`-1` 错误、`-2` 超时 |
| `lume_net_send` | `i64(i64, pointer, i64)` | 尽量发送完整字节数 |
| `lume_net_close` | `void(i64)` | 关闭 fd |
| `lume_net_last_error` | `void(pointer, i64)` | 把最近错误写入调用方提供的缓冲区 |
| `lume_net_pollset_clear` | `i64()` | 清空 poll 集合，把所有槽位设为无效 |
| `lume_net_pollset_set` | `i64(i64, i64)` | 把 fd 写入指定槽位（`0` 保留给监听 fd） |
| `lume_net_poll` | `i64(i64)` | 等待集合就绪，返回就绪数、`0` 超时、`-1` 错误 |
| `lume_net_pollset_ready` | `i64(i64)` | 查询槽位状态：`1` 可读、`0` 无事件、`-1` 错误/挂断 |

`pointer` 参数用 `ffi_buffer(size, alignment)` 提供本机内存。C 侧只在使用期间
持有该指针，不保留到调用返回之后。

## 多客户端事件循环

`main()` 用 `mono_ms()` 设置单调时钟空闲窗口，并把监听 fd 放在 poll 集合槽位
`0`、客户端放在 `1..63`。Lume 侧用一个 `ffi_buffer(512, 8)` 保存每个槽位对应的
fd；C 侧只维护平台相关的 `pollfd` 数组，不把 `pollfd` 作为按值聚合穿过 ABI：

```lume
idle_start:int = mono_ms()
while mono_ms() - idle_start < 10000
    ready:any = lume_net_poll(500)
    if ready > 0
        idle_start := mono_ms()
        i:int = 0
        while i < 64
            state:any = lume_net_pollset_ready(i)
            if state > 0
                if i is 0
                    accept_pending(server, slots)
                else
                    fd:any = ffi_buffer_read(slots, i * 8, "i64")
                    if fd >= 0
                        echo_ready_client(fd, i, slots, buffer)
                    end
                end
            end
            i := i + 1
        end
    end
end
```

`poll` 超时返回 `0`；`accept` / `recv` 在就绪后以 `timeout_ms = 0` 调用，不会阻塞。
连接关闭或出错时，Lume 把槽位 fd 写回 `-1` 并调用 `lume_net_pollset_set(i, -1)`，
C 侧把该槽位设为平台无效 fd，后续 `poll` 自动忽略。

## 当前范围

* 已支持：IPv4、单线程 `poll` 多客户端并发、按连接边界检查的 `ffi_buffer`、单调时钟空闲超时、跨平台 C 垫片。
* 尚未支持：TLS、IPv6、可配置端口/地址、写半关闭时的背压处理。
* JIT/AOT 阶段仍可切换到静态 extern；当前先保持一套动态 FFI 分派语义。
* 红线保持：`pollfd` / `sockaddr_in` 不穿透按值聚合 ABI；未引入 rawptr / unsafe，也未依赖锁原语。
