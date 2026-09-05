pub fn parse_command(line: &str) -> Result<(&str, Vec<&str>), String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Err("空命令".to_string());
    }
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
        "setex" => {
            if args.len() < 3 {
                return Err(
                    "setex 命令需要至少 3 个参数: setex <key> <seconds> <value>".to_string(),
                );
            }
            let key = args[0];
            if key.is_empty() {
                return Err("key 不能为空".to_string());
            }
            if args[1].parse::<u64>().is_err() {
                return Err(format!(
                    "seconds 必须是正整数（秒），实际为 '{}'",
                    args[1]
                ));
            }
            Ok(("setex", args.to_vec()))
        }
        "expire" => {
            if args.len() != 2 {
                return Err("expire 命令需要 2 个参数: expire <key> <seconds>".to_string());
            }
            let key = args[0];
            if key.is_empty() {
                return Err("key 不能为空".to_string());
            }
            if args[1].parse::<u64>().is_err() {
                return Err(format!(
                    "seconds 必须是正整数（秒），实际为 '{}'",
                    args[1]
                ));
            }
            Ok(("expire", args.to_vec()))
        }
        "ttl" => {
            if args.len() != 1 {
                return Err("ttl 命令需要 1 个参数: ttl <key>".to_string());
            }
            let key = args[0];
            if key.is_empty() {
                return Err("key 不能为空".to_string());
            }
            Ok(("ttl", args.to_vec()))
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
        "compact" => {
            if !args.is_empty() {
                return Err("compact 命令不需要参数".to_string());
            }
            Ok(("compact", vec![]))
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

pub fn resp_ok(value: Option<&str>) -> String {
    match value {
        Some(v) => format!("OK {}\n", v),
        None => "OK\n".to_string(),
    }
}
pub fn resp_notfound() -> String {
    "NOTFOUND\n".to_string()
}
pub fn resp_err(msg: &str) -> String {
    format!("ERR {}\n", msg)
}
pub fn resp_pong() -> String {
    "PONG\n".to_string()
}
pub fn resp_status(keys: usize, clients: usize) -> String {
    format!("STATUS keys={} clients={}\n", keys, clients)
}
pub fn resp_keys(keys: &[String]) -> String {
    if keys.is_empty() {
        return "OK\n".to_string();
    }
    format!("OK {}\n", keys.join(","))
}

pub fn format_response_for_human(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return "(提示) 收到空响应".to_string();
    }
    if let Some((prefix, rest)) = trimmed.split_once(' ') {
        match prefix {
            "OK" => {
                if rest.is_empty() {
                    return "(成功) 操作成功".to_string();
                }
                if rest.contains(',') {
                    let keys: Vec<&str> = rest.split(',').collect();
                    let mut output = String::from("(成功) 共有以下键:\n");
                    for (i, key) in keys.iter().enumerate() {
                        output.push_str(&format!("  {}. {}\n", i + 1, key));
                    }
                    output
                } else {
                    format!("(成功) {}", rest)
                }
            }
            "NOTFOUND" => "(提示) 键不存在".to_string(),
            "ERR" => format!("(错误) {}", rest),
            "PONG" => "PONG".to_string(),
            "STATUS" => {
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
        match trimmed {
            "OK" => "(成功) 操作成功".to_string(),
            "NOTFOUND" => "(提示) 键不存在".to_string(),
            other => other.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_set() {
        let (cmd, args) = parse_command("set name Alice").unwrap();
        assert_eq!(cmd, "set");
        assert_eq!(args, vec!["name", "Alice"]);
        let (cmd, args) = parse_command("set greeting Hello World").unwrap();
        assert_eq!(cmd, "set");
        assert_eq!(args, vec!["greeting", "Hello", "World"]);
        let (cmd, _) = parse_command("SET name Alice").unwrap();
        assert_eq!(cmd, "set");
        let (cmd, args) = parse_command("  set name Alice  ").unwrap();
        assert_eq!(cmd, "set");
        assert_eq!(args, vec!["name", "Alice"]);
        let err = parse_command("set name").unwrap_err();
        assert!(err.contains("需要至少 2 个参数"));
        let err = parse_command("set").unwrap_err();
        assert!(err.contains("需要至少 2 个参数"));
    }
    #[test]
    fn test_parse_get() {
        let (cmd, args) = parse_command("get name").unwrap();
        assert_eq!(cmd, "get");
        assert_eq!(args, vec!["name"]);
        let (cmd, _) = parse_command("GET name").unwrap();
        assert_eq!(cmd, "get");
        let err = parse_command("get").unwrap_err();
        assert!(err.contains("需要 1 个参数"));
        let err = parse_command("get name age").unwrap_err();
        assert!(err.contains("需要 1 个参数"));
    }
    #[test]
    fn test_parse_del() {
        let (cmd, args) = parse_command("del name").unwrap();
        assert_eq!(cmd, "del");
        assert_eq!(args, vec!["name"]);
        let err = parse_command("del").unwrap_err();
        assert!(err.contains("需要 1 个参数"));
    }
    #[test]
    fn test_parse_keys() {
        let (cmd, args) = parse_command("keys").unwrap();
        assert_eq!(cmd, "keys");
        assert!(args.is_empty());
        let (cmd, _) = parse_command("KEYS").unwrap();
        assert_eq!(cmd, "keys");
        let err = parse_command("keys a").unwrap_err();
        assert!(err.contains("不需要参数"));
    }
    #[test]
    fn test_parse_clear() {
        let (cmd, args) = parse_command("clear").unwrap();
        assert_eq!(cmd, "clear");
        assert!(args.is_empty());
        let (cmd, _) = parse_command("CLEAR").unwrap();
        assert_eq!(cmd, "clear");
        let err = parse_command("clear all").unwrap_err();
        assert!(err.contains("不需要参数"));
    }

    #[test]
    fn test_parse_compact() {
        let (cmd, args) = parse_command("compact").unwrap();
        assert_eq!(cmd, "compact");
        assert!(args.is_empty());
        let (cmd, _) = parse_command("COMPACT").unwrap();
        assert_eq!(cmd, "compact");
        let err = parse_command("compact now").unwrap_err();
        assert!(err.contains("不需要参数"));
    }
    #[test]
    fn test_parse_status() {
        let (cmd, args) = parse_command("status").unwrap();
        assert_eq!(cmd, "status");
        assert!(args.is_empty());
        let err = parse_command("status all").unwrap_err();
        assert!(err.contains("不需要参数"));
    }
    #[test]
    fn test_parse_ping() {
        let (cmd, args) = parse_command("ping").unwrap();
        assert_eq!(cmd, "ping");
        assert!(args.is_empty());
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
    #[test]
    fn test_parse_setex() {
        let (cmd, args) = parse_command("setex code 60 abc123").unwrap();
        assert_eq!(cmd, "setex");
        assert_eq!(args, vec!["code", "60", "abc123"]);
        let (_, args) = parse_command("SETEX k 5 hello world").unwrap();
        assert_eq!(args, vec!["k", "5", "hello", "world"]);
        assert!(parse_command("setex k 60").unwrap_err().contains("3 个参数"));
        assert!(parse_command("setex").unwrap_err().contains("3 个参数"));
        assert!(parse_command("setex k abc v").unwrap_err().contains("正整数"));
        assert!(parse_command("setex k -5 v").unwrap_err().contains("正整数"));
    }
    #[test]
    fn test_parse_expire() {
        let (cmd, args) = parse_command("expire k 30").unwrap();
        assert_eq!(cmd, "expire");
        assert_eq!(args, vec!["k", "30"]);
        assert!(parse_command("expire k").unwrap_err().contains("2 个参数"));
        assert!(parse_command("expire k 30 extra").unwrap_err().contains("2 个参数"));
        assert!(parse_command("expire k xyz").unwrap_err().contains("正整数"));
    }
    #[test]
    fn test_parse_ttl() {
        let (cmd, args) = parse_command("ttl k").unwrap();
        assert_eq!(cmd, "ttl");
        assert_eq!(args, vec!["k"]);
        assert!(parse_command("ttl").unwrap_err().contains("1 个参数"));
        assert!(parse_command("ttl a b").unwrap_err().contains("1 个参数"));
    }
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
