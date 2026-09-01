//! 键值存储 + WAL 持久化（成员 A 负责实现）
//! TODO：以下全是占位代码，A 把 todo!() 全替换成真实实现即可。

#![allow(unused, dead_code)] // A/B/C 填代码阶段允许有未使用的导入/字段，不刷屏 warning

use std::collections::BTreeMap;
use std::fs::{create_dir_all, File, OpenOptions};
use std::io::{BufRead, BufReader, Write as IoWrite};
use std::path::Path;
use std::sync::Mutex;

use crate::error::{KvError, Result};

/// 持久化键值存储。
/// - 内存用 BTreeMap（keys 自带序）
/// - 文件用 Append-Only 行日志：SET,key,value / DEL,key
pub struct KvStore {
    /// TODO(A)：把内存数据放这，BTreeMap<String, String>
    pub data: BTreeMap<String, String>,
    /// TODO(A)：日志文件句柄，每次 set/del 追加一行后 sync_all()
    pub log_file: Mutex<File>,
}

/// 对外线程安全别名（成员 C 直接用这个类型）
pub type SharedKvStore = std::sync::Arc<std::sync::Mutex<KvStore>>;

impl KvStore {
    /// 打开（或创建）一个 KV 存储。
    /// - 如果文件存在：按顺序逐行重放日志恢复 data
    /// - 如果文件不存在：创建新文件，空库启动
    /// - 如果文件格式损坏：返回 Err，**不自动清空**
    pub fn open(path: &str) -> Result<Self> {
        // TODO(A)：真实实现
        // 建议步骤：
        //  1) 保证父目录存在：create_dir_all(Path::new(path).parent().unwrap_or(Path::new(".")))
        //  2) 读已有文件（如果存在）逐行重放进 data：
        //        "SET,k,v" => data.insert(k, v);  v 可能包含逗号（只有前两个是分隔符）
        //        "DEL,k"   => data.remove(k);
        //     任何格式错误（行无法拆分、空 key、SET 只有 2 段…）→ 立即 return Err
        //     注意 value 里可能包含逗号，所以 SET 行按逗号 splitn(3, ',')
        //  3) OpenOptions::new().append(true).create(true).open(path) 拿到 log_file
        //  4) Ok(KvStore { data, log_file: Mutex::new(file) })
        let _path = path;
        todo!("KvStore::open —— 成员 A 实现")
    }

    /// 写入或覆盖。
    /// 顺序：写日志(SET,k,v) → flush + sync_all → 改 data → 返回 Ok
    pub fn set(&self, key: String, value: String) -> Result<()> {
        // TODO(A)：真实实现
        // 要点：
        // - key.trim().is_empty() => return Err(KvError("key 不能为空".into()))
        // - log_file.lock() 拿锁 → write_all(format!("SET,{key},{value}\n").as_bytes())
        //   → flush() → sync_all()（保证落到磁盘）→ 失败都是 Err
        // - 然后再 self.data.insert(key, value);
        if key.trim().is_empty() {
            return Err(KvError("key 不能为空".into()));
        }
        let _ = value;
        todo!("KvStore::set —— 成员 A 实现")
    }

    /// 删除。返回 Ok(true) 表示确实删了；Ok(false) 表示键本来就不存在（不写日志）。
    pub fn del(&self, key: &str) -> Result<bool> {
        // TODO(A)：真实实现
        // 要点：key 存在才写 "DEL,{key}\n" 日志；不存在直接 Ok(false)
        if key.trim().is_empty() {
            return Err(KvError("key 不能为空".into()));
        }
        todo!("KvStore::del —— 成员 A 实现")
    }

    /// 查询（只读内存，不碰文件）。
    pub fn get(&self, key: &str) -> Option<String> {
        // TODO(A)：真实实现 → self.data.get(key).cloned()
        let _ = key;
        todo!("KvStore::get —— 成员 A 实现")
    }

    /// 列出所有键，有序。
    pub fn keys(&self) -> Vec<String> {
        // TODO(A)：真实实现 → self.data.keys().cloned().collect()
        todo!("KvStore::keys —— 成员 A 实现")
    }

    /// 当前数据条数。
    pub fn len(&self) -> usize {
        // TODO(A)：真实实现 → self.data.len()
        todo!("KvStore::len —— 成员 A 实现")
    }

    /// 空库判断（clippy 喜欢有这个）
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// =====================================================================
// 单元测试（成员 A 写真实用例，下面给出框架直接填断言就行）
// 零依赖：不用 tempfile crate，自己建 target/storage-tests/<uuid> 临时目录
// =====================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_dir(suffix: &str) -> (std::path::PathBuf, String) {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join("storage-tests")
            .join(format!("test-{n}-{suffix}"));
        create_dir_all(&dir).unwrap();
        let db = dir.join("kv.db");
        (dir, db.to_string_lossy().to_string())
    }

    // 基本增删改查（A 实现后跑）
    #[test]
    #[ignore = "等成员 A 实现 KvStore 后去掉 ignore"]
    fn basic_set_get_del() {
        let (_dir, path) = unique_dir("basic");
        let kv = KvStore::open(&path).unwrap();
        kv.set("k1".into(), "v1".into()).unwrap();
        assert_eq!(kv.get("k1").as_deref(), Some("v1"));
        kv.set("k1".into(), "v2".into()).unwrap();
        assert_eq!(kv.get("k1").as_deref(), Some("v2"));
        assert!(kv.del("k1").unwrap());
        assert_eq!(kv.get("k1"), None);
        assert!(!kv.del("k1").unwrap());
    }

    // keys 有序
    #[test]
    #[ignore = "等成员 A 实现 KvStore 后去掉 ignore"]
    fn keys_sorted() {
        let (_dir, path) = unique_dir("keys");
        let kv = KvStore::open(&path).unwrap();
        for k in ["banana", "apple", "cherry"] {
            kv.set(k.into(), "v".into()).unwrap();
        }
        assert_eq!(kv.keys(), vec!["apple", "banana", "cherry"]);
        assert_eq!(kv.len(), 3);
    }

    // 重启恢复（最关键的测试）
    #[test]
    #[ignore = "等成员 A 实现 KvStore 后去掉 ignore"]
    fn restart_recovery() {
        let (dir, path) = unique_dir("recover");
        {
            let kv = KvStore::open(&path).unwrap();
            kv.set("a".into(), "1".into()).unwrap();
            kv.set("b".into(), "2".into()).unwrap();
            kv.del("a").unwrap();
        }
        let kv2 = KvStore::open(&path).unwrap();
        assert_eq!(kv2.get("a"), None);
        assert_eq!(kv2.get("b").as_deref(), Some("2"));
        assert_eq!(kv2.len(), 1);
        // 清掉测试垃圾
        let _ = std::fs::remove_dir_all(dir);
    }

    // value 里带逗号 / 空格（测试 SET,k,v 的 v 正确解析：取第一个逗号之后所有内容）
    #[test]
    #[ignore = "等成员 A 实现 KvStore 后去掉 ignore"]
    fn value_with_comma_and_space() {
        let (dir, path) = unique_dir("commas");
        {
            let kv = KvStore::open(&path).unwrap();
            kv.set("x".into(), "hello, world, foo".into()).unwrap();
        }
        let kv = KvStore::open(&path).unwrap();
        assert_eq!(kv.get("x").as_deref(), Some("hello, world, foo"));
        let _ = std::fs::remove_dir_all(dir);
    }

    // 文件损坏要 Err 不要 panic 不要清空
    #[test]
    #[ignore = "等成员 A 实现 KvStore 后去掉 ignore"]
    fn corrupted_file_is_err() {
        let (dir, path) = unique_dir("bad");
        std::fs::write(&path, "@@@@乱码垃圾内容@@@@\n").unwrap();
        let res = KvStore::open(&path);
        assert!(res.is_err(), "损坏的文件必须返回 Err，不能 panic / 静默置空");
        let _ = std::fs::remove_dir_all(dir);
    }

    // 空 key 要拒绝
    #[test]
    #[ignore = "等成员 A 实现 KvStore 后去掉 ignore"]
    fn empty_key_rejected() {
        let (_dir, path) = unique_dir("emptykey");
        let kv = KvStore::open(&path).unwrap();
        assert!(kv.set("".into(), "v".into()).is_err());
        assert!(kv.set("   ".into(), "v".into()).is_err());
        assert!(kv.del("").is_err());
    }
}
