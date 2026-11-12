//! 目的：租约的并发准入——到上限拒绝、跨租约真并行。
//! 管：准入计数、表锁范围。
//! 不管：执行本身。
//! 联动：`src/capabilities/vm/api.rs`。

use super::super::prelude::*;
use crate::capabilities::vm::domain::guest::{GuestImage, GuestRun, GuestSpec};

/// 目的：并发准入——到上限拒绝、同一租约重启放行、0 = 不限制。
#[test]
pub(crate) fn guest_admission_caps_concurrency() {
    let two = vec!["a".to_string(), "b".to_string()];
    assert!(
        exec::admit_guest(&two, "a", 2).is_ok(),
        "同一租约重启不占新名额"
    );
    assert!(exec::admit_guest(&two, "c", 2).is_err(), "到上限要拒");
    assert!(exec::admit_guest(&two, "c", 3).is_ok(), "没到上限放行");
    assert!(exec::admit_guest(&two, "c", 0).is_ok(), "0 = 不限制");
    assert!(exec::admit_guest(&[], "c", 2).is_ok());
}

/// 目的：跨租约的执行要真并行——老的表锁把两个租约串起来（前一个跑完才轮到后一个）；
///   这里两个租约的 exec 必须见面（各自最多等对方 3 秒）。
#[test]
pub(crate) fn guests_on_different_leases_run_concurrently() {
    use crate::capabilities::vm::api::Vm;
    use crate::capabilities::vm::ports::{GuestChannel, GuestVm, SharePlan, VmEngine};
    use std::sync::atomic::AtomicBool;

    #[derive(Clone)]
    struct Rv {
        in_flight: Arc<AtomicUsize>,
        met: Arc<AtomicBool>,
    }
    struct RvChan(Rv);
    impl GuestChannel for RvChan {
        fn exec(&mut self, _run: &GuestRun) -> ToolOutcome {
            self.0.in_flight.fetch_add(1, Ordering::SeqCst);
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while std::time::Instant::now() < deadline {
                if self.0.in_flight.load(Ordering::SeqCst) >= 2 {
                    self.0.met.store(true, Ordering::SeqCst);
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            self.0.in_flight.fetch_sub(1, Ordering::SeqCst);
            ToolOutcome {
                ok: true,
                output: "ok".to_string(),
            }
        }
    }
    struct RvVm(RvChan);
    impl GuestVm for RvVm {
        fn channel(&mut self) -> &mut dyn GuestChannel {
            &mut self.0
        }
        fn stop(&mut self) {}
    }
    struct RvEngine(Rv);
    impl VmEngine for RvEngine {
        fn id(&self) -> &'static str {
            "rv"
        }
        fn supports(&self, _guest: &GuestImage) -> bool {
            true
        }
        fn boot(&self, _spec: &GuestSpec, _share: &SharePlan) -> Result<Box<dyn GuestVm>, String> {
            Ok(Box::new(RvVm(RvChan(self.0.clone()))))
        }
    }

    let rv = Rv {
        in_flight: Arc::new(AtomicUsize::new(0)),
        met: Arc::new(AtomicBool::new(false)),
    };
    let vm = crate::capabilities::vm::service::VmService::new(
        Arc::new(RvEngine(rv.clone())),
        vec![Arc::new(FakeShare::new())],
    );
    for lease in ["lease-a", "lease-b"] {
        let image = GuestImage::linux("t", PathBuf::from("/x/rootfs.qcow2"), None, None);
        vm.guest_start(&GuestSpec {
            lease: lease.to_string(),
            image,
            mounts: Vec::new(),
            net: false,
            engine_path: None,
            share_path: None,
            shares: Vec::new(),
            seed: None,
            isolated: false,
            cache_disk: None,
        })
        .expect("拉起");
    }
    let run = GuestRun {
        command: "true".to_string(),
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: String::new(),
        timeout_secs: 30,
        output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
    };
    std::thread::scope(|s| {
        for lease in ["lease-a", "lease-b"] {
            let vm = &vm;
            let run = &run;
            s.spawn(move || {
                let _ = vm.guest_run(lease, run);
            });
        }
    });
    assert!(
        rv.met.load(Ordering::SeqCst),
        "两个租约的 exec 要同时在飞（跨租约没被串行化）"
    );
}
