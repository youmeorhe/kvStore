# Rust 可持久化键值存储系统（KV Store）

> 河海大学「高级系统编程技术」课程设计 · 两天冲刺 MVP 版

## 功能

- 内存键值存储（`BTreeMap`）
- **Append-Only 行日志持久化**：重启不丢数据；损坏文件明确报错，不静默覆盖
- TCP 服务器，多客户端并发连接（thread-per-connection + `Arc<Mutex>`）
- 命令行客户端，支持：`set` / `get` / `del` / `keys` / `status` / `ping` / `quit` / `help`

## 构建

要求已安装 Rust 工具链（rustup + cargo）：

```bash
cargo build --release
# 产物：
#   target/release/server (.exe on Windows)
#   target/release/client (.exe on Windows)
```

## 运行

### 终端 1：启动服务器

```bash
# 默认端口 6379，数据文件 ./kv.db
cargo run --bin server

# 自定义：
cargo run --bin server -- --port 6380 --data ./my-kv.db

# 查看帮助：
cargo run --bin server -- --help
```

启动后会打印：
```
===== KV Store Server =====
监听地址:  0.0.0.0:6379
数据文件:  ./kv.db
数据条数:  0
按 Ctrl+C 停止服务
```

### 终端 2：启动客户端

```bash
# 默认连 127.0.0.1:6379
cargo run --bin client

# 连远端：
cargo run --bin client -- 192.168.1.10:6379
```

### 命令示例

```
> set 课程名称 Rust程序设计
(OK)
> get 课程名称
(OK) Rust程序设计
> keys
  [1] 课程名称
> status
数据条数: 1
活跃连接: 1
> del 课程名称
(OK)
> get 课程名称
(提示) 键不存在
> ping
PONG
> help
（打印帮助文本）
> quit
```

## 测试

```bash
# 单元测试
cargo test --lib

# 端到端集成测试（需要先 cargo build 出 server 可执行文件）
cargo build
cargo test --test integration -- --test-threads=1
```

## 团队分工

见仓库根目录上一层的 [任务分工.md](../任务分工.md)。

- 成员 A：`src/storage.rs`
- 成员 B：`src/protocol.rs` + `src/bin/client.rs`
- 成员 C：`src/bin/server.rs`
- 成员 D：测试、工程化、README、demo 脚本、报告
