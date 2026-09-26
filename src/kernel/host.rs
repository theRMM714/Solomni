//! 宿主能力探测：**只问事实**——不执行任何程序、不安装、不写任何东西。
//! 机制（读环境变量、查路径存在性、按平台判定虚拟化能力）在适配层；
//! 只按结论做判断，所以调用方不出现 std::env 与 is_file / is_dir。
//! 为什么在 kernel：它不认识任何业务概念，且被「执行档位」与自检共用。

use std::path::Path;

pub trait HostProbe: Send + Sync {
    /// 这个路径存在且是文件。
    fn is_file(&self, path: &Path) -> bool;
    /// 这个路径存在且是目录。
    fn is_dir(&self, path: &Path) -> bool;
    /// PATH 上有没有这个可执行文件（只查存在性，不执行它；平台扩展名由适配层处理）。
    fn has_exe(&self, name: &str) -> bool;
    /// 本机能不能起硬件虚拟化（只问事实，不起任何虚拟机）。
    fn hypervisor_available(&self) -> bool;
}
