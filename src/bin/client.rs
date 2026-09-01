//! 命令行客户端（成员 B 负责实现）
//! 用法：
//!   cargo run --bin client                    # 默认连 127.0.0.1:6379
//!   cargo run --bin client -- 10.0.0.1:6379   # 连指定地址

#![allow(unused, dead_code)] // B 填代码阶段允许未使用的导入

use kvstore::protocol;
use anyhow::Result;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;

fn main() {
    // 顶层 main 里打印 Result 的错误，不让进程直接 Rust panic
    if let Err(e) = run() {
        eprintln!("(退出) {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    // TODO(B)：真实实现，步骤：
    // 1) 拿 args()，第一个非程序名的参数当 addr，默认 "127.0.0.1:6379"
    let addr = std::env::args().nth(1).unwrap_or_else(|| "127.0.0.1:6379".into());
    let _addr = addr; // 占位，B 实现时删掉

    // 2) TcpStream::connect(addr)
    //    失败 → println!("(错误) 连不上服务器: {e}"); return Ok(())
    todo!("client run() —— 成员 B 实现。步骤参考任务分工.md B 章节 client.rs")

    // 3) println!("> 已连接 {addr}，输入 help 查看帮助，quit 退出");
    // 4) stream.set_read_timeout / set_write_timeout 可选（不用也行）
    //    let reader_stream = stream.try_clone()?;
    //    let mut reader = BufReader::new(reader_stream);
    //    let mut writer = stream;
    //    let stdin = std::io::stdin();
    // 5) loop:
    //    print!("> "); io::stdout().flush()?;
    //    let mut input = String::new();
    //    match stdin.lock().read_line(&mut input) {
    //        Ok(0) => { println!("断开连接"); break; }  // Ctrl+D / EOF
    //        Ok(_) => {}
    //        Err(e) => return Err(e.into()),
    //    }
    //    let trimmed_cmd = input.trim();
    //    if trimmed_cmd.is_empty() { continue; }
    //    if matches!(trimmed_cmd, "help" | "?") {
    //        print!("{}", protocol::help_text());
    //        continue;
    //    }
    //    // 发给 server（原输入保留末尾换行，因为 server 用 read_line）
    //    writer.write_all(input.as_bytes())?;
    //    writer.flush()?;
    //    let mut resp = String::new();
    //    match reader.read_line(&mut resp) {
    //        Ok(0) => { println!("(提示) 服务器已断开连接"); break; }
    //        Ok(_) => {}
    //        Err(e) => return Err(e.into()),
    //    }
    //    let pretty = protocol::format_response_for_human(resp.trim_end_matches(&['\n','\r'][..]));
    //    println!("{pretty}");
    //    // 如果输入是 quit，处理完响应就退出
    //    if protocol::parse_command(trimmed_cmd).map(|(c,_)| c == "quit").unwrap_or(false) {
    //        break;
    //    }
}
