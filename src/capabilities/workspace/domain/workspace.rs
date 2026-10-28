//! 工作区与沙箱的纯数据定义与寻址（机制在适配层）。
//! 布局：session/<工作名>/work（用户投喂与成品，本工作共享）
//!       session/<工作名>/<agent实例名>/（该 agent 的私有沙箱）。
//! 寻址：给 AI 的只有**真实绝对路径**（根目录由适配层给出；拒绝时把允许的根列回去）。
//! 核心把路径与三个根（共享区 / 私有沙箱 / 成员模块目录）比对来判定可达范围：
//! 必须是**绝对路径**、组件里不含 . 与 ..、且落在某个允许的根之内；
//! 越界、相对路径、空段一律拒绝，并把系统工具允许的根列回去（如实报错，不纠正）。

use crate::capabilities::permission::api::{Decision, PermissionSet, Reach};
use crate::capabilities::prompt::api::RefRoots;
use crate::kernel::api::slash;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

/// 投喂文件名净化（策略在 conductor）：只允许单个文件名，挡掉路径分隔符与上级跳转。
pub fn safe_file_name(name: &str) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("文件名不能为空".to_string());
    }
    if n.contains('/') || n.contains('\\') || n == "." || n == ".." {
        return Err("文件名不能包含路径分隔符".to_string());
    }
    if n.chars().any(|c| c.is_control()) {
        return Err("文件名不能包含控制字符".to_string());
    }
    Ok(n.to_string())
}

/// 寻址落点（用于回执文案与越界提示）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// 本工作共享区。
    Shared,
    /// 本 agent 私有沙箱。
    Private,
    /// 本 agent 的成员模块目录。
    Module(String),
}

/// 一个 agent 实例的沙箱：寻址根 + 它的能力模块目录。
/// 权限策略在此收口：可达范围只有这三个落点，且模块必须是本 agent 的成员。
#[derive(Debug, Clone)]
pub struct Sandbox {
    /// 工作名（提示词里如实告知在替谁干活）。
    pub work_name: String,
    /// 目的：本席所属会话的 id（会话租约；同名 agent 的多个并发会话靠它区分围栏归属）。
    pub session: String,
    /// 该 agent 的实例名。
    pub agent: String,
    /// 本工作共享区（session/<工作名>/work）——**主副本**，agent 会话默认只读。
    pub shared: PathBuf,
    /// 本 agent 私有沙箱（session/<工作名>/<agent>）——也是它的工作副本。
    pub private: PathBuf,
    /// 成员模块：模块 id → 模块目录。
    pub modules: BTreeMap<String, PathBuf>,
    /// 目的：有 <root>/userdata 目录的模块 id（工作区扫描读出的事实；围栏派生据此决定派不派这条）。
    pub modules_with_userdata: std::collections::BTreeSet<String>,
    /// 本席位的生效权限（reach / decision / 共享区范围）：由 conductor 解析后注入。
    /// 私有沙箱不受共享区路径范围约束，但仍受 profile 的写决定约束。
    pub permissions: PermissionSet,
    /// 目的：这场会话的路径翻译器（虚拟机档 guest ↔ 宿主；本机档 None = 恒等）。
    /// 约束：只用来把模型给的写法译成宿主路径**再校验**；安全判断仍然只有 `resolve` 一处。
    pub path_codec: Option<std::sync::Arc<dyn crate::kernel::ports::PathCodec>>,
}

impl Sandbox {
    /// 解析模型给的路径：必须是**绝对路径**、组件里不含 . 与 ..、且落在某个允许的根之内。
    /// 返回（落点, 归一化后的绝对路径）；拒绝时把允许的真实根目录列全。
    pub fn resolve(&self, raw: &str) -> Result<(Place, PathBuf), String> {
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
                // 认不出本环境的写法：说清是写法不对，而不是“越界”；允许的根照旧列回去。
                None => {
                    return Err(deny(format!(
                        "路径不是本环境的写法（虚拟机档用 guest 内路径，宿主路径在这里不存在）：{}",
                        spec
                    )))
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
        let mut best: Option<(Place, usize)> = None;
        for (place, base) in self.roots() {
            if norm.starts_with(&base) {
                let n = base.as_os_str().len();
                if best.as_ref().map(|(_, bn)| n > *bn).unwrap_or(true) {
                    best = Some((place, n));
                }
            }
        }
        match best {
            Some((place, _)) => Ok((place, norm)),
            None => Err(deny(format!("路径越界：{}", spec))),
        }
    }

    /// 目的：该落点对应的宿主根（真实路径校验用；`Place::Module` 取模块根）。
    pub fn root_of(&self, place: &Place) -> Option<&Path> {
        match place {
            Place::Shared => Some(self.shared.as_path()),
            Place::Private => Some(self.private.as_path()),
            Place::Module(id) => self.modules.get(id).map(|p| p.as_path()),
        }
    }

    /// 该落点这一席能不能写：内置文件工具永远不直接写共享主副本（写入走 work_commit）；
    /// 私有沙箱不受路径范围约束，但仍要 profile 的写决定允许；模块目录对内置文件工具永久只读。
    pub fn can_write(&self, place: &Place, _path: &Path) -> bool {
        if self.permissions.decisions.write != Decision::Allow {
            return false;
        }
        match place {
            Place::Shared => false,
            Place::Private => self.permissions.reach.contains(Reach::Private),
            Place::Module(_) => false,
        }
    }

    /// 该落点这一席能不能读：共享区按 reach 与路径范围；私有沙箱与模块目录只看 reach 与读决定。
    pub fn can_read(&self, place: &Place, path: &Path) -> bool {
        if self.permissions.decisions.read != Decision::Allow {
            return false;
        }
        match place {
            Place::Shared => {
                self.permissions.reach.contains(Reach::SharedRead)
                    && self.permissions.read_ok(&self.rel_to(&self.shared, path))
            }
            Place::Private => self.permissions.reach.contains(Reach::Private),
            Place::Module(_) => self.permissions.reach.contains(Reach::ModuleRead),
        }
    }

    /// 写被拒时的如实说明（按落点给不同原因）。
    pub fn write_refusal(&self, place: &Place, path: &Path) -> String {
        if self.permissions.decisions.write != Decision::Allow {
            return format!(
                "当前档位「{}」不允许写入；要写入请把会话档位切到「工作区内修改」或更高。",
                self.permissions.profile.label()
            );
        }
        match place {
            Place::Shared => {
                if !self.permissions.reach.contains(Reach::SharedWrite) {
                    format!(
                        "当前档位「{}」没有共享区提交权限；要提交请把会话档位切到「工作区内修改」或更高。",
                        self.permissions.profile.label()
                    )
                } else {
                    let roots = self.permissions.write_roots_listing();
                    format!(
                        "共享区只读：{} 不在允许提交的路径内（允许：{}）。主副本不直接写，请用 work_commit 提交。",
                        self.rel_to(&self.shared, path),
                        roots
                    )
                }
            }
            Place::Module(id) => format!(
                "模块 {} 的目录对 agent 的内置文件工具只读（含 userdata/）：模块数据由模块自己的工具 / 服务进程维护；请写进你自己的沙箱（工作副本）。",
                id
            ),
            Place::Private => format!(
                "当前档位「{}」不允许写入私有沙箱。",
                self.permissions.profile.label()
            ),
        }
    }

    /// 读被拒时的如实说明。
    pub fn read_refusal(&self, place: &Place, path: &Path) -> String {
        let roots = self.permissions.read_roots_listing();
        let rel = match place {
            Place::Shared => self.rel_to(&self.shared, path),
            Place::Private => self.rel_to(&self.private, path),
            Place::Module(_) => self.rel_to(&self.shared, path),
        };
        format!(
            "读权限受限：{} 不在允许读取的路径内（允许：{}）。",
            rel, roots
        )
    }

    /// 一条绝对路径相对某个根的工作区相对形式（`/` 分隔），供权限条目比对。
    pub fn rel_to(&self, root: &Path, path: &Path) -> String {
        path.strip_prefix(root)
            .map(slash)
            .unwrap_or_else(|_| slash(path))
    }

    /// 允许的根：共享区、私有沙箱、每个成员模块目录（顺序稳定，便于取最长匹配）。
    fn roots(&self) -> Vec<(Place, PathBuf)> {
        let mut v = vec![
            (Place::Shared, self.shared.clone()),
            (Place::Private, self.private.clone()),
        ];
        for (id, root) in &self.modules {
            v.push((Place::Module(id.clone()), root.clone()));
        }
        v
    }

    /// 目的：宿主路径 → 该环境里给模型看的写法（有翻译器就译，没有就是宿主写法）。
    fn in_view(&self, host: &Path) -> String {
        match self.path_codec.as_ref() {
            Some(codec) => codec.to_view(host),
            None => slash(host),
        }
    }

    /// 系统工具允许的根清单（错误文案里列全，模型照着改）——按本环境的写法给。
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

/// 一次工作内的全部 agent 沙箱（发言席就是 agent，沙箱与 agent 一一对应）。
#[derive(Debug, Clone, Default)]
pub struct Sandboxes {
    /// 本工作共享区的真实根（@ 改写要它；代拟还没名单时也有）。
    pub shared: PathBuf,
    pub list: Vec<Sandbox>,
    /// 目的：这场会话的路径翻译器（虚拟机档 guest↔宿主；本机档 None = 恒等）。
    /// 约束：@ 改写发生在名单确认**之前**（协作需求），那时 `list` 还是空的，所以这里单独存一份。
    pub codec: Option<Arc<dyn crate::kernel::ports::PathCodec>>,
}

impl Sandboxes {
    /// 某 agent 实例的沙箱；不存在 = 该 agent 没被分配工作区（调用方如实报错）。
    pub fn for_agent(&self, agent: &str) -> Option<&Sandbox> {
        self.list.iter().find(|s| s.agent == agent)
    }

    /// 目的：用户输入里 @ 引用要的根（共享区 + 本会话的路径翻译器）——协作 / 主持人的话用它改写。
    /// 约束：private 恒为 None——这些入口没有"自己的沙箱"（与说话人无关的共读文本）。
    pub fn ref_roots(&self) -> RefRoots {
        RefRoots {
            work: self.shared.clone(),
            private: None,
            path_codec: self.codec.clone(),
        }
    }
}

/// 一次工作的文件清单（给前端的 @ 菜单用）：相对路径，**一律用 / 分隔**，排序稳定。
#[derive(Debug, Clone, Default)]
pub struct WorkFiles {
    /// work/ 共享区下的文件。
    pub work: Vec<String>,
    /// agent 实例名 → 其私有沙箱下的文件。
    pub agents: BTreeMap<String, Vec<String>>,
}

/// 工作区寻址根（由 `Workdirs` 端口如实给出；本能力的 service 据此拼装沙箱）。
#[derive(Debug, Clone)]
pub struct WorkRoots {
    /// 本工作共享区。
    pub shared: PathBuf,
    /// agent 实例名 → 私有沙箱目录。
    pub agents: BTreeMap<String, PathBuf>,
    /// 版本库目录（`session/<工作名>/.work`）：提交记录、内容寻址对象与各 agent 的拉取基线。
    pub store: PathBuf,
}

/// 一个区的用量：文件数 + 总字节（真实 stat，不是清单条数）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AreaUsage {
    pub files: usize,
    pub bytes: u64,
}

/// 一次工作的**工作区用量**（删除前如实交代）：总数 + 共享区/各 agent 沙箱的分项。
/// 只数文件（目录不计），字节取文件真实大小。
#[derive(Debug, Clone, Default, Serialize)]
pub struct WorkUsage {
    /// 全部文件数（共享区 + 各 agent 沙箱）。
    pub files: usize,
    /// 全部总字节。
    pub bytes: u64,
    /// 共享区 `work/`。
    pub work: AreaUsage,
    /// agent 实例名 → 它的私有沙箱。
    pub agents: BTreeMap<String, AreaUsage>,
}

impl WorkUsage {
    /// 按分项汇总总数（调用方只填分项，避免两处各算一遍）。
    pub fn total(work: AreaUsage, agents: BTreeMap<String, AreaUsage>) -> WorkUsage {
        let mut files = work.files;
        let mut bytes = work.bytes;
        for a in agents.values() {
            files += a.files;
            bytes += a.bytes;
        }
        WorkUsage {
            files,
            bytes,
            work,
            agents,
        }
    }
}
