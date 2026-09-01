//! 服务器入口（成员 C 负责实现）
//! 用法：
//!   cargo run --bin server
//!   cargo run --bin server -- --port 6380 --data ./my.db
//!   cargo run --bin server -- --help

#![allow(unused, dead_code)] // C 填代码阶段允许未使用的导入

use kvstore::{protocol, KvError, Result, SharedKvStore};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::Arc;

// =====================================================================
// A 实现完 KvStore 后，C 取消下面这行的注释；在自测阶段先写 DummyKv
// use kvstore::storage::KvStore;
// =====================================================================

fn main() {
    if let Err(e) = run() {
        eprintln!("(fatal) {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    // TODO(C)：第一步，解析 args（极简手写，不用 clap）
    // - --port N     → 端口，默认 6379
    // - --data PATH  → 数据文件路径，默认 "./kv.db"
    // - --help / -h  → 打印用法 return Ok(())
    let port: u16 = 6379;          // TODO(C): 从 args 读
    let data_path: String = "./kv.db".into();  // TODO(C): 从 args 读
    print_usage_if_help();                   // TODO(C): 如果有 --help 先打印再 return

    // TODO(C)：第二步，打开存储：
    //     A 好之后：
    //         let kv: SharedKvStore = Arc::new(std::sync::Mutex::new(KvStore::open(&data_path)?));
    //     A 没好之前自测：用文件底部的 DummyKv
    //         let kv: Arc<std::sync::Mutex<DummyKv>> = Arc::new(std::sync::Mutex::new(DummyKv::open(&data_path)?));
    //         然后把 dispatch 里的类型改一下也能跑
    let _ = (port, data_path); // 占位，C 删
    todo!("server run() —— 成员 C 实现。步骤参考任务分工.md C 章节 & 本文件顶部注释");

    /* TODO(C)：第三步，主线程 accept 循环骨架，直接抄这段：

    let clients = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind(format!("0.0.0.0:{port}"))?;
    {
        let g = kv.lock().unwrap();
        println!("===== KV Store Server =====");
        println!("监听地址:  0.0.0.0:{port}");
        println!("数据文件:  {data_path}");
        println!("数据条数:  {}", g.len());
        println!("按 Ctrl+C 停止服务");
    }

    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let kv = Arc::clone(&kv);
                let c = Arc::clone(&clients);
                c.fetch_add(1, SeqCst);
                std::thread::spawn(move || {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let _ = handle_client(s, kv, c.clone());
                    }));
                    c.fetch_sub(1, SeqCst);
                });
            }
            Err(e) => eprintln!("(warn) accept 失败: {e}"),
        }
    }
    Ok(())
    */
}

// —— 帮助（C 填） ——
fn print_usage_if_help() {
    // 扫 args().any(|a| a == "--help" || a == "-h") 打印：
    //   用法：server [--port N] [--data PATH]
    //     --port N      监听端口，默认 6379
    //     --data PATH   持久化数据文件路径，默认 ./kv.db
    //     --help, -h    打印本帮助
}

// —— 单连接处理循环（C 填） ——
#[allow(dead_code)]
fn handle_client(stream: TcpStream, kv: SharedKvStore, clients: Arc<AtomicUsize>) -> Result<()> {
    // let mut reader = BufReader::new(stream.try_clone()?);
    // let mut writer = stream;
    // let mut line = String::new();
    // loop {
    //     line.clear();
    //     match reader.read_line(&mut line) {
    //         Ok(0) => break,  // EOF：客户端关了
    //         Ok(_) => {}
    //         Err(_) => break,
    //     }
    //     let trimmed = line.trim_end_matches(&['\n','\r'][..]);
    //     let is_quit = protocol::parse_command(trimmed)
    //         .map(|(c,_)| c == "quit")
    //         .unwrap_or(false);
    //     let resp = match dispatch(trimmed, &kv, &clients) {
    //         Ok(r) => r,
    //         Err(KvError(e)) => protocol::resp_err(&e),
    //     };
    //     // 发送：单个命令 write 失败就断连，不影响 server 主循环
    //     if writer.write_all(resp.as_bytes()).is_err() { break; }
    //     let _ = writer.flush();
    //     if is_quit { break; }
    // }
    let _ = (stream, kv, clients);
    unimplemented!("handle_client —— C 实现")
}

// —— 单条命令分发，返回一行响应字符串（C 填） ——
#[allow(dead_code)]
fn dispatch(line: &str, kv: &SharedKvStore, clients: &Arc<AtomicUsize>) -> std::result::Result<String, KvError> {
    // let (cmd, args) = protocol::parse_command(line)
    //     .map_err(KvError)?;
    // match cmd {
    //     "set" => {
    //         let key = args[0].to_string();
    //         let value = args[1..].join(" ");          // 多词拼接 value
    //         kv.lock().unwrap().set(key, value)?;      // 🔴 锁只在这一行持有
    //         Ok(protocol::resp_ok(None))
    //     }
    //     "get" => {
    //         let v = kv.lock().unwrap().get(args[0]);  // 🔴 锁立即释放
    //         Ok(match v.as_deref() {
    //             Some(s) => protocol::resp_ok(Some(s)),
    //             None    => protocol::resp_notfound(),
    //         })
    //     }
    //     "del" => {
    //         let existed = kv.lock().unwrap().del(args[0])?;
    //         Ok(if existed { protocol::resp_ok(None) } else { protocol::resp_notfound() })
    //     }
    //     "keys" => {
    //         let list = kv.lock().unwrap().keys();
    //         Ok(protocol::resp_keys(&list))
    //     }
    //     "status" => {
    //         let n = kv.lock().unwrap().len();
    //         let c = clients.load(SeqCst);
    //         Ok(protocol::resp_status(n, c))
    //     }
    //     "ping" => Ok(protocol::resp_pong()),
    //     "quit" => Ok(protocol::resp_ok(Some("再见"))),
    //     _    => unreachable!(),
    // }
    let _ = (line, kv, clients);
    unimplemented!("dispatch —— C 实现，范式写在上面注释里")
}

// =====================================================================
// 临时占位：A 的 KvStore 没好之前，C 用这个自测网络/并发功能。
// A 代码合并 main 那天，C 全局搜索替换：DummyKv → KvStore
// =====================================================================
#[allow(dead_code)]
pub struct DummyKv;

#[allow(dead_code)]
impl DummyKv {
    pub fn open(_path: &str) -> Result<Self> { Ok(Self) }
    pub fn set(&self, _k: String, _v: String) -> Result<()> { Ok(()) }
    pub fn del(&self, _k: &str) -> Result<bool> { Ok(true) }
    pub fn get(&self, k: &str) -> Option<String> { Some(format!("dummy_{k}")) }
    pub fn keys(&self) -> Vec<String> { vec!["a".into(), "b".into(), "c".into()] }
    pub fn len(&self) -> usize { 3 }
    pub fn is_empty(&self) -> bool { false }
}

// C 自测时在 run() 里先用：
//     let kv: Arc<Mutex<DummyKv>> = Arc::new(Mutex::new(DummyKv::open(&data_path)?));
// 并且把 handle_client / dispatch 的参数类型改成对应类型（A 好后再统一切回 SharedKvStore）
