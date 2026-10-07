//! 工作区与沙箱的纯数据定义与寻址（机制在适配层）。
//! 布局：session/<工作名>/work（用户投喂与成品，本工作共享）
//!       session/<工作名>/<agent实例名>/（该 agent 的私有沙箱）。
//! 寻址：给 AI 的只有**真实绝对路径**（根目录由适配层给出，经提示词册如实告知）。
//! 核心把路径与三个根（共享区 / 私有沙箱 / 成员模块目录）比对来判定可达范围：
//! 必须是**绝对路径**、组件里不含 . 与 ..、且落在某个允许的根之内；
//! 越界、相对路径、空段一律拒绝，并把允许的根目录列回去（如实报错，不纠正）。

use crate::capabilities::permission::api::Permissions;
use crate::capabilities::prompt::api::ToolTexts;
use crate::kernel::api::slash;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

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
    /// 该 agent 的实例名。
    pub agent: String,
    /// 本工作共享区（session/<工作名>/work）——**主副本**，agent 会话默认只读。
    pub shared: PathBuf,
    /// 共享主副本这一席能不能写：agent 会话 false（写入走 work_commit），核心会话 true。
    /// 它只影响**写**；读仍然可按 `resolve` 落到 Shared。
    pub shared_writable: bool,
    /// 本 agent 私有沙箱（session/<工作名>/<agent>）——也是它的工作副本。
    pub private: PathBuf,
    /// 成员模块：模块 id → 模块目录。
    pub modules: BTreeMap<String, PathBuf>,
    /// 目的：有 <root>/userdata 目录的模块 id（工作区扫描读出的事实；围栏派生据此决定派不派这条）。
    pub modules_with_userdata: std::collections::BTreeSet<String>,
    /// 本席位的生效权限（读/提交白黑名单、模块写授权、决定粒度）：由 conductor 解析后注入。
    /// 私有沙箱不在它的管辖内（永远全权）；它只管共享工作区与模块目录。
    pub permissions: Permissions,
    /// 模型侧文案（来自 prompts/）：**共享一份**（提示词能力给的 `Arc`），不是状态、也不深拷贝。
    /// 路径拒绝文案由**本模块自己**渲染，所以这条是正常的业务间依赖（经 prompt 的能力面）。
    pub texts: std::sync::Arc<ToolTexts>,
}

impl Sandbox {
    /// 解析模型给的路径：必须是**绝对路径**、组件里不含 . 与 ..、且落在某个允许的根之内。
    /// 返回（落点, 归一化后的绝对路径）；拒绝时把允许的真实根目录列全。
    pub fn resolve(&self, raw: &str) -> Result<(Place, PathBuf), String> {
        let spec = raw.trim();
        // 所有拒绝都附上"允许的根目录"，模型照着改。
        let deny = |why: String| {
            self.texts.render(
                &self.texts.roots_wrapper,
                &[("why", why), ("roots", self.roots_listing())],
            )
        };
        if spec.is_empty() {
            return Err(deny(self.texts.path_empty.clone()));
        }
        let p = Path::new(spec);
        if !p.is_absolute() {
            return Err(deny(self.texts.render(
                &self.texts.path_need_absolute,
                &[("path", spec.to_string())],
            )));
        }
        // 空段（连续分隔符）：components() 会吞掉，这里如实挡掉。
        let flat = spec.replace('\\', "/");
        if flat.trim_start_matches('/').contains("//") {
            return Err(deny(self.texts.render(
                &self.texts.path_empty_segment,
                &[("path", spec.to_string())],
            )));
        }
        let mut norm = PathBuf::new();
        for c in p.components() {
            match c {
                Component::Prefix(_) | Component::RootDir => norm.push(c.as_os_str()),
                Component::Normal(s) => norm.push(s),
                Component::CurDir => {
                    return Err(deny(
                        self.texts
                            .render(&self.texts.path_cur_dir, &[("path", spec.to_string())]),
                    ))
                }
                Component::ParentDir => {
                    return Err(deny(self.texts.render(
                        &self.texts.path_parent_dir,
                        &[("path", spec.to_string())],
                    )))
                }
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
            None => Err(deny(self.texts.render(
                &self.texts.path_outside_roots,
                &[("path", spec.to_string())],
            ))),
        }
    }

    /// 该落点这一席能不能写：共享主副本只读时写类工具一律拒绝；私有沙箱永远全权；
    /// 模块目录默认只读，只有 module_write 命中该模块（或落在 <module>/userdata/ 下）才可写。
    pub fn can_write(&self, place: &Place, path: &Path) -> bool {
        match place {
            // 共享主副本：这一席可写**且**路径落在提交白名单内（黑名单已并入 write_ok）。
            Place::Shared => {
                self.shared_writable && self.permissions.write_ok(&self.rel_to(&self.shared, path))
            }
            Place::Private => true,
            Place::Module(id) => self.permissions.module_write_ok(id) || self.is_userdata(id, path),
        }
    }

    /// 该落点这一席能不能读：共享主副本按读白名单/黑名单；私有沙箱与模块目录照旧可读。
    pub fn can_read(&self, place: &Place, path: &Path) -> bool {
        match place {
            Place::Shared => self.permissions.read_ok(&self.rel_to(&self.shared, path)),
            _ => true,
        }
    }

    /// 写被拒时的如实说明（按落点给不同原因）。
    pub fn write_refusal(&self, place: &Place, path: &Path) -> String {
        match place {
            // 主副本本身只读，还是路径越出提交白名单——两种原因说清楚，模型才知道怎么改。
            Place::Shared => {
                if !self.shared_writable {
                    self.shared_read_only()
                } else {
                    let roots = self.permissions.write_roots_listing();
                    self.texts.render(
                        &self.texts.write_scope_denied,
                        &[("path", self.rel_to(&self.shared, path)), ("roots", roots)],
                    )
                }
            }
            Place::Module(id) => self
                .texts
                .render(&self.texts.module_write_denied, &[("id", id.clone())]),
            Place::Private => self.shared_read_only(),
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
        self.texts.render(
            &self.texts.read_scope_denied,
            &[("path", rel), ("roots", roots)],
        )
    }

    /// <module>/userdata/：模块自己的跨任务状态区，恒可写（不被模块只读默认挡住）。
    fn is_userdata(&self, id: &str, path: &Path) -> bool {
        match self.modules.get(id) {
            Some(root) => path.starts_with(root.join("userdata")),
            None => false,
        }
    }

    /// 一条绝对路径相对某个根的工作区相对形式（`/` 分隔），供权限条目比对。
    pub fn rel_to(&self, root: &Path, path: &Path) -> String {
        path.strip_prefix(root)
            .map(slash)
            .unwrap_or_else(|_| slash(path))
    }

    /// 写共享主副本被拒时的如实说明（文案在提示词册）。
    pub fn shared_read_only(&self) -> String {
        self.texts.write_shared_read_only.clone()
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

    /// 允许的根目录清单（错误文案里列全，模型照着改）。
    pub fn roots_listing(&self) -> String {
        let mut lines = vec![
            self.texts
                .render(&self.texts.roots_shared, &[("root", slash(&self.shared))]),
            self.texts
                .render(&self.texts.roots_private, &[("root", slash(&self.private))]),
        ];
        for (id, root) in &self.modules {
            lines.push(self.texts.render(
                &self.texts.roots_module,
                &[("id", id.clone()), ("root", slash(root))],
            ));
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
}

impl Sandboxes {
    /// 某 agent 实例的沙箱；不存在 = 该 agent 没被分配工作区（调用方如实报错）。
    pub fn for_agent(&self, agent: &str) -> Option<&Sandbox> {
        self.list.iter().find(|s| s.agent == agent)
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
