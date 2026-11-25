//! 目的：呈现层的入站面集合：把同一个核心手柄按**屏幕各取一面**拆成窄契约，每个渠道只认自己那一面。
//! 管：`Faces` 的字段与 `Faces::from_handle`；字段类型全部是各单元自己的入站契约或值。
//! 不管：任何判定与编排（在 `runtime` 与各能力）；字段之外的装配顺序（在 `src/main.rs`）。
//! 联动：`src/main.rs` 造出手柄后调 `Faces::from_handle`，再补上 `provision` / `storage` / `toolchains`；`src/presentation/{cli,web}` 各屏只取需要的字段。

use crate::units::catalog::modules::api::ModulesOps;
use crate::units::catalog::registry::api::RegistryOps;
use crate::units::conversation::session::api::HistoryOps;
use crate::units::runtime::api::{ActionOps, Handle, LogOps, RuntimeOps};
use crate::units::turn::turn::api::EventFeed;
use crate::units::turn::turn::api::TurnOps;
use crate::units::work::api::WorkOps;
use std::sync::Arc;

/// 目的：呈现层的入站面集合：每一屏只依赖它需要的那一面，不再有一个装着全部会话命令的桶。
///   克隆廉价；契约测试可以用假实现替换任意一个字段。
#[derive(Clone)]
pub struct Faces {
    /// 目的：工作的用例面：建工作、配置与编辑、文件与提交历史、运行报告与核心推荐。
    pub work: Arc<dyn WorkOps + Send + Sync>,
    /// 目的：回合的用例面：说一句、继续、写需求、回答裁决卡、看裁决队、撤回同意、压缩、在跑吗。
    pub turn: Arc<dyn TurnOps + Send + Sync>,
    /// 目的：顶层自己的编排用例：启用、改需求、回档、停止。
    pub runtime: Arc<dyn RuntimeOps + Send + Sync>,
    pub registry: Arc<dyn RegistryOps + Send + Sync>,
    pub history: Arc<dyn HistoryOps + Send + Sync>,
    /// 目的：模块公地的只读面（清单事实的上游是 `modules`）。
    pub modules: Arc<dyn ModulesOps + Send + Sync>,
    /// 目的：事件台的**只读**面：呈现层订阅事实，但写面（`Events`）不出本层。
    pub events: Arc<dyn EventFeed>,
    /// 目的：动作能力：目录 + 分发（CLI 与 Web 共用同一份声明与同一处授权）。
    pub actions: Arc<dyn ActionOps + Send + Sync>,
    /// 目的：日志能力：呈现层只经它埋点（**不持有端口对象**）。
    pub log: Arc<dyn LogOps + Send + Sync>,
    /// 目的：常驻服务的统一管理面（组合根注入的同一份，经核心手柄转发）。
    pub residents: Arc<dyn crate::units::residents::api::ResidentOps + Send + Sync>,
    /// 目的：隐秘字段的统一管理面（组合根注入的同一份，经核心手柄转发）。
    pub secrets: Arc<dyn crate::units::catalog::secrets::api::SecretOps + Send + Sync>,
    /// 目的：依赖补齐的统一面（组合根注入；默认未接入，如实失败不假装可用）。
    pub provision: Arc<dyn crate::units::provision::api::Provision + Send + Sync>,
    /// 目的：存储空间的统一面（组合根注入；默认空实现，测试可替换）。
    pub storage: Arc<dyn crate::units::maintenance::storage::api::Storage + Send + Sync>,
    /// 目的：基础工具链的启动事实（组合根注入；测试替身为 null）。
    pub toolchains: serde_json::Value,
}

impl Faces {
    /// 目的：组合根用：把同一个核心手柄按屏幕拆开（同一个执行线程、同一个事件台）。
    pub fn from_handle(h: &Handle) -> Faces {
        Faces {
            work: Arc::new(h.clone()),
            turn: Arc::new(h.clone()),
            runtime: Arc::new(h.clone()),
            registry: Arc::new(h.clone()),
            history: Arc::new(h.clone()),
            modules: Arc::new(h.clone()),
            events: h.events(),
            actions: Arc::new(h.clone()),
            log: Arc::new(h.clone()),
            residents: Arc::clone(&h.residents),
            secrets: Arc::clone(&h.secrets),
            provision: Arc::new(crate::units::provision::api::NoProvision),
            storage: Arc::new(crate::units::maintenance::storage::api::NoStorage),
            toolchains: serde_json::Value::Null,
        }
    }
}
