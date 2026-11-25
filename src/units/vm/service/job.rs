//! 目的：要等 guest 的那一段命令——按租约销毁，不在核心线程上跑。
//! 管：一次销毁的入参（租约清单）与执行顺序（逐条 guest_stop，幂等）。
//! 不管：谁该被销毁（调用方按会话树与围栏账算出租约）与围栏撤权。
//! 联动：\`src/units/vm/api.rs\` 导出 \`GuestStop\`；\`work\` 的改配与 \`lifecycle\` 的删除都用它。

use crate::units::vm::api::Vm;
use std::sync::Arc;

/// 目的：一次 guest 销毁（慢：先请 guest 断电并等它落盘，超时才杀）——在调用方线程上跑。
/// 参数：vm 是 guest 引擎；leases 是要销毁的租约名。
/// 返回：无——\`guest_stop\` 本就在失配时静默（幂等）。
/// 约束：同一个租约可以被重复销毁；调用方负责算出租约集合。
pub struct GuestStop {
    vm: Arc<dyn Vm + Send + Sync>,
    leases: Vec<String>,
}

impl GuestStop {
    /// 目的：组装备好的一段（毫秒级，核心侧调用）。
    /// 参数：vm 是 guest 引擎；leases 是要销毁的租约名（顺序即销毁顺序）。
    /// 返回：待执行的任务对象。
    /// 约束：不在这里做 IO——真正的销毁在 \`run\` 里。
    pub fn new(vm: Arc<dyn Vm + Send + Sync>, leases: Vec<String>) -> GuestStop {
        GuestStop { vm, leases }
    }

    /// 目的：执行销毁——这里要等 guest 落盘，别在核心线程上跑。
    /// 参数：无（消费自 \`new\` 装好的入参）。
    /// 返回：无。
    /// 约束：逐条调用 \`guest_stop\`，失败不外抛（引擎侧幂等且已记账）。
    pub fn run(self) {
        for lease in &self.leases {
            self.vm.guest_stop(lease);
        }
    }
}
