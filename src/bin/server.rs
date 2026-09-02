use std::env;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::thread;

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
    let reader = match stream.try_clone() {
        Ok(r) => BufReader::new(r),
        Err(_) => return, // 克隆失败，放弃此连接
    };
    let mut reader = reader;
    let mut writer = BufWriter::new(stream);
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break, // 客户端关闭
            Ok(_) => {
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
            Err(_) => break,
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
