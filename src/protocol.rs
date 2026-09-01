//! 命令解析 + 响应格式化 + 客户端人类可读输出（成员 B 负责实现）
//! TODO：以下全是占位代码，B 把 todo!() 全替换成真实实现即可。

#![allow(unused)] // B 填代码阶段允许未使用导入

/// 解析一行用户输入（已经去掉末尾换行），返回 (命令字小写, 参数列表)。
/// 错误 String 直接就是返回给客户端的 ERR 内容（"ERR xxx\n" 里的 xxx）。
///
/// 必须支持的命令：
///   set <key> <value...>   —— value 多词要拼成一个（args[1..] 全拼）
///   get <key>
///   del <key>
///   keys | status | ping | quit
/// 大小写不敏感。
pub fn parse_command(line: &str) -> Result<(&str, Vec<&str>), String> {
    // TODO(B)：真实实现
    // 检查点：
    // 1. line.trim() 为空 → Err("空命令")
    // 2. 用 split_whitespace 分词，parts[0] 转小写作命令字
    // 3. 参数数量校验：set>=2 args, get/del==1, others==0
    // 4. set/del/get 时 key == "" → Err("key 不能为空")
    // 5. 未知命令 → Err(format!("未知命令: {}", parts[0]))
    let _ = line;
    todo!("parse_command —— 成员 B 实现")
}

// ============== 响应格式化（服务端 C 调用，客户端也能读）==============

pub fn resp_ok(value: Option<&str>) -> String {
    // TODO(B)： "OK {value}\n"；value=None → "OK\n"
    let _ = value;
    todo!("resp_ok —— 成员 B 实现")
}

pub fn resp_notfound() -> String {
    // TODO(B)："NOTFOUND\n"
    todo!("resp_notfound —— 成员 B 实现")
}

pub fn resp_err(msg: &str) -> String {
    // TODO(B)："ERR {msg}\n"
    let _ = msg;
    todo!("resp_err —— 成员 B 实现")
}

pub fn resp_pong() -> String {
    // TODO(B)："PONG\n"
    todo!("resp_pong —— 成员 B 实现")
}

pub fn resp_status(keys: usize, clients: usize) -> String {
    // TODO(B)："STATUS keys={keys} clients={clients}\n"
    let _ = (keys, clients);
    todo!("resp_status —— 成员 B 实现")
}

pub fn resp_keys(keys: &[String]) -> String {
    // TODO(B)：空 → "OK\n"；非空 → "OK k1,k2,k3\n"（逗号分隔，不要额外空格）
    let _ = keys;
    todo!("resp_keys —— 成员 B 实现")
}

// ============== 客户端：把一行响应解释成人看的东西 ==============

/// 输入是 server 返回来的**已经去掉末尾 \n** 的一行字符串；
/// 输出是人类喜欢看的，支持多行（keys 会被拆成带序号的列表）。
pub fn format_response_for_human(line: &str) -> String {
    // TODO(B)：
    // 匹配前缀：
    //   "OK " 或 "OK" 剩余内容：
    //     - 如果剩余内容里包含逗号（keys）：split(',') 后每行 "  [1] k1"
    //     - 否则输出 "(OK) xxx" 或 "(OK)"
    //   "NOTFOUND"       → "(提示) 键不存在"
    //   "ERR xxx"        → "(错误) xxx"
    //   "PONG"           → "PONG"
    //   "STATUS keys=... clients=..." → 拆成两行 "数据条数: N\n活跃连接: M"
    //   其他 → 原样输出，前面加个 "? "
    let _ = line;
    todo!("format_response_for_human —— 成员 B 实现")
}

// ============== 帮助文本（client 里 help 命令打印）==============

pub fn help_text() -> &'static str {
    // B 可以直接写死
    "\
可用命令：
  set <key> <value...>   写入或覆盖键值
  get <key>              查询键
  del <key>              删除键
  keys                   列出所有键（按字典序）
  status                 查看服务器：数据条数 + 活跃连接数
  ping                   心跳测试
  help                   显示本帮助
  quit                   退出客户端
"
}

// =====================================================================
// 单元测试（成员 B 填写，覆盖所有合法 + 非法情况）
// =====================================================================
#[cfg(test)]
mod tests {
    use super::*;

    // —— 合法命令正例 ——

    #[test]
    #[ignore = "等成员 B 实现 parse_command 后去掉 ignore"]
    fn parse_set() {
        let (c, a) = parse_command("set name Alice").unwrap();
        assert_eq!(c, "set");
        assert_eq!(a, vec!["name", "Alice"]);
    }

    #[test]
    #[ignore = "等成员 B 实现"]
    fn parse_set_multi_word_value() {
        // value 里多个词，参数列表里要保留给 B 自己 join
        let (_c, a) = parse_command("set msg hello world rust").unwrap();
        // 注意：多词拼接的责任放在调用方 set handler 做（C 的 server.rs 里 join）
        // 这里只要返回 [msg, hello, world, rust] 就行
        assert_eq!(a[0], "msg");
        assert_eq!(a.len(), 4);
    }

    #[test]
    #[ignore = "等成员 B 实现"]
    fn parse_case_insensitive() {
        let (c, a) = parse_command("GET mykey").unwrap();
        assert_eq!(c, "get");
        assert_eq!(a, vec!["mykey"]);
    }

    #[test]
    #[ignore = "等成员 B 实现"]
    fn parse_no_args_cmds() {
        assert_eq!(parse_command("keys").unwrap().0, "keys");
        assert_eq!(parse_command("  status  ").unwrap().0, "status");
        assert_eq!(parse_command("PING").unwrap().0, "ping");
        assert_eq!(parse_command("Quit").unwrap().0, "quit");
    }

    // —— 参数数量错误 ——

    #[test]
    #[ignore = "等成员 B 实现"]
    fn err_set_without_args() {
        assert!(parse_command("set").is_err());
    }

    #[test]
    #[ignore = "等成员 B 实现"]
    fn err_get_without_key() {
        assert!(parse_command("get").is_err());
    }

    #[test]
    #[ignore = "等成员 B 实现"]
    fn err_del_too_many_args() {
        assert!(parse_command("del a b").is_err());
    }

    #[test]
    #[ignore = "等成员 B 实现"]
    fn err_keys_with_args() {
        assert!(parse_command("keys 123").is_err());
    }

    // —— 其他错误 ——

    #[test]
    #[ignore = "等成员 B 实现"]
    fn err_unknown_cmd() {
        let e = parse_command("foobar hello").unwrap_err();
        assert!(e.contains("未知命令"), "err = {e}");
    }

    #[test]
    #[ignore = "等成员 B 实现"]
    fn err_empty_line() {
        assert!(parse_command("").is_err());
        assert!(parse_command("   ").is_err());
    }

    // —— 响应格式 ——

    #[test]
    #[ignore = "等成员 B 实现"]
    fn resp_formats() {
        assert_eq!(resp_ok(Some("hi")), "OK hi\n");
        assert_eq!(resp_ok(None), "OK\n");
        assert_eq!(resp_notfound(), "NOTFOUND\n");
        assert_eq!(resp_err("no good"), "ERR no good\n");
        assert_eq!(resp_pong(), "PONG\n");
        assert_eq!(resp_status(5, 2), "STATUS keys=5 clients=2\n");
        assert_eq!(resp_keys(&["a".into(), "b".into(), "c".into()]), "OK a,b,c\n");
        assert_eq!(resp_keys(&[]), "OK\n");
    }

    // —— 客户端展示 ——

    #[test]
    #[ignore = "等成员 B 实现"]
    fn human_format() {
        assert!(format_response_for_human("OK hello").contains("hello"));
        assert!(format_response_for_human("NOTFOUND").contains("不存在"));
        assert!(format_response_for_human("ERR boom").contains("错误"));
        assert_eq!(format_response_for_human("PONG"), "PONG");
        // keys 列表带序号
        let h = format_response_for_human("OK k1,k2,k3");
        assert!(h.contains("[1]"));
        assert!(h.contains("[2]"));
        assert!(h.contains("[3]"));
    }
}
