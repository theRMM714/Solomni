/**
 * L4 旅程清单（唯一的声明处）：**一段用户可见的连续行为**一个 id。
 * 驱动（tests/cross-platform/e2e/driver.js）按 id 包段执行，specs 按 id 声明"谁覆盖了哪一段"，
 * 门禁（tests/gates/structure/specs.js）双向对账：清单里有、驱动里没有 = 声明空转；反过来 = 未登记的旅程。
 *
 * deps 是**执行顺序约束**的显式写法：e2e 共用一份夹具与同一串会话状态，后面的段用得到前面的段
 * 留下的工作（比如上传场景要用单 agent 那段建出的会话）。选某一段时按 deps 把前置段一起带出来，
 * 别指望"只跑一段"能凭空造出前置状态。
 *
 * requires 是环境档（缺了如实记 [探针]，不算通过）：由驱动自己判，这里只做声明。
 */
"use strict";

const JOURNEYS = [
  { id: "registry", unit: "catalog", title: "登记处：供应商 + 模型 + 两个 agent", deps: [] },
  { id: "suggest", unit: "catalog", title: "推荐：复用登记处的 agent，不代拟模型", deps: ["registry"] },
  { id: "single-write", unit: "conversation", title: "单 agent：内置 write 落私沙箱 + 转录结构 + 删除回档", deps: ["registry"] },
  { id: "rewind-archive", unit: "conversation", title: "留档回档：标记可恢复，转录与沙箱一起展回", deps: ["registry"] },
  { id: "shell", unit: "tools", title: "零模块 shell：成功与退出码都落成工具行（真机围栏档）", deps: ["registry"], requires: ["E2E_FENCE_LIVE"] },
  { id: "modules", unit: "tools", title: "多模块 cwd + 漏写 module 如实报错", deps: ["registry"] },
  { id: "files", unit: "conversation", title: "上传不覆盖 + 文件清单 + @ 引用改写（含空格与句读）", deps: ["single-write"] },
  { id: "tool-ask", unit: "tools", title: "工具级确认：allow / deny / allow_turn 三态", deps: ["registry"] },
  { id: "bad-envelope", unit: "tools", title: "非法工具信封：失败工具行且 JSON 不上屏", deps: ["registry"] },
  { id: "unclosed-envelope", unit: "tools", title: "未闭合信封：正文保留、坏 JSON 不上屏", deps: ["registry"] },
  { id: "abs-path", unit: "tools", title: "外部工具用真实绝对路径读投喂文件", deps: ["registry"] },
  { id: "patch", unit: "tools", title: "自由格式补丁：原文落盘、不上屏", deps: ["registry"] },
  { id: "ref-collab", unit: "conversation", title: "协作里引用私沙：如实说明只有该 agent 能读", deps: ["registry"] },
  { id: "prose", unit: "tools", title: "正文 + 工具信封同一轮：正文进转录，信封不上屏", deps: ["registry"] },
  { id: "collab", unit: "conversation", title: "协作五阶段：待审 → 开工 → 交付", deps: ["registry"] },
  { id: "delegate", unit: "conversation", title: "代拟：复用 + 组装，确认后名单写回 meta", deps: ["registry"] },
  { id: "collab-state-machine", unit: "conversation", title: "协作状态机六条判据：收敛 / 退场 / 上限 / 返工 / 撤回 / ask 中止", deps: ["registry"] },
  { id: "mcp", unit: "tools", title: "MCP 常驻服务：真进程调用，回执与本地摘要对账", deps: ["registry"] },
  { id: "real-tools", unit: "tools", title: "三语言真模块链路 + 资产契约（PNG / MP4）", deps: ["registry"] },
  { id: "run-module", unit: "tools", title: "不经 AI 直跑模块工具（含运行包 PATH）", deps: ["registry"] },
  { id: "native-tools", unit: "llm", title: "原生工具调用：一次回复两个调用，协议形状由假供应商核对", deps: ["registry"] },
  { id: "proxy", unit: "conversation", title: "代理模式：自己挑人、建子工作、停止即全停", deps: ["registry"] },
  { id: "vm", unit: "vm", title: "虚拟机档：guest 内工具与 shell、外部工作区、删除即销毁", deps: ["registry"], requires: ["SOLOMNI_VM_LIVE 或 SOLOMNI_VM_QEMU"] },
  { id: "parent-death", unit: "residents", title: "父死子亡：核心被强杀后常驻服务进程跟着消失", deps: ["registry", "mcp"] },
];

const BY_ID = new Map(JOURNEYS.map((j) => [j.id, j]));

/** 递归展开依赖（自己也算）——选一段时把前置段一起带出来。 */
function withDeps(ids) {
  const out = new Set();
  const walk = (id) => {
    if (out.has(id)) return;
    out.add(id);
    const j = BY_ID.get(id);
    if (!j) return;
    for (const d of j.deps) walk(d);
  };
  for (const id of ids) walk(id);
  return out;
}

/** 缺依赖 / 重复 / 依赖不存在的静态校验；返回问题列表（空数组 = 干净）。 */
function selfCheck() {
  const problems = [];
  const seen = new Set();
  for (const j of JOURNEYS) {
    if (seen.has(j.id)) problems.push("旅程清单：id 重复 " + j.id);
    seen.add(j.id);
    for (const d of j.deps) {
      if (!BY_ID.has(d)) problems.push("旅程清单：" + j.id + " 依赖不存在的旅程 " + d);
      if (d === j.id) problems.push("旅程清单：" + j.id + " 依赖自己");
    }
    if (!j.unit) problems.push("旅程清单：" + j.id + " 缺 unit");
    if (!j.title) problems.push("旅程清单：" + j.id + " 缺 title");
  }
  // 环检测：deps 有环会让展开死循环。
  const state = new Map();
  const visit = (id) => {
    if (state.get(id) === 1) { problems.push("旅程清单：依赖成环（" + id + "）"); return; }
    if (state.get(id) === 2) return;
    state.set(id, 1);
    for (const d of (BY_ID.get(id) || { deps: [] }).deps) visit(d);
    state.set(id, 2);
  };
  for (const j of JOURNEYS) visit(j.id);
  return problems;
}

module.exports = { JOURNEYS, BY_ID, withDeps, selfCheck };
