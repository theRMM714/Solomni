//! 目的：删除会话的收尾段——先停 guest，再删会话目录，最后把围栏撤权甩到后台。
//! 管：`SessionDelete`（一次收尾的装配）与 `FenceRelease`（按租约逐个撤权）；调用方线程只起线程。
//! 不管：删前回收什么、子树怎么算（`conductor` 的 `delete_bookkeeping`）；会话存储本身（`session`）。
//! 联动：`conductor` 的 `delete_bookkeeping` 产出 `SessionDelete`，由调用方线程 `run`。

use std::sync::Arc;

use crate::capabilities::session::api::History;
use crate::kernel::ports::Log;

/// 目的：按租约撤围栏授权的回调——由核心把 `ToolExec::release_by_lease` 包成闭包注入，测试可直接换成记录型替身。
pub type ReleaseByLease = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// 目的：删除收尾里的围栏撤权段：租约清单 + 撤销回调 + 失败日志。
/// 约束：在调用方线程上**只起线程**；真正撤权在后台跑，进程退出时线程被杀留下的无归属条目由启动对账回收。
pub struct FenceRelease {
    leases: Vec<String>,
    release: ReleaseByLease,
    log: Arc<dyn Log + Send + Sync>,
}

impl FenceRelease {
    /// 目的：组装备好的撤权段（核心侧调用，毫秒级）。
    /// 参数：`leases` 待撤权的会话租约；`release` 撤销回调；`log` 失败日志。
    pub fn new(
        leases: Vec<String>,
        release: ReleaseByLease,
        log: Arc<dyn Log + Send + Sync>,
    ) -> FenceRelease {
        FenceRelease {
            leases,
            release,
            log,
        }
    }

    /// 目的：起后台线程按租约逐个撤权；线程起不来就同步兜底，不能把撤权丢掉。
    pub fn spawn(self) {
        if self.leases.is_empty() {
            return;
        }
        let leases = self.leases;
        let release = Arc::clone(&self.release);
        let log = Arc::clone(&self.log);
        let worker = {
            let leases = leases.clone();
            let release = Arc::clone(&release);
            let log = Arc::clone(&log);
            move || {
                for lease in leases {
                    if let Err(e) = release(&lease) {
                        log.warn(
                            "lifecycle::release_fences",
                            &format!("按会话撤销围栏授权未完成（{}）：{}", lease, e),
                        );
                    }
                }
            }
        };
        match std::thread::Builder::new()
            .name("fence-release".to_string())
            .spawn(worker)
        {
            Ok(_) => {}
            Err(e) => {
                log.warn(
                    "lifecycle::release_fences",
                    &format!("后台撤权线程起不来，改为同步撤权：{}", e),
                );
                for lease in &leases {
                    if let Err(e) = release(lease) {
                        log.warn(
                            "lifecycle::release_fences",
                            &format!("按会话撤销围栏授权未完成（{}）：{}", lease, e),
                        );
                    }
                }
            }
        }
    }
}

/// 目的：删除会话的收尾——**先停 guest，再删会话目录，最后把撤权甩到后台**。
/// 约束：在调用方线程上执行；Windows 上 guest / 共享进程可能还占着会话目录里的文件，顺序反过来会 sharing violation。
///   先删目录让产品根内的授权自动走「路径不在 → 只销账」快分支；外部根的递归撤权不占用户等待路径。
pub struct SessionDelete {
    stop: Option<Box<dyn FnOnce() + Send>>,
    history: Arc<dyn History + Send + Sync>,
    /// 目的：待删目录（已按**先子后父**排好）。
    dirs: Vec<String>,
    /// 目的：调用方要的“目标会话是否真的删掉了”。
    target: String,
    /// 目的：围栏撤权段；没有租约或测试替身不需要时为 None。
    fences: Option<FenceRelease>,
}

impl SessionDelete {
    /// 目的：组装备好的一次收尾（核心侧调用，毫秒级）。
    /// 参数：`stop` 停 guest 的闭包，没有要停的 guest 时为 `None`；`history` 会话存储；
    ///  `dirs` 待删目录（已按先子后父排好）；`target` 用户点删的那个会话；`fences` 撤权段。
    pub fn new(
        stop: Option<Box<dyn FnOnce() + Send>>,
        history: Arc<dyn History + Send + Sync>,
        dirs: Vec<String>,
        target: String,
        fences: Option<FenceRelease>,
    ) -> SessionDelete {
        SessionDelete {
            stop,
            history,
            dirs,
            target,
            fences,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        stop: Option<Box<dyn FnOnce() + Send>>,
        history: Arc<dyn History + Send + Sync>,
        dirs: Vec<String>,
        target: String,
    ) -> SessionDelete {
        SessionDelete {
            stop,
            history,
            dirs,
            target,
            fences: None,
        }
    }

    /// 目的：测试用构造：带一个可观察的撤权回调，钉住「后台执行、目录已删、失败也撤」三条契约。
    #[cfg(test)]
    pub(crate) fn for_test_with_fences(
        stop: Option<Box<dyn FnOnce() + Send>>,
        history: Arc<dyn History + Send + Sync>,
        dirs: Vec<String>,
        target: String,
        leases: Vec<String>,
        release: ReleaseByLease,
        log: Arc<dyn Log + Send + Sync>,
    ) -> SessionDelete {
        SessionDelete {
            stop,
            history,
            dirs,
            target,
            fences: Some(FenceRelease {
                leases,
                release,
                log,
            }),
        }
    }

    /// 目的：执行收尾：先停 guest（若调用方注入），再先子后父删会话目录，最后后台按租约撤权。
    pub fn run(self) -> Result<bool, String> {
        let SessionDelete {
            stop,
            history,
            dirs,
            target,
            fences,
        } = self;
        if let Some(stop) = stop {
            stop();
        }
        let mut result = Ok(false);
        for sid in dirs {
            match history.delete(&sid) {
                Ok(ok) => {
                    if sid == target {
                        result = Ok(ok);
                    }
                }
                Err(e) => {
                    // 逐目录早退：删到一半失败就停，但撤权仍要执行。
                    result = Err(e);
                    break;
                }
            }
        }
        if let Some(fences) = fences {
            fences.spawn();
        }
        result
    }
}
