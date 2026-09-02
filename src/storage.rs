use std::collections::BTreeMap;
use std::fs::{create_dir_all, File, OpenOptions};
use std::io::{BufRead, BufReader, Write as IoWrite};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};

/// key 合法性校验：非空且不含逗号。
/// 逗号是日志行分隔符，key 含逗号会导致落盘后无法与分隔符区分（数据混淆），
/// 因此显式拒绝（fail-fast），而不是静默存成错误的 key
fn validate_key(key: &str) -> Result<()> {
    if key.trim().is_empty() {
        return Err(anyhow!("key 不能为空"));
    }
    if key.contains(',') {
        return Err(anyhow!("key 不能包含逗号（日志分隔符），请使用其他字符"));
    }
    Ok(())
}

/// 带过期信息的值条目（TTL 支持）
#[derive(Clone, Debug, PartialEq)]
pub struct ValueEntry {
    pub value: String,
    /// 过期时刻（进程内单调时钟）。None = 永不过期
    pub expire_at: Option<Instant>,
}

impl ValueEntry {
    /// 永不过期条目
    pub fn permanent(value: String) -> Self {
        Self { value, expire_at: None }
    }
    /// 是否存活（未过期）
    pub fn alive(&self) -> bool {
        match self.expire_at {
            None => true,
            Some(t) => Instant::now() < t,
        }
    }
}

/// 当前 Unix 时间戳（毫秒）。用于把过期时刻序列化进日志：
/// Instant 无法跨进程/跨重启使用，落盘必须用绝对时间，加载时再换算回 Instant
fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// 把"now + ttl 秒"换算成 Unix 毫秒时间戳（用于写日志）
fn unix_ms_after_secs(secs: u64) -> u64 {
    now_unix_ms() + secs * 1000
}

/// 把 Unix 毫秒时间戳换算回 Instant（用于重放加载）。
/// 已过期的时间戳会得到一个已过去的 Instant，由存活检查统一处理
fn instant_from_unix_ms(ms: u64) -> Instant {
    let now = now_unix_ms();
    if ms <= now {
        Instant::now()
    } else {
        Instant::now() + Duration::from_millis(ms - now)
    }
}

/// 持久化键值存储。
/// - 内存用 BTreeMap（值带可选过期时刻）
/// - 文件用 Append-Only 行日志：
///   `SET,key,value`（永不过期）/ `SETX,key,value,unix_ms`（带过期）
///   `DEL,key` / `EXPIRE,key,unix_ms`
pub struct KvStore {
    pub data: BTreeMap<String, ValueEntry>,
    pub log_file: Mutex<File>,
    /// 数据文件路径（clear 截断、compact rename 时需要）
    pub path: std::path::PathBuf,
}

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

                if let Some(rest) = line.strip_prefix("SET,") {
                    // SET,key,value（永不过期）
                    let (key, value) = rest
                        .split_once(',')
                        .ok_or_else(|| anyhow!("日志格式错误：SET 行需要 key 和 value，实际为 '{}'", line))?;
                    if key.is_empty() {
                        return Err(anyhow!("日志格式错误：SET 行 key 为空，实际为 '{}'", line));
                    }
                    data.insert(key.to_string(), ValueEntry::permanent(value.to_string()));
                } else if let Some(rest) = line.strip_prefix("SETX,") {
                    // SETX,key,value,unix_ms（带过期）。
                    // value 本身可含逗号：先从左边切出 key，再从右边切出时间戳，中间全是 value
                    let (key, rest) = rest
                        .split_once(',')
                        .ok_or_else(|| anyhow!("日志格式错误：SETX 行需要 key/value/时间戳，实际为 '{}'", line))?;
                    let (value, ts_str) = rest
                        .rsplit_once(',')
                        .ok_or_else(|| anyhow!("日志格式错误：SETX 行需要过期时间戳，实际为 '{}'", line))?;
                    let expire_ms: u64 = ts_str
                        .parse()
                        .map_err(|_| anyhow!("日志格式错误：SETX 行时间戳无效，实际为 '{}'", line))?;
                    if key.is_empty() {
                        return Err(anyhow!("日志格式错误：SETX 行 key 为空，实际为 '{}'", line));
                    }
                    data.insert(
                        key.to_string(),
                        ValueEntry { value: value.to_string(), expire_at: Some(instant_from_unix_ms(expire_ms)) },
                    );
                } else if let Some(rest) = line.strip_prefix("EXPIRE,") {
                    // EXPIRE,key,unix_ms（对已存在的 key 设置/覆盖过期时刻；
                    // key 不存在则忽略——与运行时 expire 对不存在键返回 false 的语义一致）
                    let (key, ts_str) = rest
                        .split_once(',')
                        .ok_or_else(|| anyhow!("日志格式错误：EXPIRE 行需要 key/时间戳，实际为 '{}'", line))?;
                    let expire_ms: u64 = ts_str
                        .parse()
                        .map_err(|_| anyhow!("日志格式错误：EXPIRE 行时间戳无效，实际为 '{}'", line))?;
                    if let Some(entry) = data.get_mut(key) {
                        entry.expire_at = Some(instant_from_unix_ms(expire_ms));
                    }
                } else if let Some(rest) = line.strip_prefix("DEL,") {
                    // DEL,key
                    if rest.is_empty() {
                        return Err(anyhow!("日志格式错误：DEL 行需要 key，实际为 '{}'", line));
                    }
                    data.remove(rest);
                } else {
                    return Err(anyhow!("未知日志行格式：'{}'", line));
                }
            }
            // 重放完成：清理已过期的条目（宕机期间到期的不占内存、不对外可见）
            let expired: Vec<String> = data
                .iter()
                .filter(|(_, e)| !e.alive())
                .map(|(k, _)| k.clone())
                .collect();
            for k in expired {
                data.remove(&k);
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
        })
    }

    /// 写入或覆盖
    /// 顺序：写日志(SET,k,v) → flush + sync_all → 改 data → 返回 Ok
    pub fn set(&mut self, key: String, value: String) -> Result<()> {
        validate_key(&key)?;

        // 写日志
        let log_line = format!("SET,{},{}\n", key, value);
        {
            let mut file_guard = self.log_file.lock().unwrap();
            file_guard.write_all(log_line.as_bytes())?;
            file_guard.flush()?;
            file_guard.sync_all()?;
        } // file_guard 在此显式释放，避免与后面的 &mut self 借用冲突

        // 修改内存（set 覆盖语义：同时清除旧 TTL，与 Redis SET 行为一致）
        self.data.insert(key, ValueEntry::permanent(value));
        Ok(())
    }

    /// 写入并设置过期秒数（TTL）。
    /// 顺序与 set 相同：写日志(SETX) → 刷盘 → 改内存
    pub fn setex(&mut self, key: String, secs: u64, value: String) -> Result<()> {
        validate_key(&key)?;
        let expire_ms = unix_ms_after_secs(secs);
        let log_line = format!("SETX,{},{},{}\n", key, value, expire_ms);
        {
            let mut file_guard = self.log_file.lock().unwrap();
            file_guard.write_all(log_line.as_bytes())?;
            file_guard.flush()?;
            file_guard.sync_all()?;
        }
        self.data.insert(
            key,
            ValueEntry {
                value,
                expire_at: Some(Instant::now() + Duration::from_secs(secs)),
            },
        );
        Ok(())
    }

    /// 给已存在的 key 设置过期秒数。返回 Ok(false) 表示键不存在（不写日志）。
    /// 已过期的 key 视为不存在。
    pub fn expire(&mut self, key: &str, secs: u64) -> Result<bool> {
        validate_key(key)?;
        // 只对"存活"的 key 生效
        let alive = self.data.get(key).is_some_and(|e| e.alive());
        if !alive {
            return Ok(false);
        }
        let expire_ms = unix_ms_after_secs(secs);
        let log_line = format!("EXPIRE,{},{}\n", key, expire_ms);
        {
            let mut file_guard = self.log_file.lock().unwrap();
            file_guard.write_all(log_line.as_bytes())?;
            file_guard.flush()?;
            file_guard.sync_all()?;
        }
        if let Some(entry) = self.data.get_mut(key) {
            entry.expire_at = Some(Instant::now() + Duration::from_secs(secs));
        }
        Ok(true)
    }

    /// 查询剩余生存时间（秒）。返回值约定与 Redis 一致：
    /// - Some(-1)：键存在且永不过期
    /// - Some(-2)：键不存在或已过期
    /// - Some(n>=0)：剩余 n 秒
    pub fn ttl(&self, key: &str) -> Option<i64> {
        match self.data.get(key) {
            None => Some(-2),
            Some(e) if !e.alive() => Some(-2),
            Some(e) => match e.expire_at {
                None => Some(-1),
                Some(t) => {
                    let remain = t.saturating_duration_since(Instant::now());
                    // 向上取整：还剩 1ms 也算剩 1 秒，避免"刚 setex 2 秒就查 ttl 得 1"
                    Some(remain.as_secs() as i64 + if remain.subsec_millis() > 0 { 1 } else { 0 })
                }
            },
        }
    }

    /// 删除。返回 Ok(true) 表示确实删了；Ok(false) 表示键本来就不存在（不写日志）。
    /// 已过期的键视为不存在（返回 false）。
    pub fn del(&mut self, key: &str) -> Result<bool> {
        validate_key(key)?;

        let alive = self.data.get(key).is_some_and(|e| e.alive());
        if alive {
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
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// 查询（惰性删除：访问到已过期的键时顺手从内存移除）。
    /// 需要 &mut self——代价是调用方须持写锁，但 server 本来就是单锁模型，无额外开销。
    /// 内存删除不写 DEL 日志：重放时过期条目会被统一清理，无需日志也能保持一致
    pub fn get(&mut self, key: &str) -> Option<String> {
        match self.data.get(key) {
            Some(e) if e.alive() => Some(e.value.clone()),
            Some(_) => {
                self.data.remove(key); // 惰性删除
                None
            }
            None => None,
        }
    }

    /// 列出所有键，有序（过滤已过期）
    pub fn keys(&self) -> Vec<String> {
        self.data
            .iter()
            .filter(|(_, e)| e.alive())
            .map(|(k, _)| k.clone())
            .collect()
    }

    /// 当前数据条数（过滤已过期）
    pub fn len(&self) -> usize {
        self.data.iter().filter(|(_, e)| e.alive()).count()
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
    pub fn file_size(&self) -> Result<u64> {
        let f = self.log_file.lock().unwrap();
        Ok(f.metadata()?.len())
    }

    /// 手动压缩：把内存最终状态重写成等价的最小日志，原子替换旧文件。
    /// 返回 (压缩前字节数, 压缩后字节数)。
    /// 幂等：对已压缩的库再压，前=后。
    ///
    /// 压缩效果：同一 key 的多次 SET 只保留最后一次；已删除 key 的
    /// SET/DEL 记录全部清除。重放压缩前后日志得到的状态完全一致。
    ///
    /// 流程（Windows 句柄顺序是关键）：
    /// ① 最终状态写 kv.db.tmp（create + sync_all，句柄随作用域关闭）
    /// ② 在 tmp 路径上打开追加句柄，replace 换进 log_file，drop 旧句柄
    ///    （Windows 下 rename 的目标文件被自己占用会失败，必须先释放旧句柄；
    ///      新句柄指向文件对象本身，rename 后自动"变成"正式文件句柄）
    /// ③ fs::rename(tmp → kv.db) 原子替换
    /// 任何一步失败：kv.db 原封不动，残留 tmp 由下次 open() 清理。
    pub fn compact(&mut self) -> Result<(u64, u64)> {
        let before = self.file_size()?;
        let tmp_path = compact_tmp_path(&self.path);

        // ① 写临时文件（只写存活条目；过期的键在这里被永久清除——"压缩即清理"）
        {
            let mut tmp = File::create(&tmp_path)?;
            for (k, e) in &self.data {
                if !e.alive() {
                    continue;
                }
                match e.expire_at {
                    None => writeln!(tmp, "SET,{},{}", k, e.value)?,
                    // 过期时刻用绝对 Unix 毫秒回写：压缩后重启仍能正确恢复剩余 TTL
                    Some(t) => {
                        let remain = t.saturating_duration_since(Instant::now());
                        let ms = now_unix_ms() + remain.as_millis() as u64;
                        writeln!(tmp, "SETX,{},{},{}", k, e.value, ms)?;
                    }
                }
            }
            tmp.sync_all()?;
        } // tmp 句柄在此关闭

        // ② 换句柄 + 释放旧文件占用
        let new_handle = OpenOptions::new().append(true).open(&tmp_path)?;
        let old = std::mem::replace(&mut self.log_file, Mutex::new(new_handle));
        drop(old);

        // ③ 原子替换
        std::fs::rename(&tmp_path, &self.path)?;

        let after = self.file_size()?;
        Ok((before, after))
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
        let mut kv2 = KvStore::open(&path).unwrap();
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
        let mut kv = KvStore::open(&path).unwrap();
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
        let mut kv2 = KvStore::open(&path).unwrap();
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

    // ===== 日志压缩测试（手动触发）=====

    // 压缩①：同 key 反复写 → 手动压缩 → 文件缩回 1 行
    #[test]
    fn compact_shrinks_overwritten_log() {
        let (dir, path) = unique_dir("compact1");
        {
            let mut kv = KvStore::open(&path).unwrap();
            // 同一个 key 写 50 次：50 行日志，有效数据只有最后 1 条
            for i in 0..50 {
                kv.set("hot".into(), format!("value-{i}")).unwrap();
            }
            let (before, after) = kv.compact().unwrap();
            assert!(before >= 50 * 15, "压缩前应至少 50 行");
            assert_eq!(after, "SET,hot,value-49\n".len() as u64, "压缩后只 1 行");
            // 文件内容精确等于 1 行
            let content = fs::read_to_string(&path).unwrap();
            assert_eq!(content, "SET,hot,value-49\n");
            // 无残留 tmp
            let tmp = std::path::Path::new(&dir).join("kv.db.tmp");
            assert!(!tmp.exists(), "压缩后不应残留 tmp 文件");
            // 幂等：再压一次，前后相等
            let (b2, a2) = kv.compact().unwrap();
            assert_eq!(b2, a2);
            assert_eq!(a2, after);
        }
        // 数据正确性不受影响
        let mut kv2 = KvStore::open(&path).unwrap();
        assert_eq!(kv2.get("hot").as_deref(), Some("value-49"));
        let _ = fs::remove_dir_all(&dir);
    }

    // 压缩②：压缩后重开 → replay 数据完整（含删除的键、带逗号的 value）
    #[test]
    fn compact_then_restart_keeps_data() {
        let (dir, path) = unique_dir("compact2");
        {
            let mut kv = KvStore::open(&path).unwrap();
            for i in 0..30 {
                kv.set("a".into(), format!("va{i}")).unwrap(); // 废数据
            }
            kv.set("keep1".into(), "hello".into()).unwrap();
            kv.set("keep2".into(), "world, foo".into()).unwrap();
            kv.set("gone".into(), "x".into()).unwrap();
            kv.del("gone").unwrap();
            kv.compact().unwrap();
        }
        let mut kv2 = KvStore::open(&path).unwrap();
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
            for i in 0..20 {
                kv.set("x".into(), format!("v{i}")).unwrap();
            }
            kv.compact().unwrap();
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
            kv.compact().unwrap();
        }
        let mut kv2 = KvStore::open(&path).unwrap();
        assert_eq!(kv2.get("post"), None);
        assert_eq!(kv2.get("y").as_deref(), Some("w19"));
        assert_eq!(kv2.get("final").as_deref(), Some("done"));
        let _ = fs::remove_dir_all(&dir);
    }

    // 压缩④：全干货压缩 → 前后字节数近似相等（无废数据可压）
    #[test]
    fn compact_all_live_data_is_stable() {
        let (dir, path) = unique_dir("compact4");
        {
            let mut kv = KvStore::open(&path).unwrap();
            for i in 0..60 {
                kv.set(format!("key-{i:03}"), format!("value-{i:03}")).unwrap();
            }
            let (before, after) = kv.compact().unwrap();
            // 60 条干货：压缩前后都应是 60 行，大小几乎相同
            assert_eq!(before, after);
            assert_eq!(kv.len(), 60);
            let content = fs::read_to_string(&path).unwrap();
            let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
            assert_eq!(lines.len(), 60);
        }
        let mut kv2 = KvStore::open(&path).unwrap();
        for i in 0..60 {
            assert_eq!(
                kv2.get(&format!("key-{i:03}")).as_deref(),
                Some(format!("value-{i:03}").as_str())
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }

    // ===== TTL 过期测试 =====

    // TTL①：setex 后立即 get 有值；过期后 get 触发惰性删除
    #[test]
    fn ttl_basic_expiry() {
        let (dir, path) = unique_dir("ttl1");
        {
            let mut kv = KvStore::open(&path).unwrap();
            kv.setex("code".into(), 1, "abc".into()).unwrap();
            assert_eq!(kv.get("code").as_deref(), Some("abc"));
            // ttl 查询：1 秒过期刚写入，剩余应为 1（向上取整）
            assert_eq!(kv.ttl("code"), Some(1));
            std::thread::sleep(std::time::Duration::from_millis(1100));
            // 过期：get 触发惰性删除
            assert_eq!(kv.get("code"), None);
            assert_eq!(kv.ttl("code"), Some(-2));
            assert!(!kv.keys().contains(&"code".to_string()));
            assert_eq!(kv.len(), 0);
        }
        // 重启：宕机期间已过期 → 重放后清理
        let mut kv2 = KvStore::open(&path).unwrap();
        assert_eq!(kv2.get("code"), None);
        assert_eq!(kv2.len(), 0);
        let _ = fs::remove_dir_all(&dir);
    }

    // TTL②：expire 给已有 key 加过期；ttl 三态（剩余/-1 永久/-2 不存在）
    #[test]
    fn ttl_expire_and_three_states() {
        let (dir, path) = unique_dir("ttl2");
        let mut kv = KvStore::open(&path).unwrap();
        kv.set("perm".into(), "v".into()).unwrap();
        assert_eq!(kv.ttl("perm"), Some(-1)); // 永不过期
        assert_eq!(kv.ttl("missing"), Some(-2)); // 不存在
        // expire 已存在的 key
        assert!(kv.expire("perm", 3600).unwrap());
        assert_eq!(kv.ttl("perm"), Some(3600));
        // expire 不存在的 key → false，不写日志
        assert!(!kv.expire("ghost".into(), 10).unwrap());
        // set 覆盖会清除 TTL（Redis SET 语义）
        kv.set("perm".into(), "v2".into()).unwrap();
        assert_eq!(kv.ttl("perm"), Some(-1));
        let _ = fs::remove_dir_all(&dir);
    }

    // TTL③：TTL 跨重启——重启后剩余 TTL 依然正确、未过期的存活
    #[test]
    fn ttl_survives_restart() {
        let (dir, path) = unique_dir("ttl3");
        {
            let mut kv = KvStore::open(&path).unwrap();
            kv.setex("long".into(), 3600, "still-here".into()).unwrap();
            kv.setex("short".into(), 1, "will-expire".into()).unwrap();
            kv.set("perm".into(), "forever".into()).unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(1100)); // short 过期
        let mut kv2 = KvStore::open(&path).unwrap();
        assert_eq!(kv2.get("perm").as_deref(), Some("forever"));
        assert_eq!(kv2.get("long").as_deref(), Some("still-here"));
        // 剩余 TTL 应接近 3600（重放时按剩余时间重算）
        let t = kv2.ttl("long").unwrap();
        assert!(t >= 3590 && t <= 3600, "剩余 TTL 应约 3600，实际 {t}");
        assert_eq!(kv2.get("short"), None); // 重放后被清理
        assert_eq!(kv2.len(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    // TTL④：压缩保留 TTL；value 带逗号的 SETX 行解析正确（key 不允许逗号）
    #[test]
    fn ttl_compact_and_comma_value() {
        let (dir, path) = unique_dir("ttl4");
        {
            let mut kv = KvStore::open(&path).unwrap();
            kv.setex("withcomma".into(), 3600, "a,b,c".into()).unwrap();
            kv.setex("dead".into(), 1, "x".into()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(1100));
            kv.compact().unwrap(); // dead 已过期 → 压缩清除；withcomma 的 TTL 保留
        }
        let mut kv2 = KvStore::open(&path).unwrap();
        // value 带逗号：SETX,k,v,ts 从右边切时间戳，中间全是 value
        assert_eq!(kv2.get("withcomma").as_deref(), Some("a,b,c"));
        assert!(kv2.ttl("withcomma").unwrap() > 0, "压缩后 TTL 应保留");
        assert_eq!(kv2.get("dead"), None);
        assert_eq!(kv2.len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    // key 含逗号要拒绝：逗号是日志分隔符，含逗号的 key 落盘后会与分隔符混淆
    #[test]
    fn comma_key_rejected() {
        let (dir, path) = unique_dir("commakey");
        let mut kv = KvStore::open(&path).unwrap();
        assert!(kv.set("a,b".into(), "v".into()).is_err());
        assert!(kv.setex("a,b".into(), 10, "v".into()).is_err());
        assert!(kv.expire("a,b", 10).is_err());
        assert!(kv.del("a,b").is_err());
        // 拒绝的写入不落日志
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.is_empty(), "被拒绝的命令不应写日志，实际：{content}");
        let _ = fs::remove_dir_all(&dir);
    }

    // TTL⑤：旧格式日志（无 SETX/EXPIRE）向后兼容
    #[test]
    fn ttl_backward_compat_old_log() {
        let (dir, path) = unique_dir("ttl5");
        fs::write(&path, "SET,a,1\nDEL,b\nSET,c,3\n").unwrap();
        let mut kv = KvStore::open(&path).unwrap();
        assert_eq!(kv.get("a").as_deref(), Some("1"));
        assert_eq!(kv.get("c").as_deref(), Some("3"));
        assert_eq!(kv.ttl("a"), Some(-1)); // 旧数据永不过期
        let _ = fs::remove_dir_all(&dir);
    }
}
