//! kvstore 库入口 —— 按模块划分职责。
//! 成员 A 改 storage.rs，成员 B 改 protocol.rs + bin/client.rs，成员 C 改 bin/server.rs。

pub mod error;
pub mod protocol;
pub mod storage;

// 给 C 用的：线程安全的存储句柄别名
pub use storage::SharedKvStore;

// 导出给 bin/server.rs / bin/client.rs 直接用，不用每个人再 use kvstore::error::Result
pub use error::{KvError, Result};
