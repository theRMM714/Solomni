//! 目的：构建能力的用例实现——按模块声明在目标环境把产物编出来，并复验入口。
//! 管：本机档（宿主 + 产品围栏）与虚拟机档（guest + 编译包）两条构建路径、构建期能力选包、缓存命中、编完复验。
//! 不管：源指纹与状态文件的机制（`detail/fingerprint.rs`）、包库与引擎的机制（`packages` / `vm` / `provision`）、CLI 文案（`diagnostics`）。
//! 联动：`ProcessRunner` / `HostProbe` / `Log` 是 kernel 的共享机制；其余依赖走各能力的 api。

use crate::capabilities::builds::api::{Builds, Built, Target};
use crate::capabilities::builds::detail::cache;
use crate::capabilities::builds::detail::fingerprint as fp;
use crate::capabilities::modules::api::{BuildDecl, Module};
use crate::capabilities::packages::api::{
    build_packages, diagnose_text, plan_host_requests, requests_for, Facts, Library, Packages,
    PinPolicy, PlannedPackage, Provider,
};
use crate::capabilities::provision::api::Provision;
use crate::capabilities::storage::api::Storage;
use crate::capabilities::vm::api::{
    self as vm, ExecPlan, GuestOs, GuestRun, GuestSpec, MissingEntry, Vm,
};
use crate::kernel::api::{FenceSpec, Tier};
use crate::kernel::ports::{FenceHost, HostProbe, Log, ProcessRunner};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 目的：构建能力——持包库、虚拟机、补齐与执行器，按 `build:` 在目标环境编出模块产物。
pub struct BuildsService {
    root: PathBuf,
    packages: Arc<dyn Packages + Send + Sync>,
    vm: Arc<dyn Vm + Send + Sync>,
    provision: Arc<dyn Provision + Send + Sync>,
    storage: Arc<dyn Storage + Send + Sync>,
    runner: Arc<dyn ProcessRunner + Send + Sync>,
    fence: Arc<dyn FenceHost + Send + Sync>,
    probe: Arc<dyn HostProbe + Send + Sync>,
    log: Arc<dyn Log + Send + Sync>,
    /// 项目内基础工具链的只读安装根：本机档构建围栏要放行它们（编译器要读自己的 libexec / 动态库）。
    toolchain_roots: Vec<PathBuf>,
}

impl BuildsService {
    /// 目的：组合根专用——注入包库、虚拟机、补齐、执行器与工具链安装根（具体实现只在组合根 new）。
    /// 参数：`root` = 产品根；`packages` / `vm` / `provision` = 各能力的用例面；`runner` = 外部进程执行；`probe` = 宿主事实；`log` = 运行日志；`toolchain_roots` = 基础工具链安装根。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        root: PathBuf,
        packages: Arc<dyn Packages + Send + Sync>,
        vm: Arc<dyn Vm + Send + Sync>,
        provision: Arc<dyn Provision + Send + Sync>,
        storage: Arc<dyn Storage + Send + Sync>,
        runner: Arc<dyn ProcessRunner + Send + Sync>,
        fence: Arc<dyn FenceHost + Send + Sync>,
        probe: Arc<dyn HostProbe + Send + Sync>,
        log: Arc<dyn Log + Send + Sync>,
        toolchain_roots: Vec<PathBuf>,
    ) -> BuildsService {
        BuildsService {
            root,
            packages,
            vm,
            provision,
            storage,
            runner,
            fence,
            probe,
            log,
            toolchain_roots,
        }
    }

    /// 目的：目标环境的操作系统名（入口判据与产物形态按它分）。
    /// 错误：虚拟根不在场或画像不可用。
    fn target_os(&self, target: &Target) -> Result<String, String> {
        match target {
            Target::Host => Ok(std::env::consts::OS.to_string()),
            Target::Guest(guest) => {
                let dir = self.guest_dir(guest)?;
                let image = vm::classify_base(&dir, self.probe.as_ref())
                    .map_err(|why| format!("虚拟根「{}」不可用（{}）", guest, why))?;
                Ok(image.os.as_str().to_string())
            }
        }
    }

    /// 目的：虚拟根的目录（名字 → `.vm/guest/<名>`）；不在场如实拒绝。
    fn guest_dir(&self, guest: &str) -> Result<PathBuf, String> {
        let dir = vm::resolve_guest(&self.root, guest)
            .map_err(|e| format!("虚拟根「{}」解析失败：{}", guest, e))?;
        if !self.probe.is_dir(&dir) {
            return Err(format!(
                "本机没有虚拟根「{}」：先 solomni --provision --ensure --yes（或换一个虚拟根）",
                guest
            ));
        }
        Ok(dir)
    }

    /// 目的：这个模块在目标环境里还缺哪些入口（与建会话时的入口判据同一把尺子）。
    /// 参数：`module` = 模块；`target_os` = 目标平台。
    /// 返回：缺的入口（空 = 都在场）。
    fn entries_left(&self, module: &Module, target_os: &str) -> Vec<MissingEntry> {
        vm::missing_entries_for(std::slice::from_ref(module), target_os, &|p| {
            self.probe.is_executable(p)
                && (target_os == std::env::consts::OS || vm::runs_on_target(p, target_os))
        })
    }

    /// 目的：缓存命中吗——目标与源指纹都得一致，且入口都在场（否则等于没编）。
    fn fresh(
        &self,
        state_path: &Path,
        target: &str,
        target_os: &str,
        fingerprint: &str,
        module: &Module,
    ) -> bool {
        let Some(prev) = fp::read_state(state_path) else {
            return false;
        };
        prev.target == target
            && prev.fingerprint == fingerprint
            && self.entries_left(module, target_os).is_empty()
    }

    /// 目的：构建的收尾——复验入口、落状态；失败如实报（本机档与虚拟机档共用）。
    fn finish(
        &self,
        module: &Module,
        target: &str,
        target_os: &str,
        fingerprint: &str,
        state_path: &Path,
        output: &str,
    ) -> Result<(), String> {
        let left = self.entries_left(module, target_os);
        if !left.is_empty() {
            return Err(format!(
                "构建命令报了成功，但入口仍不在场：{}——脚本可能吞掉了失败（Windows 的 `if errorlevel` 对崩溃码不成立，用 `|| exit /b 1`），或产物写错了地方；检查 build.command{}",
                left.iter()
                    .map(|e| format!("{}（工具 {}）", e.command, e.tool))
                    .collect::<Vec<_>>()
                    .join("、"),
                tail(output)
            ));
        }
        fp::write_state(
            state_path,
            &fp::BuildState {
                module: module.manifest.id.clone(),
                target: target.to_string(),
                fingerprint: fingerprint.to_string(),
            },
        )
    }

    /// 目的：按声明把这批模块编出来（入口预判已由调用方做过）——本机档逐个在宿主编，虚拟机档共用一个 guest。
    /// 参数：`jobs` = 模块与构建声明；`target` = 目标环境；`target_os` = 目标平台；`net` = 放不放行出站网络。
    /// 返回：每个模块这次的事实（顺序与 `jobs` 一致）。
    fn run_jobs(
        &self,
        jobs: Vec<(Module, BuildDecl)>,
        target: &Target,
        target_os: &str,
        net: bool,
    ) -> Result<Vec<Built>, String> {
        match target {
            Target::Host => {
                let mut out = Vec::new();
                for (module, decl) in &jobs {
                    out.push(self.host(module, decl, target_os, net)?);
                }
                Ok(out)
            }
            Target::Guest(guest) => self.guest(&jobs, guest, target_os, net),
        }
    }

    /// 目的：本机档构建——在宿主上按声明的构建命令编，围栏由产品给（只写模块目录与状态目录）。
    fn host(
        &self,
        module: &Module,
        decl: &BuildDecl,
        target_os: &str,
        net: bool,
    ) -> Result<Built, String> {
        let base = self.packages.base();
        let current_path = std::env::var_os("PATH").unwrap_or_default();
        let has_exe = |name: &str| self.probe.has_exe(name);
        let bin_ok = |path: &Path| self.probe.is_dir(path);
        let facts = Facts {
            has_exe: &has_exe,
            bin_ok: &bin_ok,
            current_path: &current_path,
            base: &base.active,
            host_fallback: &base.fallback,
        };
        let requests = requests_for(&decl.needs, &format!("build:{}", module.manifest.id));
        let plan = plan_host_requests(
            &requests,
            &BTreeMap::new(),
            &self.packages.library(),
            PinPolicy::First,
            &facts,
        );
        if !plan.problems.is_empty() {
            return Err(format!(
                "本机没有这些构建期能力：{}（装工具链、把适用于本机目标的包放进 runtimes/、自己编好放进来，或让作者发产物）",
                diagnose_text(&plan.problems)
            ));
        }
        let providers = providers_of(&plan);
        let fingerprint = fp::source(&module.root, decl, &providers)?;
        let state_path = fp::state_path(&self.root, &module.manifest.id);
        if self.fresh(&state_path, "host", target_os, &fingerprint, module) {
            return Ok(Built {
                module: module.manifest.id.clone(),
                target: "host".to_string(),
                skipped: true,
                output: String::new(),
            });
        }
        let state_dir = self.root.join(".vm").join("build-state");
        let home = state_dir.join(&module.manifest.id).join("home");
        std::fs::create_dir_all(&home).map_err(|e| format!("建状态目录失败：{}", e))?;
        // 构建也是一次调用：按次 lease → Windows 上独立容器身份，跑完自己的 profile/ACE 自己收。
        let agent = format!("build:{}", module.manifest.id);
        let fence = FenceSpec {
            agent: agent.clone(),
            lease: crate::kernel::api::call_lease(&agent),
            rw: vec![module.root.clone(), state_dir.join(&module.manifest.id)],
            ro: self.toolchain_roots.clone(),
            ro_tree: plan.roots.clone(),
            private: home,
            cwd: module.root.clone(),
            net,
        };
        self.log.info(
            "builds::host",
            &format!(
                "模块 {} 在宿主上编（出站网络：{}）",
                module.manifest.id, net
            ),
        );
        let out = self
            .runner
            .run(&fence, &decl.command, "{}", &[], plan.path.as_deref(), None);
        // 按次收尾：撤 ACE、删按次 profile 与临时目录；失败如实记录（残留由 reconcile 按死归属回收）。
        if let Err(e) = self.fence.release_call(&fence) {
            self.log
                .warn("builds::host", &format!("围栏按次收尾未完成：{}", e));
        }
        if !out.ok {
            return Err(format!("构建命令失败（目标 host）{}", tail(&out.output)));
        }
        self.finish(
            module,
            "host",
            target_os,
            &fingerprint,
            &state_path,
            &out.output,
        )?;
        Ok(Built {
            module: module.manifest.id.clone(),
            target: "host".to_string(),
            skipped: false,
            output: out.output,
        })
    }

    /// 目的：虚拟机档构建——在 guest 本地工作区（缓存卷）里编，产物按差异回写宿主那份模块目录。
    /// 参数：`jobs` = 这批要编的模块与构建声明（一次调用共用一台构建 guest）；`guest` = 虚拟根名；`target_os` = 目标平台；`net` = 放不放行出站网络。
    /// 约束：同一虚拟根一把构建锁（缓存卷独占）；任何一步失败都不回写、不落状态。
    fn guest(
        &self,
        jobs: &[(Module, BuildDecl)],
        guest: &str,
        target_os: &str,
        net: bool,
    ) -> Result<Vec<Built>, String> {
        let dir = self.guest_dir(guest)?;
        let image = vm::classify_base(&dir, self.probe.as_ref())
            .map_err(|why| format!("虚拟根「{}」不可用（{}）", guest, why))?;
        // 构建命令与缓存卷都按 POSIX 装配：非 Linux guest 不具备这条链，如实拒绝而不是半路炸。
        if image.os != GuestOs::Linux {
            return Err(format!(
                "虚拟机档构建只支持 Linux guest（虚拟根「{}」的 os 是 {}）",
                guest,
                image.os.as_str()
            ));
        }
        let deps = self.provision.status().unwrap_or_default();
        let pick = |prefix: &str| {
            deps.iter()
                .find(|d| d.present && d.id.starts_with(prefix))
                .map(|d| d.probe.clone())
        };
        let Some(engine_path) = pick("vm.qemu.") else {
            return Err("没有可用的 QEMU：先 solomni --provision --ensure --yes".to_string());
        };
        let share_tool = if std::env::consts::OS == "windows" {
            pick("vm.smbd.")
        } else {
            pick("vm.virtiofsd.")
        };
        let platform = vm::guest_platform(image.os, image.arch);
        let eligible = Library::for_target(
            &self.packages.library(),
            &crate::capabilities::packages::api::Target::guest(&platform),
        );
        // 逐个定包、算指纹、判缓存：命中的模块不进这次 guest，结果照样按入参顺序给回。
        let mut plans: Vec<GuestPlan> = Vec::new();
        let mut done: Vec<Option<Built>> = Vec::new();
        for (module, decl) in jobs {
            let (packages, missing) = build_packages(&decl.needs, &eligible, &image.runtimes);
            if !missing.is_empty() {
                return Err(format!(
                    "guest「{}」里没有这些构建期能力，包库也没提供：{}（把适用于该 guest 目标的包放进 runtimes/，或给虚拟根装一个）",
                    guest,
                    missing.join("、")
                ));
            }
            let providers: Vec<String> = packages
                .iter()
                .map(|p| format!("{}@{}", p.id, p.version))
                .collect();
            let fingerprint = fp::source(&module.root, decl, &providers)?;
            let state_path = fp::state_path(&self.root, &module.manifest.id);
            if self.fresh(&state_path, guest, target_os, &fingerprint, module) {
                done.push(Some(Built {
                    module: module.manifest.id.clone(),
                    target: guest.to_string(),
                    skipped: true,
                    output: String::new(),
                }));
                continue;
            }
            done.push(None);
            plans.push(GuestPlan {
                module: module.clone(),
                decl: decl.clone(),
                packages,
                providers,
                fingerprint,
                state_path,
            });
        }
        if plans.is_empty() {
            return Ok(done.into_iter().flatten().collect());
        }
        // 缓存卷：同一虚拟根独占；起 guest 前先按存储预算回收一次（当前目标卷受保护）。
        let _headroom = self
            .storage
            .ensure_headroom(Some(guest))
            .map_err(|e| format!("构建前空间检查失败：{}", e))?;
        let volume = cache::volume_path(&self.root, guest);
        let _lock = cache::VolumeLock::acquire(cache::lock_path(&self.root, guest))?;
        if !self.probe.is_file(&volume) {
            let gib = self.storage.policy().build_volume_gib.max(1);
            vm::create_disk(&engine_path, &volume, gib.saturating_mul(1024))
                .map_err(|e| format!("建构建缓存卷失败：{}", e))?;
        }
        let lease = format!("build-{}", guest);
        let mut rw: Vec<PathBuf> = plans.iter().map(|p| p.module.root.clone()).collect();
        rw.sort();
        rw.dedup();
        let fence = FenceSpec {
            agent: "build".to_string(),
            lease: lease.clone(),
            rw,
            ro: Vec::new(),
            ro_tree: Vec::new(),
            private: PathBuf::new(),
            cwd: PathBuf::new(),
            net,
        };
        let plan = ExecPlan {
            tier: Tier::Vm,
            net,
            guest: Some(guest.to_string()),
            packages: merged_packages(plans.iter()),
        };
        let mounts = vm::mounts_from(&plan, &fence, &image.bridge());
        let spec = GuestSpec {
            lease: lease.clone(),
            image: image.clone(),
            mounts,
            net,
            engine_path: Some(engine_path),
            share_path: share_tool.clone(),
            shares: vm::share_chain(
                std::env::consts::OS,
                share_tool.as_deref(),
                self.probe.as_ref(),
            ),
            seed: None,
            isolated: true,
            cache_disk: Some(volume),
        };
        self.log.info(
            "builds::guest",
            &format!(
                "模块 {} 在 guest「{}」里编（本地工作区，出站网络：{}）",
                plans
                    .iter()
                    .map(|p| p.module.manifest.id.as_str())
                    .collect::<Vec<_>>()
                    .join("、"),
                guest,
                net
            ),
        );
        self.vm
            .guest_start(&spec)
            .map_err(|e| format!("起 guest 失败：{}", e))?;
        let result = self.build_in_guest(&lease, &plans, &image, &plan, guest, target_os);
        self.vm.guest_stop(&lease);
        let built = result?;
        let mut it = built.into_iter();
        Ok(done
            .into_iter()
            .map(|slot| slot.unwrap_or_else(|| it.next().expect("构建结果与计划一一对应")))
            .collect())
    }

    /// 目的：在这台构建 guest 里跑完这批模块——挂卷、逐模块同步源、本地构建、差异回写、复验落状态。
    fn build_in_guest(
        &self,
        lease: &str,
        plans: &[GuestPlan],
        image: &vm::GuestImage,
        plan: &ExecPlan,
        guest: &str,
        target_os: &str,
    ) -> Result<Vec<Built>, String> {
        if !self
            .vm
            .guest_wait_agent(lease, std::time::Duration::from_secs(15 * 60))
        {
            return Err("等 guest 的 qemu-guest-agent 超时".to_string());
        }
        // 运行包先接进 guest（prefix 的 bin 进 PATH、system 的载荷叠写进系统路径），再跑构建。
        for script in vm::package_setup_scripts(plan) {
            let _ = self.guest_script(lease, &script, 120);
        }
        // 缓存卷：挂载 + 卷级对账 + 按龄/排队清理（整批只做一次）。
        let purge = cache::read_purge(&self.root, guest);
        let keep: Vec<String> = plans.iter().map(|p| p.module.manifest.id.clone()).collect();
        let keep_secs = self.storage.policy().cache_keep_days.saturating_mul(86_400);
        let mounted = self.guest_script(
            lease,
            &cache::mount_script_with(&cache::volume_epoch(image), keep_secs, &keep, &purge),
            120,
        );
        if !mounted.ok {
            return Err(format!("准备构建缓存卷失败{}", tail(&mounted.output)));
        }
        if !purge.is_empty() {
            cache::clear_purge(&self.root, guest);
        }
        let mut out = Vec::new();
        for job in plans {
            out.push(self.build_one(lease, job, plans.len(), image, guest, target_os)?);
        }
        let usage = self.guest_script(lease, &cache::usage_script(), 120);
        if usage.ok {
            let mut modules: std::collections::BTreeMap<String, (u64, u64)> =
                std::collections::BTreeMap::new();
            for line in usage.output.lines() {
                let mut it = line.split('\t');
                let (Some(id), Some(bytes), Some(used)) = (it.next(), it.next(), it.next()) else {
                    continue;
                };
                if let (Ok(b), Ok(u)) = (bytes.parse::<u64>(), used.parse::<u64>()) {
                    modules.insert(id.to_string(), (b, u));
                }
            }
            if !modules.is_empty() {
                cache::write_meta(&self.root, guest, &modules);
            }
        }
        Ok(out)
    }

    /// 目的：编一个模块——模块对账 → 源同步 → 快照 → 本地构建 → 差异回写 → 复验落状态 → 收尾。
    #[allow(clippy::too_many_arguments)]
    fn build_one(
        &self,
        lease: &str,
        job: &GuestPlan,
        total: usize,
        image: &vm::GuestImage,
        guest: &str,
        target_os: &str,
    ) -> Result<Built, String> {
        let id = job.module.manifest.id.clone();
        let src = image.bridge().to_guest(&job.module.root);
        let name = |what: &str, why: String| module_why(&id, total, format!("{}{}", what, why));
        let step = |what: &str, out: crate::kernel::api::ToolOutcome| {
            if out.ok {
                Ok(())
            } else {
                Err(name(what, tail(&out.output)))
            }
        };
        step(
            "模块工作区对账失败",
            self.guest_script(
                lease,
                &cache::module_script(&id, &cache::module_epoch(&job.decl.needs, &job.providers)),
                120,
            ),
        )?;
        let entry = crate::capabilities::modules::api::local_entry(&job.decl.command);
        step(
            "同步模块源失败",
            self.guest_script(lease, &cache::sync_in_script(&id, &src, entry), 600),
        )?;
        step(
            "记录构建前快照失败",
            self.guest_script(lease, &cache::snapshot_script(&id), 120),
        )?;
        let env = vec![
            ("HOME".to_string(), cache::home(&id)),
            ("TMPDIR".to_string(), cache::tmp(&id)),
            ("TMP".to_string(), cache::tmp(&id)),
            ("TEMP".to_string(), cache::tmp(&id)),
        ];
        let command = format!(
            "cd {} && {}",
            cache::sh_quote(&cache::workspace(&id)),
            job.decl.command
        );
        let built = self.vm.guest_run(
            lease,
            &GuestRun {
                command,
                args_json: "{}".to_string(),
                env,
                cwd: String::new(),
                timeout_secs: 30,
                output_limit: crate::capabilities::vm::api::OutputLimit::Head,
            },
        );
        // 构建失败：缓存留着（现场可复现），但清临时目录并 trim，避免失败也把卷撑大。
        if !built.ok {
            let _ = self.guest_script(lease, &cache::cleanup_script(&id), 120);
            return Err(name(
                "构建命令失败",
                format!(
                    "{}；缓存留在 {}（怀疑缓存陈旧就在设置→存储空间清理该虚拟根缓存）",
                    tail(&built.output),
                    cache::workspace(&id)
                ),
            ));
        }
        step(
            "回写产物失败",
            self.guest_script(
                lease,
                &cache::sync_out_script(&id, &src, &product_entries(&job.module)),
                600,
            ),
        )?;
        self.finish(
            &job.module,
            guest,
            target_os,
            &job.fingerprint,
            &job.state_path,
            &built.output,
        )?;
        let _ = self.guest_script(lease, &cache::cleanup_script(&id), 120);
        Ok(Built {
            module: id,
            target: guest.to_string(),
            skipped: false,
            output: built.output,
        })
    }

    /// 目的：一段构建链内部用的 guest 脚本（输出保留开头与结尾）。
    fn guest_script(
        &self,
        lease: &str,
        script: &str,
        timeout_secs: u64,
    ) -> crate::kernel::api::ToolOutcome {
        self.vm.guest_run(
            lease,
            &GuestRun {
                command: script.to_string(),
                args_json: "{}".to_string(),
                env: Vec::new(),
                cwd: String::new(),
                timeout_secs,
                output_limit: crate::capabilities::vm::api::OutputLimit::HeadAndTail,
            },
        )
    }
}

impl Builds for BuildsService {
    fn ensure_many(
        &self,
        modules: &[&Module],
        target: &Target,
        net: bool,
    ) -> Result<Vec<Built>, String> {
        let target_os = self.target_os(target)?;
        let mut slots: Vec<Option<Built>> = Vec::new();
        let mut jobs: Vec<(Module, BuildDecl)> = Vec::new();
        let mut job_slots: Vec<usize> = Vec::new();
        for (i, module) in modules.iter().enumerate() {
            let left = self.entries_left(module, &target_os);
            let skip = |slots: &mut Vec<Option<Built>>| {
                slots.push(Some(Built {
                    module: module.manifest.id.clone(),
                    target: target.label(),
                    skipped: true,
                    output: String::new(),
                }));
            };
            let Some(decl) = module.manifest.build.clone() else {
                // 没有 build 段：入口在目标环境是否可用只能看入口本身。
                if left.is_empty() {
                    skip(&mut slots);
                    continue;
                }
                return Err(format!(
                    "模块 {} 的工具 {} 入口不在场：{}（模块没声明 build 段：要自带产物，或按模块 README 自己编一次）",
                    module.manifest.id,
                    left.first().map(|e| e.tool.as_str()).unwrap_or(""),
                    left.first().map(|e| e.command.as_str()).unwrap_or("")
                ));
            };
            // 有 build 段：入口存在不等于**这个目标**的产物。只有构建状态明确属于当前目标才跳过；
            // 状态是别的目标（宿主产物进 guest）就重编。状态缺失视为随仓自带产物，仍跳过。
            if left.is_empty()
                && fp::read_state(&fp::state_path(&self.root, &module.manifest.id))
                    .map(|prev| prev.target == target.label())
                    .unwrap_or(true)
            {
                skip(&mut slots);
                continue;
            }
            job_slots.push(i);
            slots.push(None);
            jobs.push(((*module).clone(), decl));
        }
        if !jobs.is_empty() {
            let built = self.run_jobs(jobs, target, &target_os, net)?;
            let mut it = built.into_iter();
            for i in job_slots {
                slots[i] = Some(it.next().expect("构建结果与计划一一对应"));
            }
        }
        Ok(slots
            .into_iter()
            .map(|b| b.expect("每个槽都有结果"))
            .collect())
    }

    fn build(&self, module: &Module, target: &Target, net: bool) -> Result<Built, String> {
        let target_os = self.target_os(target)?;
        let Some(decl) = module.manifest.build.clone() else {
            return Err(format!(
                "模块 {} 没声明 build 段：在 module.yaml 里写 build.command（见 MODULE_SPEC 的「构建」）",
                module.manifest.id
            ));
        };
        self.run_jobs(vec![(module.clone(), decl)], target, &target_os, net)?
            .pop()
            .ok_or_else(|| "构建没有给出结果".to_string())
    }
}

/// 目的：一个待编模块的全部准备事实（包、指纹、状态落点）——批量构建时一次算好。
struct GuestPlan {
    module: Module,
    decl: BuildDecl,
    packages: Vec<PlannedPackage>,
    providers: Vec<String>,
    fingerprint: String,
    state_path: PathBuf,
}

/// 目的：这批模块要的运行包合起来（同一份只留一次，顺序按第一次出现）。
fn merged_packages<'a>(plans: impl Iterator<Item = &'a GuestPlan>) -> Vec<PlannedPackage> {
    let mut out: Vec<PlannedPackage> = Vec::new();
    for p in plans.flat_map(|p| p.packages.iter()) {
        if !out.iter().any(|q| q.id == p.id && q.version == p.version) {
            out.push(p.clone());
        }
    }
    out
}

/// 目的：模块声明的工具入口（相对模块根）——回写时一律带回宿主，宿主那份可能被手工删掉。
fn product_entries(module: &Module) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tool in module.manifest.tools.values() {
        if let Some(entry) = crate::capabilities::modules::api::local_entry(&tool.command) {
            if !out.iter().any(|x| x == entry) {
                out.push(entry.to_string());
            }
        }
    }
    out
}

/// 目的：批量构建的错误带上是哪个模块（单模块时保持原来的文案，不重复说）。
fn module_why(id: &str, total: usize, why: String) -> String {
    if total > 1 {
        format!("模块 {}：{}", id, why)
    } else {
        why
    }
}

/// 目的：选型结果 → 指纹用的提供者清单（`能力=提供者`）。
fn providers_of(plan: &crate::capabilities::packages::api::PackagePlan) -> Vec<String> {
    plan.providers
        .iter()
        .map(|(cap, p)| match p {
            Provider::Base { id, version } | Provider::Package { id, version } => {
                format!("{}={}@{}", cap, id, version)
            }
            Provider::Host => format!("{}=host", cap),
        })
        .collect()
}

/// 目的：构建失败时把命令输出收成一段（没有输出就明说）。
fn tail(output: &str) -> String {
    let text = output.trim();
    if text.is_empty() {
        "：命令没有输出".to_string()
    } else {
        format!("：{}", text)
    }
}
