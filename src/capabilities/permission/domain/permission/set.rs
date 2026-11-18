//! 目的：一份生效权限的求交结果——profile、reach、各 effect 的处置、共享区范围与黑名单。
//! 管：`PermissionSet` 的构造、父层与子层求交、按 effect 取值，以及黑名单合并。
//! 不管：声明形状（`layer.rs` / `scope.rs`）、档位预设（`profile.rs`）、路径命中的细节（`path.rs`）。
//! 联动：`workspace` 做落点判定、`tools` 做围栏判定都读它；入站面见 `permission/api.rs`（docs/permission/README.md）。

use serde::Serialize;
use std::collections::BTreeMap;

use super::*;

/// 目的：一份生效权限：profile + reach + decision + 共享区范围 + 全局黑名单。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PermissionSet {
    /// 目的：生效档位（可能是父档位与子收窄后的档位）。
    pub profile: ProfileId,
    /// 目的：生效 reach。
    pub reach: ReachSet,
    /// 目的：生效 decision。
    pub decisions: Decisions,
    /// 目的：共享区读范围。
    pub shared_read: ScopeMode,
    /// 目的：共享区写范围。
    pub shared_write: ScopeMode,
    /// 目的：黑名单（全局硬底线 + 会话追加）。
    pub deny: Vec<String>,
    /// 目的：逐工具决定（根 ceiling 设定；子会话继承）。
    pub tools: BTreeMap<String, Decision>,
}

impl Default for PermissionSet {
    fn default() -> PermissionSet {
        PermissionDefaults::default().to_permission_set()
    }
}

impl PermissionSet {
    /// 目的：共享区一条相对路径能不能读。
    pub fn read_ok(&self, rel: &str) -> bool {
        scope_ok(&self.shared_read, &self.deny, rel)
    }

    /// 目的：共享区一条相对路径能不能提交。
    pub fn write_ok(&self, rel: &str) -> bool {
        scope_ok(&self.shared_write, &self.deny, rel)
    }

    /// 目的：读范围清单。
    pub fn read_roots_listing(&self) -> String {
        self.shared_read.listing()
    }

    /// 目的：写范围清单。
    pub fn write_roots_listing(&self) -> String {
        self.shared_write.listing()
    }

    /// 目的：读决定是否放行（不看 reach：判定"这一档读不读"）。
    pub fn read_allowed(&self) -> bool {
        self.decisions.read == Decision::Allow
    }

    /// 目的：写决定是否放行（不看 reach：判定"这一档写不写"）。
    pub fn write_allowed(&self) -> bool {
        self.decisions.write == Decision::Allow
    }

    /// 目的：这个 reach 的读是否开放：读决定允许，且这一项在 reach 里。
    pub fn may_read(&self, reach: Reach) -> bool {
        self.read_allowed() && self.reach.contains(reach)
    }

    /// 目的：这个 reach 的写是否开放：写决定允许，且这一项在 reach 里。
    /// 约束：共享区提交还要逐路径过 `write_ok`；这里只回答"这一席有没有这个写面"。
    pub fn may_write(&self, reach: Reach) -> bool {
        self.write_allowed() && self.reach.contains(reach)
    }

    /// 目的：共享区读是否开放：读决定允许、reach 含 shared-read，且范围不是显式无权限。
    pub fn may_read_shared(&self) -> bool {
        self.may_read(Reach::SharedRead) && !matches!(self.shared_read, ScopeMode::None)
    }

    /// 目的：写私有沙箱是否开放（内置文件工具与 `work_lost` 恢复共用这一条判定）。
    pub fn may_write_private(&self) -> bool {
        self.may_write(Reach::Private)
    }

    /// 目的：这一次调用（`granted` = 档位 reach 与工具最大 reach 求交后的结果）能不能写这个 reach。
    /// 约束：围栏按**本次授予**的 reach 派生，所以档位判定与本次判定必须同时成立。
    pub fn may_write_granted(&self, granted: &ReachSet, reach: Reach) -> bool {
        self.may_write(reach) && granted.contains(reach)
    }

    /// 目的：这一席的网络面是否开放（围栏的 net 开关问这一条）。
    pub fn allows_network(&self) -> bool {
        self.reach.contains(Reach::Network)
    }

    /// 目的：这个 scope 的执行处置是不是放行（不看逐工具覆盖：判定"这一档能不能跑"）。
    pub fn may_exec(&self, scope: CallScope) -> bool {
        match scope {
            CallScope::External => !self.decisions.exec_external.is_deny(),
            _ => !self.decisions.exec_sandbox.is_deny(),
        }
    }

    /// 目的：这次调用该用哪个 decision（逐工具覆盖 > profile 的 effect/scope 决定）。
    pub fn decision_for(&self, tool: &str, effect: Effect, scope: CallScope) -> Decision {
        if let Some(d) = self.tools.get(tool) {
            return *d;
        }
        match effect {
            Effect::Read => self.decisions.read,
            Effect::Write => self.decisions.write,
            Effect::Exec => match scope {
                CallScope::External => self.decisions.exec_external,
                _ => self.decisions.exec_sandbox,
            },
            Effect::None => Decision::Allow,
        }
    }

    /// 目的：这个工具在当前档位下要不要发放（deny 的工具不出现，ask 的出现）。
    pub fn offers(
        &self,
        tool: &str,
        effect: Effect,
        max_reach: &ReachSet,
        has_scope: bool,
    ) -> bool {
        if effect == Effect::None {
            return true;
        }
        let granted = self.reach.intersection(max_reach);
        if granted.is_empty() {
            return false;
        }
        if has_scope {
            let sandbox = self.decision_for(tool, effect, CallScope::Sandbox);
            let external = self.decision_for(tool, effect, CallScope::External);
            !(sandbox.is_deny() && external.is_deny())
        } else {
            !self.decision_for(tool, effect, CallScope::None).is_deny()
        }
    }

    /// 目的：按工具最大 reach 与本次 scope 求出生效 reach。
    pub fn grant(
        &self,
        max_reach: &ReachSet,
        requested: Option<&ReachSet>,
    ) -> Result<ReachSet, String> {
        let mut granted = self.reach.intersection(max_reach);
        if let Some(req) = requested {
            if !req.is_subset(&self.reach) {
                return Err("这次 scope 超出当前档位授予的可达范围".to_string());
            }
            if !req.is_subset(max_reach) {
                return Err("这个工具不支持本次 scope 要求的全部可达范围".to_string());
            }
            granted = granted.intersection(req);
        }
        Ok(granted)
    }

    /// 目的：应用根会话的 ceiling（可换档、可收窄、可覆盖路径与逐工具决定）。
    pub fn apply_root(&self, layer: &PermissionLayer) -> Result<PermissionSet, String> {
        layer.validate()?;
        let mut out = self.clone();
        if let Some(profile) = layer.profile {
            let spec = profile.spec();
            out.profile = profile;
            out.reach = spec.reach;
            out.decisions = spec.decisions;
        }
        if let Some(reach) = &layer.reach {
            if !reach.is_subset(&out.reach) {
                return Err("根会话的 reach 不能超出所选档位".to_string());
            }
            out.reach = reach.clone();
        }
        if let Some(shared) = &layer.shared {
            if let Some(v) = &shared.read {
                out.shared_read = v.clone();
            }
            if let Some(v) = &shared.write {
                out.shared_write = v.clone();
            }
            append_deny(&mut out.deny, &shared.deny);
        }
        if let Some(edit) = &layer.decisions {
            out.decisions = out.decisions.with_edit(edit, false)?;
        }
        for (tool, decision) in &layer.tools {
            out.tools.insert(tool.clone(), *decision);
        }
        Ok(out)
    }

    /// 目的：应用子会话的 narrowing（只能换更低档、只能收窄）。
    pub fn apply_narrowing(&self, layer: &PermissionLayer) -> Result<PermissionSet, String> {
        layer.validate()?;
        if !layer.tools.is_empty() {
            return Err("子会话不能逐工具改决定；要更严请换更低档位".to_string());
        }
        let mut out = self.clone();
        if let Some(profile) = layer.profile {
            if profile.rank() > self.profile.rank() {
                return Err("子会话不能选择比父会话更宽的档位".to_string());
            }
            let spec = profile.spec();
            out.profile = profile;
            out.reach = out.reach.intersection(&spec.reach);
            out.decisions = out.decisions.stricter(spec.decisions);
        }
        if let Some(reach) = &layer.reach {
            if !reach.is_subset(&out.reach) {
                return Err("子会话的 reach 只能收窄".to_string());
            }
            out.reach = reach.clone();
        }
        if let Some(shared) = &layer.shared {
            if let Some(v) = &shared.read {
                if !v.is_subset_of(&out.shared_read) {
                    return Err("子会话的共享区读范围只能收窄".to_string());
                }
                out.shared_read = out.shared_read.intersect(v);
            }
            if let Some(v) = &shared.write {
                if !v.is_subset_of(&out.shared_write) {
                    return Err("子会话的共享区写范围只能收窄".to_string());
                }
                out.shared_write = out.shared_write.intersect(v);
            }
            append_deny(&mut out.deny, &shared.deny);
        }
        if let Some(edit) = &layer.decisions {
            out.decisions = out.decisions.with_edit(edit, true)?;
        }
        Ok(out)
    }

    /// 目的：把共享区读写整个拿掉——外部工作区授权为 `none` 时，这条会话的内置文件工具与 shell 都读不到共享区。
    /// 约束：只动这一份生效权限，不改档位与别的 reach；共享区路径范围一并置 `None`，免得清单还声称可读。
    pub fn without_shared(mut self) -> PermissionSet {
        let kept = self
            .reach
            .to_vec()
            .into_iter()
            .filter(|r| !matches!(r, Reach::SharedRead | Reach::SharedWrite));
        self.reach = kept.collect();
        self.shared_read = ScopeMode::None;
        self.shared_write = ScopeMode::None;
        self
    }

    /// 目的：并上全局硬黑名单。
    pub fn with_hard_deny(mut self, deny: &[String]) -> PermissionSet {
        append_deny(&mut self.deny, deny);
        self
    }

    /// 目的：当前是否完全没有可用 reach（配置视图与拒绝文案用）。
    pub fn is_no_access(&self) -> bool {
        self.reach.is_empty()
    }
}

fn scope_ok(scope: &ScopeMode, deny: &[String], rel: &str) -> bool {
    if hit(deny, rel) {
        return false;
    }
    scope.allows(rel)
}

fn append_deny(out: &mut Vec<String>, add: &[String]) {
    for entry in add {
        if !out.iter().any(|e| normalize(e) == normalize(entry)) {
            out.push(entry.clone());
        }
    }
}
