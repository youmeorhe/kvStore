use kvstore::protocol;
use std::env;
use std::io::{self, BufRead, BufReader, Write};
use std::net::TcpStream;
fn main() {
    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:6379".to_string());
    let stream = match TcpStream::connect(&addr) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("(错误) 连接服务器失败: {}", e);
            std::process::exit(1);
        }
    };
    println!("> 已连接 {}，输入 help 查看帮助,quit 退出", addr);
    println!();
    let mut writer = stream.try_clone().expect("克隆流失败");
    let mut reader = BufReader::new(stream);
    loop {
        print!("> ");
        io::stdout().flush().expect("刷新输出失败");
        let mut input = String::new();
        match io::stdin().read_line(&mut input) {
            Ok(0) => {
                println!("断开连接");
                break;
            }
            Ok(_) => {
                let trimmed = input.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if trimmed.eq_ignore_ascii_case("help") {
                    print_help();
                    continue;
                }
                if trimmed.eq_ignore_ascii_case("quit") {
                    if let Err(e) = writer.write_all(input.as_bytes()) {
                        eprintln!("(错误) 发送 quit 命令失败: {}", e);
                    }
                    if let Err(e) = writer.flush() {
                        eprintln!("(错误) 刷新输出失败: {}", e);
                    }
                    break;
                }
                if let Err(e) = writer.write_all(input.as_bytes()) {
                    eprintln!("(错误) 发送命令失败: {}", e);
                    eprintln!("(提示) 服务器已断开连接");
                    break;
                }
                if let Err(e) = writer.flush() {
                    eprintln!("(错误) 刷新输出失败: {}", e);
                    break;
                }
                let mut response = String::new();
                match reader.read_line(&mut response) {
                    Ok(0) => {
                        println!("(提示) 服务器已断开连接");
                        break;
                    }
                    Ok(_) => {
                        let human = protocol::format_response_for_human(&response);
                        println!("{}", human.trim_end_matches('\n'));
                    }
                    Err(e) => {
                        eprintln!("(错误) 读取响应失败: {}", e);
                        break;
                    }
                }
            }
            Err(e) => {
                eprintln!("(错误) 读取输入失败: {}", e);
                break;
            }
        }
    }
}
fn print_help() {
    println!(
        r#"可用命令：
  set <key> <value...>   写入/覆盖键值对
  get <key>              查询键对应的值
  del <key>              删除键
  setex <key> <seconds> <value...> 写入并设置过期秒数
  expire <key> <seconds> 给已存在的键设置过期秒数
  ttl <key>              查询键的剩余生存时间（秒）
  keys                   列出所有键
  clear                  清空所有数据
  compact                手动压缩日志文件
  status                 查看服务器状态
  ping                   心跳测试
  quit                   退出客户端
  help                   显示此帮助信息"#
    );
    println!();
}
