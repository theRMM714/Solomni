//! 目的：建工作的两段与 guest 启动——占名、备工作区、装通道、起 guest、失败撤销。
//! 管：工作名唯一化、准备段、提交段、guest 启动装配、撤销。
//! 不管：配置改配（`config.rs`）、报告（`report.rs`）、guest 进程本身（vm 能力）。
//! 联动：句柄层按两段调用；vm / workspace / packages 的接口在这里被组合。
//!   本方法族只碰 history / registry / workspace / vm 这四个字段。

use super::*;

/// 目的：这次创建要不要在宿主上付一次性的机器级只读授权。
/// 约束：只有本机档写宿主 ACL；虚拟机档的隔离是挂载边界（`machine_ro_tree` 以只读挂载兑现），不写宿主 ACL。
fn pays_host_grant(meta: &SessionMeta) -> bool {
    let once = meta
        .workspace
        .as_ref()
        .is_some_and(|b| b.grant == crate::capabilities::workspace::api::WorkspaceGrant::Workspace);
    once && meta.exec.tier == crate::kernel::api::Tier::Host
}

impl Conductor {
    /// 目的：工作名的缺省与唯一化（命名策略在 `session`；这里只提供"存在吗"）。
    pub fn unique_work_name(&self, base: &str, fallback: &str) -> String {
        crate::capabilities::session::api::unique_work_name(base, fallback, |n| {
            self.sessions.contains_key(n) || self.history.load(n).is_ok()
        })
    }
}

impl Conductor {
    /// 目的：测试与内部编排用的同步入口（准备 → 起 guest → 提交，全在当前线程）。
    /// 约束：生产走句柄的两段（`prepare_work` / `commit_work`），核心线程不陪着等 QEMU。
    #[cfg(test)]
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

    /// 目的：建工作的同步实现（测试与内部编排用）：准备 → 起 guest → 提交。
    /// 约束：生产走句柄的两段；`parent` = 编排归属（代理建的**子工作**，与父会话共用顶层那个 work/）。
    #[cfg(test)]
    pub(crate) fn create_work_inner(
        &mut self,
        spec: WorkSpec,
        parent: Option<&str>,
    ) -> Result<WorkOpened, String> {
        let (prepared, boot) = self.prepare_work(spec, parent)?;
        if let Some(boot) = boot
            && let Err(e) = boot.run()
        {
            self.abort_work(prepared);
            return Err(e);
        }
        self.commit_work(prepared)
    }

    /// 目的：把名单里每个 agent 的通道解析成**确定值**（核心默认或显式模型），失败带具体原因。
    /// 约束：建工作/起 guest 之前调用——没有可用模型就不该留下任何会话或工作区；
    ///   协作还要求核心自己的通道（编排 / 整理 / 验收都走它）。
    fn resolve_channels(
        &self,
        metas: &[AgentMeta],
        mode: WorkMode,
    ) -> Result<(BTreeMap<String, Channel>, Option<Channel>), String> {
        let mut channels = BTreeMap::new();
        for a in metas {
            let c = self
                .registry
                .channel(a.model.as_deref())
                .map_err(|e| format!("agent {} 没有可用模型通道：{}", a.name, e))?;
            channels.insert(a.name.clone(), c);
        }
        let core = if mode == WorkMode::Collab {
            Some(
                self.registry
                    .core_channel()
                    .map_err(|e| format!("核心没有可用模型通道：{}", e))?,
            )
        } else {
            None
        };
        Ok((channels, core))
    }

    /// 目的：解析本次创建的工作区绑定：只有顶层会话能指定，路径按档位策略校验（见 workspace_root）；
    ///   子会话沿父链继承同一个根，不接受自选。
    fn resolve_workspace_binding(
        &self,
        raw: Option<&std::path::Path>,
        grant: Option<crate::capabilities::session::api::WorkspaceGrant>,
        tier: crate::kernel::api::Tier,
        parent: Option<&str>,
    ) -> Result<Option<crate::capabilities::session::api::WorkspaceBinding>, String> {
        if let Some(p) = parent {
            if raw.is_some() || grant.is_some() {
                return Err("子会话不能自选工作区或授权粒度：沿父会话继承".to_string());
            }
            let top = self.work_root(p)?;
            return Ok(self.history.meta(&top)?.workspace);
        }
        let Some(raw) = raw else {
            if grant
                .is_some_and(|g| g != crate::capabilities::session::api::WorkspaceGrant::Session)
            {
                return Err("没有指定外部工作区时，工作区授权粒度无意义".to_string());
            }
            return Ok(None);
        };
        let probe = self.probe.as_ref();
        let product = super::workspace_root::product_root(&self.root, probe);
        let root = super::workspace_root::resolve(raw, tier, &product, probe)?;
        Ok(Some(crate::capabilities::session::api::WorkspaceBinding {
            root,
            grant: grant.unwrap_or_default(),
        }))
    }

    /// 目的：创建的第一段（核心，毫秒级）——校验 / 占名 / 建工作区 / 出计划；要起 guest 时把那一段交出去。
    /// 返回：准备结果 + 可选的那段"要等 guest"的活（虚拟机档才有）。
    /// 错误：形态、名字、档位、名单、模型通道或选型不成立时如实拒绝，什么都不留下。
    pub(crate) fn prepare_work(
        &mut self,
        spec: WorkSpec,
        parent: Option<&str>,
    ) -> Result<(PreparedWork, Option<GuestBoot>), String> {
        validate_work_name(&spec.name)?;
        if self.sessions.contains_key(&spec.name)
            || self.history.load(&spec.name).is_ok()
            || self.starting.contains(&spec.name)
        {
            return Err(format!("工作名已存在：{}", spec.name));
        }
        let workspace = self.resolve_workspace_binding(
            spec.workspace.as_deref(),
            spec.workspace_grant,
            spec.tier,
            parent,
        )?;
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
            // 代理会话跑在核心默认模型上：没有它就不建（工作区都还没有，失败什么都不留）。
            self.registry
                .core_channel()
                .map_err(|e| format!("代理会话没有可用模型通道：{}", e))?;
            let (sid, facts) = self.create_proxy_with_facts(
                &spec.name,
                now_ts(),
                spec.tier,
                spec.guest.as_deref(),
                spec.permission
                    .unwrap_or(crate::capabilities::permission::api::ProfileId::AllReach),
                workspace,
            )?;
            return Ok((
                PreparedWork::Ready(WorkOpened {
                    sid,
                    agents: Vec::new(),
                    facts,
                }),
                None,
            ));
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
        // 校验 agent：名字合法、模块与模型真实存在；模块可跨 agent 共用（各自独立沙箱与工具面）
        for a in &spec.agents {
            crate::capabilities::registry::api::validate_name(&a.name)?;
            crate::capabilities::registry::api::check_reserved(&a.name, &reserved)?;
            for id in &a.modules {
                if !roster.modules.iter().any(|m| &m.manifest.id == id) {
                    return Err(format!("无此模块：{}", id));
                }
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
                modules: dedup_modules(&a.modules),
                model: a.model.clone(),
                chara: a.chara.trim().to_string(),
            });
        }
        // **模型通道前置判据**：合并后的名单里每个 agent 都要有确定通道；协作还要求核心通道。
        // 放在建工作区 / 起 guest 之前——失败就不留下任何半成品。
        let (channels, core_channel) = self.resolve_channels(&metas, spec.mode)?;
        // 模块扁平清单（展示用；顺序按 agent 名单展开，同一模块被多个 agent 共用时只留一份）
        let module_ids = flat_modules(&metas);
        // 代拟：给了 agent 就不再代拟（名单已经有了，不让核心盖掉用户的选择）。
        let delegate = spec.delegate && metas.is_empty();

        let name = spec.name.clone();
        let agent_names: Vec<String> = metas.iter().map(|m| m.name.clone()).collect();
        let meta = SessionMeta {
            context_freshness: false,
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
            // 环境属于主会话：子会话整段沿用父会话（档位 / 虚拟根 / 网络 / 定版）；
            // 顶层来自**用户在创建向导里的选择**（默认 = 设置里的档位）。承载不了由下面如实拒绝。
            exec: match parent {
                Some(p) => self.history.meta(p)?.exec,
                None => crate::kernel::api::ExecSpec {
                    tier: spec.tier,
                    guest: spec.guest.clone(),
                    ..crate::kernel::api::ExecSpec::default()
                },
            },
            // 绑定只写在工作根上；子会话沿 parent 链继承，不复制一份。
            workspace: if parent.is_none() {
                workspace.clone()
            } else {
                None
            },
            sandbox: match parent {
                // 子会话只带自己的 narrowing；根 ceiling 沿祖先链现算，不再复制父权限。
                Some(_) => Default::default(),
                None => crate::capabilities::permission::api::SessionSandbox {
                    ceiling: spec.permission.map(|profile| {
                        crate::capabilities::permission::api::PermissionLayer {
                            profile: Some(profile),
                            ..Default::default()
                        }
                    }),
                    narrowing: None,
                },
            },
            thinking: None,
            run: RunState::Active,
        };
        // 承载校验：默认档位的前置条件不具备时**不允许创建虚拟机档会话**（用户环境问题，不是选型问题）。
        // 必须在建工作区之前收口——拒绝就该什么都不留下。
        if let Some(why) = crate::capabilities::vm::api::tier_refusal(
            &self.root,
            &meta.exec,
            self.qemu_path().as_deref(),
            self.share_tool_path().as_deref(),
            self.probe.as_ref(),
        ) {
            return Err(why);
        }
        // 工作区：整棵树只有顶层一个 work/（或外部绑定的根）；各 agent 沙箱按实例名建在会话目录下。
        // 先把绑定刷成布局投影，再建沙箱；失败即失败，不假装已建。
        self.workspace
            .bind(&root, workspace.as_ref().map(|w| w.root.as_path()))?;
        self.workspace.prepare(&root, &agent_names)?;
        if workspace.is_some() {
            // 首次绑定：把工作区现有文件导入成初始提交，agent 立刻就能 pull；失败不挡建会话。
            if let Err(e) = self.workspace.work_import_root(&root, now_ts()) {
                self.log.warn(
                    "conductor::create_work",
                    &format!("工作区初始导入失败：{}", e),
                );
            }
        }
        let sandboxes = self.sandboxes(&meta, &roster)?;
        // 扫描事实如实埋点：本会话用到的模块里，哪些声明的运行包不在包库（缺包同样会在下面按计划拒绝，文件与 guest 声明都看）。
        // 虚拟机档缺包在「开始」前就拒绝；这里的警告只补一条扫描事实。本机档由宿主计划在工具执行时如实拒绝。
        let session_modules: Vec<Module> = roster
            .modules
            .iter()
            .filter(|m| module_ids.iter().any(|id| id == &m.manifest.id))
            .cloned()
            .collect();
        if meta.exec.tier == crate::kernel::api::Tier::Vm {
            let guest_runtimes = self.guest_runtimes_of(&meta.exec);
            for (id, caps) in pkgs::absent(&session_modules, &self.packages.library()) {
                let caps: Vec<String> = caps
                    .into_iter()
                    .filter(|c| !guest_runtimes.iter().any(|g| g == c))
                    .collect();
                if caps.is_empty() {
                    continue;
                }
                self.log.warn(
                    "conductor::create_work",
                    &format!("模块 {} 声明的运行包不在包库：{}", id, caps.join("、")),
                );
            }
        }
        // 执行选型完整性（「开始」即冻结）：选型不成立或**缺包**都如实拒绝（没有工具，用户完不成工作）。
        // 入口**按目标环境**判：虚拟机档按 guest 的 os 解析名字，跨平台还看文件形式（宿主编的产物跑不了）。
        let guest = self.guest_os_of(&meta.exec).map(|g| g.as_str());
        let target_os = guest.unwrap_or(std::env::consts::OS);
        let missing: Vec<crate::capabilities::vm::api::MissingEntry> =
            crate::capabilities::vm::api::missing_entries_for(&session_modules, target_os, &|p| {
                self.probe.is_executable(p)
                    && (target_os == std::env::consts::OS
                        || crate::capabilities::vm::api::runs_on_target(p, target_os))
            });
        let plan = self.plan_for_spec(
            &meta.exec,
            &session_modules,
            &missing,
            meta.exec.guest.as_deref(),
        )?;
        self.log.info(
            "conductor::create_work",
            &crate::capabilities::vm::api::plan_summary(&plan),
        );

        // 占名：准备与提交之间隔着一次 guest 启动，同名并发创建不能都通过上面那道检查。
        self.starting.insert(name.clone());
        // 子会话与父会话共用同一个 session/ 目录：只对顶层会话记得"失败要撤掉空目录"。
        let discard = if parent.is_none() {
            Some((name.clone(), agent_names.clone()))
        } else {
            None
        };
        // 代拟名单还没确认：沙箱/模块都不知道，先不起 guest；第一次真正生成前 ensure_guest 再起。
        let boot = if meta.exec.tier == crate::kernel::api::Tier::Vm
            && !(meta.delegate && meta.agents.is_empty())
        {
            // 共享 guest：按整棵树装配（proxy 子工作都进代理根那一台），租约取作用域根。
            let (lease, scope_meta) = self
                .guest_scope(&meta)?
                .ok_or_else(|| "虚拟机档会话没有解析出 guest 作用域".to_string())?;
            let tree = self.tree_meta(&scope_meta, Some(&meta))?;
            let tree_plan = self.plan_for_meta_with(&tree, &roster)?;
            let tree_sandboxes = self.sandboxes(&tree, &roster)?;
            // 运行包不做动态加载：新子会话的包已被这台 guest 装载就直接加入；
            // 引入新包时要等树空闲后按聚合后的 meta 重建这台共享 guest。
            let want: Vec<String> = tree_plan
                .packages
                .iter()
                .map(|p| format!("{}@{}", p.id, p.version))
                .collect();
            let active = self.vm.guest_leases().iter().any(|l| l == &lease);
            let have = self.vm.guest_packages(&lease);
            let restart = needs_guest_packages_rebuild(active, &want, have.as_deref());
            if restart {
                let running = self
                    .subtree_of(&scope_meta.name)
                    .into_iter()
                    .find(|s| self.running.contains(s));
                if let Some(s) = running {
                    return Err(format!(
                        "子会话引入了这台 guest 还没装载的运行包：先停掉正在生成的 {}，再重建共享 guest",
                        s
                    ));
                }
            }
            Some(self.guest_boot(&tree, &tree_plan, &tree_sandboxes, restart, &lease))
        } else {
            None
        };
        Ok((
            PreparedWork::Pending(Box::new(PendingWork {
                meta: Box::new(meta),
                mode: spec.mode,
                metas,
                task: spec.task,
                delegate,
                name,
                agent_names,
                sandboxes,
                channels,
                core_channel,
                discard,
            })),
            boot,
        ))
    }

    /// 目的：创建的第二段（核心，毫秒级）——装配会话对象、落盘身份、开场事实。
    /// 约束：guest 已经起好了（或本机档不需要）；落盘失败要撤掉占名与刚建的空工作区目录。
    pub(crate) fn commit_work(&mut self, prepared: PreparedWork) -> Result<WorkOpened, String> {
        let pending = match prepared {
            // 代理形态在准备段里就建完了：这里原样交回。
            PreparedWork::Ready(opened) => return Ok(opened),
            PreparedWork::Pending(p) => p,
        };
        let PendingWork {
            meta,
            mode,
            metas,
            task,
            delegate,
            name,
            agent_names,
            sandboxes,
            mut channels,
            core_channel,
            discard,
        } = *pending;
        let roster = self.scan();

        let (session, mut events) = match mode {
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
                let channel = channels
                    .remove(&a.name)
                    .ok_or_else(|| format!("缺少 agent {} 的模型通道", a.name))?;
                let unavailable = self.unavailable_modules(&meta.exec, &chosen);
                let guest = self.guest_os_of(&meta.exec);
                let host_plan = if meta.exec.tier == crate::kernel::api::Tier::Host {
                    self.host_plan_for(&meta.exec, &chosen, pkgs::PinPolicy::Require)
                } else {
                    pkgs::PackagePlan::default()
                };
                let (mut s, opened) = self.build_single(
                    a,
                    &chosen,
                    channel,
                    &sb,
                    unavailable,
                    self.network_allowed(),
                    meta.exec.tier,
                    guest,
                    self.guest_lease_for(&meta)?,
                    &host_plan,
                );
                // 过期清理是**落盘事实**：装配时按 meta 装回开关，默认关。
                s.set_context_freshness(meta.context_freshness);
                (Session::Single(s), opened)
            }
            WorkMode::Collab => {
                let task = task.as_deref().unwrap_or("").trim().to_string();
                // 裁决队按会话 id 取：核心各关卡与这一席的工具级确认共用同一条队。
                let door = self.desk_of(&name);
                let core_channel =
                    core_channel.ok_or_else(|| "协作会话缺少核心模型通道".to_string())?;
                let mut cs = CollabSession::start(
                    Arc::clone(&self.llm),
                    Arc::clone(&self.vm),
                    Arc::clone(&self.modules),
                    self.registry.snapshot(),
                    core_channel,
                    Arc::clone(&self.prompt),
                    Arc::clone(&self.systools),
                    Arc::clone(&self.tools),
                    Arc::clone(&self.log),
                    meta.exec.clone(),
                    self.guest_lease_for(&meta)?,
                    metas.clone(),
                    delegate,
                    sandboxes.clone(),
                    Arc::clone(&self.workspace),
                    door,
                )?;
                let mut out = Vec::new();
                cs.set_task(&task, &mut |e| out.push(e));
                (Session::Collab(cs), out)
            }
        };
        // 会话身份落盘：失败即失败（名字即目录，这是承诺，不假装已保存）。
        self.starting.remove(&name);
        if let Err(e) = self.history.create(&meta) {
            if let Some((session, agents)) = discard {
                let _ = self.workspace.discard(&session, &agents);
            }
            return Err(e);
        }
        self.sessions.insert(name.clone(), session);
        self.record_events(&name, &mut events);
        // 只有本机档 + workspace 粒度才付机器级只读授权（一次性成本）：会话已经落盘，甩到后台慢慢付，创建本身保持毫秒级。
        if pays_host_grant(&meta)
            && let Some(binding) = meta.workspace.as_ref()
        {
            crate::capabilities::workspace::api::ensure_in_background(
                &self.root.join(".home"),
                &binding.root,
            );
        }
        // 事实（开场转录 + 提示）随结果交出：核心不持有事件台，发布是 api 层的职责。
        Ok(WorkOpened {
            sid: name,
            agents: agent_names,
            facts: events,
        })
    }

    /// 目的：创建失败（guest 起不来 / 落盘失败）时撤销准备段占下的东西：名字占位 + 这次建出来的空工作区目录。
    pub(crate) fn abort_work(&mut self, prepared: PreparedWork) {
        if let PreparedWork::Pending(p) = prepared {
            self.starting.remove(&p.name);
            if let Some((session, agents)) = p.discard {
                let _ = self.workspace.discard(&session, &agents);
            }
        }
    }

    /// 目的：把"停 guest"这一段打包成交给调用方线程的活（销毁要等它落盘，秒级）。
    pub(crate) fn guest_stop(&self, lease: &str) -> super::guest_job::GuestStop {
        super::guest_job::GuestStop::new(Arc::clone(&self.vm), vec![lease.to_string()])
    }

    /// 目的：按一条会话的现状准备"确保 guest"那一段（启用、发言、成员回合共用）。
    /// 返回：None = 本机档 / proxy 父会话 / 已有活 guest / 代拟名单还没确认；有 = 要在调用方线程跑。
    pub(crate) fn prepare_guest_boot(
        &self,
        sid: &str,
    ) -> Result<Option<super::guest_job::GuestBoot>, String> {
        let meta = self.history.meta(sid)?;
        if meta.run == RunState::Closed {
            return Err(format!("会话 {} 已关闭：终态，不能再启用", sid));
        }
        let Some((lease, scope_meta)) = self.guest_scope(&meta)? else {
            return Ok(None);
        };
        if self.vm.guest_leases().iter().any(|l| l == &lease) {
            return Ok(None);
        }
        // 代拟名单还没确认：沙箱/模块都还不知道，等第一次真正生成前再起（见 guest_scope / spawn）。
        if scope_meta.agents.is_empty() && scope_meta.delegate {
            return Ok(None);
        }
        let roster = self.scan();
        // 共享 guest 的装配按整棵树算：proxy 根自己没有名单，子工作的 agent / 模块 / 包都要进来。
        let tree = self.tree_meta(&scope_meta, None)?;
        let plan = self.plan_for_meta_with(&tree, &roster)?;
        let sandboxes = self.sandboxes(&tree, &roster)?;
        Ok(Some(
            self.guest_boot(&tree, &plan, &sandboxes, false, &lease),
        ))
    }

    /// 目的：把"起 guest"这一段打包成交给调用方线程的活——核心只准备参数，不在这里拉起。
    pub(crate) fn guest_boot(
        &self,
        _meta: &SessionMeta,
        plan: &crate::capabilities::vm::api::ExecPlan,
        sandboxes: &crate::capabilities::workspace::api::Sandboxes,
        restart: bool,
        lease: &str,
    ) -> GuestBoot {
        let share_tool = self.share_tool_path();
        // 开发/应急开关打开时，本进程的 VM 会话强制走 copy（不追加成回退链）；
        // 普通进程没有这个开关，share_chain 只给用户档。
        let shares = if crate::capabilities::vm::api::copy_share_enabled() {
            vec!["copy"]
        } else {
            crate::capabilities::vm::api::share_chain(
                std::env::consts::OS,
                share_tool.as_deref(),
                self.probe.as_ref(),
            )
        };
        GuestBoot::new(
            Arc::clone(&self.vm),
            self.root.clone(),
            Arc::clone(&self.probe),
            Arc::clone(&self.log),
            self.registry.app().max_guests,
            self.host_read_roots(),
            self.qemu_path(),
            share_tool,
            shares,
            lease.to_string(),
            self.network_allowed(),
            plan.clone(),
            sandboxes.clone(),
            restart,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(
        tier: crate::kernel::api::Tier,
        grant: crate::capabilities::workspace::api::WorkspaceGrant,
    ) -> SessionMeta {
        SessionMeta {
            name: "w".to_string(),
            mode: "single".to_string(),
            delegate: false,
            modules: Vec::new(),
            task: None,
            ts: 0,
            agents: Vec::new(),
            exec: crate::kernel::api::ExecSpec {
                tier,
                ..Default::default()
            },
            workspace: Some(crate::capabilities::session::api::WorkspaceBinding {
                root: std::path::PathBuf::from("C:\\ws"),
                grant,
            }),
            sandbox: Default::default(),
            thinking: None,
            parent: None,
            node: None,
            delegation: None,
            context_freshness: false,
            run: Default::default(),
        }
    }

    /// 目的：workspace 粒度只有本机档付宿主的机器级授权；虚拟机档与其它粒度都不付。
    #[test]
    fn host_grant_is_paid_only_by_the_host_tier_with_workspace_grant() {
        use crate::capabilities::workspace::api::WorkspaceGrant;
        use crate::kernel::api::Tier;
        assert!(pays_host_grant(&meta(
            Tier::Host,
            WorkspaceGrant::Workspace
        )));
        assert!(!pays_host_grant(&meta(Tier::Vm, WorkspaceGrant::Workspace)));
        assert!(!pays_host_grant(&meta(Tier::Host, WorkspaceGrant::Session)));
        assert!(!pays_host_grant(&meta(Tier::Host, WorkspaceGrant::None)));
    }
}
