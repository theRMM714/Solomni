//! 目的：虚拟机档两件外部工具的位置事实——按设置里的登记取齐，供就绪判据与启动装配共用。
//! 管：把设置值交给补齐面解析成 `VmTools`（登记值 + 项目内补齐那份），以及取落地路径给命令行。
//! 不管：guest 怎么起（`create.rs` / vm 能力）、依赖怎么装（provision）。
//! 联动：建工作、会话配置、运行报告与 guest 启动装配读这里；本方法族只碰 registry / provision 两个字段。

use super::*;

impl Work {
    /// 目的：本机虚拟机档的两件工具位置（一份口径：设置登记 + 项目内补齐；平台差异在 provision）。
    /// 约束：**每次判据取用时现算**——补齐作业装完即生效，不需要重启进程；显式登记永远优先。
    pub(crate) fn virtual_tools(&self, registry: &dyn Registry) -> crate::units::vm::api::VmTools {
        let app = registry.app();
        crate::units::provision::api::vm_tools(
            self.provision.as_ref(),
            std::env::consts::OS,
            &app.qemu_path,
            &app.virtiofsd_path,
            &app.smbd_path,
        )
    }
}
