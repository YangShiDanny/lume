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

服务默认监听 `0.0.0.0:18080`。`test_echo.sh` 会启动服务、用 C 客户端发送
`hello lume`、校验回显，然后等待服务在空闲超时后退出。

## 文件

* `examples/server_echo/src/net.c`：跨平台 C ABI 垫片。
* `examples/server_echo/src/main.lume`：Lume echo server 与客户端处理。
* `examples/server_echo/manifest.toml`：C 模块、extern 签名与导出符号。
* `examples/server_echo/test_echo.c`、`test_echo.sh`：自动化回环验证。

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

`pointer` 参数用 `ffi_buffer(size, alignment)` 提供本机内存。C 侧只在使用期间
持有该指针，不保留到调用返回之后。

## 空闲超时

`main()` 用 `mono_ms()` 设置单调时钟 deadline：

```lume
deadline:int = mono_ms() + 5000
while mono_ms() < deadline
    client:any = lume_net_accept(server, 500)
    if client >= 0
        echo_client(client)
        lume_net_close(client)
        deadline := mono_ms() + 5000
    end
end
```

`accept` / `recv` 的超时返回 `-2`，不会让 Lume 报错；真正错误才返回 `-1` 并写
`lume_net_last_error`。每个连接处理完都会关闭，服务端在最后关闭监听 fd。

## 当前范围

* 已支持：IPv4、单线程 accept/echo 循环、按连接边界检查的 `ffi_buffer`、单调时钟空闲超时、跨平台 C 垫片。
* 尚未支持：同时服务多个客户端、TLS、IPv6、可配置端口/地址、非阻塞 poll fd 集合。
* JIT/AOT 阶段仍可切换到静态 extern；当前先保持一套动态 FFI 分派语义。
