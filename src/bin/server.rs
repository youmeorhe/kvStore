use std::env;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const MAX_LINE_BYTES: usize = 1 * 1024 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(600);

use kvstore::protocol;
use anyhow::{anyhow, Result};
use kvstore::storage::KvStore;

type SharedKvStore = Arc<Mutex<KvStore>>;

fn main() -> Result<()> {
    let (port, data_path) = parse_args()?;

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

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let kv = Arc::clone(&kv);
                let clients = Arc::clone(&clients);
                clients.fetch_add(1, SeqCst);

                thread::spawn(move || {
                    let clients_inner = Arc::clone(&clients);
                    let _ = std::panic::catch_unwind(move || {
                        handle_client(stream, kv, clients_inner);
                    });
                    clients.fetch_sub(1, SeqCst);
                });
            }
            Err(e) => eprintln!("(warn) accept 失败: {}", e),
        }
    }

    Ok(())
}

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

fn handle_client(stream: TcpStream, kv: SharedKvStore, clients: Arc<AtomicUsize>) {
    let read_stream = match stream.try_clone() {
        Ok(r) => r,
        Err(_) => return,
    };
    let _ = read_stream.set_read_timeout(Some(READ_TIMEOUT));
    let mut reader = BufReader::new(read_stream);
    let mut writer = BufWriter::new(stream);
    let mut raw = Vec::new();

    loop {
        raw.clear();
        match read_line_limited(&mut reader, &mut raw, MAX_LINE_BYTES) {
            Ok(0) => break,
            Ok(_) => {
                let line = String::from_utf8_lossy(&raw);
                let cmd_line = line.trim_end_matches('\n').trim_end_matches('\r');
                if cmd_line.is_empty() {
                    continue;
                }

                let resp = match dispatch(cmd_line, &kv, &clients) {
                    Ok(r) => r,
                    Err(e) => protocol::resp_err(&e),
                };

                if writer.write_all(resp.as_bytes()).is_err() {
                    break;
                }
                if writer.flush().is_err() {
                    break;
                }

                if cmd_line.trim().eq_ignore_ascii_case("quit") {
                    break;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::InvalidInput => {
                let _ = writer.write_all(
                    protocol::resp_err("请求超过 1MB 上限，连接已断开").as_bytes(),
                );
                let _ = writer.flush();
                break;
            }
            Err(_) => break,
        }
    }
}

fn read_line_limited(
    reader: &mut BufReader<TcpStream>,
    buf: &mut Vec<u8>,
    max_bytes: usize,
) -> io::Result<usize> {
    buf.clear();
    loop {
        let available = match reader.fill_buf() {
            Ok(a) => a,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if available.is_empty() {
            return Ok(buf.len());
        }
        match available.iter().position(|&b| b == b'\n') {
            Some(pos) => {
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
                .unwrap_or_else(|p| p.into_inner())
                .set(key, value)
                .map_err(|e| e.to_string())?;
            Ok(protocol::resp_ok(None))
        }
        "get" => {
            let mut guard = kv.lock().unwrap_or_else(|p| p.into_inner());
            match guard.get(args[0]) {
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
        "setex" => {
            let secs: u64 = args[1].parse().unwrap();
            let key = args[0].to_string();
            let value = args[2..].join(" ");
            kv.lock()
                .unwrap_or_else(|p| p.into_inner())
                .setex(key, secs, value)
                .map_err(|e| e.to_string())?;
            Ok(protocol::resp_ok(Some(&format!(
                "已设置，{} 秒后过期",
                secs
            ))))
        }
        "expire" => {
            let secs: u64 = args[1].parse().unwrap();
            let ok = kv
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .expire(args[0], secs)
                .map_err(|e| e.to_string())?;
            if ok {
                Ok(protocol::resp_ok(Some(&format!(
                    "已设置，{} 秒后过期",
                    secs
                ))))
            } else {
                Ok(protocol::resp_notfound())
            }
        }
        "ttl" => {
            let t = kv
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .ttl(args[0])
                .unwrap_or(-2);
            Ok(protocol::resp_ok(Some(&t.to_string())))
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
        _ => Err(format!("未知命令: {}", cmd)),
    }
}
