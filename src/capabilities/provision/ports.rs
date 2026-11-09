//! 目的：provision 的出站端口——编排需要什么就声明什么，实现放在 detail。
//! 管：取数（网络/摘要）这一组抽象；下游按端口签名实现，service 只按语义调用。
//! 不管：具体协议、超时、代理（在 detail::fetch）；清单怎么解析（在 service）。
//! 联动：service 持有 Arc<dyn Fetcher>；组合根构造 detail::fetch::HttpFetch 注入。

use std::path::Path;

/// 目的：取数与摘要——下载资产、读目录页文本、算文件 sha256。
pub trait Fetcher: Send + Sync {
    /// 目的：取一个文本页（列包名用）；非 2xx 如实报错。
    fn text(&self, url: &str) -> Result<String, String>;

    /// 目的：算文件摘要（小写十六进制）——完整性判据。
    fn digest(&self, path: &Path) -> Result<String, String>;

    /// 目的：把 URL 下到目标文件——选源后带续传；半截文件留作下次断点。
    fn download(&self, url: &str, dest: &Path, on_line: &mut dyn FnMut(&str))
        -> Result<(), String>;
}

/// 目的：宿主文件系统的少数落点操作——编排只表达"建这个目录 / 删这个文件"，具体错误文案在适配层。
pub trait HostFs: Send + Sync {
    /// 目的：这个路径在不在（文件或目录都算）。
    fn exists(&self, path: &Path) -> bool;
    fn is_file(&self, path: &Path) -> bool;
    fn is_dir(&self, path: &Path) -> bool;
    fn mkdir_all(&self, path: &Path) -> Result<(), String>;
    fn remove_file(&self, path: &Path) -> Result<(), String>;
    fn remove_dir_all(&self, path: &Path) -> Result<(), String>;
    /// 目的：给已落盘的文件补执行位（unix 0644 → 0755；别的平台空操作）。
    fn make_executable(&self, path: &Path);
}

/// 目的：安装台账与在场的判定——编排只问"这条装好了吗 / 记下装好了 / 清掉它"，落盘格式在适配层。
/// 约束：`snapshot` 与 `record` / `forget` 都以磁盘为准，编排不缓存台账。
pub trait Ledger: Send + Sync {
    /// 目的：当前台账（依赖 id → 安装身份）。
    fn snapshot(&self) -> std::collections::BTreeMap<String, String>;
    /// 目的：这条依赖的探针是否已在场（文件或目录）。
    fn present(&self, dep: &crate::capabilities::provision::domain::Dep) -> bool;
    /// 目的：读一个虚拟根目录里的画像（`guest.yaml`；缺省 = 空画像）。
    fn profile(&self, base: &Path) -> crate::capabilities::provision::domain::GuestProfile;
    /// 目的：登记"这条按当前身份装好了"——每装好一条就落一次盘。
    fn record(&self, dep: &crate::capabilities::provision::domain::Dep) -> Result<(), String>;
    /// 目的：清掉这条依赖自己产出的那份东西（只动它自己的探针 / 安装目录）。
    fn clear(&self, dep: &crate::capabilities::provision::domain::Dep) -> Result<(), String>;
    /// 目的：从台账里划掉一条（旧版本重装前）。
    fn forget(&self, id: &str) -> Result<(), String>;
    /// 目的：把已在场的基础工具链登记进 `.tools/toolchains/active.json`（启动期只读这一份）。
    fn sync_active(
        &self,
        all: &[crate::capabilities::provision::domain::Dep],
    ) -> Result<(), String>;
}

/// 目的：本能力要观察的长驻子进程——编排只问"现在有哪些、有几个 guest 在跑"。
pub trait Procs: Send + Sync {
    /// 目的：台账里的全部条目，逐条带上"此刻还活着吗"。
    fn entries(&self) -> Vec<crate::capabilities::provision::api::ProcInfo>;
    /// 目的：有没有 purpose = guest 的进程此刻还活着（还原虚拟根前的前置检查）。
    fn guest_running(&self) -> bool;
}

/// 目的：按安装方式落盘——编排只按清单的 `kind` 选一个方法，机制与外部工具在适配层。
pub trait Installers: Send + Sync {
    /// 目的：归档解包（`strip` = 去掉的路径层数）。
    fn extract(&self, archive: &Path, dest: &Path, strip: usize) -> Result<(), String>;
    /// 目的：单文件依赖——按 probe 的文件名复制进安装目录。
    fn file(&self, cached: &Path, dest: &Path, probe: &str) -> Result<(), String>;
    /// 目的：AppImage 自解包并把内容搬进安装目录（保持原布局）。
    fn appimage(
        &self,
        cached: &Path,
        dest: &Path,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(), String>;
    /// 目的：基础工具链——解归档并按需写编译器包装。
    fn toolchain(
        &self,
        cached: &Path,
        dest: &Path,
        dep: &crate::capabilities::provision::domain::Dep,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(), String>;
    /// 目的：guest 磁盘——单文件落盘并写一份画像（运行期状态在里面，不覆盖）。
    fn guest_disk(
        &self,
        cached: &Path,
        dest: &Path,
        probe: &str,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(), String>;
}

/// 目的：guest 根文件系统镜像——按清单把镜像装出来，服务层不碰 ext4 工具链。
pub trait GuestImage: Send + Sync {
    /// 目的：按清单条目把 rootfs 镜像装出来（内核 / QGA / initrd / 落盘）。
    fn install(
        &self,
        dep: &crate::capabilities::provision::domain::Dep,
        cached: &Path,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(), String>;
}
