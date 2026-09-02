use std::env;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// 单行请求的最大字节数（1MB）。超过则拒绝并断开连接，
/// 防止恶意/异常客户端发送无换行超长数据导致内存无限增长。
const MAX_LINE_BYTES: usize = 1 * 1024 * 1024;
/// 空闲连接读超时：超时时间内没有收到任何数据则断开，
/// 防止空闲客户端的线程永远阻塞、线程数无限增长。
const READ_TIMEOUT: Duration = Duration::from_secs(600);

// ----- 依赖成员B的协议模块（必须存在） -----
use kvstore::protocol;
// ----- 成员A的真实存储 -----
use anyhow::{anyhow, Result};
use kvstore::storage::KvStore;

// ----- 线程安全的共享存储句柄 -----
type SharedKvStore = Arc<Mutex<KvStore>>;

// ----- 主函数 -----
fn main() -> Result<()> {
    let (port, data_path) = parse_args()?;

    // 打开存储：文件存在则重放日志恢复，不存在则新建空库
    let kv = Arc::new(Mutex::new(KvStore::open(&data_path)?));

    let clients = Arc::new(AtomicUsize::new(0));

    let listener = TcpListener::bind(format!("0.0.0.0:{}", port))?;
    println!(
        "===== KV Store Server =====\n\
         监听地址:  0.0.0.0:{}\n\
         数据文件:  {}\n\
         数据条数:  {}\n\
         按 Ctrl+C 停止服务",
        port,
        data_path,
        kv.lock().unwrap_or_else(|p| p.into_inner()).len()
    );

    // 接受连接循环
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let kv = Arc::clone(&kv);
                let clients = Arc::clone(&clients);
                clients.fetch_add(1, SeqCst);

                thread::spawn(move || {
                    // 捕获 panic，单个连接崩溃不影响服务
                    // 注意：闭包会 move 变量，所以给闭包克隆一份 clients
                    let clients_inner = Arc::clone(&clients);
                    let _ = std::panic::catch_unwind(move || {
                        handle_client(stream, kv, clients_inner);
                    });
                    // 客户端线程结束，减少计数
                    clients.fetch_sub(1, SeqCst);
                });
            }
            Err(e) => eprintln!("(warn) accept 失败: {}", e),
        }
    }

    Ok(())
}

// ----- 解析命令行参数 -----
fn parse_args() -> Result<(u16, String)> {
    let args: Vec<String> = env::args().collect();
    let mut port = 6379;
    let mut data_path = "./kv.db".to_string();

    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--port" => {
                if let Some(p) = iter.next() {
                    port = p.parse().map_err(|_| anyhow!("无效端口"))?;
                } else {
                    return Err(anyhow!("--port 需要参数"));
                }
            }
            "--data" => {
                if let Some(p) = iter.next() {
                    data_path = p.clone();
                } else {
                    return Err(anyhow!("--data 需要参数"));
                }
            }
            "--help" | "-h" => {
                println!(
                    "用法: {} [--port PORT] [--data DATA_PATH]\n\
                     默认端口: 6379\n\
                     默认数据文件: ./kv.db",
                    args[0]
                );
                std::process::exit(0);
            }
            _ => {
                return Err(anyhow!("未知参数: {}, 使用 --help 查看用法", arg));
            }
        }
    }

    Ok((port, data_path))
}

// ----- 处理单个客户端连接 -----
fn handle_client(stream: TcpStream, kv: SharedKvStore, clients: Arc<AtomicUsize>) {
    // std 的 TcpStream 没有 split()（那是 tokio 的 API），
    // 拆分读写用 try_clone()：reader 拿一份克隆，writer 直接用本体
    let read_stream = match stream.try_clone() {
        Ok(r) => r,
        Err(_) => return, // 克隆失败，放弃此连接
    };
    // 空闲超时设置在读句柄上（读超时只影响 read 侧）
    let _ = read_stream.set_read_timeout(Some(READ_TIMEOUT));
    let mut reader = BufReader::new(read_stream);
    let mut writer = BufWriter::new(stream);
    let mut raw = Vec::new();

    loop {
        raw.clear();
        match read_line_limited(&mut reader, &mut raw, MAX_LINE_BYTES) {
            Ok(0) => break, // 客户端关闭
            Ok(_) => {
                // 字节转字符串（容错非 UTF-8）
                let line = String::from_utf8_lossy(&raw);
                // 去除结尾换行符
                let cmd_line = line.trim_end_matches('\n').trim_end_matches('\r');
                if cmd_line.is_empty() {
                    continue;
                }

                // 分发命令，获得响应
                let resp = match dispatch(cmd_line, &kv, &clients) {
                    Ok(r) => r,
                    Err(e) => protocol::resp_err(&e), // 协议层错误格式化
                };

                // 发送响应
                if writer.write_all(resp.as_bytes()).is_err() {
                    break; // 写入失败，断开连接
                }
                if writer.flush().is_err() {
                    break;
                }

                // quit 命令断开连接
                if cmd_line.trim().eq_ignore_ascii_case("quit") {
                    break;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::InvalidInput => {
                // 单行超过上限：回错误并断开。
                // 断开而非继续：超长行的剩余部分还堆在网络流里，
                // 继续读会污染后续命令的边界
                let _ = writer.write_all(
                    protocol::resp_err("请求超过 1MB 上限，连接已断开").as_bytes(),
                );
                let _ = writer.flush();
                break;
            }
            Err(_) => break, // 读错误 / 空闲超时：断开回收线程
        }
    }
}

/// 带最大长度限制的按行读取。
///
/// 不用 `BufRead::read_line`：它会把一整行（可能无限长）全部读进内存后才返回，
/// 无法在读取过程中止损。这里基于 `fill_buf`/`consume` 手动分段搬运：
/// 每段先检查累计长度，超限立即返回 `InvalidInput` 错误，内存占用严格有界。
/// 返回值：读到的字节数（0 = 对端关闭）。
fn read_line_limited(
    reader: &mut BufReader<TcpStream>,
    buf: &mut Vec<u8>,
    max_bytes: usize,
) -> io::Result<usize> {
    buf.clear();
    loop {
        // 查看内核/用户态缓冲区里已有的字节（不拷出）
        let available = match reader.fill_buf() {
            Ok(a) => a,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if available.is_empty() {
            // 对端关闭（EOF）
            return Ok(buf.len());
        }
        match available.iter().position(|&b| b == b'\n') {
            Some(pos) => {
                // 找到换行：连同换行符一起取出，本行结束
                buf.extend_from_slice(&available[..=pos]);
                reader.consume(pos + 1);
                if buf.len() > max_bytes {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "行超过长度上限",
                    ));
                }
                return Ok(buf.len());
            }
            None => {
                // 缓冲区里还没有换行：整段取出，继续等下一段
                buf.extend_from_slice(available);
                let n = available.len();
                reader.consume(n);
                if buf.len() > max_bytes {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "行超过长度上限",
                    ));
                }
            }
        }
    }
}

// ----- 命令分发函数（返回响应字符串） -----
fn dispatch(
    line: &str,
    kv: &SharedKvStore,
    clients: &Arc<AtomicUsize>,
) -> std::result::Result<String, String> {
    let (cmd, args) = protocol::parse_command(line)?;

    match cmd {
        "set" => {
            let key = args[0].to_string();
            let value = args[1..].join(" ");
            kv.lock()
                .unwrap_or_else(|p| p.into_inner()) // 锁中毒恢复：不因一次 panic 全服瘫痪
                .set(key, value)
                .map_err(|e| e.to_string())?;
            Ok(protocol::resp_ok(None))
        }
        "get" => {
            let value = kv.lock().unwrap_or_else(|p| p.into_inner()).get(args[0]);
            match value {
                Some(v) => Ok(protocol::resp_ok(Some(&v))),
                None => Ok(protocol::resp_notfound()),
            }
        }
        "del" => {
            let existed = kv
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .del(args[0])
                .map_err(|e| e.to_string())?;
            if existed {
                Ok(protocol::resp_ok(None))
            } else {
                Ok(protocol::resp_notfound())
            }
        }
        "keys" => {
            let keys = kv.lock().unwrap_or_else(|p| p.into_inner()).keys();
            Ok(protocol::resp_keys(&keys))
        }
        "clear" => {
            let removed = kv
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clear()
                .map_err(|e| e.to_string())?;
            Ok(protocol::resp_ok(Some(&format!("已清空 {} 条数据", removed))))
        }
        "compact" => {
            let (before, after) = kv
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .compact()
                .map_err(|e| e.to_string())?;
            Ok(protocol::resp_ok(Some(&format!(
                "日志压缩完成：{}B → {}B",
                before, after
            ))))
        }
        "status" => {
            let n = kv.lock().unwrap_or_else(|p| p.into_inner()).len();
            let c = clients.load(SeqCst);
            Ok(protocol::resp_status(n, c))
        }
        "ping" => Ok(protocol::resp_pong()),
        "quit" => Ok(protocol::resp_ok(Some("再见"))),
        // parse_command 理论上已过滤未知命令，这里兜底防御
        _ => Err(format!("未知命令: {}", cmd)),
    }
}
