//! 协议解析 & 响应格式化模块
//!
//! 负责：
//! 1. 解析客户端发来的命令字符串
//! 2. 格式化服务端响应
//! 3. 客户端响应的人性化显示
// ============ 命令解析 ============
/// 解析一行命令，返回 (命令字, 参数列表)
///
/// # 返回
/// - `Ok((cmd, args))`: 解析成功，命令字全小写
/// - `Err(msg)`: 解析失败，返回错误信息（会发给客户端）
///
/// # 规则
/// - 空行/全空格 → `Err("空命令")`
/// - 命令字大小写不敏感
/// - `set` 至少 2 个参数 (key + value)，value 多词自动拼接
/// - `get`/`del` 恰好 1 个参数
/// - `keys`/`status`/`ping`/`quit` 0 个参数
pub fn parse_command(line: &str) -> Result<(&str, Vec<&str>), String> {
    let trimmed = line.trim();
    // 空行检查
    if trimmed.is_empty() {
        return Err("空命令".to_string());
    }
    // 按空格分割，保留空字符串（用于检测 "" 作为 key）
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.is_empty() {
        return Err("空命令".to_string());
    }
    let cmd = parts[0].to_lowercase();
    let args = &parts[1..];
    match cmd.as_str() {
        "set" => {
            if args.len() < 2 {
                return Err("set 命令需要至少 2 个参数: set <key> <value>".to_string());
            }
            let key = args[0];
            if key.is_empty() {
                return Err("key 不能为空".to_string());
            }
            // 所有参数作为 value，空格拼接
            Ok(("set", args.to_vec()))
        }
        "get" => {
            if args.len() != 1 {
                return Err("get 命令需要 1 个参数: get <key>".to_string());
            }
            let key = args[0];
            if key.is_empty() {
                return Err("key 不能为空".to_string());
            }
            Ok(("get", args.to_vec()))
        }
        "del" => {
            if args.len() != 1 {
                return Err("del 命令需要 1 个参数: del <key>".to_string());
            }
            let key = args[0];
            if key.is_empty() {
                return Err("key 不能为空".to_string());
            }
            Ok(("del", args.to_vec()))
        }
        "keys" => {
            if !args.is_empty() {
                return Err("keys 命令不需要参数".to_string());
            }
            Ok(("keys", vec![]))
        }
        "clear" => {
            if !args.is_empty() {
                return Err("clear 命令不需要参数".to_string());
            }
            Ok(("clear", vec![]))
        }
        "status" => {
            if !args.is_empty() {
                return Err("status 命令不需要参数".to_string());
            }
            Ok(("status", vec![]))
        }
        "ping" => {
            if !args.is_empty() {
                return Err("ping 命令不需要参数".to_string());
            }
            Ok(("ping", vec![]))
        }
        "quit" => {
            if !args.is_empty() {
                return Err("quit 命令不需要参数".to_string());
            }
            Ok(("quit", vec![]))
        }
        unknown => Err(format!("未知命令: {}", unknown)),
    }
}
// ============ 服务端响应格式化 ============
/// 返回 OK 响应，带可选内容
pub fn resp_ok(value: Option<&str>) -> String {
    match value {
        Some(v) => format!("OK {}\n", v),
        None => "OK\n".to_string(),
    }
}
/// 返回 NOTFOUND 响应
pub fn resp_notfound() -> String {
    "NOTFOUND\n".to_string()
}
/// 返回 ERR 响应
pub fn resp_err(msg: &str) -> String {
    format!("ERR {}\n", msg)
}
/// 返回 PONG 响应
pub fn resp_pong() -> String {
    "PONG\n".to_string()
}
/// 返回 STATUS 响应
pub fn resp_status(keys: usize, clients: usize) -> String {
    format!("STATUS keys={} clients={}\n", keys, clients)
}
/// 返回 keys 列表响应
pub fn resp_keys(keys: &[String]) -> String {
    if keys.is_empty() {
        return "OK\n".to_string();
    }
    format!("OK {}\n", keys.join(","))
}
// ============ 客户端人性化显示 ============
/// 将服务端响应行转换为用户友好文本
pub fn format_response_for_human(line: &str) -> String {
    let trimmed = line.trim();
    // 空行
    if trimmed.is_empty() {
        return "(提示) 收到空响应".to_string();
    }
    // 按第一个空格分割，判断类型
    if let Some((prefix, rest)) = trimmed.split_once(' ') {
        match prefix {
            "OK" => {
                if rest.is_empty() {
                    return "(成功) 操作成功".to_string();
                }
                // 检查是否是 keys 列表 (逗号分隔)
                if rest.contains(',') {
                    let keys: Vec<&str> = rest.split(',').collect();
                    let mut output = String::from("(成功) 共有以下键:\n");
                    for (i, key) in keys.iter().enumerate() {
                        output.push_str(&format!("  {}. {}\n", i + 1, key));
                    }
                    output
                } else {
                    // 普通值
                    format!("(成功) {}", rest)
                }
            }
            "NOTFOUND" => "(提示) 键不存在".to_string(),
            "ERR" => format!("(错误) {}", rest),
            "PONG" => "PONG".to_string(),
            "STATUS" => {
                // 解析 status 字段
                let mut keys_count = "?";
                let mut clients_count = "?";
                for part in rest.split_whitespace() {
                    if let Some((k, v)) = part.split_once('=') {
                        match k {
                            "keys" => keys_count = v,
                            "clients" => clients_count = v,
                            _ => {}
                        }
                    }
                }
                format!(
                    "=== 服务器状态 ===\n数据条数: {}\n活跃连接: {}",
                    keys_count, clients_count
                )
            }
            unknown => format!("(未知响应前缀: {}) {}", unknown, rest),
        }
    } else {
        // 无空格：按整词匹配（如 "OK"、"NOTFOUND"、"PONG"）
        match trimmed {
            "OK" => "(成功) 操作成功".to_string(),
            "NOTFOUND" => "(提示) 键不存在".to_string(),
            other => other.to_string(),
        }
    }
}
// ============ 测试 ============
#[cfg(test)]
mod tests {
    use super::*;
    // ====== parse_command 测试 ======
    #[test]
    fn test_parse_set() {
        // 正常 set
        let (cmd, args) = parse_command("set name Alice").unwrap();
        assert_eq!(cmd, "set");
        assert_eq!(args, vec!["name", "Alice"]);
        // set 多词 value（空格拼接）
        let (cmd, args) = parse_command("set greeting Hello World").unwrap();
        assert_eq!(cmd, "set");
        assert_eq!(args, vec!["greeting", "Hello", "World"]);
        // 大小写不敏感
        let (cmd, _) = parse_command("SET name Alice").unwrap();
        assert_eq!(cmd, "set");
        // 前后空格
        let (cmd, args) = parse_command("  set name Alice  ").unwrap();
        assert_eq!(cmd, "set");
        assert_eq!(args, vec!["name", "Alice"]);
        // 参数不足
        let err = parse_command("set name").unwrap_err();
        assert!(err.contains("需要至少 2 个参数"));
        // 只有命令字
        let err = parse_command("set").unwrap_err();
        assert!(err.contains("需要至少 2 个参数"));
    }
    #[test]
    fn test_parse_get() {
        // 正常 get
        let (cmd, args) = parse_command("get name").unwrap();
        assert_eq!(cmd, "get");
        assert_eq!(args, vec!["name"]);
        // 大小写
        let (cmd, _) = parse_command("GET name").unwrap();
        assert_eq!(cmd, "get");
        // 缺少参数
        let err = parse_command("get").unwrap_err();
        assert!(err.contains("需要 1 个参数"));
        // 参数过多
        let err = parse_command("get name age").unwrap_err();
        assert!(err.contains("需要 1 个参数"));
    }
    #[test]
    fn test_parse_del() {
        let (cmd, args) = parse_command("del name").unwrap();
        assert_eq!(cmd, "del");
        assert_eq!(args, vec!["name"]);
        // 参数不足
        let err = parse_command("del").unwrap_err();
        assert!(err.contains("需要 1 个参数"));
    }
    #[test]
    fn test_parse_keys() {
        let (cmd, args) = parse_command("keys").unwrap();
        assert_eq!(cmd, "keys");
        assert!(args.is_empty());
        // 大小写
        let (cmd, _) = parse_command("KEYS").unwrap();
        assert_eq!(cmd, "keys");
        // 带参数
        let err = parse_command("keys a").unwrap_err();
        assert!(err.contains("不需要参数"));
    }
    #[test]
    fn test_parse_clear() {
        let (cmd, args) = parse_command("clear").unwrap();
        assert_eq!(cmd, "clear");
        assert!(args.is_empty());
        // 大小写
        let (cmd, _) = parse_command("CLEAR").unwrap();
        assert_eq!(cmd, "clear");
        // 带参数
        let err = parse_command("clear all").unwrap_err();
        assert!(err.contains("不需要参数"));
    }
    #[test]
    fn test_parse_status() {
        let (cmd, args) = parse_command("status").unwrap();
        assert_eq!(cmd, "status");
        assert!(args.is_empty());
        // 带参数
        let err = parse_command("status all").unwrap_err();
        assert!(err.contains("不需要参数"));
    }
    #[test]
    fn test_parse_ping() {
        let (cmd, args) = parse_command("ping").unwrap();
        assert_eq!(cmd, "ping");
        assert!(args.is_empty());
        // 带参数
        let err = parse_command("ping hello").unwrap_err();
        assert!(err.contains("不需要参数"));
    }
    #[test]
    fn test_parse_quit() {
        let (cmd, args) = parse_command("quit").unwrap();
        assert_eq!(cmd, "quit");
        assert!(args.is_empty());
    }
    #[test]
    fn test_parse_empty() {
        let err = parse_command("").unwrap_err();
        assert!(err.contains("空命令"));

        let err = parse_command("   ").unwrap_err();
        assert!(err.contains("空命令"));
    }
    #[test]
    fn test_parse_unknown() {
        let err = parse_command("unknown").unwrap_err();
        assert!(err.contains("未知命令"));
    }
    // ====== resp_* 测试 ======
    #[test]
    fn test_resp_ok() {
        assert_eq!(resp_ok(None), "OK\n");
        assert_eq!(resp_ok(Some("hello")), "OK hello\n");
        assert_eq!(resp_ok(Some("hello world")), "OK hello world\n");
    }
    #[test]
    fn test_resp_notfound() {
        assert_eq!(resp_notfound(), "NOTFOUND\n");
    }
    #[test]
    fn test_resp_err() {
        assert_eq!(resp_err("something wrong"), "ERR something wrong\n");
    }
    #[test]
    fn test_resp_pong() {
        assert_eq!(resp_pong(), "PONG\n");
    }
    #[test]
    fn test_resp_status() {
        assert_eq!(resp_status(5, 2), "STATUS keys=5 clients=2\n");
        assert_eq!(resp_status(0, 1), "STATUS keys=0 clients=1\n");
    }
    #[test]
    fn test_resp_keys() {
        assert_eq!(resp_keys(&[]), "OK\n");
        assert_eq!(resp_keys(&["a".to_string()]), "OK a\n");
        assert_eq!(resp_keys(&["a".to_string(), "b".to_string()]), "OK a,b\n");
    }
    // ====== format_response_for_human 测试 ======
    #[test]
    fn test_format_ok() {
        assert_eq!(format_response_for_human("OK\n"), "(成功) 操作成功");
        assert_eq!(format_response_for_human("OK hello\n"), "(成功) hello");
        assert_eq!(
            format_response_for_human("OK hello world\n"),
            "(成功) hello world"
        );
    }
    #[test]
    fn test_format_ok_keys() {
        let result = format_response_for_human("OK a,b,c\n");
        assert!(result.contains("共有以下键"));
        assert!(result.contains("1. a"));
        assert!(result.contains("2. b"));
        assert!(result.contains("3. c"));
    }
    #[test]
    fn test_format_notfound() {
        assert_eq!(format_response_for_human("NOTFOUND\n"), "(提示) 键不存在");
    }
    #[test]
    fn test_format_err() {
        assert_eq!(
            format_response_for_human("ERR something wrong\n"),
            "(错误) something wrong"
        );
    }
    #[test]
    fn test_format_pong() {
        assert_eq!(format_response_for_human("PONG\n"), "PONG");
    }
    #[test]
    fn test_format_status() {
        let result = format_response_for_human("STATUS keys=5 clients=2\n");
        assert!(result.contains("数据条数: 5"));
        assert!(result.contains("活跃连接: 2"));
    }
    #[test]
    fn test_format_empty() {
        assert_eq!(format_response_for_human(""), "(提示) 收到空响应");
        assert_eq!(format_response_for_human("\n"), "(提示) 收到空响应");
    }
    #[test]
    fn test_format_unknown_prefix() {
        let result = format_response_for_human("UNKNOWN hello\n");
        assert!(result.contains("未知响应前缀"));
    }
}
