//! **工作与会话配置**：运行包报告、会话配置的读改（session_config / edit_session）、建工作与上传、文件视图。
//!
//! 工作名唯一化与会话视图（在世会话 × 历史的并集）也在这里。
//!
//! 它是 Conductor 的一个方法族：与 mod.rs 同在 service 模块下（子模块看得见父模块的私有字段），
//! 方法取 pub(crate)（或 pub）供兄弟族与 conductor/api.rs 调用。

use super::*;

impl Conductor {
    /// 运行能力报告：模块声明的能力、包库里的可用版本、缺失清单与虚拟机档诊断。
    /// 「清单即事实」：每次调用重扫模块清单与包库；本机档不装载运行包，missing 只作事实呈现。
    pub fn runtime_report(&self, tier: crate::kernel::api::Tier) -> RuntimeReport {
        let roster = self.workspace.roster();
        let lib = self.workspace.library();
        let spec = crate::capabilities::workspace::api::ExecSpec {
            tier,
            ..crate::capabilities::workspace::api::ExecSpec::default()
        };
        let diagnoses = if tier == crate::kernel::api::Tier::Vm {
            crate::capabilities::workspace::api::vm_diagnoses(&roster.modules, &lib, &spec)
        } else {
            Vec::new()
        };
        let readiness = crate::capabilities::workspace::api::tier_readiness(
            &spec,
            self.qemu_path(),
            self.probe.as_ref(),
        );
        RuntimeReport {
            tier: tier.as_str().to_string(),
            declared: crate::capabilities::workspace::api::declared(&roster.modules),
            available: lib.capability_versions(),
            missing: crate::capabilities::workspace::api::absent(&roster.modules, &lib),
            diagnoses,
            tier_ready: readiness.ready(),
            tier_missing: readiness.missing().iter().map(|s| s.to_string()).collect(),
            rejected: roster.rejected.clone(),
            rejected_packages: lib.rejected.clone(),
        }
    }

    /// 新建工作的**档位选择**（默认档 + 虚拟机档可用性与逐项前置）：与「开始」的校验同源。
    /// 为什么单独一条读面：创建向导还没有 sid，拿不到会话配置视图；而设置里的默认档与
    /// 虚拟机档的承载探针是**与某条会话无关**的事实。
    pub fn tier_choices(&self) -> TierChoices {
        let default = self.registry.app().tier;
        // 裸虚拟机档探针：基础根属于会话选型（在会话的 exec 段里），创建时还没填，按未指定探。
        let vm_probe = crate::capabilities::workspace::api::ExecSpec {
            tier: crate::kernel::api::Tier::Vm,
            ..crate::capabilities::workspace::api::ExecSpec::default()
        };
        let inputs = crate::capabilities::workspace::api::VmInputs {
            base: vm_probe.base.as_deref(),
            qemu: self.qemu_path(),
            probe: self.probe.as_ref(),
        };
        TierChoices {
            default: default.as_str().to_string(),
            vm_available: crate::capabilities::workspace::api::tier_readiness(
                &vm_probe,
                self.qemu_path(),
                self.probe.as_ref(),
            )
            .ready(),
            vm_unavailable_reason: crate::capabilities::workspace::api::tier_refusal(
                &vm_probe,
                self.qemu_path(),
                self.probe.as_ref(),
            )
            .unwrap_or_default(),
            vm_requirements: crate::capabilities::workspace::api::vm_requirements(&inputs),
        }
    }

    /// 本档位下不能执行工具的模块（模块 id → 缺的能力名）：建会话与重建时收口给工具环境。
    pub(crate) fn unavailable_modules(
        &self,
        spec: &crate::capabilities::workspace::api::ExecSpec,
        modules: &[Module],
    ) -> BTreeMap<String, Vec<String>> {
        crate::capabilities::workspace::api::unavailable(spec, modules, &self.workspace.library())
    }

    /// 配置视图：把「能改什么、现在是什么、缺什么」如实给出（每次读取都重扫模块清单与包库）。
    pub fn session_config(&self, sid: &str) -> Result<SessionConfig, String> {
        let (meta, events) = self.history_open(sid)?;
        let tier = meta.exec.tier;
        // 虚拟机档的承载探针：用用户填的基础根（若有），否则问"裸虚拟机档"能不能成立。
        let vm_probe = crate::capabilities::workspace::api::ExecSpec {
            tier: crate::kernel::api::Tier::Vm,
            base: meta.exec.base.clone(),
            ..crate::capabilities::workspace::api::ExecSpec::default()
        };
        Ok(SessionConfig {
            sid: meta.name.clone(),
            mode: meta.mode.clone(),
            started: session_started(&events),
            agents: meta
                .agents
                .iter()
                .map(|a| ConfigAgent {
                    name: a.name.clone(),
                    modules: a.modules.clone(),
                    model: a.model.clone().unwrap_or_default(),
                    // 视图要把现值给出去；编辑回包缺了它 = 保留现值（见 SessionEdit 的字段说明）。
                    permissions: Some(a.permissions.clone()),
                })
                .collect(),
            tier: tier.as_str().to_string(),
            base: meta.exec.base.clone(),
            net: meta.exec.net,
            pins: meta.exec.pins.clone(),
            runtime: self.runtime_report(tier),
            runtimes_dir: crate::kernel::api::slash(&self.workspace.runtimes_dir()),
            tier_ready: crate::capabilities::workspace::api::tier_readiness(
                &meta.exec,
                self.qemu_path(),
                self.probe.as_ref(),
            )
            .ready(),
            tier_missing: crate::capabilities::workspace::api::tier_readiness(
                &meta.exec,
                self.qemu_path(),
                self.probe.as_ref(),
            )
            .missing()
            .iter()
            .map(|s| s.to_string())
            .collect(),
            // 虚拟机档能不能选**与当前档位无关**：本机档会话也要如实告诉用户 vm 现在不可用（界面据此禁用）。
            // 逐项清单一起给出：界面照抄"缺哪几项、每项怎么补"，不自己编话。
            vm_available: crate::capabilities::workspace::api::tier_readiness(
                &vm_probe,
                self.qemu_path(),
                self.probe.as_ref(),
            )
            .ready(),
            vm_unavailable_reason: crate::capabilities::workspace::api::tier_refusal(
                &vm_probe,
                self.qemu_path(),
                self.probe.as_ref(),
            )
            .unwrap_or_default(),
            vm_requirements: crate::capabilities::workspace::api::vm_requirements(
                &crate::capabilities::workspace::api::VmInputs {
                    base: vm_probe.base.as_deref(),
                    qemu: self.qemu_path(),
                    probe: self.probe.as_ref(),
                },
            ),
        })
    }

    /// 编辑提交：校验 → 写回 meta.yaml（名单与选型的唯一真相）→ 追加一条旁路配置记录 → 丢掉内存会话。
    /// 生效点：下一次访问按新配置从转录重建会话对象（所以改完不必重开会话）。
    /// 冻结：流水里有内容（会话已经开过）时，agent 名单与形态不可改——换人请新建会话。
    pub fn edit_session(&mut self, sid: &str, edit: SessionEdit) -> Result<(), String> {
        if self.running.contains(sid) {
            return Err(Self::running_refusal(sid));
        }
        let (meta, events) = self.history_open(sid)?;
        match meta.mode.as_str() {
            "single" | "collab" => {}
            // 代理会话没有名单可编辑（决定权整块交给核心）：如实说明，不报"未知形态"。
            "proxy" => return Err("代理会话没有可编辑的名单：它不是一个 agent 工作".to_string()),
            other => {
                return Err(format!(
                    "未知会话形态：{}（只认 single / collab / proxy）",
                    other
                ))
            }
        }
        if session_started(&events) {
            let old: Vec<String> = meta.agents.iter().map(|a| a.name.clone()).collect();
            let new: Vec<String> = edit.agents.iter().map(|a| a.name.clone()).collect();
            if old != new {
                return Err(
                    "这轮会话已经开过：agent 名单与形态冻结（要换人请新建会话）".to_string()
                );
            }
        }
        // 校验：模块与模型真实存在；同一模块不得同属两个 agent（沙箱与发言归属会歧义）。
        let roster = self.scan();
        let reserved = self.reserved_names();
        let mut seen: Vec<String> = Vec::new();
        let mut metas: Vec<AgentMeta> = Vec::new();
        // 旧权限按名字留档：这次编辑没给某个 agent 的 permissions（None）= 保留它的现值。
        let old_permissions: std::collections::BTreeMap<
            String,
            crate::capabilities::permission::api::PermissionsOverride,
        > = meta
            .agents
            .iter()
            .map(|x| (x.name.clone(), x.permissions.clone()))
            .collect();
        for a in &edit.agents {
            crate::capabilities::registry::api::validate_name(&a.name)?;
            crate::capabilities::registry::api::check_reserved(&a.name, &reserved)?;
            if let Some(p) = &a.permissions {
                crate::capabilities::permission::api::validate_override(p)?;
            }
            for id in &a.modules {
                if !roster.modules.iter().any(|m| &m.manifest.id == id) {
                    return Err(format!("无此模块：{}", id));
                }
                if seen.iter().any(|x| x == id) {
                    return Err(format!(
                        "模块 {} 被多个 agent 同时使用；同一模块只能属于一个 agent",
                        id
                    ));
                }
                seen.push(id.clone());
            }
            if !a.model.is_empty() {
                if !self.registry.has_model(&a.model) {
                    return Err(format!("无此模型：{}", a.model));
                }
                self.registry.resolve(&a.model)?;
            }
            metas.push(AgentMeta {
                name: a.name.clone(),
                transient: !self.registry.has_agent(&a.name),
                modules: a.modules.clone(),
                model: if a.model.is_empty() {
                    None
                } else {
                    Some(a.model.clone())
                },
                permissions: a
                    .permissions
                    .clone()
                    .unwrap_or_else(|| old_permissions.get(&a.name).cloned().unwrap_or_default()),
            });
        }
        if metas.is_empty() {
            return Err("至少要有一个 agent".to_string());
        }
        if meta.mode == "single" && metas.len() != 1 {
            return Err("单 agent 形态只接受一个 agent（模块数不限）".to_string());
        }
        // 档位：与「开始」同一把尺子——虚拟机档的选型不成立（多版本未定版 / 定版不存在 / 路径冲突）如实拒绝。
        let tier = match edit.tier.as_str() {
            "host" => crate::kernel::api::Tier::Host,
            "vm" => crate::kernel::api::Tier::Vm,
            other => return Err(format!("未知执行档位：{}（只认 host / vm）", other)),
        };
        let spec = crate::capabilities::workspace::api::ExecSpec {
            tier,
            base: edit.base.clone(),
            pins: edit.pins.clone(),
            net: edit.net,
        };
        // 承载校验：前置条件不具备时**不允许改入虚拟机档**（用户环境问题，不是选型问题）。
        // 界面上的"能不能选"由 SessionConfig 的 tier_ready 说同一件事，两处不会各说各话。
        // 已经在虚拟机档上的会话只校验**它自己那几项**（基础根等）：改模块、改模型、定版、开网络都不该被拦住——
        // 一条已存在的会话连改都不让改，是拿用户自己的记录当人质。
        let staying_vm =
            meta.exec.tier == crate::kernel::api::Tier::Vm && tier == crate::kernel::api::Tier::Vm;
        if staying_vm {
            // 留在 vm 档：只校验用户这次填的基础根（填错路径就是填错路径），
            // 不拿"本机能不能提供 vm 档"去拦一条已经存在的会话。
            let base_item = crate::capabilities::workspace::api::tier_readiness(
                &spec,
                self.qemu_path(),
                self.probe.as_ref(),
            )
            .requirements
            .into_iter()
            .find(|r| r.id == "base");
            if let Some(item) = base_item {
                if !item.met {
                    return Err(format!("{}：{}", item.detail, item.how));
                }
            }
        } else if let Some(why) = crate::capabilities::workspace::api::tier_refusal(
            &spec,
            self.qemu_path(),
            self.probe.as_ref(),
        ) {
            return Err(why);
        }
        let session_modules: Vec<Module> = roster
            .modules
            .iter()
            .filter(|m| seen.iter().any(|id| id == &m.manifest.id))
            .cloned()
            .collect();
        let plan = crate::capabilities::workspace::api::plan(
            &spec,
            &session_modules,
            &self.workspace.library(),
        )
        .map_err(|diags| crate::capabilities::workspace::api::diagnose_text(&diags))?;
        self.log.info(
            "conductor::edit_session",
            &format!(
                "sid={}；{}",
                sid,
                crate::capabilities::workspace::api::plan_summary(&plan)
            ),
        );

        let mut new_meta = meta.clone();
        new_meta.modules = metas.iter().flat_map(|a| a.modules.clone()).collect();
        new_meta.agents = metas;
        new_meta.exec = spec;
        self.history.save_meta(&new_meta)?;
        // 权限或模块可能变了：撤掉按**旧配置**写下的围栏授权；下一次工具执行按新配置重授（即时生效）。
        self.release_session_fences(&meta);
        self.record_config(sid, &new_meta);
        // 内存里那份是按旧配置装的：丢掉它，下一次访问按新配置从转录重建（转录即状态，不丢内容）。
        self.sessions.remove(sid);
        Ok(())
    }

    /// 撤掉一次会话按**给定 meta** 写下的围栏授权（权限或模块变更后调用）。
    /// 其它平台没有持久授权（release 是空操作）；Windows 撤 ACE，下一次 `prepare_fence` 按实际 ACE 重授。
    fn release_session_fences(&self, meta: &SessionMeta) {
        let roster = self.workspace.roster();
        match self.sandboxes(meta, &roster) {
            Ok(sandboxes) => {
                for sb in &sandboxes.list {
                    let spec =
                        crate::capabilities::tools::api::FenceSpec::from_sandbox(sb, meta.exec.net)
                            .with_read_only(self.fence_read_roots());
                    if let Err(e) = self.tools.release_fence(&spec) {
                        self.log.warn(
                            "conductor::release_session_fences",
                            &format!("撤销围栏授权未完成：{}", e),
                        );
                    }
                }
            }
            Err(e) => self.log.warn(
                "conductor::release_session_fences",
                &format!("取沙箱失败，未撤销授权：{}", e),
            ),
        }
    }

    /// 追加一条旁路配置记录：只作呈现与审计（不进模型上下文，回放与状态派生都跳过它）。
    pub(crate) fn record_config(&self, sid: &str, meta: &SessionMeta) {
        let ev = serde_json::json!({
            "type": "config",
            "ts": now_ts(),
            "mode": meta.mode,
            "tier": meta.exec.tier.as_str(),
            "base": meta.exec.base,
            "net": meta.exec.net,
            "pins": meta.exec.pins,
            "agents": meta.agents.iter().map(|a| serde_json::json!({
                "name": a.name,
                "modules": a.modules,
                "model": a.model,
            })).collect::<Vec<_>>(),
        });
        if let Err(e) = self.history.append(sid, &[ev]) {
            self.log.warn(
                "conductor::record_config",
                &format!("配置记录落盘失败：{}", e),
            );
        }
    }

    /// 工作名的缺省与唯一化（命名策略在 `session`；这里只提供"存在吗"）。
    pub fn unique_work_name(&self, base: &str, fallback: &str) -> String {
        crate::capabilities::session::api::unique_work_name(base, fallback, |n| {
            self.sessions.contains_key(n) || self.history.load(n).is_ok()
        })
    }

    /// 本次模型调用的通道参数：**预算与"能不能流式"都取全局设置**（讨论、执行、验收、单 agent 共用一份）。
    /// `want_stream` 是调用方这一次的意愿（呈现层按回包形状给）：**设置是上限，调用方可以在本次放弃流式**；
    /// 设置关掉时一律非流式。两处各判一次迟早会打架，所以判据只在这里。
    pub(crate) fn llm_opts(&self, want_stream: bool) -> crate::capabilities::llm::api::LlmOpts {
        let app = self.registry.app();
        crate::capabilities::llm::api::LlmOpts {
            stream: app.streaming && want_stream,
            timeout_secs: app.llm_timeout_secs,
        }
    }

    /// 设置里登记的 QEMU 可执行文件路径（默认空 = 兜底看 PATH）。产品不自带、不下载 QEMU。
    pub(crate) fn qemu_path(&self) -> Option<&str> {
        let p = self.registry.app().qemu_path.trim();
        if p.is_empty() {
            None
        } else {
            Some(p)
        }
    }

    /// 用户显式授权的只读根（`settings.yaml` 的 `fence_read`）。
    /// 策略层只带事实：哪些目录只读可达由用户定，只读位怎么落由适配层定。
    /// 空 = 一个都不放行（默认不动本机任何权限项）。
    pub(crate) fn fence_read_roots(&self) -> Vec<std::path::PathBuf> {
        self.registry
            .app()
            .fence_read
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(std::path::PathBuf::from)
            .collect()
    }

    // ---- 会话中心（前端只持 id） ----

    /// 会话列表视图（进行中的工作）。形态取落盘 meta（单一真相，不在内存里留影子状态）。
    pub fn session_views(&self, history: &[HistoryView]) -> Vec<SessionView> {
        // 在表里的会话 + **正在生成的会话**（后者对象在工作线程上，但它确实存在、也确实在跑）。
        // 漏掉它们会让界面以为会话不见了。
        let running: Vec<(String, bool)> = self
            .running
            .iter()
            .map(|sid| (sid.clone(), false))
            .collect();
        let running_now: std::collections::BTreeSet<String> =
            self.running.iter().cloned().collect();
        let listed: Vec<(String, bool)> = self
            .sessions
            .iter()
            .map(|(sid, s)| {
                let done = match s {
                    Session::Collab(c) => c.is_done(),
                    Session::Single(_) => false,
                };
                (sid.clone(), done)
            })
            .chain(running)
            .collect();
        listed
            .into_iter()
            .map(|(sid, done)| {
                let entry = history.iter().find(|h| h.name == sid);
                let mode = entry.map(|h| h.mode.clone()).unwrap_or_default();
                // 运行态取落盘事实（不在内存里留影子状态）；拿不到（生成中 / 未落盘）按正常运行。
                let run = entry
                    .map(|h| h.run.as_str())
                    .unwrap_or("active")
                    .to_string();
                // 记的档位来自落盘 meta（权威）：环境后来变了也要如实提示——**不拦打开**（记录是用户的）。
                let exec = entry.map(|h| h.exec.clone()).unwrap_or_default();
                let readiness = crate::capabilities::workspace::api::tier_readiness(
                    &exec,
                    self.qemu_path(),
                    self.probe.as_ref(),
                );
                // 「改需求」能力位：有本次需求行才给。在表里看会话种类；不在表里（生成中/未打开）
                // 看落盘 meta 的形态（那是名单与形态的单一真相）。
                let can_update_task = match self.sessions.get(&sid) {
                    Some(Session::Collab(_)) => true,
                    Some(Session::Single(_)) => false,
                    None => mode == "collab",
                };
                // 待裁决：对象不在表里（正在生成）时拿不到，如实给 None（推的卡片事件会补上）。
                // 形状与推的那个事件**同一份**（快照与推都只从 Pending::event 来）。
                let pending = match self.sessions.get(&sid) {
                    Some(Session::Collab(c)) => c.open_card_json(),
                    _ => None,
                };
                SessionView {
                    running: running_now.contains(&sid),
                    sid,
                    mode,
                    done,
                    tier: exec.tier.as_str().to_string(),
                    tier_ready: readiness.ready(),
                    tier_missing: readiness.missing().iter().map(|s| s.to_string()).collect(),
                    can_update_task,
                    run,
                    pending,
                }
            })
            .collect()
    }

    pub fn session_exists(&self, name: &str) -> bool {
        self.sessions.contains_key(name)
    }

    /// 创建工作：形态 + 参与的 agent（+ 协作需求）→ 建出会话、落盘身份、备好工作区。
    /// 一切选择来自用户；核心只做校验与机械装配，不替用户选。
    pub fn create_work(&mut self, spec: WorkSpec) -> Result<WorkOpened, String> {
        self.create_work_inner(spec, None)
    }

    /// 这棵子树里已经用过的 agent 实例名（实例名就是沙箱目录名，必须**全树唯一**）。
    /// 顶层会话名可以重复（落点是自己的目录），但共用同一个 work/ 时同名 agent 会撞同一个沙箱。
    fn subtree_agent_names(&self, root: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for sid in self.subtree_of(root) {
            if let Ok(m) = self.history.meta(&sid) {
                for a in &m.agents {
                    if !out.iter().any(|x| x == &a.name) {
                        out.push(a.name.clone());
                    }
                }
            }
        }
        out
    }

    /// 建工作的唯一实现：`parent` = 编排归属（代理建的**子工作**）。
    /// 子工作与父会话**共用顶层那一个 work/**，落点在父会话目录的 `children/` 下。
    pub(crate) fn create_work_inner(
        &mut self,
        spec: WorkSpec,
        parent: Option<&str>,
    ) -> Result<WorkOpened, String> {
        validate_work_name(&spec.name)?;
        if self.sessions.contains_key(&spec.name) || self.history.load(&spec.name).is_ok() {
            return Err(format!("工作名已存在：{}", spec.name));
        }
        // **代理形态**（第三人形态）：没有名单、没有需求——用户选这一形态就是**授予全权**。
        // 它不是一个 agent 工作，装配与其余形态没有共同点，所以在这里就地分岔、不往下走。
        if spec.mode == WorkMode::Proxy {
            if !spec.agents.is_empty() || spec.task.is_some() {
                return Err(
                    "代理形态不接受 agent 名单或本次需求：决定权是整块交给核心的".to_string(),
                );
            }
            if parent.is_some() {
                return Err("子工作不能建成代理形态：代理不能往里套代理".to_string());
            }
            let (sid, facts) = self.create_proxy_with_facts(&spec.name, now_ts(), spec.tier)?;
            return Ok(WorkOpened {
                sid,
                agents: Vec::new(),
                facts,
            });
        }
        // 代拟路径（协作、未给 agent）允许先空着，由核心按需求拟名单；其余形态必须有 agent。
        if spec.agents.is_empty() && !spec.delegate {
            return Err("至少要有一个 agent".to_string());
        }
        // **单模式的组合语义**：点名了多个 agent = 把它们的模块并成一个**临时组合**（去重、保序；
        // 模型取核心默认）。规则只有这一处——前端不再自己拼（见 ARCHITECTURE.md §一）。
        let mut spec = spec;
        if spec.mode == WorkMode::Single && spec.agents.len() > 1 {
            spec.agents = vec![merge_into_one(&spec.agents, &spec.name)];
        }
        let roster = self.scan();
        let reserved = self.reserved_names();
        // 校验 agent：名字合法、模块与模型真实存在；同一模块不得同时属于两个 agent（沙箱会歧义）
        let mut seen_modules: Vec<String> = Vec::new();
        for a in &spec.agents {
            crate::capabilities::registry::api::validate_name(&a.name)?;
            crate::capabilities::registry::api::check_reserved(&a.name, &reserved)?;
            for id in &a.modules {
                if !roster.modules.iter().any(|m| &m.manifest.id == id) {
                    return Err(format!("无此模块：{}", id));
                }
                if seen_modules.iter().any(|x| x == id) {
                    return Err(format!(
                        "模块 {} 被多个 agent 同时使用；同一模块只能属于一个 agent",
                        id
                    ));
                }
                seen_modules.push(id.clone());
            }
            if let Some(mid) = &a.model {
                if !self.registry.has_model(mid) {
                    return Err(format!("无此模型：{}", mid));
                }
                self.registry.resolve(mid)?;
            }
        }
        // 形态约束（模块可以为空，上面只校验真实存在与归属不冲突）
        match spec.mode {
            WorkMode::Single => {
                if spec.agents.len() != 1 {
                    return Err("单 agent 形态只接受一个 agent（模块数不限）".to_string());
                }
            }
            // 代理形态在函数开头就返回了（它没有 agent 名单）；这里只为穷尽，不产生行为。
            WorkMode::Proxy => {}
            WorkMode::Collab => {
                if spec.task.as_deref().unwrap_or("").trim().is_empty() {
                    return Err("协作模式必须填写本次需求".to_string());
                }
            }
        }
        // 工作根：顶层会话就是自己；子会话沿父链走到顶（整棵树只有一个 work/）。
        let root = match parent {
            Some(p) => self.work_root(p)?,
            None => spec.name.clone(),
        };
        // 实例名在**整棵子树**里唯一：共用同一个 work/ 时同名 agent 会撞同一个沙箱目录。
        // 重名 → 尾号（用户不改名时的兜底），绝不重名。顶层根还不存在时子树名单为空。
        let mut taken: Vec<String> = self.subtree_agent_names(&root);
        let mut metas: Vec<AgentMeta> = Vec::new();
        for a in &spec.agents {
            let name = crate::capabilities::registry::api::unique_instance_name(&a.name, &taken);
            taken.push(name.clone());
            metas.push(AgentMeta {
                name,
                transient: a.transient,
                modules: a.modules.clone(),
                model: a.model.clone(),
                permissions: Default::default(),
            });
        }
        // 模块扁平清单（展示用；顺序按 agent 名单展开）
        let module_ids: Vec<String> = metas.iter().flat_map(|m| m.modules.clone()).collect();
        // 代拟：给了 agent 就不再代拟（名单已经有了，不让核心盖掉用户的选择）。
        let delegate = spec.delegate && metas.is_empty();

        let name = spec.name.clone();
        let agent_names: Vec<String> = metas.iter().map(|m| m.name.clone()).collect();
        let meta = SessionMeta {
            name: name.clone(),
            mode: mode_str(spec.mode).to_string(),
            delegate,
            modules: module_ids.clone(),
            task: spec.task.clone(),
            ts: now_ts(),
            agents: metas.clone(),
            // 编排归属（代理建的子工作 = Some(父)）；节点子会话由 spawn_sub_session 另建。
            parent: parent.map(|s| s.to_string()),
            node: None,
            delegation: None,
            // 档位来自**用户在创建向导里的选择**（默认 = 设置里的档位）；承载不了由下面如实拒绝。
            exec: crate::capabilities::workspace::api::ExecSpec {
                tier: spec.tier,
                ..crate::capabilities::workspace::api::ExecSpec::default()
            },
            run: RunState::Active,
        };
        // 承载校验：默认档位的前置条件不具备时**不允许创建虚拟机档会话**（用户环境问题，不是选型问题）。
        // 必须在建工作区之前收口——拒绝就该什么都不留下。
        if let Some(why) = crate::capabilities::workspace::api::tier_refusal(
            &meta.exec,
            self.qemu_path(),
            self.probe.as_ref(),
        ) {
            return Err(why);
        }
        // 工作区：整棵树只有顶层一个 work/；各 agent 沙箱按实例名建在它下面。
        // （失败即失败，不假装已建；代拟确认名单时再补建。）
        self.workspace.prepare(&root, &agent_names)?;
        let sandboxes = self.sandboxes(&meta, &roster)?;
        // 扫描事实如实埋点：本会话用到的模块里，哪些声明的运行包不在包库（缺包不等于崩溃，工具按档位不可用）。
        let session_modules: Vec<Module> = roster
            .modules
            .iter()
            .filter(|m| module_ids.iter().any(|id| id == &m.manifest.id))
            .cloned()
            .collect();
        for (id, caps) in
            crate::capabilities::workspace::api::absent(&session_modules, &self.workspace.library())
        {
            self.log.warn(
                "conductor::create_work",
                &format!("模块 {} 声明的运行包不在包库：{}", id, caps.join("、")),
            );
        }
        // 执行选型的完整性检查（「开始」即冻结）：虚拟机档的选型不成立（多版本未定版 / 定版不存在 /
        // 路径冲突）如实拒绝；只是缺包的照常开始——那是该模块的工具不可用（降级而非崩溃）。装配阶段按同一份计划取包。
        let plan = crate::capabilities::workspace::api::plan(
            &meta.exec,
            &session_modules,
            &self.workspace.library(),
        )
        .map_err(|diags| crate::capabilities::workspace::api::diagnose_text(&diags))?;
        self.log.info(
            "conductor::create_work",
            &crate::capabilities::workspace::api::plan_summary(&plan),
        );

        let (session, mut events) = match spec.mode {
            // 代理形态在函数开头就建好返回了：它没有 agent 名单，不走这条装配路。
            WorkMode::Proxy => return Err("代理形态没有 agent 名单，不经这条装配路".to_string()),
            // 单 agent（模块数不限）。
            WorkMode::Single => {
                let a = metas.first().ok_or("至少要有一个 agent")?;
                let sb = sandboxes
                    .for_agent(&a.name)
                    .cloned()
                    .ok_or_else(|| format!("缺少 agent {} 的沙箱", a.name))?;
                let chosen: Vec<Module> = a
                    .modules
                    .iter()
                    .filter_map(|id| {
                        roster
                            .modules
                            .iter()
                            .find(|m| &m.manifest.id == id)
                            .cloned()
                    })
                    .collect();
                let channel = self.channel_of(a.model.as_deref());
                let unavailable = self.unavailable_modules(&meta.exec, &chosen);
                let (s, opened) =
                    self.build_single(a, &chosen, channel, &sb, unavailable, meta.exec.net);
                (Session::Single(s), opened)
            }
            WorkMode::Collab => {
                let task = spec.task.as_deref().unwrap_or("").trim().to_string();
                let mut cs = CollabSession::start(
                    Arc::clone(&self.llm),
                    Arc::clone(&self.workspace),
                    self.registry.snapshot(),
                    Arc::clone(&self.prompt),
                    Arc::clone(&self.systools),
                    Arc::clone(&self.tools),
                    Arc::clone(&self.log),
                    meta.exec.clone(),
                    metas.clone(),
                    delegate,
                    sandboxes.clone(),
                )?;
                let mut out = Vec::new();
                cs.set_task(&task, &mut |e| out.push(e));
                (Session::Collab(cs), out)
            }
        };
        // 会话身份落盘：失败即失败（名字即目录，这是承诺，不假装已保存）。
        self.history.create(&meta)?;
        self.sessions.insert(name.clone(), session);
        self.record_events(&name, &mut events);
        // 事实（开场转录 + 提示）随结果交出：核心不持有事件台，发布是 api 层的职责。
        Ok(WorkOpened {
            sid: name,
            agents: agent_names,
            facts: events,
        })
    }

    /// 界面投喂：把文件写进本次工作的 work/。
    /// 返回 Ok(false) = 同名文件已存在且未选择覆盖（交前端让用户决定：覆盖/改名/取消）。
    pub fn work_upload(
        &mut self,
        sid: &str,
        name: &str,
        bytes: &[u8],
        overwrite: bool,
    ) -> Result<bool, String> {
        // 策略在 conductor：先净化文件名，再交给工作区端口（机制只看已净化的名字）。
        let name = crate::capabilities::workspace::api::safe_file_name(name)?;
        if !self.sessions.contains_key(sid) && self.history.load(sid).is_err() {
            return Err(format!("无此会话：{}", sid));
        }
        if self.workspace.work_has(sid, &name) && !overwrite {
            return Ok(false);
        }
        // 用户投喂 = 一次**权威提交**（作者 user）：共享区只有提交这一条写路径。
        // 锚到主会话当时的下一条行号，回档才能把这次投喂算进某个转录点。
        let work = self.work_root(sid).unwrap_or_else(|_| sid.to_string());
        let line = match self.sessions.get(sid) {
            Some(Session::Single(s)) => s.next_line,
            Some(Session::Collab(c)) => c.next_line,
            None => 0,
        };
        self.workspace
            .work_commit_user(&work, &name, bytes, now_ts(), line)?;
        Ok(true)
    }

    /// 本工作可引用的文件清单 + 真实根（前端 @ 菜单与「长路径缩写」用）。
    /// 名单取自会话 meta（活动会话与历史会话都以 meta.agents 为权威）；列目录的机制在 Workspace 端口，
    /// 根取自沙箱（与文件清单同源）：agents 与 roots.agents 同序同名。
    pub fn files_view(&self, sid: &str) -> Result<FilesView, String> {
        let (meta, _) = self
            .history
            .load(sid)
            .map_err(|_| format!("无此会话：{}", sid))?;
        let names: Vec<String> = meta.agents.iter().map(|a| a.name.clone()).collect();
        let files = self.workspace.files(sid, &names)?;
        let usage = self.workspace.usage(sid, &names)?;
        let roster = self.scan();
        let sandboxes = self.sandboxes(&meta, &roster)?;
        let mut agents: Vec<FilesAgentView> = Vec::new();
        let mut agent_roots: Vec<FilesAgentRootView> = Vec::new();
        for a in &meta.agents {
            agents.push(FilesAgentView {
                name: a.name.clone(),
                files: files.agents.get(&a.name).cloned().unwrap_or_default(),
            });
            agent_roots.push(FilesAgentRootView {
                name: a.name.clone(),
                root: sandboxes
                    .for_agent(&a.name)
                    .map(|s| crate::kernel::api::slash(&s.private))
                    .unwrap_or_default(),
            });
        }
        Ok(FilesView {
            work: files.work,
            agents,
            roots: FilesRootsView {
                work: crate::kernel::api::slash(&sandboxes.shared),
                agents: agent_roots,
            },
            usage,
        })
    }
}
