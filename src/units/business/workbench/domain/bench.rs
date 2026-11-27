//! 目的：一席的工作面形状与寻址语义——读写落在哪个区、哪些路径在允许的根内。
//! 管：`Bench`（一席的寻址根、权限面、共享区归属、路径翻译器与过滤名单）与它的纯判定：
//!   必须是**绝对路径**、组件里不含 . 与 ..、且落在某个允许的根之内（共享区 / 私有沙箱 / 成员模块目录）；
//!   越界、相对路径、空段一律拒绝，并把允许的根列回去（如实报错，不纠正）。
//! 不管：根从哪来（`workspace` 的席位组装）、路径拼接与目录创建（`workspace` 的适配层）。
//! 联动：`tools` 的内置文件工具用它定位读写；`agentloop` 把它交给围栏与提示词；`versions` 用工作副本根。

use crate::kernel::api::path::slash;
use crate::units::business::workbench::domain::area::Area;
use crate::units::pure::permission::api::PermissionSet;
use crate::units::pure::permission::api::SharedScope;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

/// 目的：一个 agent 实例的工作台：寻址根 + 它的能力模块目录。
///    权限策略在此收口：可达范围只有这三个落点，且模块必须是本 agent 的成员。
#[derive(Debug, Clone)]
pub struct Bench {
    /// 目的：工作名（提示词里如实告知在替谁干活）。
    pub work_name: String,
    /// 目的：本席所属会话的 id（会话租约；同名 agent 的多个并发会话靠它区分围栏归属）。
    pub session: String,
    /// 目的：该 agent 的实例名。
    pub agent: String,
    /// 目的：本工作共享区（session/<工作名>/work）——**主副本**，agent 会话默认只读。
    pub shared: PathBuf,
    /// 目的：本 agent 私有沙箱（session/<工作名>/<agent>）——也是它的工作副本。
    pub private: PathBuf,
    /// 目的：成员模块：模块 id → 模块目录。
    pub modules: BTreeMap<String, PathBuf>,
    /// 目的：模块 id → 私有区（`data/modules/<id>/userdata`）：**只有私有区可用的模块才进表**。
    ///    事实由 `catalog` 扫描读出、形状由 `content` 给；围栏派生据此决定派不派模块的那条写授予。
    pub module_data: BTreeMap<String, PathBuf>,
    /// 目的：本席位的网络档位（离线 / 联网）：由 runtime 解析后注入。
    ///    文件面不再按档位打权限——沙箱恒可读写、模块目录恒只读、工具链与 host_read_roots 恒只读。
    pub permissions: PermissionSet,
    /// 目的：本席位的共享区路径归属（读 / 写范围与黑名单）：由 runtime 解析后注入。
    ///    只约束**共享区**（主副本）；私有沙箱与模块目录不看它。
    pub scope: SharedScope,
    /// 目的：这场会话的路径翻译器（虚拟机档 guest ↔ 宿主；本机档 None = 恒等）。
    /// 约束：只用来把模型给的写法译成宿主路径**再校验**；安全判断仍然只有 `resolve` 一处。
    pub path_codec: Option<Arc<dyn crate::kernel::ports::PathCodec>>,
    /// 目的：过滤规则文件（版本库目录里的 `.workignore`）：检索面（grep）与内容面（哪些文件算工作区内容）共用这一份。
    /// 约束：只读这份规则；文件不存在时调用方按种子名单兜底，不在这里建目录。
    pub filter_rules: PathBuf,
}

impl Bench {
    /// 目的：解析模型给的路径：必须是**绝对路径**、组件里不含 . 与 ..、且落在某个允许的根之内。
    ///    返回（落点, 归一化后的绝对路径）；拒绝时把允许的真实根目录列全。
    pub fn resolve(&self, raw: &str) -> Result<(Area, PathBuf), String> {
        let spec = raw.trim();
        // 所有拒绝都附上"系统工具允许的根"，模型照着改。
        let deny = |why: String| {
            format!(
                "{}。系统工具允许的根（模块工具不查这张表，由围栏 / guest 边界约束）：\n{}",
                why,
                self.roots_listing()
            )
        };
        if spec.is_empty() {
            return Err(deny("路径不能为空".to_string()));
        }
        // 模型侧写法 → 宿主写法（虚拟机档 guest→宿主；本机档恒等）。翻译只影响寻址；安全判断仍在下面。
        let host: PathBuf = match self.path_codec.as_ref() {
            Some(codec) => match codec.to_host(spec) {
                Some(p) => p,
                // 认不出本环境的写法：说清是写法不对，而不是"越界"；允许的根照旧列回去。
                None => {
                    return Err(deny(format!(
                        "路径不是本环境的写法（虚拟机档用 guest 内路径，宿主路径在这里不存在）：{}",
                        spec
                    )));
                }
            },
            None => PathBuf::from(spec),
        };
        if !host.is_absolute() {
            return Err(deny(format!("需要绝对路径，收到：{}", spec)));
        }
        // 空段（连续分隔符）：components() 会吞掉，这里如实挡掉。
        let flat = host.to_string_lossy().replace('\\', "/");
        if flat.trim_start_matches('/').contains("//") {
            return Err(deny(format!("路径含空段（连续分隔符）：{}", spec)));
        }
        let p: &Path = &host;
        let mut norm = PathBuf::new();
        for c in p.components() {
            match c {
                Component::Prefix(_) | Component::RootDir => norm.push(c.as_os_str()),
                Component::Normal(s) => norm.push(s),
                Component::CurDir => return Err(deny(format!("路径不能含 . 段：{}", spec))),
                Component::ParentDir => return Err(deny(format!("路径不能含 .. 段：{}", spec))),
            }
        }
        // 落在哪个根之内：取匹配到的最长根（最具体）。
        let mut best: Option<(Area, usize)> = None;
        for (area, base) in self.roots() {
            if norm.starts_with(&base) {
                let n = base.as_os_str().len();
                if best.as_ref().map(|(_, bn)| n > *bn).unwrap_or(true) {
                    best = Some((area, n));
                }
            }
        }
        match best {
            Some((area, _)) => Ok((area, norm)),
            None => Err(deny(format!("路径越界：{}", spec))),
        }
    }

    /// 目的：该落点对应的宿主根（真实路径校验用；`Area::Module` 取模块根）。
    pub fn root_of(&self, area: &Area) -> Option<&Path> {
        match area {
            Area::Shared => Some(self.shared.as_path()),
            Area::Private => Some(self.private.as_path()),
            Area::Module(id) => self.modules.get(id).map(|p| p.as_path()),
        }
    }

    /// 目的：该落点这一席能不能写：内置文件工具永远不直接写共享主副本（写入走 work_commit）；
    ///    私有沙箱恒可写；模块目录对内置文件工具永久只读。
    pub fn can_write(&self, area: &Area) -> bool {
        matches!(area, Area::Private)
    }

    /// 目的：该落点这一席能不能读：共享区按路径归属；私有沙箱与模块目录恒可读。
    pub fn can_read(&self, area: &Area, path: &Path) -> bool {
        match area {
            Area::Shared => self.scope.read_ok(&self.rel_to(&self.shared, path)),
            Area::Private | Area::Module(_) => true,
        }
    }

    /// 目的：写被拒时的如实说明（按落点给不同原因）。
    pub fn write_refusal(&self, area: &Area, path: &Path) -> String {
        match area {
            Area::Shared => {
                let rel = self.rel_to(&self.shared, path);
                if self.scope.write_ok(&rel) {
                    format!(
                        "共享区只读：{} 是主副本，不直接写；改动请写进你的沙箱（工作副本）再用 work_commit 提交。",
                        rel
                    )
                } else {
                    format!(
                        "共享区只读：{} 不在允许提交的路径内（允许：{}）。主副本不直接写，请用 work_commit 提交。",
                        rel,
                        self.scope.write_roots_listing()
                    )
                }
            }
            Area::Module(id) => format!(
                "模块 {} 的目录对 agent 的内置文件工具只读（含 userdata/）：模块数据由模块自己的工具 / 服务进程维护；请写进你自己的沙箱（工作副本）。",
                id
            ),
            Area::Private => {
                "私有沙箱恒可写；这次被拒不是权限问题，请把该场景报告给产品维护者。".to_string()
            }
        }
    }

    /// 目的：读被拒时的如实说明。
    pub fn read_refusal(&self, area: &Area, path: &Path) -> String {
        let roots = self.scope.read_roots_listing();
        let rel = match area {
            Area::Shared => self.rel_to(&self.shared, path),
            Area::Private => self.rel_to(&self.private, path),
            Area::Module(_) => self.rel_to(&self.shared, path),
        };
        format!(
            "读权限受限：{} 不在允许读取的路径内（允许：{}）。",
            rel, roots
        )
    }

    /// 目的：一条绝对路径相对某个根的工作区相对形式（`/` 分隔），供权限条目比对。
    pub fn rel_to(&self, root: &Path, path: &Path) -> String {
        path.strip_prefix(root)
            .map(slash)
            .unwrap_or_else(|_| slash(path))
    }

    /// 允许的根：共享区、私有沙箱、每个成员模块目录（顺序稳定，便于取最长匹配）。
    fn roots(&self) -> Vec<(Area, PathBuf)> {
        let mut v = vec![
            (Area::Shared, self.shared.clone()),
            (Area::Private, self.private.clone()),
        ];
        for (id, root) in &self.modules {
            v.push((Area::Module(id.clone()), root.clone()));
        }
        v
    }

    /// 目的：宿主路径 → 该环境里给模型看的写法（有翻译器就译，没有就是宿主写法）。
    pub fn in_view(&self, host: &Path) -> String {
        match self.path_codec.as_ref() {
            Some(codec) => codec.to_view(host),
            None => slash(host),
        }
    }

    /// 目的：系统工具允许的根清单（错误文案里列全，模型照着改）——按本环境的写法给。
    pub fn roots_listing(&self) -> String {
        let mut lines = vec![
            format!(
                "- 本次工作的共享区（主副本，只读）：{}",
                self.in_view(&self.shared)
            ),
            format!("- 你私有的沙箱：{}", self.in_view(&self.private)),
        ];
        for (id, root) in &self.modules {
            lines.push(format!("- 模块 {} 的目录：{}", id, self.in_view(root)));
        }
        lines.join("\n")
    }
}
