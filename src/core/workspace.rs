//! 工作区与沙箱的纯数据定义与寻址（机制在适配层）。
//! 布局：session/<工作名>/work（用户投喂与成品，本工作共享）
//!       session/<工作名>/<agent实例名>/（该 agent 的私有沙箱）。
//! 寻址：给 AI 的只有**真实绝对路径**（根目录由适配层给出，经提示词册如实告知）。
//! 核心把路径与三个根（共享区 / 私有沙箱 / 成员模块目录）比对来判定可达范围：
//! 必须是**绝对路径**、组件里不含 . 与 ..、且落在某个允许的根之内；
//! 越界、相对路径、空段一律拒绝，并把允许的根目录列回去（如实报错，不纠正）。

use crate::core::prompt::ToolTexts;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

/// 路径的**书写形式**（给模型看、进提示词与 JSON 的）：一律用 / 分隔。
/// Windows 的反斜杠在 JSON 字符串里是转义符（"D:\a" 会解析失败），所以对外只给 /——两边系统都认。
pub fn slash(p: &std::path::Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// 投喂文件名净化（策略在 core）：只允许单个文件名，挡掉路径分隔符与上级跳转。
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
    /// 本工作共享区（session/<工作名>/work）。
    pub shared: PathBuf,
    /// 本 agent 私有沙箱（session/<工作名>/<agent>）。
    pub private: PathBuf,
    /// 成员模块：模块 id → 模块目录。
    pub modules: BTreeMap<String, PathBuf>,
    /// 模型侧文案（来自 prompts.yaml）：随沙箱注入，不是状态。
    pub texts: ToolTexts,
    /// 内置工具的参数契约（来自 prompts.yaml 的 builtin_tools）：说明与校验都按它来。
    pub builtin_tools: crate::core::schema::ToolBook,
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

/// 工作区寻址根（由 Workspace 端口如实给出；core 据此拼装沙箱）。
#[derive(Debug, Clone)]
pub struct WorkRoots {
    /// 本工作共享区。
    pub shared: PathBuf,
    /// agent 实例名 → 私有沙箱目录。
    pub agents: BTreeMap<String, PathBuf>,
}
