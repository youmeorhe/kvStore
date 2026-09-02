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
    /// 数据文件路径（clear 截断、compact rename 时需要）
    pub path: std::path::PathBuf,
    /// 日志压缩触发阈值（字节）。超过且废数据过半才压缩。测试可调小。
    pub compact_threshold: u64,
}

/// 默认压缩阈值：1MB
pub const DEFAULT_COMPACT_THRESHOLD: u64 = 1024 * 1024;

/// 对外线程安全别名
pub type SharedKvStore = std::sync::Arc<std::sync::Mutex<KvStore>>;

/// 压缩临时文件路径：kv.db → kv.db.tmp
fn compact_tmp_path(path: &Path) -> std::path::PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".tmp");
    std::path::PathBuf::from(s)
}

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

        // 4) 清理上次压缩失败可能残留的 tmp 文件（内容不完整，不可用）
        let tmp_path = compact_tmp_path(path_obj);
        let _ = std::fs::remove_file(&tmp_path);

        Ok(KvStore {
            data,
            log_file: Mutex::new(file),
            path: path_obj.to_path_buf(),
            compact_threshold: DEFAULT_COMPACT_THRESHOLD,
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
        {
            let mut file_guard = self.log_file.lock().unwrap();
            file_guard.write_all(log_line.as_bytes())?;
            file_guard.flush()?;
            file_guard.sync_all()?;
        } // file_guard 在此显式释放，避免与后面的 &mut self 借用冲突

        // 修改内存
        self.data.insert(key, value);

        // 落盘后检查是否需要压缩
        self.maybe_compact();
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
            {
                let mut file_guard = self.log_file.lock().unwrap();
                file_guard.write_all(log_line.as_bytes())?;
                file_guard.flush()?;
                file_guard.sync_all()?;
            } // file_guard 在此显式释放

            // 删除内存
            self.data.remove(key);

            // 落盘后检查是否需要压缩
            self.maybe_compact();
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

    /// 清空所有数据（内存 + 日志文件），返回清掉的条数。
    /// 幂等：空库调用返回 Ok(0)。
    ///
    /// 截断实现：另开一个 truncate(true) 的句柄把文件清零。
    /// （Windows 下 append 句柄没有截断权限，set_len 会报"拒绝访问"，
    ///   只能重新以 write+truncate 打开；append 句柄不受影响，
    ///   append 模式的写永远落在当前文件末尾。）
    pub fn clear(&mut self) -> Result<usize> {
        let removed = self.data.len();
        // 先截断磁盘，再清内存（与 set 的"先日志后内存"顺序一致）
        let trunc = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&self.path)?;
        trunc.sync_all()?;
        drop(trunc);
        self.data.clear();
        Ok(removed)
    }

    // ================= 日志压缩（Compaction）=================

    /// 当前日志文件大小（字节）
    fn file_size(&self) -> Result<u64> {
        let f = self.log_file.lock().unwrap();
        Ok(f.metadata()?.len())
    }

    /// 内存有效数据的近似字节数（"SET,key,value\n" 每条开销 6 + 逗号 + 换行 ≈ 8）
    fn live_bytes(&self) -> u64 {
        self.data
            .iter()
            .map(|(k, v)| (k.len() + v.len() + 8) as u64)
            .sum()
    }

    /// 写操作落盘后的检查点：双条件防抖触发压缩。
    /// 条件1：文件超过 compact_threshold；
    /// 条件2：有效数据不足文件一半（压缩有实际收益）。
    /// 全干货文件不压，避免"压完还那么大→每次写都压缩"的性能雪崩。
    /// 压缩失败只 warn 不上抛（降级运行，不中断服务）。
    fn maybe_compact(&mut self) {
        let size = match self.file_size() {
            Ok(s) => s,
            Err(_) => return,
        };
        if size < self.compact_threshold {
            return;
        }
        if self.live_bytes() * 2 > size {
            return; // 干货过半，压了没收益
        }
        if let Err(e) = self.compact() {
            eprintln!("[warn] 日志压缩失败，继续使用旧日志: {e:#}");
        }
    }

    /// 压缩：把内存最终状态重写成等价的最小日志，原子替换旧文件。
    ///
    /// 流程（Windows 句柄顺序是关键）：
    /// ① 最终状态写 kv.db.tmp（create + sync_all，句柄随作用域关闭）
    /// ② 在 tmp 路径上打开追加句柄，replace 换进 log_file，drop 旧句柄
    ///    （Windows 下 rename 的目标文件被自己占用会失败，必须先释放旧句柄；
    ///      新句柄指向文件对象本身，rename 后自动"变成"正式文件句柄）
    /// ③ fs::rename(tmp → kv.db) 原子替换
    /// 任何一步失败：kv.db 原封不动，残留 tmp 由下次 open() 清理。
    fn compact(&mut self) -> Result<()> {
        let tmp_path = compact_tmp_path(&self.path);

        // ① 写临时文件
        {
            let mut tmp = File::create(&tmp_path)?;
            for (k, v) in &self.data {
                writeln!(tmp, "SET,{},{}", k, v)?;
            }
            tmp.sync_all()?;
        } // tmp 句柄在此关闭

        // ② 换句柄 + 释放旧文件占用
        let new_handle = OpenOptions::new().append(true).open(&tmp_path)?;
        let old = std::mem::replace(&mut self.log_file, Mutex::new(new_handle));
        drop(old);

        // ③ 原子替换
        std::fs::rename(&tmp_path, &self.path)?;
        Ok(())
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

    // 清空：内存 + 日志一起清，重启后仍是空库
    #[test]
    fn clear_then_restart_is_empty() {
        let (dir, path) = unique_dir("clear");
        {
            let mut kv = KvStore::open(&path).unwrap();
            kv.set("a".into(), "1".into()).unwrap();
            kv.set("b".into(), "2".into()).unwrap();
            kv.set("c".into(), "3".into()).unwrap();
            assert_eq!(kv.clear().unwrap(), 3);      // 清掉 3 条
            assert_eq!(kv.len(), 0);
            assert!(kv.is_empty());
            // clear 后继续写入正常工作
            kv.set("d".into(), "4".into()).unwrap();
        }
        // 重启：只有 clear 之后写入的 d 在，a/b/c 不复活
        let kv2 = KvStore::open(&path).unwrap();
        assert_eq!(kv2.len(), 1);
        assert_eq!(kv2.get("d").as_deref(), Some("4"));
        let _ = fs::remove_dir_all(&dir);
    }

    // 空库 clear 幂等，返回 0
    #[test]
    fn clear_empty_is_ok() {
        let (dir, path) = unique_dir("clearempty");
        let mut kv = KvStore::open(&path).unwrap();
        assert_eq!(kv.clear().unwrap(), 0);
        assert!(kv.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    // ===== 日志压缩测试 =====

    // 压缩①：同 key 反复写 + 小阈值 → 触发压缩，文件大幅缩小
    #[test]
    fn compact_shrinks_overwritten_log() {
        let (dir, path) = unique_dir("compact1");
        {
            let mut kv = KvStore::open(&path).unwrap();
            kv.compact_threshold = 200; // 小阈值便于触发
            // 同一个 key 写 50 次：50 行日志，有效数据只有最后 1 条
            for i in 0..50 {
                kv.set("hot".into(), format!("value-{i}")).unwrap();
            }
            // 压缩应已触发。数学模型：阈值 200B ÷ 每行约 17B ≈ 每攒 12 行触发一次压缩，
            // 循环"压缩→追加12行→再压缩"，最后一次压缩后最多残留 11 行左右。
            // 断言"远小于 50"即证明压缩在工作（无压缩时就是 50 行）。
            let content = fs::read_to_string(&path).unwrap();
            let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
            assert!(
                lines.len() <= 15,
                "压缩后行数应大幅缩小（≤15），实际 {} 行",
                lines.len()
            );
            // 最后一行必须是最后一次写入
            let last = lines.last().unwrap();
            assert!(last.starts_with("SET,hot,"));
            assert!(last.ends_with("value-49"), "日志末尾应是最后一次写入");
            // 无残留 tmp
            let tmp = std::path::Path::new(&dir).join("kv.db.tmp");
            assert!(!tmp.exists(), "压缩后不应残留 tmp 文件");
        }
        // 数据正确性不受影响
        let kv2 = KvStore::open(&path).unwrap();
        assert_eq!(kv2.get("hot").as_deref(), Some("value-49"));
        let _ = fs::remove_dir_all(&dir);
    }

    // 压缩②：压缩后重开 → replay 数据完整
    #[test]
    fn compact_then_restart_keeps_data() {
        let (dir, path) = unique_dir("compact2");
        {
            let mut kv = KvStore::open(&path).unwrap();
            kv.compact_threshold = 200;
            for i in 0..30 {
                kv.set("a".into(), format!("va{i}")).unwrap(); // 废数据
            }
            kv.set("keep1".into(), "hello".into()).unwrap();
            kv.set("keep2".into(), "world, foo".into()).unwrap();
            kv.set("gone".into(), "x".into()).unwrap();
            kv.del("gone").unwrap();
        }
        let kv2 = KvStore::open(&path).unwrap();
        assert_eq!(kv2.get("keep1").as_deref(), Some("hello"));
        assert_eq!(kv2.get("keep2").as_deref(), Some("world, foo"));
        assert_eq!(kv2.get("a").as_deref(), Some("va29")); // 最后一次
        assert_eq!(kv2.get("gone"), None);
        assert_eq!(kv2.len(), 3);
        let _ = fs::remove_dir_all(&dir);
    }

    // 压缩③：压缩后继续 set/del/clear 都正常
    #[test]
    fn compact_then_continue_writes() {
        let (dir, path) = unique_dir("compact3");
        {
            let mut kv = KvStore::open(&path).unwrap();
            kv.compact_threshold = 150;
            for i in 0..20 {
                kv.set("x".into(), format!("v{i}")).unwrap(); // 触发压缩
            }
            // 压缩后继续写：新句柄必须可用
            kv.set("post".into(), "after-compact".into()).unwrap();
            kv.del("x").unwrap();
            assert_eq!(kv.get("post").as_deref(), Some("after-compact"));
            // clear 交互：clear 后再写再压缩
            kv.clear().unwrap();
            for i in 0..20 {
                kv.set("y".into(), format!("w{i}")).unwrap();
            }
            kv.set("final".into(), "done".into()).unwrap();
        }
        let kv2 = KvStore::open(&path).unwrap();
        assert_eq!(kv2.get("post"), None);
        assert_eq!(kv2.get("y").as_deref(), Some("w19"));
        assert_eq!(kv2.get("final").as_deref(), Some("done"));
        let _ = fs::remove_dir_all(&dir);
    }

    // 压缩④：全是干货 + 超阈值 → 不触发压缩（防抖）
    #[test]
    fn compact_skipped_when_data_is_live() {
        let (dir, path) = unique_dir("compact4");
        let mut kv = KvStore::open(&path).unwrap();
        kv.compact_threshold = 300; // 小于即将写入的总量
        // 写 60 个不同的 key：全是干货，无废数据
        for i in 0..60 {
            kv.set(format!("key-{i:03}"), format!("value-{i:03}")).unwrap();
        }
        let content = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
        // 60 条全部保留 = 没被压缩重写（压缩会写 "SET,key-000,value-000" 顺序相同，
        // 但关键是行数不会减少；再验证 get 全部在）
        assert_eq!(lines.len(), 60, "全干货不应触发压缩，行数应保持 60");
        assert_eq!(kv.len(), 60);
        for i in 0..60 {
            assert_eq!(
                kv.get(&format!("key-{i:03}")).as_deref(),
                Some(format!("value-{i:03}").as_str())
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
