//! 目的：入站契约的实现：把每个窄契约的每个方法落到一次 call 或一次转发上。
//! 管：WorkOps / TurnOps / RuntimeOps / RegistryOps / HistoryOps / ModulesOps / LogOps 七个实现。
//! 不管：方法体里的业务判定（在下层能力）；长步骤编排（activation / rewind / action）。
//! 联动：呈现层经 entry/faces.rs 拿到这里实现的窄契约；每个方法都进核心线程执行。

use super::*;

impl WorkOps for Handle {
    fn create_work(&self, spec: WorkSpec) -> Result<(WorkOpened, u64), String> {
        let bus = Arc::clone(&self.bus);
        // 先补编：名单里声明了 `build:` 的模块若在目标环境还没有入口，就在这一段（调用方线程）编好。
        {
            let mut ids: Vec<String> = Vec::new();
            for a in &spec.agents {
                for m in &a.modules {
                    if !ids.contains(m) {
                        ids.push(m.clone());
                    }
                }
            }
            self.ensure_built(&ids, spec.tier, spec.guest.as_deref())?;
        }
        // 两段：核心准备（毫秒级）→ 起 guest（这一段）→ 提交。核心线程不陪着等 QEMU。
        let (prepared, boot) = self.call(move |core| core.prepare_work(spec, None))?;
        if let Some(boot) = boot
            && let Err(e) = boot.run()
        {
            let _ = self.call(move |core| {
                core.abort_work(prepared);
                Ok(())
            });
            return Err(e);
        }
        let opened = self.call(move |core| core.commit_work(prepared))?;
        let head = bus.push(&opened.sid, &opened.facts);
        Ok((opened, head))
    }

    fn config(&self, sid: &str) -> Result<SessionConfig, String> {
        let sid = sid.to_string();
        self.call(move |core| core.session_config(&sid))
    }

    fn edit(&self, sid: &str, edit: SessionEdit) -> Result<(), String> {
        // 两段：核心校验 + 出计划；停旧 guest / 起新的要等（秒级到分钟级），在这一段上跑。
        let (prepared, job) = self.call({
            let sid = sid.to_string();
            let edit = edit.clone();
            move |core| core.prepare_edit(&sid, edit)
        })?;
        if let Some(job) = job {
            // 失败什么都不改：meta 还没落盘。
            job.run()?;
        }
        self.call(move |core| core.commit_edit(prepared))
    }

    fn upload(&self, sid: &str, name: &str, bytes: &[u8], overwrite: bool) -> Result<bool, String> {
        let sid = sid.to_string();
        let name = name.to_string();
        let bytes = bytes.to_vec();
        self.call(move |core| core.work_upload(&sid, &name, &bytes, overwrite))
    }

    fn files(&self, sid: &str) -> Result<FilesView, String> {
        let sid = sid.to_string();
        self.call(move |core| core.files_view(&sid))
    }

    fn work_log(
        &self,
        sid: &str,
        limit: usize,
        before: Option<u64>,
    ) -> Result<crate::capabilities::workspace::api::WorkLog, String> {
        let sid = sid.to_string();
        self.call(move |core| core.work_log(&sid, limit, before))
    }

    fn work_commit_view(
        &self,
        sid: &str,
        commit: u64,
    ) -> Result<crate::capabilities::workspace::api::CommitDetail, String> {
        let sid = sid.to_string();
        self.call(move |core| core.work_commit_view(&sid, commit))
    }

    fn work_commit_file(
        &self,
        sid: &str,
        commit: u64,
        path: &str,
    ) -> Result<crate::capabilities::workspace::api::CommitFileView, String> {
        let sid = sid.to_string();
        let path = path.to_string();
        self.call(move |core| core.work_commit_file(&sid, commit, &path))
    }

    fn exists(&self, sid: &str) -> Result<bool, String> {
        let sid = sid.to_string();
        self.call(move |core| Ok(core.session_exists(&sid)))
    }

    fn unique_work_name(&self, base: &str, fallback: &str) -> Result<String, String> {
        let (base, fallback) = (base.to_string(), fallback.to_string());
        self.call(move |core| Ok(core.unique_work_name(&base, &fallback)))
    }

    fn session_views(&self, history: &[HistoryView]) -> Result<Vec<SessionView>, String> {
        let history = history.to_vec();
        self.call(move |core| Ok(core.session_views(&history)))
    }

    fn runtime_report(&self, tier: Tier) -> Result<RuntimeReport, String> {
        self.call(move |core| Ok(core.runtime_report(tier)))
    }

    fn tier_choices(&self) -> Result<TierChoices, String> {
        self.call(|core| Ok(core.tier_choices()))
    }

    fn suggest_models(&self, task: &str, mode: WorkMode) -> Result<Vec<AgentSuggestion>, String> {
        let task = task.to_string();
        let (agents, rows) = self.call(move |core| core.suggest_models(&task, mode))?;
        // 核心的行**照推**（推是底层收发消息的统一定律，一次推荐也不例外），推到系统会话：
        // 它不在任何会话表里，前端因此不会为它建标签页；落盘策略是 Drop（只推不留）。
        if !rows.is_empty() {
            self.bus.push(
                crate::capabilities::conversation::session::api::SYSTEM_SID_SUGGEST,
                &rows,
            );
        }
        Ok(agents)
    }
}

impl TurnOps for Handle {
    fn say(&self, sid: &str, text: &str, out: Output) -> Result<Advance, String> {
        crate::capabilities::turn::turn::api::single_generation(
            self,
            sid,
            Some(text.to_string()),
            out,
        )
    }

    fn continue_flow(&self, sid: &str, out: Output) -> Result<Advance, String> {
        // 「继续」= 「停止」的逆操作：先把整棵子树解冻，再按形态接着走。
        // 单 agent / 代理补一条「继续」用户行作为这一轮的起因；协作不需要用户发言，从断点推进。
        let resumed = self.call({
            let s = sid.to_string();
            move |core| core.turn.resume_subtree(&s)
        })?;
        if resumed {
            let mode = self
                .call({
                    let s = sid.to_string();
                    move |core| Ok(core.session_mode(&s))
                })
                .unwrap_or_default();
            if mode == Some(crate::capabilities::conversation::session::api::Mode::Collab) {
                return crate::capabilities::turn::turn::api::collab_generation(
                    self,
                    sid,
                    CollabWork::Resume,
                );
            }
            return crate::capabilities::turn::turn::api::single_generation(
                self,
                sid,
                Some("继续。".to_string()),
                out,
            );
        }
        crate::capabilities::turn::turn::api::single_generation(self, sid, None, out)
    }

    fn set_task(&self, sid: &str, text: &str) -> Result<Advance, String> {
        // 短步骤（不调模型之外的东西；但代拟路径要接着拟名单与落盘）——留在核心线程上。
        let (sid, text) = (sid.to_string(), text.to_string());
        let bus = Arc::clone(&self.bus);
        self.call(move |core| {
            let events = core.turn.collab_set_task(
                &sid,
                &text,
                core.registry.as_ref(),
                core.probe.as_ref(),
            )?;
            let head = bus.push(&sid, &events);
            Ok(Advance { head })
        })
    }

    /// **回答一张裁决卡**（唯一的回答口）：与「停止」同一条直路——回答**先落定**
    /// （校验 + 出队 + 记一条回答，都在核心线程上跑完），处置再脱离调用方跑。
    /// 为什么必须先落定：界面据此立刻看到队首换人；等待方（工具级确认那种）当场就被唤醒。
    fn answer_card(
        &self,
        sid: &str,
        card: &str,
        option: &str,
        note: &str,
    ) -> Result<Advance, String> {
        let (answered, events) = self.call({
            let (sid, card, option, note) = (
                sid.to_string(),
                card.to_string(),
                option.to_string(),
                note.to_string(),
            );
            move |core| core.turn.take_card(&sid, &card, &option, &note)
        })?;
        let mut head = self.bus.push(sid, &events);
        match answered {
            // 等在工作线程上的那一关（工具级确认）：回答已经写进等待格，没有要推进的事。
            Answered::Waiting(_) => Ok(Advance { head }),
            Answered::Gate(ticket, _) => {
                // 代拟名单这一关要写回 meta 并建沙箱：短步骤，留在核心线程上收尾。
                if matches!(
                    ticket.pending,
                    crate::capabilities::conversation::session::api::Pending::ConfirmSlate
                ) {
                    let more = self.call({
                        let sid = sid.to_string();
                        move |core| {
                            core.turn.dispose_and_advance(
                                &sid,
                                &ticket,
                                core.registry.as_ref(),
                                core.probe.as_ref(),
                            )
                        }
                    })?;
                    head = self.bus.push(sid, &more);
                    return Ok(Advance { head });
                }
                // 放行 / 重派 / 请教 / 判明确性：长流程——脱离调用方点火（回答本身已经落定）。
                crate::capabilities::turn::turn::api::spawn_detached_dispose(self, sid, ticket);
                Ok(Advance { head })
            }
        }
    }

    /// 当前挂着的那一队裁决（队首卡 + 后面还在等的那几张）：CLI 与 Web 照同一份渲染。
    /// 约束：判据只有这一处——核心各关卡与工具级确认共用同一条队。
    fn open_queue(&self, sid: &str) -> Result<Option<DecisionQueue>, String> {
        let sid = sid.to_string();
        self.call(move |core| {
            core.turn
                .open_queue(&sid, core.registry.as_ref(), core.probe.as_ref())
        })
    }

    fn withdraw_agree(&self, sid: &str, agent: &str) -> Result<Advance, String> {
        let sid = sid.to_string();
        let agent = agent.to_string();
        let bus = Arc::clone(&self.bus);
        self.call(move |core| {
            let events = core.turn.withdraw_agree(
                &sid,
                &agent,
                core.registry.as_ref(),
                core.probe.as_ref(),
            )?;
            let head = bus.push(&sid, &events);
            Ok(Advance { head })
        })
    }

    fn compact(&self, sid: &str) -> Result<Advance, String> {
        crate::capabilities::turn::turn::api::compact(self, sid)
    }

    fn is_running(&self, sid: &str) -> bool {
        self.jobs.is_running(sid)
    }
}

impl RuntimeOps for Handle {
    fn activate(&self, sid: &str) -> Result<SessionView, String> {
        self.activate_session(sid)
    }

    fn update_task(&self, sid: &str, text: &str) -> Result<Vec<serde_json::Value>, String> {
        let sid = sid.to_string();
        let text = text.to_string();
        // 改需求改写盘上转录并推重置快照上事件台，刷新/多端才看到同一条需求；
        // 重建归这一层（生命周期能力是宿主的）：turn 只交回「哪条会话 + 事件切片 + 下一个行号」。
        let outcome = self.call({
            let sid = sid.clone();
            move |core| {
                let req = core.turn.update_task_outcome(
                    &sid,
                    &text,
                    core.registry.as_ref(),
                    core.probe.as_ref(),
                )?;
                let active = core.lifecycle.restore_session(
                    &mut core.turn,
                    RebuildDeps {
                        registry: core.registry.as_ref(),
                        probe: core.probe.as_ref(),
                    },
                    &req.sid,
                    &req.meta,
                    &req.events,
                    req.next,
                )?;
                Ok(RewindOutcome {
                    main: active,
                    children: Vec::new(),
                })
            }
        })?;
        self.publish_rewound(&sid, &outcome);
        Ok(outcome.main)
    }

    fn rewind(&self, sid: &str, target: RewindTarget) -> Result<Vec<serde_json::Value>, String> {
        Ok(self.rewind_with_reset(sid, target, false)?.main)
    }

    /// 停止 = **把整棵子树落成「已停止」**（拦住之后的一切派发与唤醒），再取消正在跑的生成；
    /// 这两步都不走命令队列里的长操作，生成期间照样立刻生效。
    /// **顺序不能反**：先冻态再取消——否则被停会话收尾写的那条"这一轮结束"会先把父会话叫醒。
    /// 用户「继续」（`continue_flow`）是它的逆操作。
    fn stop(&self, sid: &str) -> Vec<String> {
        let sid_owned = sid.to_string();
        let _ = self.call(move |core| core.turn.set_subtree_run(&sid_owned, RunState::Stopped));
        // **停止 = 拒绝**：整棵子树里等用户裁决的队一律作废（落盘 + 如实外送），等待方因此解开。
        self.void_gates(sid);
        self.stop_tree(sid).unwrap_or_default()
    }
}

impl RegistryOps for Handle {
    fn providers(&self) -> Result<Vec<ProviderView>, String> {
        self.call(|core| Ok(core.registry().provider_views()))
    }

    fn upsert_provider(&self, id: &str, base_url: &str, api_key: &str) -> Result<(), String> {
        let (id, base_url, api_key) = (id.to_string(), base_url.to_string(), api_key.to_string());
        self.call(move |core| core.registry_write(|r| r.provider_upsert(&id, &base_url, &api_key)))
    }

    fn remove_provider(&self, id: &str) -> Result<bool, String> {
        let id = id.to_string();
        self.call(move |core| core.registry_write(|r| r.provider_remove(&id)))
    }

    fn models(&self) -> Result<Vec<ModelView>, String> {
        self.call(|core| Ok(core.registry().model_views()))
    }

    fn core_model(&self) -> Result<Option<String>, String> {
        self.call(|core| Ok(core.registry().core_model()))
    }

    fn upsert_model(
        &self,
        draft: crate::capabilities::catalog::registry::api::ModelDraft,
    ) -> Result<(), String> {
        // 保存前实测（原生工具与回放形状）与落盘判据都在 registry：本层只把它交到核心线程上跑。
        self.call(move |core| {
            core.registry_write(|r| {
                crate::capabilities::catalog::registry::api::save_model(r, draft)
            })
        })
    }

    fn remove_model(&self, id: &str) -> Result<bool, String> {
        let id = id.to_string();
        self.call(move |core| core.registry_write(|r| r.model_remove(&id)))
    }

    fn set_core_model(&self, id: &str) -> Result<bool, String> {
        let id = id.to_string();
        self.call(move |core| core.registry_mut().core_set_model(&id))
    }

    fn agents(&self) -> Result<Vec<AgentView>, String> {
        self.call(|core| Ok(core.registry().agent_views()))
    }

    fn pick_agents(&self, names: &[String]) -> Result<Vec<AgentView>, String> {
        let names = names.to_vec();
        self.call(move |core| core.registry().pick_agents(&names))
    }

    fn upsert_agent(
        &self,
        name: &str,
        modules: &[String],
        model: &str,
        note: &str,
        chara: &str,
    ) -> Result<(), String> {
        let (name, modules, model, note, chara) = (
            name.to_string(),
            modules.to_vec(),
            model.to_string(),
            note.to_string(),
            chara.to_string(),
        );
        // 保留名与模块存在性都在 registry 的保存用例里；清单与保留名两样事实由这里取一份给它。
        self.call(move |core| {
            let roster = core.modules_handle().roster();
            let reserved = core.systools_handle().reserved_names();
            let draft = crate::capabilities::catalog::registry::api::AgentDraft {
                name,
                modules,
                model,
                note,
                chara,
            };
            core.registry_write(|r| {
                crate::capabilities::catalog::registry::api::save_agent(
                    r, &draft, &roster, &reserved,
                )
            })
        })
    }

    fn remove_agent(&self, name: &str) -> Result<bool, String> {
        let name = name.to_string();
        self.call(move |core| core.registry_mut().agent_remove(&name))
    }

    fn settings(&self) -> Result<AppSettings, String> {
        self.call(|core| Ok(core.registry().app_settings()))
    }

    fn set_settings(&self, app: AppSettings) -> Result<(), String> {
        self.call(move |core| core.registry_write(|r| r.set_app_settings(app)))
    }

    fn discover_models(&self, provider_id: &str) -> Result<Vec<String>, String> {
        let provider_id = provider_id.to_string();
        self.call(move |core| core.registry().discover_models(&provider_id))
    }

    fn probe_model_tools(
        &self,
        id: &str,
    ) -> Result<crate::capabilities::llm::api::ProbeOutcome, String> {
        let id = id.to_string();
        self.call(move |core| core.registry_mut().probe_model_tools(&id))
    }

    fn probe_replay_shape(
        &self,
        id: &str,
    ) -> Result<crate::capabilities::llm::api::ReplayReport, String> {
        let id = id.to_string();
        self.call(move |core| core.registry().probe_replay_shape(&id))
    }
}

impl crate::capabilities::conversation::session::api::HistoryOps for Handle {
    fn list(&self) -> Result<Vec<HistoryView>, String> {
        self.call(|core| Ok(core.lifecycle.history_list()))
    }

    fn open(&self, name: &str) -> Result<(SessionMeta, Vec<serde_json::Value>), String> {
        let name = name.to_string();
        self.call(move |core| core.lifecycle.history_open(&name))
    }

    fn delete(&self, name: &str) -> Result<bool, String> {
        let name = name.to_string();
        // 核心只做毫秒级的账；guest 销毁与目录删除在这一段（调用方线程）上按序执行。
        let delete =
            self.call(move |core| core.lifecycle.delete_bookkeeping(&mut core.turn, &name))?;
        delete.run()
    }
}

impl crate::capabilities::catalog::modules::api::ModulesOps for Handle {
    fn roster(&self) -> Result<Roster, String> {
        self.call(|core| Ok(core.modules_handle().roster()))
    }

    fn module_enabled(&self, id: &str) -> Result<bool, String> {
        let id = id.to_string();
        self.call(move |core| Ok(core.module_enabled(&id)))
    }

    fn service_enabled(&self, module: &str, service: &str) -> Result<bool, String> {
        let module = module.to_string();
        let service = service.to_string();
        self.call(move |core| Ok(core.service_enabled(&module, &service)))
    }
}

impl LogOps for Handle {
    fn info(&self, at: &str, msg: &str) {
        self.log.info(at, msg)
    }

    fn warn(&self, at: &str, msg: &str) {
        self.log.warn(at, msg)
    }

    fn error(&self, at: &str, msg: &str) {
        self.log.error(at, msg)
    }
}
