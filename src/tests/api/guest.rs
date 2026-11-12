//! 目的：guest 诊断动作——只给人，报告原样带出，缺 guest 如实拒绝。
//! 管：目录可见性、报告字段映射、缺 lease 的错误。
//! 不管：虚拟机档实现（`src/tests/vm/`）、动作分发（`tools.rs`）。
//! 联动：`src/capabilities/vm/domain/diagnose.rs` 与 `src/capabilities/conductor/api/action.rs`。

use super::super::builders::{agent_meta, seed_session, vm_spec};
use super::super::doubles::module_of;
use super::super::doubles::InMemoryHistory;
use super::super::{ops_with, ops_with_vm};
use crate::capabilities::conductor::api::Output;
use crate::capabilities::conductor::api::{Acted, ActionCall, Caller};
use std::sync::Arc;

struct DiagnosingVm {
    report: crate::capabilities::vm::api::GuestDiagnosis,
}

impl crate::capabilities::vm::api::Vm for DiagnosingVm {
    fn guest_start(&self, _spec: &crate::capabilities::vm::api::GuestSpec) -> Result<(), String> {
        Ok(())
    }

    fn guest_run(
        &self,
        _lease: &str,
        _run: &crate::capabilities::vm::api::GuestRun,
    ) -> crate::kernel::api::ToolOutcome {
        crate::kernel::api::ToolOutcome {
            ok: false,
            output: "替身不跑命令".to_string(),
        }
    }

    fn guest_run_fenced(
        &self,
        _lease: &str,
        _run: &crate::capabilities::vm::api::GuestRun,
        _fence: &crate::kernel::api::FenceSpec,
    ) -> crate::kernel::api::ToolOutcome {
        crate::kernel::api::ToolOutcome {
            ok: false,
            output: "替身不跑命令".to_string(),
        }
    }

    fn stage_exec(
        &self,
        _lease: &str,
        _module: &str,
        _host: &std::path::Path,
        entry: &str,
    ) -> Result<String, String> {
        Ok(entry.to_string())
    }

    fn guest_packages(&self, _lease: &str) -> Option<Vec<String>> {
        None
    }

    fn guest_set_packages(&self, _lease: &str, _packages: Vec<String>) {}

    fn guest_wait_agent(&self, _lease: &str, _timeout: std::time::Duration) -> bool {
        false
    }

    fn guest_stop(&self, _lease: &str) {}

    fn guest_leases(&self) -> Vec<String> {
        Vec::new()
    }

    fn guest_diagnose(
        &self,
        _lease: &str,
    ) -> Result<crate::capabilities::vm::api::GuestDiagnosis, String> {
        Ok(self.report.clone())
    }
}

fn sample_diagnosis(lease: &str) -> crate::capabilities::vm::api::GuestDiagnosis {
    use crate::capabilities::vm::domain::diagnose as va;
    va::GuestDiagnosis {
        lease: lease.to_string(),
        uptime_secs: 1234.5,
        loadavg: [0.5, 0.4, 0.3],
        process_total: 1,
        truncated: false,
        processes: vec![va::GuestProcess {
            pid: 42,
            ppid: 1,
            state: "R".to_string(),
            cpu_percent: 12.5,
            rss_kb: 2048,
            etime_secs: 30,
            comm: "probe".to_string(),
            args: "probe --busy".to_string(),
        }],
        memory: va::GuestMemory {
            total_kb: 2_097_152,
            available_kb: 1_048_576,
            free_kb: 524_288,
            buffers_kb: 1024,
            cached_kb: 4096,
            swap_total_kb: 0,
            swap_free_kb: 0,
            used_percent: 50.0,
        },
        cpu: va::GuestCpu {
            user: 100,
            nice: 0,
            system: 50,
            idle: 900,
            iowait: 5,
            irq: 0,
            softirq: 1,
            steal: 0,
            idle_percent: 85.0,
        },
        mounts: vec![va::GuestMountInfo {
            source: "overlay".to_string(),
            target: "/".to_string(),
            fstype: "overlay".to_string(),
            total_kb: 4_194_304,
            free_kb: 2_097_152,
        }],
        guest: va::GuestMeta {
            kernel: "6.1.0".to_string(),
            nproc: 2,
        },
    }
}

#[test]
fn guest_diagnose_is_user_only_in_the_catalog() {
    let (_h, ops) = ops_with(vec![module_of("a")], Vec::new());
    let cat = ops.actions.catalog(&Caller::User, None).expect("动作目录");
    let entry = cat
        .iter()
        .find(|a| a.id == "guest_diagnose")
        .expect("人该在目录里看到诊断动作");
    assert!(
        entry
            .params
            .iter()
            .any(|p| p.name == "session_id" && p.required),
        "{:?}",
        entry
            .params
            .iter()
            .map(|p| (&p.name, p.required))
            .collect::<Vec<_>>()
    );

    let role = Caller::Role {
        role: "executor".to_string(),
        work: "w".to_string(),
        agent: "a".to_string(),
    };
    let cat = ops.actions.catalog(&role, None).expect("动作目录");
    assert!(
        !cat.iter().any(|a| a.id == "guest_diagnose"),
        "角色面不该列出诊断动作"
    );
    let err = ops
        .actions
        .act(ActionCall {
            id: "guest_diagnose".to_string(),
            args: serde_json::json!({ "session_id": "w" }),
            caller: role,
            out: Output::Final,
        })
        .unwrap_err();
    assert!(err.contains("不接受这个调用者"), "{}", err);
}

#[test]
fn guest_diagnose_returns_the_guest_report_and_names_missing_guests() {
    let hist = Arc::new(InMemoryHistory::new());
    seed_session(
        &hist,
        "vmw",
        "single",
        vec![agent_meta("a", &["a"], None)],
        vm_spec(),
    );
    seed_session(
        &hist,
        "hostw",
        "single",
        vec![agent_meta("a", &["a"], None)],
        Default::default(),
    );
    seed_session(&hist, "p", "proxy", Vec::new(), vm_spec());
    let vm: Arc<dyn crate::capabilities::vm::api::Vm + Send + Sync> = Arc::new(DiagnosingVm {
        report: sample_diagnosis("vmw"),
    });
    let (_h, ops) = ops_with_vm(vec![module_of("a")], Arc::clone(&hist), vm);

    match ops
        .actions
        .act(ActionCall {
            id: "guest_diagnose".to_string(),
            args: serde_json::json!({ "session_id": "vmw" }),
            caller: Caller::User,
            out: Output::Final,
        })
        .expect("诊断虚拟机档会话")
    {
        Acted::Done(v) => {
            assert_eq!(v.get("lease").and_then(|x| x.as_str()), Some("vmw"));
            assert_eq!(v.get("process_total").and_then(|x| x.as_u64()), Some(1));
            assert_cpu_and_memory_sections(&v);
            let procs = v
                .get("processes")
                .and_then(|x| x.as_array())
                .expect("进程表");
            assert_eq!(
                procs[0].get("comm").and_then(|x| x.as_str()),
                Some("probe"),
                "{v}"
            );
            assert_eq!(
                procs[0].get("rss_kb").and_then(|x| x.as_u64()),
                Some(2048),
                "{v}"
            );
        }
        _ => panic!("诊断要给结构化结果"),
    }

    // 本机档：没有 guest 可诊断。
    let err = ops
        .actions
        .act(ActionCall {
            id: "guest_diagnose".to_string(),
            args: serde_json::json!({ "session_id": "hostw" }),
            caller: Caller::User,
            out: Output::Final,
        })
        .unwrap_err();
    assert!(err.contains("本机档"), "{}", err);

    // 代理父会话：自己没有 guest，要指向子工作。
    let err = ops
        .actions
        .act(ActionCall {
            id: "guest_diagnose".to_string(),
            args: serde_json::json!({ "session_id": "p" }),
            caller: Caller::User,
            out: Output::Final,
        })
        .unwrap_err();
    assert!(err.contains("代理父会话"), "{}", err);
}

fn assert_cpu_and_memory_sections(v: &serde_json::Value) {
    assert!(
        v.get("memory")
            .and_then(|m| m.get("total_kb"))
            .and_then(|x| x.as_u64())
            .is_some(),
        "内存段：{v}"
    );
    assert!(
        v.get("cpu")
            .and_then(|c| c.get("idle_percent"))
            .and_then(|x| x.as_f64())
            .is_some(),
        "CPU 段：{v}"
    );
    assert!(
        v.get("loadavg").and_then(|l| l.as_array()).is_some(),
        "负载段：{v}"
    );
    assert!(
        v.get("mounts").and_then(|m| m.as_array()).is_some(),
        "挂载段：{v}"
    );
}
