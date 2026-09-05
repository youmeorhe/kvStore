use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU16, Ordering::SeqCst};
use std::sync::Arc;
use std::time::Duration;

fn pick_port() -> u16 {
    static NEXT: AtomicU16 = AtomicU16::new(60000);
    let mut port = NEXT.fetch_add(1, SeqCst);
    loop {
        if let Ok(l) = std::net::TcpListener::bind(("127.0.0.1", port)) {
            drop(l);
            return port;
        }
        port = NEXT.fetch_add(1, SeqCst);
        if port > 64999 {
            NEXT.store(60000, SeqCst);
            port = 60000;
        }
    }
}

fn make_data_dir(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, SeqCst);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("integ-tests")
        .join(format!("{n}-{tag}"));
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn server_bin() -> PathBuf {
    let exe_suffix = if cfg!(windows) { ".exe" } else { "" };

    if let Ok(p) = std::env::var("CARGO_BIN_EXE_server") {
        let p = PathBuf::from(p);
        if p.exists() {
            return p;
        }
    }

    let fallback = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join(format!("server{exe_suffix}"));
    if fallback.exists() {
        return fallback.clone();
    }

    panic!(
        "找不到 server 二进制！请先执行 `cargo build`。\n查找路径: {}",
        fallback.display()
    );
}

struct ServerHandle {
    child: Option<Child>,
    pub addr: String,
    pub data_path: PathBuf,
    data_dir: PathBuf,
    port: u16,
}

impl ServerHandle {
    fn start(tag: &str) -> Self {
        let port = pick_port();
        let data_dir = make_data_dir(tag);
        let data_path = data_dir.join("kv.db");

        let bin = server_bin();
        let child = Command::new(&bin)
            .arg("--port")
            .arg(port.to_string())
            .arg("--data")
            .arg(&data_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|e| panic!("启动 server 子进程失败 ({bin:?}): {e}"));

        let addr = format!("127.0.0.1:{port}");
        let mut waited_ms = 0;
        while waited_ms < 3000 {
            if TcpStream::connect(&addr).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
            waited_ms += 50;
        }

        Self {
            child: Some(child),
            addr,
            data_path,
            data_dir,
            port,
        }
    }

    fn connect(&self) -> TcpStream {
        TcpStream::connect(&self.addr)
            .unwrap_or_else(|e| panic!("连不上测试 server {}: {e}", self.addr))
    }

    fn stop_preserve_data(mut self) -> (PathBuf, PathBuf, u16) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let ret = (self.data_dir.clone(), self.data_path.clone(), self.port);
        std::mem::forget(self);
        ret
    }

    fn restart(data_dir: &Path, data_path: &Path, port: u16) -> Self {
        let bin = server_bin();
        let child = Command::new(&bin)
            .arg("--port")
            .arg(port.to_string())
            .arg("--data")
            .arg(data_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("restart server 失败");
        let addr = format!("127.0.0.1:{port}");
        let mut waited_ms = 0;
        while waited_ms < 3000 {
            if TcpStream::connect(&addr).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
            waited_ms += 50;
        }
        Self {
            child: Some(child),
            addr,
            data_path: data_path.to_path_buf(),
            data_dir: data_dir.to_path_buf(),
            port,
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = std::fs::remove_dir_all(&self.data_dir);
    }
}

fn send_line(stream: &mut TcpStream, line: &str) -> String {
    let mut msg = line.to_string();
    msg.push('\n');
    stream.write_all(msg.as_bytes()).expect("写 server 失败");
    stream.flush().expect("flush 失败");

    let mut reader = BufReader::new(stream.try_clone().expect("clone tcp 失败"));
    let mut buf = String::new();
    let n = reader
        .read_line(&mut buf)
        .expect("读 server 响应失败（server 可能挂了）");
    if n == 0 {
        panic!("server 提前关闭了连接");
    }
    buf.trim_end_matches(&['\n', '\r'][..]).to_string()
}

trait Ignore {
    fn ignore(self);
}
impl Ignore for String {
    fn ignore(self) {}
}

#[test]
fn t01_basic_set_get_del() {
    let srv = ServerHandle::start("basic");
    let mut c = srv.connect();

    assert_eq!(send_line(&mut c, "set name Alice"), "OK");
    assert_eq!(send_line(&mut c, "get name"), "OK Alice");
    assert_eq!(send_line(&mut c, "set name Bob"), "OK");
    assert_eq!(send_line(&mut c, "get name"), "OK Bob");
    assert_eq!(send_line(&mut c, "del name"), "OK");
    assert_eq!(send_line(&mut c, "get name"), "NOTFOUND");
    assert_eq!(send_line(&mut c, "del name"), "NOTFOUND");
}

#[test]
fn t02_keys_and_status() {
    let srv = ServerHandle::start("keys");
    let mut c1 = srv.connect();
    let _c2 = srv.connect();

    send_line(&mut c1, "set banana v").ignore();
    send_line(&mut c1, "set apple v").ignore();
    send_line(&mut c1, "set cherry v").ignore();

    assert_eq!(send_line(&mut c1, "keys"), "OK apple,banana,cherry");
    let st = send_line(&mut c1, "status");
    assert!(st.starts_with("STATUS keys=3 clients="), "status={st}");
    let n_clients: usize = st
        .split("clients=")
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    assert!(n_clients >= 2, "活跃连接数应该 >= 2，status={st}");
}

#[test]
fn t03_bad_cmd_doesnt_break_connection() {
    let srv = ServerHandle::start("badcmd");
    let mut c = srv.connect();
    let e = send_line(&mut c, "foobar hello world");
    assert!(e.starts_with("ERR "), "错误命令应该返回 ERR: {e}");
    assert_eq!(send_line(&mut c, "ping"), "PONG");
    assert_eq!(send_line(&mut c, "get missing"), "NOTFOUND");
}

#[test]
fn t04_restart_recovery() {
    let (dir, dpath, port) = {
        let srv = ServerHandle::start("recovery");
        {
            let mut c = srv.connect();
            send_line(&mut c, "set course Rust程序设计").ignore();
            send_line(&mut c, "set team 4people").ignore();
            send_line(&mut c, "del team").ignore();
        }
        srv.stop_preserve_data()
    };
    let srv2 = ServerHandle::restart(&dir, &dpath, port);
    let mut c = srv2.connect();
    assert_eq!(send_line(&mut c, "get course"), "OK Rust程序设计");
    assert_eq!(send_line(&mut c, "get team"), "NOTFOUND");
}

#[test]
fn t06_clear_all_data() {
    let (dir, dpath, port) = {
        let srv = ServerHandle::start("clear");
        {
            let mut c = srv.connect();
            send_line(&mut c, "set a 1").ignore();
            send_line(&mut c, "set b 2").ignore();
            send_line(&mut c, "set c 3").ignore();
            assert_eq!(send_line(&mut c, "clear"), "OK 已清空 3 条数据");
            assert_eq!(send_line(&mut c, "keys"), "OK");
            assert!(send_line(&mut c, "status").starts_with("STATUS keys=0"));
            assert_eq!(send_line(&mut c, "clear"), "OK 已清空 0 条数据");
            assert_eq!(send_line(&mut c, "set d 4"), "OK");
        }
        srv.stop_preserve_data()
    };
    let srv2 = ServerHandle::restart(&dir, &dpath, port);
    let mut c = srv2.connect();
    assert_eq!(send_line(&mut c, "get a"), "NOTFOUND");
    assert_eq!(send_line(&mut c, "get d"), "OK 4");
    assert!(send_line(&mut c, "status").starts_with("STATUS keys=1"));
}

#[test]
fn t07_manual_compact() {
    let (dir, dpath, port) = {
        let srv = ServerHandle::start("compact");
        {
            let mut c = srv.connect();
            for i in 0..30 {
                send_line(&mut c, &format!("set hot v{i}")).ignore();
            }
            send_line(&mut c, "set keep hello").ignore();
            send_line(&mut c, "set gone temp").ignore();
            send_line(&mut c, "del gone").ignore();
            let size_before = std::fs::metadata(&srv.data_path).unwrap().len();
            let resp = send_line(&mut c, "compact");
            assert!(resp.starts_with("OK 日志压缩完成："), "实际响应：{resp}");
            assert_eq!(send_line(&mut c, "get hot"), "OK v29");
            assert_eq!(send_line(&mut c, "get keep"), "OK hello");
            assert_eq!(send_line(&mut c, "get gone"), "NOTFOUND");
            let size_after = std::fs::metadata(&srv.data_path).unwrap().len();
            assert!(
                size_after < size_before,
                "压缩后 {size_after}B 应小于压缩前 {size_before}B"
            );
            assert_eq!(send_line(&mut c, "set post ok"), "OK");
        }
        srv.stop_preserve_data()
    };
    let srv2 = ServerHandle::restart(&dir, &dpath, port);
    let mut c = srv2.connect();
    assert_eq!(send_line(&mut c, "get hot"), "OK v29");
    assert_eq!(send_line(&mut c, "get keep"), "OK hello");
    assert_eq!(send_line(&mut c, "get gone"), "NOTFOUND");
    assert_eq!(send_line(&mut c, "get post"), "OK ok");
    assert!(send_line(&mut c, "status").starts_with("STATUS keys=3"));
}

#[test]
fn t08_oversized_line_guard() {
    use std::io::{Read, Write};

    let srv = ServerHandle::start("oversize");
    let mut normal = srv.connect();
    assert_eq!(send_line(&mut normal, "ping"), "PONG");

    let mut evil = srv.connect();
    let payload = vec![b'A'; 2 * 1024 * 1024];
    evil.write_all(&payload).unwrap();
    evil.flush().unwrap();

    let mut buf = [0u8; 256];
    let n = evil.read(&mut buf).unwrap();
    let first = String::from_utf8_lossy(&buf[..n]);
    assert!(
        first.contains("ERR") && first.contains("1MB"),
        "应返回超长错误提示，实际：{first}"
    );
    match evil.read(&mut buf) {
        Ok(0) => {}
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {}
        other => panic!("连接应已断开（EOF 或 RST），实际：{other:?}"),
    }

    assert_eq!(send_line(&mut normal, "set still ok"), "OK");
    assert_eq!(send_line(&mut normal, "get still"), "OK ok");
    assert_eq!(send_line(&mut normal, "ping"), "PONG");
}

#[test]
fn t09_ttl_basic() {
    let srv = ServerHandle::start("ttl");
    let mut c = srv.connect();

    assert_eq!(send_line(&mut c, "set perm forever"), "OK");
    assert_eq!(send_line(&mut c, "ttl perm"), "OK -1");

    assert_eq!(send_line(&mut c, "setex code 2 abc"), "OK 已设置，2 秒后过期");
    assert_eq!(send_line(&mut c, "get code"), "OK abc");
    let t = send_line(&mut c, "ttl code");
    let secs: i64 = t.strip_prefix("OK ").unwrap().parse().unwrap();
    assert!(secs >= 1 && secs <= 2, "ttl 剩余应 1~2 秒，实际：{t}");

    assert_eq!(send_line(&mut c, "expire perm 2"), "OK 已设置，2 秒后过期");
    assert_eq!(send_line(&mut c, "expire missing 5"), "NOTFOUND");
    assert_eq!(send_line(&mut c, "ttl missing"), "OK -2");

    std::thread::sleep(Duration::from_secs(3));

    assert_eq!(send_line(&mut c, "get code"), "NOTFOUND");
    assert_eq!(send_line(&mut c, "get perm"), "NOTFOUND");
    assert_eq!(send_line(&mut c, "keys"), "OK");
    assert!(send_line(&mut c, "status").starts_with("STATUS keys=0"));
}

#[test]
fn t05_concurrent_clients() {
    use std::sync::Barrier;

    let srv = ServerHandle::start("conc");
    let addr = srv.addr.clone();
    const N: usize = 8;
    const PER: usize = 200;

    let barrier = Arc::new(Barrier::new(N));
    let mut threads = Vec::with_capacity(N);

    for i in 0..N {
        let b = Arc::clone(&barrier);
        let addr = addr.clone();
        threads.push(std::thread::spawn(move || {
            let mut s =
                TcpStream::connect(&addr).unwrap_or_else(|e| panic!("线程{i} 连不上 server: {e}"));
            b.wait();
            for j in 0..PER {
                let k = format!("t{i}_k{j}");
                let v = format!("v{i}_{j}");
                let r1 = send_line(&mut s, &format!("set {k} {v}"));
                assert_eq!(r1, "OK", "线程{i} set 失败: {r1}");
                let r2 = send_line(&mut s, &format!("get {k}"));
                assert_eq!(r2, format!("OK {v}"), "线程{i} get 不一致");
            }
        }));
    }
    for t in threads {
        t.join().expect("线程 panic");
    }

    let mut c = srv.connect();
    let st = send_line(&mut c, "status");
    let expected = format!("keys={}", N * PER);
    assert!(
        st.contains(&expected),
        "并发写入后数据条数应该 {expected}，实际 status={st}"
    );
}
