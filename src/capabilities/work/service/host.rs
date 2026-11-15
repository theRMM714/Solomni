//! 目的：宿主侧外部工具的取法——QEMU / virtiofsd / smbd 的路径与共享档工具选择。
//! 管：读设置里的登记值并转调依赖补齐能力；按平台选共享档要用的那一个。
//! 不管：登记值优先与装了兜底的顺序（`provision` 的 `tool_path`）、guest 怎么起（`create.rs` / vm 能力）、依赖怎么装。
//! 联动：建工作与 guest 启动装配命令行时读这里。

use super::*;
use crate::capabilities::provision::api::HostTool;

impl Work {
    /// 目的：本机实际要用的 QEMU 可执行文件：设置里登记了就用它，没登记就由依赖补齐能力给兜底。
    /// 约束：**每次判据取用时现算**（不是启动时定一次）——补齐作业装完即生效，不需要重启进程；
    ///   显式登记永远优先，这里不改用户的 `settings.yaml`。
    pub(crate) fn qemu_path(&self, registry: &dyn Registry) -> Option<String> {
        self.provision
            .tool_path(HostTool::Qemu, registry.app().qemu_path.trim())
    }

    /// 目的：同 `qemu_path`，virtiofs 守护进程。
    pub(crate) fn virtiofsd_path(&self, registry: &dyn Registry) -> Option<String> {
        self.provision
            .tool_path(HostTool::Virtiofsd, registry.app().virtiofsd_path.trim())
    }

    /// 目的：同 `qemu_path`，SMB helper。
    pub(crate) fn smbd_path(&self, registry: &dyn Registry) -> Option<String> {
        self.provision
            .tool_path(HostTool::Smbd, registry.app().smbd_path.trim())
    }

    /// 目的：本平台共享档要用的外部工具路径（Windows = smbd；其余 = virtiofsd）；都不在场给 None。
    /// 约束：组合选择与就绪检测共用这一份，避免"说要验 A、实际跑 B"。
    pub(crate) fn share_tool_path(&self, registry: &dyn Registry) -> Option<String> {
        if crate::kernel::api::IS_WINDOWS {
            self.smbd_path(registry)
        } else {
            self.virtiofsd_path(registry)
        }
    }
}
