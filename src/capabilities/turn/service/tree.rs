//! 目的：会话树的**树序与身份**——整棵子树、使用类型、角色、工作根、派发闸门与树级装配 meta。
//! 管：`subtree_of`（树序）、`session_kind_of` / `role_of`（机制册两把钥匙）、`work_root_of` / `work_root`
//!   （整棵树只有一个 work/）、`dispatch_gate` / `dispatch_target`（运行态闸门）、`workspace_binding_of`
//!   （绑定的复检）与 `tree_meta` / `tree_meta_with`（一份 guest 的树级装配 meta）。
//! 不管：事件怎么落盘（`conductor` 的 `record_events`）、协作链怎么推进；不碰 IO（meta 走 History，探测走 HostProbe）。
//! 联动：`protocol.rs` 的裁决队作废与派发用树序；`rebuild.rs` 用身份与绑定；`conductor` 的建工作 / 改配 / 视图经
//!   `turn::api` 取用。

use std::collections::BTreeMap;

use crate::capabilities::session::api::{AgentMeta, SessionMeta};
use crate::kernel::ports::HostProbe;

use crate::capabilities::turn::service::session::Turn;
use crate::capabilities::workspace::api::{product_root, recheck_root};

impl Turn {
    /// 目的：一个会话的整棵子树（含它自己，父在前、子在后）：父子关系以 `meta.parent` 为唯一判据。
    pub(crate) fn subtree_of(&self, name: &str) -> Vec<String> {
        crate::capabilities::lifecycle::api::subtree_of(&self.history_list(), name)
    }

    /// 目的：历史列表（树序）：生命周期能力按落盘事实给出的会话表。
    /// 约束：读盘失败由能力侧如实记日志并给空表，这里不改判据。
    pub(crate) fn history_list(&self) -> Vec<crate::capabilities::session::api::HistoryView> {
        crate::capabilities::lifecycle::api::list(&*self.history, &*self.log)
    }

    /// 目的：这个会话的**使用类型**（机制册的第一把钥匙）：按落盘形态派生，重建后是同一份。
    ///  协作派生的成员 / 节点会话算 **collab**（它属于那场协作），其余单 agent 会话算 single，代理算 proxy。
    pub(crate) fn session_kind_of(&self, meta: &SessionMeta) -> Result<String, String> {
        match meta.mode.as_str() {
            "proxy" => Ok("proxy".to_string()),
            "collab" => Ok("collab".to_string()),
            _ => match meta.parent.as_deref() {
                Some(p) => match self.history.meta(p) {
                    Ok(pm) if pm.mode == "collab" => Ok("collab".to_string()),
                    _ => Ok("single".to_string()),
                },
                None => Ok("single".to_string()),
            },
        }
    }

    /// 目的：这一席的**角色**（机制册的第二把钥匙，也是工具面与提示词的角色名）。
    ///  与实时建立同一把尺子：协作的成员 / 节点 = executor；代理 = core_proxy；其余单 agent 工作 = solo。
    pub(crate) fn role_of(&self, meta: &SessionMeta) -> Result<String, String> {
        if meta.mode == "proxy" {
            return Ok("core_proxy".to_string());
        }
        match meta.parent.as_deref() {
            Some(p) => match self.history.meta(p) {
                Ok(pm) if pm.mode == "collab" => Ok("executor".to_string()),
                _ => Ok("solo".to_string()),
            },
            None => Ok("solo".to_string()),
        }
    }

    /// 目的：一个会话沿父链继承的工作区根（仅外部绑定；`None` = 产品内布局）。
    /// 参数：`meta` = 会话元信息；`probe` = 宿主能力探测（规范化与元数据）。
    /// 约束：与 `work_root_of` 同一走向：自己带绑定就用它，否则看工作根的 meta。
    ///   取到根后按它所在会话的档位复检（见 `roots`）——meta 是磁盘上的文本，手改也要过同一套策略。
    pub(crate) fn workspace_binding_of(
        &self,
        meta: &SessionMeta,
        probe: &dyn HostProbe,
    ) -> Result<Option<std::path::PathBuf>, String> {
        let (root, tier) = match &meta.workspace {
            Some(binding) => (binding.root.clone(), meta.exec.tier),
            None => match &meta.parent {
                None => return Ok(None),
                Some(p) => {
                    let top = self.work_root(p)?;
                    let top_meta = self.history.meta(&top)?;
                    match top_meta.workspace {
                        Some(binding) => (binding.root, top_meta.exec.tier),
                        None => return Ok(None),
                    }
                }
            },
        };
        let product = product_root(&self.root, probe);
        recheck_root(&root, tier, &product, probe)?;
        Ok(Some(root))
    }

    /// 目的：一个会话的**工作根**，从它自己的 meta 起算：`meta` 可能**还没落盘**
    ///  （建工作区发生在落盘之前），所以有父就上溯、没有父就是它自己。
    pub(crate) fn work_root_of(&self, meta: &SessionMeta) -> Result<String, String> {
        match &meta.parent {
            None => Ok(meta.name.clone()),
            Some(p) => self.work_root(p),
        }
    }

    /// 目的：一个会话的**工作根**（顶层会话名）：沿 `meta.parent` 一路走到顶。
    ///  整棵树不论嵌套多少层只有一个 `work/`——共享区与 agent 沙箱都锚在它上面。
    pub(crate) fn work_root(&self, name: &str) -> Result<String, String> {
        let mut cur = self.history.meta(name)?;
        let mut guard = 0usize;
        while let Some(p) = cur.parent.clone() {
            guard += 1;
            if guard > 64 {
                return Err(format!("会话父子关系成环：{}", name));
            }
            cur = self.history.meta(&p)?;
        }
        Ok(cur.name)
    }

    /// 目的：**运行态闸门**：派发 / 唤醒前过它，停止与关闭都拒绝。
    ///  拿不到 meta（系统会话等未落盘）就当正常运行——运行态是落盘事实，
    ///  不存在的会话自有别的检查兜底（这里不替它报"无此会话"）。
    pub(crate) fn dispatch_gate(&self, sid: &str) -> Result<(), String> {
        match self.history.meta(sid) {
            Ok(meta) => match meta.dispatch_refusal() {
                Some(e) => Err(e),
                None => Ok(()),
            },
            Err(_) => Ok(()),
        }
    }

    /// 目的：派发目标的形态 + 这一次派发放不放行（放行 = Ok(mode)）。
    ///  代理转达与叫醒共用这一处，免得"看不看运行态"两处口径不一致。
    pub(crate) fn dispatch_target(&self, sid: &str) -> Result<String, String> {
        let meta = self.history.meta(sid)?;
        match meta.dispatch_refusal() {
            Some(e) => Err(e),
            None => Ok(meta.mode),
        }
    }

    /// 目的：一台共享 guest 的**树级装配 meta**——把作用域根与其整棵子树的 agent 并成一份。
    /// 约束：同一 agent 名只留一份（后代覆盖祖先，便于子会话编辑后按新值装配）；exec / 形态 / 工作名取作用域根。
    ///   extra 用于"还没落盘"的待建子会话（建工作与改配的准备段要它）。
    pub(crate) fn tree_meta(
        &self,
        root: &SessionMeta,
        extra: Option<&SessionMeta>,
    ) -> Result<SessionMeta, String> {
        let extras: Vec<&SessionMeta> = extra.into_iter().collect();
        self.tree_meta_with(root, &extras)
    }

    /// 目的：同上，但是可带多份尚未落盘的 meta（改配时主会话与子树的新值要一起参与装配）。
    pub(crate) fn tree_meta_with(
        &self,
        root: &SessionMeta,
        extra: &[&SessionMeta],
    ) -> Result<SessionMeta, String> {
        let mut agents: BTreeMap<String, AgentMeta> = BTreeMap::new();
        let mut push = |m: &SessionMeta| {
            for a in &m.agents {
                agents.insert(a.name.clone(), a.clone());
            }
        };
        push(root);
        for sid in self.subtree_of(&root.name) {
            if sid == root.name {
                continue;
            }
            if let Ok(m) = self.history.meta(&sid) {
                push(&m);
            }
        }
        for m in extra {
            push(m);
        }
        let mut out = root.clone();
        out.agents = agents.into_values().collect();
        out.modules = crate::capabilities::turn::domain::session::flat_modules(&out.agents);
        Ok(out)
    }
}
