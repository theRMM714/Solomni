//! 目的：HTTP 入站契约的**唯一定义**：路由目录 + 匹配器。
//! 管：路由表（方法、路径模式、入站能力、请求/响应形状、状态码、说明）与匹配、路径参数解析、百分号解码、目录 JSON 输出。
//! 不管：请求分发与响应构造（`src/presentation/web.rs`）；文档表本身的维护（`docs/presentation/contracts.md` 的 ROUTES 段由契约测试与这里比对）。
//! 联动：`src/presentation/web.rs` 按本目录分发，所以「代码里有路由但目录里没有」在结构上不可能发生；反向缺口由 `src/tests/routes.rs` 逐条点名。

use serde_json::json;

/// 目的：一条路由。`id` 是分发键：`src/presentation/web.rs` 只按它分发，不再自己写 (method, path) 匹配。
pub struct Route {
    pub id: &'static str,
    pub method: &'static str,
    /// 目的：路径模式；`{name}` 段是参数（匹配时按段取值）。
    pub pattern: &'static str,
    /// 目的：它落到哪个入站能力（文档与覆盖检查用）。
    pub capability: &'static str,
    pub request: &'static str,
    pub response: &'static str,
    pub statuses: &'static [u16],
    pub note: &'static str,
}

/// 目的：全部 HTTP 路由。新增一条 = 在这里加一行 + 在 `src/presentation/web.rs` 加一个分支（契约测试会盯着两者）。
pub const ROUTES: &[Route] = &[
    Route {
        id: "index",
        method: "GET",
        pattern: "/",
        capability: "静态资源",
        request: "—",
        response: "index.html",
        statuses: &[200],
        note: "界面壳（no-store，改完立刻生效）",
    },
    Route {
        id: "asset",
        method: "GET",
        pattern: "/assets/{*path}",
        capability: "静态资源",
        request: "—",
        response: "编译期嵌入表里的文件",
        statuses: &[200, 404],
        note: "前端脚本 / 样式（build.rs 扫描 assets/ 生成；表外路径一律 404）",
    },
    Route {
        id: "events",
        method: "GET",
        pattern: "/api/events",
        capability: "事件台（core::api::EventBus）",
        request: "查询 sid / since",
        response: "{lines:[{seq,sid,events}],head,oldest}",
        statuses: &[200],
        note: "长轮询：有新事件立刻回，否则最多等 20s",
    },
    Route {
        id: "state",
        method: "GET",
        pattern: "/api/state",
        capability: "ModulesOps::roster + SessionOps::session_views + RegistryOps + HistoryOps::list",
        request: "—",
        response: "{modules,services,rejected,fence,providers,models,core,agents,settings,sessions,history}",
        statuses: &[200, 400],
        note: "概览状态；一律不含密钥（`modules[].runtimes` 给创建向导预判虚拟根够不够）",
    },
    Route {
        id: "actions",
        method: "GET",
        pattern: "/api/actions",
        capability: "ActionOps::catalog",
        request: "查询 sid",
        response: "{actions:[{id,desc,params,available,reason}]}",
        statuses: &[200, 400],
        note: "动作目录：这个调用者此刻能做什么（含可用性）；前端据此渲染，不写第二份清单",
    },
    Route {
        id: "action",
        method: "POST",
        pattern: "/api/actions/{id}",
        capability: "ActionOps::act",
        request: "{…按动作声明}",
        response: "{head} / {events}（重放快照） / {ok,…}",
        statuses: &[200, 400],
        note: "一次动作：参数按 systools/tools.yaml 校验 → callers 授权 → 执行 → 审计（stop / answer_card 生成期间也立刻生效）",
    },
    Route {
        id: "session.config",
        method: "GET",
        pattern: "/api/sessions/{sid}/config",
        capability: "SessionOps::config",
        request: "—",
        response: "{config}",
        statuses: &[200, 400],
        note: "配置视图：能改什么、现在是什么、缺什么",
    },
    Route {
        id: "session.files",
        method: "GET",
        pattern: "/api/sessions/{sid}/files",
        capability: "SessionOps::files",
        request: "—",
        response: "{work,agents,roots}",
        statuses: &[200, 404],
        note: "@ 菜单与长路径缩写用（相对清单 + 真实根）",
    },
    Route {
        id: "session.work",
        method: "GET",
        pattern: "/api/sessions/{sid}/work",
        capability: "SessionOps::work_log",
        request: "查询 limit / before",
        response: "{head,commits,total,truncated}",
        statuses: &[200, 404],
        note: "提交历史（新→旧，分页）：每条带 on_head（是否在当前 head 祖先链）与改动条数",
    },
    Route {
        id: "session.work.commit",
        method: "GET",
        pattern: "/api/sessions/{sid}/work/{commit}",
        capability: "SessionOps::work_commit_view / work_commit_file",
        request: "查询 path（可选）",
        response: "{id,parent,author,session,time,message,changes,changes_truncated} / {path,hash,size,media_type,text,truncated,too_large}",
        statuses: &[200, 404],
        note: "提交详情；带 path = 该提交里这个文件的受限预览（文本 16 KiB，二进制只给元信息）",
    },
    Route {
        id: "dirs",
        method: "GET",
        pattern: "/api/dirs",
        capability: "宿主目录浏览",
        request: "查询 path（可选；省略 = 用户主目录）",
        response: "{path,parent,roots:[盘符],dirs:[{name,path}]}",
        statuses: &[200, 400],
        note: "建会话时选工作区目录用；只在本机 Web 上提供目录列表，不读写文件内容",
    },
    Route {
        id: "settings.get",
        method: "GET",
        pattern: "/api/settings",
        capability: "RegistryOps::settings",
        request: "—",
        response: "{settings}",
        statuses: &[200, 400],
        note: "基本设置",
    },
    Route {
        id: "history.list",
        method: "GET",
        pattern: "/api/history",
        capability: "HistoryOps::list",
        request: "—",
        response: "{sessions}",
        statuses: &[200, 400],
        note: "落盘会话清单",
    },
    Route {
        id: "history.open",
        method: "GET",
        pattern: "/api/history/{name}",
        capability: "HistoryOps::open",
        request: "—",
        response: "{meta,events,live,head,shell}",
        statuses: &[200, 404],
        note: "读回历史：盘上转录 + 事件台上它之外的实时尾巴 + 合流时的头部序号（水位）",
    },
    Route {
        id: "history.delete",
        method: "POST",
        pattern: "/api/history/{name}/delete",
        capability: "HistoryOps::delete",
        request: "—",
        response: "{ok}",
        statuses: &[200, 400],
        note: "删除会话时核心会请求撤销该会话的围栏授权",
    },
    Route {
        id: "tiers",
        method: "GET",
        pattern: "/api/tiers",
        capability: "ConductorOps::tier_choices",
        request: "—",
        response: "{tiers:{default,vm_available,vm_unavailable_reason,vm_requirements,guests,available}}",
        statuses: &[200, 400],
        note: "新建工作的档位选择：默认档 + 虚拟机档的**机器能力**可用性与逐项前置 + 可选虚拟根清单（虚拟根是会话级，不在这里判）",
    },
    Route {
        id: "processes",
        method: "GET",
        pattern: "/api/processes",
        capability: "Provision::processes",
        request: "—",
        response: "{procs:[{pid,alive,purpose,owner,since}]}",
        statuses: &[200, 400],
        note: "现在在跑什么：产品亲手起的长驻子进程（QEMU / 共享进程）与它们的归属；`alive=false` 表示账上还留着、进程已没（等对账清）",
    },
    Route {
        id: "setup.status",
        method: "GET",
        pattern: "/api/setup/status",
        capability: "Provision::status + job",
        request: "查询 since / wait",
        response: "{deps:[{id,guest,present,install,probe,source,url,size_mb,note}],job:{running,seq,lines,error},missing,pending}",
        statuses: &[200, 400],
        note: "依赖补齐的状态 + 作业进度：`pending` 是还没装的条目 id（界面据此决定装什么）；`wait=1` 挂起等新行（长轮询，最多 20s）",
    },
    Route {
        id: "setup.ensure",
        method: "POST",
        pattern: "/api/setup/ensure",
        capability: "Provision::status + start",
        request: "{deps?:[id], confirm?:bool}",
        response: "{preview:[{id,url,size_mb,present}]} / {ok,deps}",
        statuses: &[200, 400],
        note: "不点名 = 只补现在缺的那些；confirm 才起下载安装作业，否则只回预览（要下载哪些、多大）；都装好了直接回 {ok,deps:[]}",
    },
    Route {
        id: "setup.job",
        method: "DELETE",
        pattern: "/api/setup/job",
        capability: "Provision::cancel",
        request: "—",
        response: "{ok}",
        statuses: &[200],
        note: "取消当前补齐作业（即发即忘）",
    },
    Route {
        id: "storage",
        method: "GET",
        pattern: "/api/storage",
        capability: "Storage::report",
        request: "—",
        response: "{report:{generatedAt,managedBytes,capBytes,categories,policy,job}}",
        statuses: &[200, 400],
        note: "空间盘点：构建缓存 / 下载缓存 / 过期可写层 / 日志 / 虚拟根 / 会话 / 模块产物",
    },
    Route {
        id: "storage.gc",
        method: "POST",
        pattern: "/api/storage/gc",
        capability: "Storage::gc",
        request: "{apply?:bool}",
        response: "{ok,report:{actions,skipped,freedBytes,overCap}}",
        statuses: &[200, 400],
        note: "按预算回收一次；apply=false 只预览，不删",
    },
    Route {
        id: "storage.clean",
        method: "POST",
        pattern: "/api/storage/clean",
        capability: "Storage::clean",
        request: "{kind,id?,module?,confirm:bool}",
        response: "{report} / {started:{running,seq,lines}}",
        statuses: &[200, 400],
        note: "显式清理整卷/按模块/下载缓存/可写层/日志；guest-reset 启动还原作业（必须 confirm）",
    },
    Route {
        id: "suggest",
        method: "POST",
        pattern: "/api/suggest-models",
        capability: "ConductorOps::suggest_models",
        request: "{task,mode}",
        response: "{ok,agents}",
        statuses: &[200, 400],
        note: "核心推荐 agent 草案（带理由；用户可改）",
    },
];

/// 目的：命中结果：路由 + 路径参数（按 pattern 里出现的名字取值；已百分号解码）。
pub struct Matched {
    pub route: &'static Route,
    pub params: Vec<(&'static str, String)>,
}

impl Matched {
    /// 目的：取一个路径参数；目录保证只出现 pattern 里声明过的名字。
    pub fn param(&self, name: &str) -> String {
        self.params
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    }
}

/// 目的：按目录匹配一次请求（方法 + 路径；查询串不参与匹配）。
/// 约束：末段 `{*name}` 是尾部通配，吞下其余一个或多个段，逐段解码后用 `/` 连接。
pub fn matches(method: &str, url: &str) -> Option<Matched> {
    let path = url.split('?').next().unwrap_or("");
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    for route in ROUTES {
        if route.method != method {
            continue;
        }
        let pat: Vec<&str> = route.pattern.split('/').filter(|s| !s.is_empty()).collect();
        let tail = pat
            .last()
            .and_then(|p| p.strip_prefix("{*"))
            .and_then(|x| x.strip_suffix('}'));
        let fixed = if tail.is_some() {
            pat.len() - 1
        } else {
            pat.len()
        };
        let len_ok = match tail {
            Some(_) => segs.len() > fixed,
            None => segs.len() == fixed,
        };
        if !len_ok {
            continue;
        }
        let mut params: Vec<(&'static str, String)> = Vec::new();
        let mut hit = true;
        for (p, s) in pat[..fixed].iter().zip(segs.iter()) {
            if let Some(name) = p.strip_prefix('{').and_then(|x| x.strip_suffix('}')) {
                // p 取自 route.pattern（'static），所以参数名同样是 'static。
                params.push((name, url_decode(s)));
            } else if p != s {
                hit = false;
                break;
            }
        }
        if hit {
            if let Some(name) = tail {
                let rest: Vec<String> = segs[fixed..].iter().map(|s| url_decode(s)).collect();
                params.push((name, rest.join("/")));
            }
            return Some(Matched { route, params });
        }
    }
    None
}

/// 目的：路径段百分号解码（%XX；路径里没有 '+' 语义）。前端用 encodeURIComponent，故必须解回来。
pub fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// 目的：目录的 JSON 形态（`--print-routes` 与契约测试共用同一份事实）。
pub fn catalog_json() -> serde_json::Value {
    json!(ROUTES
        .iter()
        .map(|r| json!({
            "id": r.id,
            "method": r.method,
            "pattern": r.pattern,
            "capability": r.capability,
            "request": r.request,
            "response": r.response,
            "statuses": r.statuses,
            "note": r.note,
        }))
        .collect::<Vec<_>>())
}
