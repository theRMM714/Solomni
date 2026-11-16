//! 目的：宿主侧工具与网络开关的取法——QEMU / virtiofsd / smbd 路径、宿主只读根。
//! 管：取登记值并转填补齐面的 `tool_path`、网络开关、只读根解析。
//! 不管：guest 怎么起（`create.rs` / vm 能力）、依赖怎么装与宿主工具兜底（provision）。
//! 联动：建工作与 guest 启动装配命令行时读这里。
//!   本方法族只碰 registry / provision 这两个字段。

use super::*;
use crate::capabilities::provision::api::HostTool;

impl Conductor {
    /// 目的：本机实际要用的 QEMU 可执行文件：设置里登记了就用它，没登记就采用依赖补齐装进项目内的那一个。
    /// 约束：**每次判据取用时现算**（不是启动时定一次）——补齐作业装完即生效，不需要重启进程；
    ///   显式登记永远优先，这里不改用户的 `settings.yaml`。
    pub(crate) fn qemu_path(&self) -> Option<String> {
        let app = self.registry.app();
        self.provision
            .tool_path(HostTool::Qemu, app.qemu_path.trim())
    }

    /// 目的：同 `qemu_path`，virtiofs 守护进程。
    pub(crate) fn virtiofsd_path(&self) -> Option<String> {
        let app = self.registry.app();
        self.provision
            .tool_path(HostTool::Virtiofsd, app.virtiofsd_path.trim())
    }

    /// 目的：同 `qemu_path`，SMB helper。
    pub(crate) fn smbd_path(&self) -> Option<String> {
        let app = self.registry.app();
        self.provision
            .tool_path(HostTool::Smbd, app.smbd_path.trim())
    }

    /// 目的：本平台共享档要用的外部工具路径（Windows = smbd；其余 = virtiofsd）；都不在场给 None。
    /// 约束：组合选择与就绪检测共用这一份，避免"说要验 A、实际跑 B"。
    pub(crate) fn share_tool_path(&self) -> Option<String> {
        if crate::kernel::api::IS_WINDOWS {
            self.smbd_path()
        } else {
            self.virtiofsd_path()
        }
    }

    /// 目的：全局网络开关（工具 / guest / 常驻服务 / 构建都看它）。
    pub(crate) fn network_allowed(&self) -> bool {
        self.registry.app().sandbox.policy.network
    }

    /// 目的：用户显式授权的主机只读根（`settings.yaml` 的 `sandbox.policy.host_read_roots`）。
    ///  策略层只带事实：哪些目录只读可达由用户定，只读位怎么落由适配层定。
    ///  空 = 一个都不放行（默认不动本机任何权限项）。
    pub(crate) fn host_read_roots(&self) -> Vec<std::path::PathBuf> {
        self.registry
            .app()
            .sandbox
            .policy
            .host_read_roots
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(std::path::PathBuf::from)
            .collect()
    }
}
