use std::collections::BTreeMap;
use std::fs::{create_dir_all, File, OpenOptions};
use std::io::{BufRead, BufReader, Write as IoWrite};
use std::path::Path;
use std::sync::Mutex;

use anyhow::{anyhow, Result};

/// 持久化键值存储。
/// - 内存用 BTreeMap
/// - 文件用 Append-Only 行日志：SET,key,value / DEL,key
pub struct KvStore {
    pub data: BTreeMap<String, String>,
    pub log_file: Mutex<File>,
}

/// 对外线程安全别名
pub type SharedKvStore = std::sync::Arc<std::sync::Mutex<KvStore>>;

impl KvStore {
    /// 打开（或创建）一个 KV 存储。
    /// - 如果文件存在：按顺序逐行重放日志恢复 data
    /// - 如果文件不存在：创建新文件，空库启动
    /// - 如果文件格式损坏：返回 Err，**不自动清空**
    pub fn open(path: &str) -> Result<Self> {
        let path_obj = Path::new(path);
        // 1) 保证父目录存在
        if let Some(parent) = path_obj.parent() {
            create_dir_all(parent)?;
        }

        let mut data = BTreeMap::new();

        // 2) 如果文件存在，读取并恢复数据
        if path_obj.exists() {
            let file = File::open(path_obj)?;
            let reader = BufReader::new(file);
            for line in reader.lines() {
                let line = line?;
                let line = line.trim_end();
                if line.is_empty() {
                    continue;
                }

                if line.starts_with("SET,") {
                    // SET,key,value
                    let parts: Vec<&str> = line.splitn(3, ',').collect();
                    if parts.len() != 3 || parts[1].is_empty() {
                        return Err(anyhow!(
                            "日志格式错误：SET 行需要 key 和 value，实际为 '{}'",
                            line
                        ));
                    }
                    let key = parts[1].to_string();
                    let value = parts[2].to_string();
                    data.insert(key, value);
                } else if line.starts_with("DEL,") {
                    // DEL,key
                    let parts: Vec<&str> = line.splitn(2, ',').collect();
                    if parts.len() != 2 || parts[1].is_empty() {
                        return Err(anyhow!("日志格式错误：DEL 行需要 key，实际为 '{}'", line));
                    }
                    let key = parts[1].to_string();
                    data.remove(&key);
                } else {
                    return Err(anyhow!("未知日志行格式：'{}'", line));
                }
            }
        }

        // 3) 以追加模式打开文件（如果不存在则创建）
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(path_obj)?;

        Ok(KvStore {
            data,
            log_file: Mutex::new(file),
        })
    }

    /// 写入或覆盖
    /// 顺序：写日志(SET,k,v) → flush + sync_all → 改 data → 返回 Ok
    pub fn set(&mut self, key: String, value: String) -> Result<()> {
        if key.trim().is_empty() {
            return Err(anyhow!("key 不能为空"));
        }

        // 写日志
        let log_line = format!("SET,{},{}\n", key, value);
        let mut file_guard = self.log_file.lock().unwrap();
        file_guard.write_all(log_line.as_bytes())?;
        file_guard.flush()?;
        file_guard.sync_all()?;

        // 修改内存
        self.data.insert(key, value);
        Ok(())
    }

    /// 删除。返回 Ok(true) 表示确实删了；Ok(false) 表示键本来就不存在（不写日志）
    pub fn del(&mut self, key: &str) -> Result<bool> {
        if key.trim().is_empty() {
            return Err(anyhow!("key 不能为空"));
        }

        if self.data.contains_key(key) {
            // 写日志
            let log_line = format!("DEL,{}\n", key);
            let mut file_guard = self.log_file.lock().unwrap();
            file_guard.write_all(log_line.as_bytes())?;
            file_guard.flush()?;
            file_guard.sync_all()?;

            // 删除内存
            self.data.remove(key);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// 查询
    pub fn get(&self, key: &str) -> Option<String> {
        self.data.get(key).cloned()
    }

    /// 列出所有键，有序
    pub fn keys(&self) -> Vec<String> {
        self.data.keys().cloned().collect()
    }

    /// 当前数据条数
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// 空库判断
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// =====================================================================
// 单元测试
// =====================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
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

    // 基本增删改查
    #[test]
    fn basic_set_get_del() {
        let (dir, path) = unique_dir("basic");
        let mut kv = KvStore::open(&path).unwrap();
        kv.set("k1".into(), "v1".into()).unwrap();
        assert_eq!(kv.get("k1").as_deref(), Some("v1"));
        kv.set("k1".into(), "v2".into()).unwrap();
        assert_eq!(kv.get("k1").as_deref(), Some("v2"));
        assert!(kv.del("k1").unwrap());
        assert_eq!(kv.get("k1"), None);
        assert!(!kv.del("k1").unwrap());
        let _ = fs::remove_dir_all(&dir);
    }

    // keys 有序
    #[test]
    fn keys_sorted() {
        let (dir, path) = unique_dir("keys");
        let mut kv = KvStore::open(&path).unwrap();
        for k in ["banana", "apple", "cherry"] {
            kv.set(k.into(), "v".into()).unwrap();
        }
        assert_eq!(kv.keys(), vec!["apple", "banana", "cherry"]);
        assert_eq!(kv.len(), 3);
        let _ = fs::remove_dir_all(&dir);
    }

    // 重启恢复
    #[test]
    fn restart_recovery() {
        let (dir, path) = unique_dir("recover");
        {
            let mut kv = KvStore::open(&path).unwrap();
            kv.set("a".into(), "1".into()).unwrap();
            kv.set("b".into(), "2".into()).unwrap();
            kv.del("a").unwrap();
        }
        let kv2 = KvStore::open(&path).unwrap();
        assert_eq!(kv2.get("a"), None);
        assert_eq!(kv2.get("b").as_deref(), Some("2"));
        assert_eq!(kv2.len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    // value 里带逗号 / 空格
    #[test]
    fn value_with_comma_and_space() {
        let (dir, path) = unique_dir("commas");
        {
            let mut kv = KvStore::open(&path).unwrap();
            kv.set("x".into(), "hello, world, foo".into()).unwrap();
        }
        let kv = KvStore::open(&path).unwrap();
        assert_eq!(kv.get("x").as_deref(), Some("hello, world, foo"));
        let _ = fs::remove_dir_all(&dir);
    }

    // 文件损坏要 Err 不要 panic 不要清空
    #[test]
    fn corrupted_file_is_err() {
        let (dir, path) = unique_dir("bad");
        fs::write(&path, "@@@@乱码垃圾内容@@@@\n").unwrap();
        let res = KvStore::open(&path);
        assert!(
            res.is_err(),
            "损坏的文件必须返回 Err，不能 panic / 静默置空"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    // 空 key 要拒绝
    #[test]
    fn empty_key_rejected() {
        let (dir, path) = unique_dir("emptykey");
        let mut kv = KvStore::open(&path).unwrap();
        assert!(kv.set("".into(), "v".into()).is_err());
        assert!(kv.set("   ".into(), "v".into()).is_err());
        assert!(kv.del("").is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
