//! 项目统一错误类型（零依赖 MVP 版，替代 anyhow）
//! 支持：
//!   - 任意 String 做错误信息
//!   - 自动从 std::io::Error 转（? 操作符直接用）
//!   - Debug/Display 正常打印
//! 未来要换 anyhow 的话，全局 `s/KvError/anyhow::Error/g` 就行。

use std::fmt;
use std::io;

#[derive(Debug)]
pub struct KvError(pub String);

impl fmt::Display for KvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for KvError {}

impl From<String> for KvError {
    fn from(s: String) -> Self { KvError(s) }
}

impl From<&str> for KvError {
    fn from(s: &str) -> Self { KvError(s.to_string()) }
}

impl From<io::Error> for KvError {
    fn from(e: io::Error) -> Self { KvError(format!("IO 错误: {e}")) }
}

impl From<std::sync::PoisonError<std::sync::MutexGuard<'_, ()>>> for KvError {
    fn from(_: std::sync::PoisonError<std::sync::MutexGuard<'_, ()>>) -> Self {
        KvError("锁已损坏（其他线程 panic）".into())
    }
}

/// 项目统一的 Result 别名，所有返回值都用它。
pub type Result<T> = std::result::Result<T, KvError>;

/// 快捷构建错误 + 问号抛错（模拟 anyhow::anyhow! 宏）
#[macro_export]
macro_rules! kverr {
    ($($t:tt)*) => {
        $crate::error::KvError(format!($($t)*)).into()
    };
}
