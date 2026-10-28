//! 目的：权限的纯规则——reach × effect × decision、四档 profile、共享区路径范围与子会话收窄。
//! 管：权限声明（全局默认 / 会话 ceiling / 子会话 narrowing）的解析与求交；单次调用的可达范围与处置判定。
//! 不管：不认会话、不碰文件系统、不持端口、不持有状态；落盘在登记处与会话 meta，生效态每次现算。
//! 联动：workspace 用 PermissionSet 做落点判定；tools 用 Reach / Effect 生成围栏与工具面；语义见 docs/permission/README.md。

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// 目的：一次操作能到的资源域。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Reach {
    /// 目的：自己的私有沙箱。
    Private,
    /// 目的：读共享区主副本。
    SharedRead,
    /// 目的：写共享区（只有 work_commit 这条路径使用）。
    SharedWrite,
    /// 目的：读成员模块目录。
    ModuleRead,
    /// 目的：写模块自己的 userdata（只给模块工具进程）。
    ModuleWrite,
    /// 目的：用户显式授权的主机只读根。
    HostRead,
    /// 目的：出站网络。
    Network,
}

/// 目的：一组 reach；序列化成稳定排序的数组，便于配置视图与审计。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReachSet(BTreeSet<Reach>);

impl ReachSet {
    /// 目的：空集合（没有任何 reach）。
    pub fn empty() -> ReachSet {
        ReachSet(BTreeSet::new())
    }

    /// 目的：这个集合有没有这一项。
    pub fn contains(&self, r: Reach) -> bool {
        self.0.contains(&r)
    }

    /// 目的：是否为空。
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// 目的：两项的交集。
    pub fn intersection(&self, other: &ReachSet) -> ReachSet {
        ReachSet(self.0.intersection(&other.0).copied().collect())
    }

    /// 目的：自己是不是 other 的子集（收窄校验用）。
    pub fn is_subset(&self, other: &ReachSet) -> bool {
        self.0.is_subset(&other.0)
    }

    /// 目的：按稳定顺序列出（配置视图、错误文案与测试）。
    pub fn to_vec(&self) -> Vec<Reach> {
        self.0.iter().copied().collect()
    }
}

impl FromIterator<Reach> for ReachSet {
    fn from_iter<T: IntoIterator<Item = Reach>>(iter: T) -> ReachSet {
        ReachSet(iter.into_iter().collect())
    }
}

impl<const N: usize> From<[Reach; N]> for ReachSet {
    fn from(items: [Reach; N]) -> ReachSet {
        ReachSet(items.into_iter().collect())
    }
}

/// 目的：这次操作属于哪一类（决定用 profile 的哪一个 decision 字段）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    /// 目的：读类。
    Read,
    /// 目的：写类。
    Write,
    /// 目的：执行类（模块工具 / 未来的 shell）。
    Exec,
    /// 目的：不碰文件、不需要权限闸门。
    None,
}

impl Effect {
    /// 目的：旧 capability 标签到 effect 的默认映射；工具显式写了 effect 时以显式值为准。
    pub fn from_capability(capability: &str) -> Effect {
        match capability.trim() {
            "fs-read" => Effect::Read,
            "fs-write" => Effect::Write,
            "exec" => Effect::Exec,
            _ => Effect::None,
        }
    }

    /// 目的：没写 reach 时按 effect 给的默认可达范围。
    pub fn default_reach(self) -> ReachSet {
        match self {
            Effect::Read => ReachSet::from([Reach::Private, Reach::SharedRead, Reach::ModuleRead]),
            Effect::Write => ReachSet::from([Reach::Private]),
            Effect::Exec => ReachSet::from([Reach::Private]),
            Effect::None => ReachSet::empty(),
        }
    }
}

/// 目的：一次调用的处置。Deny 最严、Allow 最宽；收窄时取更严的一边。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// 目的：不执行。
    Deny,
    /// 目的：进统一裁决队。
    Ask,
    /// 目的：直接执行。
    #[default]
    Allow,
}

impl Decision {
    /// 目的：两者中更严的那个。
    pub fn stricter(self, other: Decision) -> Decision {
        self.min(other)
    }

    /// 目的：是不是拒绝。
    pub fn is_deny(self) -> bool {
        self == Decision::Deny
    }

    /// 目的：是不是要问。
    pub fn is_ask(self) -> bool {
        self == Decision::Ask
    }

    /// 目的：内部排序（0 = 最严）。
    fn rank(self) -> u8 {
        match self {
            Decision::Deny => 0,
            Decision::Ask => 1,
            Decision::Allow => 2,
        }
    }
}

/// 目的：四档用户可切换的会话权限预设（名称是呈现层文案，id 是契约）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProfileId {
    /// 目的：仅可查看。
    View,
    /// 目的：工作区内修改（默认）。
    #[default]
    Workspace,
    /// 目的：外部可达（逐次确认）。
    External,
    /// 目的：全可达（自动放行）。
    AllReach,
}

impl ProfileId {
    /// 目的：档位宽度排序（收窄校验用；高不能换低）。
    pub fn rank(self) -> u8 {
        match self {
            ProfileId::View => 0,
            ProfileId::Workspace => 1,
            ProfileId::External => 2,
            ProfileId::AllReach => 3,
        }
    }

    /// 目的：稳定 id（配置、状态栏与审计）。
    pub fn as_str(self) -> &'static str {
        match self {
            ProfileId::View => "view",
            ProfileId::Workspace => "workspace",
            ProfileId::External => "external",
            ProfileId::AllReach => "all_reach",
        }
    }

    /// 目的：用户标签。
    pub fn label(self) -> &'static str {
        match self {
            ProfileId::View => "仅可查看",
            ProfileId::Workspace => "工作区内修改",
            ProfileId::External => "外部可达（逐次确认）",
            ProfileId::AllReach => "全可达（自动放行）",
        }
    }

    /// 目的：一句话说明这个档位能做什么。
    pub fn summary(self) -> &'static str {
        match self {
            ProfileId::View => "读共享区与模块目录；写、提交与执行类工具都关闭。",
            ProfileId::Workspace => "可写自己的沙箱、提交共享区；执行类工具只能碰私有沙箱。",
            ProfileId::External => "在上一档之上可读主机只读根、可出网；越出沙箱的执行每次确认。",
            ProfileId::AllReach => {
                "reach 有多宽就授予多宽；仍受全局网络 / 主机根 / 黑名单与围栏上限。"
            }
        }
    }

    /// 目的：从配置串解析档位（只认四个 id）。
    pub fn parse(raw: &str) -> Result<ProfileId, String> {
        match raw.trim() {
            "view" => Ok(ProfileId::View),
            "workspace" => Ok(ProfileId::Workspace),
            "external" => Ok(ProfileId::External),
            "all_reach" => Ok(ProfileId::AllReach),
            other => Err(format!(
                "未知权限档位：{}（只认 view / workspace / external / all_reach）",
                other
            )),
        }
    }

    /// 目的：四档按从窄到宽排列（配置界面与校验共用）。
    pub fn all() -> [ProfileId; 4] {
        [
            ProfileId::View,
            ProfileId::Workspace,
            ProfileId::External,
            ProfileId::AllReach,
        ]
    }

    /// 目的：这个档位的 reach 与 decision 预设。
    pub fn spec(self) -> ProfileSpec {
        let (reach, decisions) = match self {
            ProfileId::View => (
                ReachSet::from([Reach::Private, Reach::SharedRead, Reach::ModuleRead]),
                Decisions {
                    read: Decision::Allow,
                    write: Decision::Deny,
                    exec_sandbox: Decision::Deny,
                    exec_external: Decision::Deny,
                },
            ),
            ProfileId::Workspace => (
                ReachSet::from([
                    Reach::Private,
                    Reach::SharedRead,
                    Reach::SharedWrite,
                    Reach::ModuleRead,
                    Reach::ModuleWrite,
                ]),
                Decisions {
                    read: Decision::Allow,
                    write: Decision::Allow,
                    exec_sandbox: Decision::Allow,
                    exec_external: Decision::Deny,
                },
            ),
            ProfileId::External => (
                ReachSet::from([
                    Reach::Private,
                    Reach::SharedRead,
                    Reach::SharedWrite,
                    Reach::ModuleRead,
                    Reach::ModuleWrite,
                    Reach::HostRead,
                    Reach::Network,
                ]),
                Decisions {
                    read: Decision::Allow,
                    write: Decision::Allow,
                    exec_sandbox: Decision::Allow,
                    exec_external: Decision::Ask,
                },
            ),
            ProfileId::AllReach => (
                ReachSet::from([
                    Reach::Private,
                    Reach::SharedRead,
                    Reach::SharedWrite,
                    Reach::ModuleRead,
                    Reach::ModuleWrite,
                    Reach::HostRead,
                    Reach::Network,
                ]),
                Decisions {
                    read: Decision::Allow,
                    write: Decision::Allow,
                    exec_sandbox: Decision::Allow,
                    exec_external: Decision::Allow,
                },
            ),
        };
        ProfileSpec {
            id: self,
            label: self.label(),
            summary: self.summary(),
            reach,
            decisions,
        }
    }
}

/// 目的：四档 profile 的完整预设（配置界面直接照抄，不在前端另写一份）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProfileSpec {
    /// 目的：档位 id。
    pub id: ProfileId,
    /// 目的：用户标签。
    pub label: &'static str,
    /// 目的：一句话说明。
    pub summary: &'static str,
    /// 目的：这一档授予的 reach。
    pub reach: ReachSet,
    /// 目的：这一档的默认 decision。
    pub decisions: Decisions,
}

/// 目的：按 effect / scope 分流的有效处置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Decisions {
    /// 目的：读类。
    pub read: Decision,
    /// 目的：写类。
    pub write: Decision,
    /// 目的：执行类、sandbox scope。
    pub exec_sandbox: Decision,
    /// 目的：执行类、external scope。
    pub exec_external: Decision,
}

impl Decisions {
    /// 目的：逐字段取更严的一边。
    pub fn stricter(self, other: Decisions) -> Decisions {
        Decisions {
            read: self.read.stricter(other.read),
            write: self.write.stricter(other.write),
            exec_sandbox: self.exec_sandbox.stricter(other.exec_sandbox),
            exec_external: self.exec_external.stricter(other.exec_external),
        }
    }

    /// 目的：应用一层显式覆盖；narrowing 为真时不允许放宽。
    fn with_edit(self, edit: &DecisionEdit, narrowing: bool) -> Result<Decisions, String> {
        let mut out = self;
        let apply =
            |label: &str, value: Option<Decision>, slot: &mut Decision| -> Result<(), String> {
                if let Some(v) = value {
                    if narrowing && v.rank() > slot.rank() {
                        return Err(format!("子会话的 {} 决定只能更严", label));
                    }
                    *slot = v;
                }
                Ok(())
            };
        apply("read", edit.read, &mut out.read)?;
        apply("write", edit.write, &mut out.write)?;
        apply("exec_sandbox", edit.exec_sandbox, &mut out.exec_sandbox)?;
        apply("exec_external", edit.exec_external, &mut out.exec_external)?;
        Ok(out)
    }
}

/// 目的：共享区的路径范围三态；空集合不再兼作整棵。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeMode {
    /// 目的：整棵共享区。
    All,
    /// 目的：显式无权限。
    None,
    /// 目的：只允许这些子树。
    Paths(Vec<String>),
}

impl ScopeMode {
    /// 目的：一条共享区相对路径在不在范围内。
    pub fn allows(&self, rel: &str) -> bool {
        match self {
            ScopeMode::All => true,
            ScopeMode::None => false,
            ScopeMode::Paths(paths) => hit(paths, rel),
        }
    }

    /// 目的：许可根清单（拒绝文案与配置视图）。
    pub fn listing(&self) -> String {
        match self {
            ScopeMode::All => "（整棵共享区）".to_string(),
            ScopeMode::None => "（无）".to_string(),
            ScopeMode::Paths(paths) => listing(paths),
        }
    }

    /// 目的：两项的交集；没有交集 = 显式无权限。
    pub fn intersect(&self, other: &ScopeMode) -> ScopeMode {
        match (self, other) {
            (ScopeMode::None, _) | (_, ScopeMode::None) => ScopeMode::None,
            (ScopeMode::All, x) | (x, ScopeMode::All) => x.clone(),
            (ScopeMode::Paths(a), ScopeMode::Paths(b)) => {
                let mut out: Vec<String> = Vec::new();
                for x in a {
                    for y in b {
                        if overlaps(x, y) {
                            let p = narrower(x, y);
                            if !out.iter().any(|e| normalize(e) == p) {
                                out.push(p);
                            }
                        }
                    }
                }
                out.sort();
                if out.is_empty() {
                    ScopeMode::None
                } else {
                    ScopeMode::Paths(out)
                }
            }
        }
    }

    /// 目的：自己是不是 parent 的子集（子会话收窄校验用）。
    pub fn is_subset_of(&self, parent: &ScopeMode) -> bool {
        match (self, parent) {
            (_, ScopeMode::All) => true,
            (ScopeMode::None, _) => true,
            (ScopeMode::All, _) => false,
            (ScopeMode::Paths(_), ScopeMode::None) => false,
            (ScopeMode::Paths(mine), ScopeMode::Paths(theirs)) => mine.iter().all(|p| {
                let p = normalize(p);
                theirs.iter().any(|q| {
                    let q = normalize(q);
                    q.is_empty() || q == "." || p == q || p.starts_with(&format!("{}/", q))
                })
            }),
        }
    }

    /// 目的：校验路径条目。
    pub fn validate(&self, label: &str) -> Result<(), String> {
        if let ScopeMode::Paths(paths) = self {
            for e in paths {
                validate_entry(e).map_err(|why| format!("{}：{}", label, why))?;
            }
        }
        Ok(())
    }
}

fn default_all() -> ScopeMode {
    ScopeMode::All
}

impl Serialize for ScopeMode {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            ScopeMode::All => s.serialize_str("all"),
            ScopeMode::None => s.serialize_str("none"),
            ScopeMode::Paths(paths) => paths.serialize(s),
        }
    }
}

impl<'de> Deserialize<'de> for ScopeMode {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<ScopeMode, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = ScopeMode;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("all / none / 路径数组")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<ScopeMode, E> {
                match v {
                    "all" => Ok(ScopeMode::All),
                    "none" => Ok(ScopeMode::None),
                    other => Err(E::custom(format!(
                        "共享区范围只认 all / none / 路径数组，收到：{}",
                        other
                    ))),
                }
            }
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<ScopeMode, A::Error> {
                let mut paths = Vec::new();
                while let Some(v) = seq.next_element::<String>()? {
                    paths.push(v);
                }
                Ok(ScopeMode::Paths(paths))
            }
        }
        d.deserialize_any(V)
    }
}

/// 目的：全局默认的共享区范围（读写必须显式给出，空数组不再兼作整棵）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedScope {
    /// 目的：读范围。
    #[serde(default = "default_all")]
    pub read: ScopeMode,
    /// 目的：写范围。
    #[serde(default = "default_all")]
    pub write: ScopeMode,
    /// 目的：追加黑名单。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny: Vec<String>,
}

impl Default for SharedScope {
    fn default() -> Self {
        SharedScope {
            read: ScopeMode::All,
            write: ScopeMode::All,
            deny: Vec::new(),
        }
    }
}

/// 目的：一层会话覆盖的共享区范围；None = 不覆盖，Some(None) = 显式无权限。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedEdit {
    /// 目的：读范围。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read: Option<ScopeMode>,
    /// 目的：写范围。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<ScopeMode>,
    /// 目的：追加黑名单。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny: Vec<String>,
}

/// 目的：一层显式 decision 覆盖。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionEdit {
    /// 目的：读类。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read: Option<Decision>,
    /// 目的：写类。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<Decision>,
    /// 目的：执行类、sandbox scope。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exec_sandbox: Option<Decision>,
    /// 目的：执行类、external scope。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exec_external: Option<Decision>,
}

/// 目的：一次调用的 scope 请求（只对声明了 scope 的执行类工具有意义）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallScope {
    /// 目的：非执行类工具：没有 scope。
    None,
    /// 目的：只碰私有沙箱。
    Sandbox,
    /// 目的：沙箱之外再加共享区 / 模块目录 / 主机只读根 / 网络（私有沙箱仍保留）。
    External,
}

impl CallScope {
    /// 目的：解析模型给的 scope 参数；省略 = sandbox。
    pub fn from_request(raw: Option<&str>) -> Result<CallScope, String> {
        match raw.map(str::trim).filter(|s| !s.is_empty()) {
            None | Some("sandbox") => Ok(CallScope::Sandbox),
            Some("external") => Ok(CallScope::External),
            Some(other) => Err(format!("scope 只认 sandbox / external，收到：{}", other)),
        }
    }

    /// 目的：这个 scope 请求对应的 reach。
    pub fn reach(self) -> ReachSet {
        match self {
            CallScope::None => ReachSet::empty(),
            CallScope::Sandbox => ReachSet::from([Reach::Private]),
            CallScope::External => ReachSet::from([
                Reach::Private,
                Reach::SharedRead,
                Reach::ModuleRead,
                Reach::HostRead,
                Reach::Network,
            ]),
        }
    }
}

/// 目的：全局默认权限（设置里的 sandbox.defaults）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionDefaults {
    /// 目的：默认档位。
    #[serde(default)]
    pub profile: ProfileId,
    /// 目的：默认共享区范围。
    #[serde(default)]
    pub shared: SharedScope,
    /// 目的：默认逐工具决定（工具名 → decision）。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tools: BTreeMap<String, Decision>,
}

impl Default for PermissionDefaults {
    fn default() -> Self {
        PermissionDefaults {
            profile: ProfileId::Workspace,
            shared: SharedScope::default(),
            tools: BTreeMap::new(),
        }
    }
}

impl PermissionDefaults {
    /// 目的：把全局默认变成一份生效权限（不含全局硬黑名单）。
    pub fn to_permission_set(&self) -> PermissionSet {
        let spec = self.profile.spec();
        PermissionSet {
            profile: self.profile,
            reach: spec.reach,
            decisions: spec.decisions,
            shared_read: self.shared.read.clone(),
            shared_write: self.shared.write.clone(),
            deny: self.shared.deny.clone(),
            tools: self.tools.clone(),
        }
    }
}

/// 目的：一层会话权限（根 ceiling 或子 narrowing）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionLayer {
    /// 目的：档位；根 = 选择预设，子 = 只能换更低档。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<ProfileId>,
    /// 目的：reach 收窄；子必须是父的子集。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reach: Option<ReachSet>,
    /// 目的：共享区范围。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared: Option<SharedEdit>,
    /// 目的：decision 覆盖；子只能更严。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decisions: Option<DecisionEdit>,
    /// 目的：逐工具决定；子会话不能再改（避免绕过 tool effect 的校验）。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tools: BTreeMap<String, Decision>,
}

impl PermissionLayer {
    /// 目的：校验条目形状（不判断父子宽窄）。
    pub fn validate(&self) -> Result<(), String> {
        if let Some(shared) = &self.shared {
            if let Some(v) = &shared.read {
                v.validate("shared.read")?;
            }
            if let Some(v) = &shared.write {
                v.validate("shared.write")?;
            }
            for e in &shared.deny {
                validate_entry(e).map_err(|why| format!("shared.deny：{}", why))?;
            }
        }
        for name in self.tools.keys() {
            if name.trim().is_empty() {
                return Err("tools 里的工具名不能为空".to_string());
            }
        }
        Ok(())
    }
}

/// 目的：会话 meta 的权限段——根只有 ceiling，子只有 narrowing。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionSandbox {
    /// 目的：根会话的上限。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ceiling: Option<PermissionLayer>,
    /// 目的：子会话的收窄层。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narrowing: Option<PermissionLayer>,
}

impl SessionSandbox {
    /// 目的：取根会话的 ceiling；子会话带了 ceiling = 非法。
    pub fn root_layer(&self) -> Result<Option<&PermissionLayer>, String> {
        if self.narrowing.is_some() {
            return Err("根会话的 sandbox 不能有 narrowing".to_string());
        }
        Ok(self.ceiling.as_ref())
    }

    /// 目的：取子会话的 narrowing；根会话带了 narrowing = 非法。
    pub fn child_layer(&self) -> Result<Option<&PermissionLayer>, String> {
        if self.ceiling.is_some() {
            return Err("子会话的 sandbox 不能有 ceiling".to_string());
        }
        Ok(self.narrowing.as_ref())
    }
}

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

    /// 目的：这次调用该用哪个 decision（逐工具覆盖 > profile 的 effect/scope 决定）。
    pub fn decision_for(
        &self,
        tool: &str,
        effect: Effect,
        scope: CallScope,
        _default: Decision,
    ) -> Decision {
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
            let sandbox = self.decision_for(tool, effect, CallScope::Sandbox, Decision::Allow);
            let external = self.decision_for(tool, effect, CallScope::External, Decision::Allow);
            !(sandbox.is_deny() && external.is_deny())
        } else {
            !self
                .decision_for(tool, effect, CallScope::None, Decision::Allow)
                .is_deny()
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

/// 目的：命中判定：点（或空）代表整棵共享区；其余按路径组件匹配目录子树。
pub fn hit(list: &[String], rel: &str) -> bool {
    let rel = normalize(rel);
    list.iter().any(|entry| {
        let e = normalize(entry);
        if e.is_empty() || e == "." {
            return true;
        }
        rel == e || rel.starts_with(&format!("{}/", e))
    })
}

/// 目的：统一书写形式：去首尾空白与斜杠、反斜杠归一成斜杠。
pub fn normalize(s: &str) -> String {
    s.trim().replace('\\', "/").trim_matches('/').to_string()
}

fn listing(entries: &[String]) -> String {
    if entries.is_empty() {
        return "（整棵共享区）".to_string();
    }
    entries
        .iter()
        .map(|e| normalize(e))
        .collect::<Vec<_>>()
        .join("、")
}

fn overlaps(a: &str, b: &str) -> bool {
    let a = normalize(a);
    let b = normalize(b);
    if a.is_empty() || a == "." || b.is_empty() || b == "." {
        return true;
    }
    a == b || a.starts_with(&format!("{}/", b)) || b.starts_with(&format!("{}/", a))
}

fn narrower(a: &str, b: &str) -> String {
    let a = normalize(a);
    let b = normalize(b);
    if a.is_empty() || a == "." {
        return b;
    }
    if b.is_empty() || b == "." {
        return a;
    }
    let ac = a.split('/').count();
    let bc = b.split('/').count();
    if ac >= bc {
        a
    } else {
        b
    }
}

/// 目的：校验一条共享区相对路径条目：非空、相对、无上级段与空段；点合法（整棵）。
pub fn validate_entry(entry: &str) -> Result<(), String> {
    let raw = entry.trim();
    if raw.is_empty() {
        return Err("路径条目不能为空".to_string());
    }
    let flat = raw.replace('\\', "/");
    if flat.starts_with('/') || flat.starts_with('\\') {
        return Err(format!("路径条目必须是相对共享区根的路径：{}", raw));
    }
    let bytes = flat.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && (bytes[0] as char).is_ascii_alphabetic() {
        return Err(format!("路径条目必须是相对共享区根的路径：{}", raw));
    }
    for seg in flat.split('/') {
        if seg.is_empty() {
            return Err(format!("路径条目含空段（连续分隔符）：{}", raw));
        }
        if seg == ".." {
            return Err(format!("路径条目不能含上级段：{}", raw));
        }
    }
    Ok(())
}

/// 目的：校验全局默认权限。
pub fn validate_defaults(d: &PermissionDefaults) -> Result<(), String> {
    d.shared.read.validate("shared.read")?;
    d.shared.write.validate("shared.write")?;
    for e in &d.shared.deny {
        validate_entry(e).map_err(|why| format!("shared.deny：{}", why))?;
    }
    for name in d.tools.keys() {
        if name.trim().is_empty() {
            return Err("tools 里的工具名不能为空".to_string());
        }
    }
    Ok(())
}

/// 目的：校验一层会话权限的条目形状。
pub fn validate_layer(layer: &PermissionLayer) -> Result<(), String> {
    layer.validate()
}
