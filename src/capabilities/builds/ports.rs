//! 目的：builds 能力的出站端口。
//! 管：SourceDigest（源指纹）、BuildStates（构建状态与按次家目录）、BuildCache（缓存卷、构建锁、guest 侧工作区）。
//! 不管：什么时候重编、走本机档还是虚拟机档（在 `service.rs`）；状态文件与脚本的文本（在 `detail/`）。
//! 联动：由 `service.rs` 持有（R12）；实现是 `detail/fingerprint.rs` 的 `BuildStore` 与 `detail/cache.rs` 的 `CacheOnDisk`。

use crate::capabilities::modules::api::{BuildDecl, Module};
use crate::capabilities::vm::api::GuestImage;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// 目的：一个模块上次编成的记录——目标与源指纹，缓存命中就比这两项。
pub struct BuiltRecord {
    /// 目的：编成的目标标签（"host" 或虚拟根名）。
    pub target: String,
    /// 目的：编成时的源指纹。
    pub fingerprint: String,
}

/// 目的：本机档构建的两处落点——状态目录与按次家目录。
pub struct BuildHome {
    /// 目的：状态目录（已建好），是围栏的可写范围之一。
    pub state: PathBuf,
    /// 目的：本次构建专用的家目录（已建好），工具链缓存与临时文件落这里。
    pub home: PathBuf,
}

/// 目的：源指纹端口——把「这次要编什么」压成一个可比较的串。
/// 约束：指纹必须覆盖构建命令、构建期能力与提供者、网络开关和全部源文件；只由 `service.rs` 持有（R12）。
pub trait SourceDigest: Send + Sync {
    /// 目的：算这个模块这次的源指纹。
    /// 参数：`module` = 模块；`decl` = 构建声明；`providers` = 本次选出的提供者（`能力=提供者@版本`）。
    /// 错误：读不到源文件。
    fn digest(
        &self,
        module: &Module,
        decl: &BuildDecl,
        providers: &[String],
    ) -> Result<String, String>;
}

/// 目的：构建状态端口——`.vm/build-state/` 的读与写，以及本机档的按次家目录。
/// 约束：一个模块一条记录，按模块 id 存；只由 `service.rs` 持有（R12）。
pub trait BuildStates: Send + Sync {
    /// 目的：上次编成的记录；`None` = 没编过（视为随仓自带的产物）。
    /// 参数：`id` = 模块 id。
    fn record(&self, id: &str) -> Option<BuiltRecord>;
    /// 目的：记下这次编成的结果。
    /// 参数：`id` = 模块 id；`record` = 目标与源指纹。
    /// 错误：写不进状态文件。
    fn record_built(&self, id: &str, record: &BuiltRecord) -> Result<(), String>;
    /// 目的：备好本机档这次构建的两处落点。
    /// 参数：`id` = 模块 id。
    /// 错误：建目录失败。
    fn prepare_home(&self, id: &str) -> Result<BuildHome, String>;
}

/// 目的：缓存卷的装配计划——挂载脚本与用量回读脚本（整批构建共一份）。
pub struct CacheMount {
    /// 目的：挂载 + 卷级对账 + 按龄/排队清理的 guest 脚本。
    pub script: String,
    /// 目的：回读各模块占用的 guest 脚本（每行 `id\t字节\t修改时间`）。
    pub usage_script: String,
}

/// 目的：一个模块在缓存卷上的工作区——guest 侧路径与这一轮要跑的脚本。
/// 约束：`workspace` 是未加引号的路径；拼进命令时一律经 `BuildCache::quote`。
pub struct ModuleWorkspace {
    /// 目的：模块工作区对账（环境标识变了就重建）。
    pub reconcile: String,
    /// 目的：把模块源同步进工作区。
    pub sync_in: String,
    /// 目的：记录构建前的文件快照。
    pub snapshot: String,
    /// 目的：按差异把产物回写宿主那份模块目录。
    pub sync_out: String,
    /// 目的：清工作区的临时目录并 trim。
    pub cleanup: String,
    /// 目的：模块的 HOME（guest 路径），工具链缓存落这里。
    pub home: String,
    /// 目的：模块的 TMPDIR（guest 路径）。
    pub tmp: String,
    /// 目的：模块的构建工作区（guest 路径），构建命令的 cwd。
    pub workspace: String,
}

/// 目的：缓存卷的独占凭据——持有期间别的构建不会并写同一块卷，drop 时释放。
pub trait CacheGuard: Send {}

/// 目的：构建缓存卷端口——卷与锁的落点、待清理账、用量账、guest 侧脚本。
/// 约束：同一虚拟根一把锁；只由 `service.rs` 持有（R12）。
pub trait BuildCache: Send + Sync {
    /// 目的：这个虚拟根的缓存卷文件（宿主路径）。
    /// 参数：`guest` = 虚拟根名。
    fn volume(&self, guest: &str) -> PathBuf;

    /// 目的：独占这个虚拟根的构建锁。
    /// 参数：`guest` = 虚拟根名。
    /// 返回：drop 即释放的凭据。
    /// 错误：等锁超时。
    fn lock(&self, guest: &str) -> Result<Box<dyn CacheGuard>, String>;

    /// 目的：已排队删除、但还没清掉的模块 id。
    /// 参数：`guest` = 虚拟根名。
    fn pending_purge(&self, guest: &str) -> Vec<String>;

    /// 目的：清掉待删除队列（挂载脚本已经清过卷里的目录）。
    /// 参数：`guest` = 虚拟根名。
    fn clear_purge(&self, guest: &str);

    /// 目的：记下各模块在卷上的占用。
    /// 参数：`guest` = 虚拟根名；`usage` = 模块 id → (字节, 最后使用时间)。
    fn record_usage(&self, guest: &str, usage: &BTreeMap<String, (u64, u64)>);

    /// 目的：装配这次构建的缓存卷脚本。
    /// 参数：`image` = 这台构建 guest 的镜像；`keep_secs` = 保留秒数（0 = 不按龄）；`keep` = 本次要编的模块；`purge` = 已排队删除的模块。
    fn mount(
        &self,
        image: &GuestImage,
        keep_secs: u64,
        keep: &[String],
        purge: &[String],
    ) -> CacheMount;

    /// 目的：装配一个模块在卷上的工作区（路径 + 这一轮的脚本）。
    /// 参数：`module_id` = 模块 id；`needs` = 构建期能力；`providers` = 它们的提供者；`src` = 模块源在 guest 里的路径；`entry` = 构建命令的入口（相对模块根）；`entries` = 要带回宿主的产物入口。
    #[allow(clippy::too_many_arguments)]
    fn workspace(
        &self,
        module_id: &str,
        needs: &[String],
        providers: &[String],
        src: &str,
        entry: Option<&str>,
        entries: &[String],
    ) -> ModuleWorkspace;

    /// 目的：把一段文本包成 guest 的 shell 字面量。
    /// 参数：`text` = 原文本。
    fn quote(&self, text: &str) -> String;
}
