use std::collections::BTreeMap;
use std::fs::{create_dir_all, File, OpenOptions};
use std::io::{BufRead, BufReader, Write as IoWrite};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};

fn validate_key(key: &str) -> Result<()> {
    if key.trim().is_empty() {
        return Err(anyhow!("key 不能为空"));
    }
    if key.contains(',') {
        return Err(anyhow!("key 不能包含逗号（日志分隔符），请使用其他字符"));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub struct ValueEntry {
    pub value: String,
    pub expire_at: Option<Instant>,
}

impl ValueEntry {
    pub fn permanent(value: String) -> Self {
        Self { value, expire_at: None }
    }
    pub fn alive(&self) -> bool {
        match self.expire_at {
            None => true,
            Some(t) => Instant::now() < t,
        }
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn unix_ms_after_secs(secs: u64) -> u64 {
    now_unix_ms() + secs * 1000
}

fn instant_from_unix_ms(ms: u64) -> Instant {
    let now = now_unix_ms();
    if ms <= now {
        Instant::now()
    } else {
        Instant::now() + Duration::from_millis(ms - now)
    }
}

pub struct KvStore {
    pub data: BTreeMap<String, ValueEntry>,
    pub log_file: Mutex<File>,
    pub path: std::path::PathBuf,
}

pub type SharedKvStore = std::sync::Arc<std::sync::Mutex<KvStore>>;

fn compact_tmp_path(path: &Path) -> std::path::PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".tmp");
    std::path::PathBuf::from(s)
}

impl KvStore {
    pub fn open(path: &str) -> Result<Self> {
        let path_obj = Path::new(path);
        if let Some(parent) = path_obj.parent() {
            create_dir_all(parent)?;
        }

        let mut data = BTreeMap::new();

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
                    let (key, value) = rest
                        .split_once(',')
                        .ok_or_else(|| anyhow!("日志格式错误：SET 行需要 key 和 value，实际为 '{}'", line))?;
                    if key.is_empty() {
                        return Err(anyhow!("日志格式错误：SET 行 key 为空，实际为 '{}'", line));
                    }
                    data.insert(key.to_string(), ValueEntry::permanent(value.to_string()));
                } else if let Some(rest) = line.strip_prefix("SETX,") {
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
                    if rest.is_empty() {
                        return Err(anyhow!("日志格式错误：DEL 行需要 key，实际为 '{}'", line));
                    }
                    data.remove(rest);
                } else {
                    return Err(anyhow!("未知日志行格式：'{}'", line));
                }
            }
            let expired: Vec<String> = data
                .iter()
                .filter(|(_, e)| !e.alive())
                .map(|(k, _)| k.clone())
                .collect();
            for k in expired {
                data.remove(&k);
            }
        }

        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(path_obj)?;

        let tmp_path = compact_tmp_path(path_obj);
        let _ = std::fs::remove_file(&tmp_path);

        Ok(KvStore {
            data,
            log_file: Mutex::new(file),
            path: path_obj.to_path_buf(),
        })
    }

    pub fn set(&mut self, key: String, value: String) -> Result<()> {
        validate_key(&key)?;

        let log_line = format!("SET,{},{}\n", key, value);
        {
            let mut file_guard = self.log_file.lock().unwrap();
            file_guard.write_all(log_line.as_bytes())?;
            file_guard.flush()?;
            file_guard.sync_all()?;
        }

        self.data.insert(key, ValueEntry::permanent(value));
        Ok(())
    }

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

    pub fn expire(&mut self, key: &str, secs: u64) -> Result<bool> {
        validate_key(key)?;
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

    pub fn ttl(&self, key: &str) -> Option<i64> {
        match self.data.get(key) {
            None => Some(-2),
            Some(e) if !e.alive() => Some(-2),
            Some(e) => match e.expire_at {
                None => Some(-1),
                Some(t) => {
                    let remain = t.saturating_duration_since(Instant::now());
                    Some(remain.as_secs() as i64 + if remain.subsec_millis() > 0 { 1 } else { 0 })
                }
            },
        }
    }

    pub fn del(&mut self, key: &str) -> Result<bool> {
        validate_key(key)?;

        let alive = self.data.get(key).is_some_and(|e| e.alive());
        if alive {
            let log_line = format!("DEL,{}\n", key);
            {
                let mut file_guard = self.log_file.lock().unwrap();
                file_guard.write_all(log_line.as_bytes())?;
                file_guard.flush()?;
                file_guard.sync_all()?;
            }

            self.data.remove(key);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn get(&mut self, key: &str) -> Option<String> {
        match self.data.get(key) {
            Some(e) if e.alive() => Some(e.value.clone()),
            Some(_) => {
                self.data.remove(key);
                None
            }
            None => None,
        }
    }

    pub fn keys(&self) -> Vec<String> {
        self.data
            .iter()
            .filter(|(_, e)| e.alive())
            .map(|(k, _)| k.clone())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.data.iter().filter(|(_, e)| e.alive()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&mut self) -> Result<usize> {
        let removed = self.data.len();
        let trunc = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&self.path)?;
        trunc.sync_all()?;
        drop(trunc);
        self.data.clear();
        Ok(removed)
    }

    pub fn file_size(&self) -> Result<u64> {
        let f = self.log_file.lock().unwrap();
        Ok(f.metadata()?.len())
    }

    pub fn compact(&mut self) -> Result<(u64, u64)> {
        let before = self.file_size()?;
        let tmp_path = compact_tmp_path(&self.path);

        {
            let mut tmp = File::create(&tmp_path)?;
            for (k, e) in &self.data {
                if !e.alive() {
                    continue;
                }
                match e.expire_at {
                    None => writeln!(tmp, "SET,{},{}", k, e.value)?,
                    Some(t) => {
                        let remain = t.saturating_duration_since(Instant::now());
                        let ms = now_unix_ms() + remain.as_millis() as u64;
                        writeln!(tmp, "SETX,{},{},{}", k, e.value, ms)?;
                    }
                }
            }
            tmp.sync_all()?;
        }

        let new_handle = OpenOptions::new().append(true).open(&tmp_path)?;
        let old = std::mem::replace(&mut self.log_file, Mutex::new(new_handle));
        drop(old);

        std::fs::rename(&tmp_path, &self.path)?;

        let after = self.file_size()?;
        Ok((before, after))
    }
}

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

    #[test]
    fn empty_key_rejected() {
        let (dir, path) = unique_dir("emptykey");
        let mut kv = KvStore::open(&path).unwrap();
        assert!(kv.set("".into(), "v".into()).is_err());
        assert!(kv.set("   ".into(), "v".into()).is_err());
        assert!(kv.del("").is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn clear_then_restart_is_empty() {
        let (dir, path) = unique_dir("clear");
        {
            let mut kv = KvStore::open(&path).unwrap();
            kv.set("a".into(), "1".into()).unwrap();
            kv.set("b".into(), "2".into()).unwrap();
            kv.set("c".into(), "3".into()).unwrap();
            assert_eq!(kv.clear().unwrap(), 3);
            assert_eq!(kv.len(), 0);
            assert!(kv.is_empty());
            kv.set("d".into(), "4".into()).unwrap();
        }
        let mut kv2 = KvStore::open(&path).unwrap();
        assert_eq!(kv2.len(), 1);
        assert_eq!(kv2.get("d").as_deref(), Some("4"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn clear_empty_is_ok() {
        let (dir, path) = unique_dir("clearempty");
        let mut kv = KvStore::open(&path).unwrap();
        assert_eq!(kv.clear().unwrap(), 0);
        assert!(kv.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn compact_shrinks_overwritten_log() {
        let (dir, path) = unique_dir("compact1");
        {
            let mut kv = KvStore::open(&path).unwrap();
            for i in 0..50 {
                kv.set("hot".into(), format!("value-{i}")).unwrap();
            }
            let (before, after) = kv.compact().unwrap();
            assert!(before >= 50 * 15, "压缩前应至少 50 行");
            assert_eq!(after, "SET,hot,value-49\n".len() as u64, "压缩后只 1 行");
            let content = fs::read_to_string(&path).unwrap();
            assert_eq!(content, "SET,hot,value-49\n");
            let tmp = std::path::Path::new(&dir).join("kv.db.tmp");
            assert!(!tmp.exists(), "压缩后不应残留 tmp 文件");
            let (b2, a2) = kv.compact().unwrap();
            assert_eq!(b2, a2);
            assert_eq!(a2, after);
        }
        let mut kv2 = KvStore::open(&path).unwrap();
        assert_eq!(kv2.get("hot").as_deref(), Some("value-49"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn compact_then_restart_keeps_data() {
        let (dir, path) = unique_dir("compact2");
        {
            let mut kv = KvStore::open(&path).unwrap();
            for i in 0..30 {
                kv.set("a".into(), format!("va{i}")).unwrap();
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
        assert_eq!(kv2.get("a").as_deref(), Some("va29"));
        assert_eq!(kv2.get("gone"), None);
        assert_eq!(kv2.len(), 3);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn compact_then_continue_writes() {
        let (dir, path) = unique_dir("compact3");
        {
            let mut kv = KvStore::open(&path).unwrap();
            for i in 0..20 {
                kv.set("x".into(), format!("v{i}")).unwrap();
            }
            kv.compact().unwrap();
            kv.set("post".into(), "after-compact".into()).unwrap();
            kv.del("x").unwrap();
            assert_eq!(kv.get("post").as_deref(), Some("after-compact"));
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

    #[test]
    fn compact_all_live_data_is_stable() {
        let (dir, path) = unique_dir("compact4");
        {
            let mut kv = KvStore::open(&path).unwrap();
            for i in 0..60 {
                kv.set(format!("key-{i:03}"), format!("value-{i:03}")).unwrap();
            }
            let (before, after) = kv.compact().unwrap();
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

    #[test]
    fn ttl_basic_expiry() {
        let (dir, path) = unique_dir("ttl1");
        {
            let mut kv = KvStore::open(&path).unwrap();
            kv.setex("code".into(), 1, "abc".into()).unwrap();
            assert_eq!(kv.get("code").as_deref(), Some("abc"));
            assert_eq!(kv.ttl("code"), Some(1));
            std::thread::sleep(std::time::Duration::from_millis(1100));
            assert_eq!(kv.get("code"), None);
            assert_eq!(kv.ttl("code"), Some(-2));
            assert!(!kv.keys().contains(&"code".to_string()));
            assert_eq!(kv.len(), 0);
        }
        let mut kv2 = KvStore::open(&path).unwrap();
        assert_eq!(kv2.get("code"), None);
        assert_eq!(kv2.len(), 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ttl_expire_and_three_states() {
        let (dir, path) = unique_dir("ttl2");
        let mut kv = KvStore::open(&path).unwrap();
        kv.set("perm".into(), "v".into()).unwrap();
        assert_eq!(kv.ttl("perm"), Some(-1));
        assert_eq!(kv.ttl("missing"), Some(-2));
        assert!(kv.expire("perm", 3600).unwrap());
        assert_eq!(kv.ttl("perm"), Some(3600));
        assert!(!kv.expire("ghost".into(), 10).unwrap());
        kv.set("perm".into(), "v2".into()).unwrap();
        assert_eq!(kv.ttl("perm"), Some(-1));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ttl_survives_restart() {
        let (dir, path) = unique_dir("ttl3");
        {
            let mut kv = KvStore::open(&path).unwrap();
            kv.setex("long".into(), 3600, "still-here".into()).unwrap();
            kv.setex("short".into(), 1, "will-expire".into()).unwrap();
            kv.set("perm".into(), "forever".into()).unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let mut kv2 = KvStore::open(&path).unwrap();
        assert_eq!(kv2.get("perm").as_deref(), Some("forever"));
        assert_eq!(kv2.get("long").as_deref(), Some("still-here"));
        let t = kv2.ttl("long").unwrap();
        assert!(t >= 3590 && t <= 3600, "剩余 TTL 应约 3600，实际 {t}");
        assert_eq!(kv2.get("short"), None);
        assert_eq!(kv2.len(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ttl_compact_and_comma_value() {
        let (dir, path) = unique_dir("ttl4");
        {
            let mut kv = KvStore::open(&path).unwrap();
            kv.setex("withcomma".into(), 3600, "a,b,c".into()).unwrap();
            kv.setex("dead".into(), 1, "x".into()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(1100));
            kv.compact().unwrap();
        }
        let mut kv2 = KvStore::open(&path).unwrap();
        assert_eq!(kv2.get("withcomma").as_deref(), Some("a,b,c"));
        assert!(kv2.ttl("withcomma").unwrap() > 0, "压缩后 TTL 应保留");
        assert_eq!(kv2.get("dead"), None);
        assert_eq!(kv2.len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn comma_key_rejected() {
        let (dir, path) = unique_dir("commakey");
        let mut kv = KvStore::open(&path).unwrap();
        assert!(kv.set("a,b".into(), "v".into()).is_err());
        assert!(kv.setex("a,b".into(), 10, "v".into()).is_err());
        assert!(kv.expire("a,b", 10).is_err());
        assert!(kv.del("a,b").is_err());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.is_empty(), "被拒绝的命令不应写日志，实际：{content}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ttl_backward_compat_old_log() {
        let (dir, path) = unique_dir("ttl5");
        fs::write(&path, "SET,a,1\nDEL,b\nSET,c,3\n").unwrap();
        let mut kv = KvStore::open(&path).unwrap();
        assert_eq!(kv.get("a").as_deref(), Some("1"));
        assert_eq!(kv.get("c").as_deref(), Some("3"));
        assert_eq!(kv.ttl("a"), Some(-1));
        let _ = fs::remove_dir_all(&dir);
    }
}
