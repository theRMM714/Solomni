/* 冒烟：转录的纯逻辑——行映射、轮合成、事件吸收（回档 / 水位 / 裁剪重对齐）。
 * 用法：node src/presentation/web/assets/app/features/transcript/transcript.smoke.cjs
 *   输出 TRANSCRIPT-SMOKE-OK / FAIL。
 * 盯的契约：行身份全读结构化字段；流式块按到达顺序累积、绝不丢；
 *   回档按「回档后的活动视图」整体重建；水位以下的批次丢掉而不是重放；裁剪后按历史重新对齐并作废实时运行态。 */
const model = require("./model.js");
const events = require("./events.js");
const scroll = require("./scroll.js");
const flow = require("../flow/flow.js");

async function main() {

const fails = [];
const check = (ok, name) => { if (!ok) fails.push(name); };

/* ---------- 行映射：种类 → 样式，身份只从字段来 ---------- */
const tool = model.lineParts({ kind: "tool", speaker: "甲", tool: { speaker: "乙" }, reply: 3 })[0];
check(tool.cls === "tool" && tool.who === "乙" && tool.text === "" && tool.reply === 3, "工具行取视图里的说话人");

const compacted = model.lineParts({ kind: "compacted", line: "摘要", payload: { up_to: 12 } })[0];
check(compacted.cls === "sys compacted" && compacted.who === "压缩"
  && compacted.text.indexOf("第 12 行之前已压成摘要") === 0, "压缩行：身份与正文分开");

// 核心写下的记录行（计划 / 验收清单 / 交付）：同一个 notice 种类，有说话人 = 署名的记录。
const rec = model.lineParts({ kind: "notice", speaker: "核心整理", line: "- 做事" })[0];
check(rec.cls === "sys notice record" && rec.who === "核心整理" && rec.text === "- 做事", "记录行带署名");
check(model.lineParts({ kind: "notice", line: "提示" })[0].cls === "sys notice", "无署名的提示行");
check(model.lineParts({ kind: "system", line: "x" })[0].cls === "sys system", "system 行");
check(model.lineParts({ line: "x", system: true })[0].cls === "sys system", "system 标志行");
const round = model.lineParts({ kind: "round", speaker: "轮次", line: "2" })[0];
check(round.cls === "sys round" && round.text === "轮次 2" && round.who === "", "轮次分隔行：身份与正文分开");
check(model.lineParts({ kind: "user", line: "做吧" })[0].who === "用户", "用户行");
check(model.lineParts({ line: "同意", verb: "agree" })[0].cls === "ok", "同意行");
check(model.lineParts({ line: "先撤", verb: "leave" })[0].cls === "sys", "离席行");
check(model.lineParts({ line: "追问", verb: "ask" })[0].cls === "plan", "追问行");
const deg = model.lineParts({ line: "正文", speaker: "甲", verb: "agree", degraded: true })[0];
check(deg.cls === "sys" && deg.who.indexOf("信封缺失") > 0, "降级行标在身份上");

/* ---------- 轮合成：正文跳过工具行，思维链按首次出现去重 ---------- */
const parts = [
  { id: 1, cls: "line", text: "第一段" },
  { id: 2, cls: "tool", text: "", reasoning: "想一想" },
  { id: 3, cls: "line", text: "  " },
  { id: 4, cls: "ok", text: "第二段", reasoning: "想一想" },
];
check(model.turnWorthy(parts[0]) && !model.turnWorthy({ id: "x", cls: "line" }), "只有带数字 id 的发言进轮");
check(model.turnKey({ reply: 5 }) === "R5" && model.turnKey({ id: 9 }) === "K9", "轮键优先 reply");
check(model.turnText(parts) === "第一段\n\n第二段", "正文跳过工具与空行");
check(model.turnReasoning(parts) === "想一想", "思维链去重且不跳过工具行");

/* ---------- 事件吸收：装配一个假 app，只验数据侧 ---------- */
const state = { sessions: new Map(), activeSid: null, nodeBySid: new Map() };
const calls = { renderStream: 0, renderAll: 0, tick: 0, refresh: 0, history: 0 };
const api = async (method, url) => {
  if (url.indexOf("/api/history/") === 0) {
    calls.history += 1;
    return { head: 7, events: [{ type: "transcript", lines: [{ id: 1, kind: "user", line: "活过来了" }] }], live: [] };
  }
  return {};
};
const app = {
  flowApply: flow.flowApply,
  renderStream: () => { calls.renderStream += 1; },
  renderAll: () => { calls.renderAll += 1; },
  scheduleLiveTick: () => { calls.tick += 1; },
  refreshState: async () => { calls.refresh += 1; },
  activeSession: () => state.sessions.get(state.activeSid),
};
events.mount({}, { g: {}, doc: {}, state, stores: {}, api, ui: {}, app });

const mkSession = (sid) => {
  const s = {
    sid, lines: [], live: [], pending: null, fold: {}, scroll: {}, sending: false, done: false,
    running: false, working: null, running_known: false, workDirty: false, readonly: false,
  };
  state.sessions.set(sid, s);
  return s;
};
const s1 = mkSession("s1");
state.activeSid = "s1";

/* ---------- 流程视图：节点进度只进流程快照，不进转录 ---------- */
const sf = mkSession("sf");
events.absorb(sf, { type: "plan_review", chain: { nodes: [{ id: "n1-1", title: "做 X", assignee: "甲", status: "pending", sub_session: "sf--a" }] } });
check(sf.flow && sf.flow.nodes.length === 1 && sf.flow.nodes[0].status === "pending", "方案审查给出整条链");
events.absorb(sf, { type: "node_started", node: "n1-1", sid: "sf--a", assignee: "甲" });
check(sf.flow.nodes[0].status === "running" && sf.flow.nodes[0].sid === "sf--a", "开工把节点置为进行中");
events.absorb(sf, { type: "report", id: "n1-1", rework: 1, text: "做完了" });
check(sf.flow.nodes[0].status === "done" && sf.flow.nodes[0].rework === 1, "回报把节点置为完成并记返工");
events.absorb(sf, { type: "review", items: [{ item: "做 X", status: "pass" }, { item: "做 Y", status: "fail" }], raw: "" });
check(sf.flow.review.pass === 1 && sf.flow.review.total === 2, "验收清单汇总到流程结论");
events.absorb(sf, { type: "delivery", ok: false });
check(sf.flow.delivery && sf.flow.delivery.ok === false, "交付结论进流程快照");
check(sf.lines.length === 0, "节点进度不进转录");
events.applySnapshot(sf, []);
check(sf.flow === null, "回档清空旧的流程快照");

check(events.absorb(s1, { type: "notice", text: "提示" }) === false, "系统提示要整帧重建");
check(s1.lines.length === 1 && s1.lines[0].cls === "sys", "系统提示进转录");

events.absorb(s1, { type: "transcript", lines: [{ id: 1, kind: "user", line: "先这样" }] });
check(s1.lines.length === 2 && s1.lines[1].cls === "user" && s1.lines[1].text === "先这样", "权威行替换乐观回显、系统行保留");

events.absorb(s1, { type: "delta", kind: "start" });
events.absorb(s1, { type: "delta", kind: "text", text: "半句" });
events.absorb(s1, { type: "delta", kind: "text", text: "补上" });
events.absorb(s1, { type: "tool_call", name: "read" });
events.absorb(s1, { type: "delta", kind: "start" });
events.absorb(s1, { type: "delta", kind: "text", text: "第二轮" });
check(s1.live.length === 3, "多轮模型调用各占一块，不丢不并");
check(s1.live[0].segments.length === 1 && s1.live[0].segments[0].text === "半句补上", "同种流式分片累积");
check(s1.live[1].kind === "tool" && s1.live[2].kind === "msg", "工具卡插在轮与轮之间");
check(s1.live[0].key !== s1.live[2].key, "每轮流式块键唯一");

events.absorb(s1, { type: "working", agent: "甲" });
check(s1.running === true && s1.working === "甲" && s1.running_known === true, "运行态以事件为准");
events.absorb(s1, { type: "ended" });
check(s1.done === true && s1.running === false && s1.live.length === 0, "结束事件收掉流式块");

events.absorb(s1, { type: "usage", total_tokens: 5, window: 9 });
check(s1.context.total_tokens === 5 && s1.context.g === 9, "用量只更新状态栏，不进转录");

/* ---------- 协作的裁决事实不自造行：计划 / 验收清单 / 交付由后端落成带 id 的行 ---------- */
const sx = mkSession("sx");
for (const ev of [
  { type: "discussion_done", round: 3, over_cap: true },
  { type: "plan", text: "方案" },
  { type: "review", items: [{ item: "甲", status: "pass" }], raw: "原文" },
  { type: "delivery", ok: true, over_rework: false },
]) events.absorb(sx, ev);
check(sx.lines.length === 0, "协作事件不造转录行（事实只来自后端落下的行）");

/* ---------- 回档：按回档后的活动视图整体重建 ---------- */
s1.lines = [{ cls: "line", text: "旧的" }];
s1.pending = { id: "p" };
events.absorb(s1, { type: "rewound", events: [{ type: "transcript", lines: [{ id: 2, kind: "user", line: "重来" }] }] });
check(s1.lines.length === 1 && s1.lines[0].text === "重来", "回档清空旧视图并重建");
check(s1.pending === null && s1.forceFull === true && s1.readonly === false, "回档撤掉裁决卡并要求整帧重建");

/* ---------- 批次应用：水位以下丢掉、未知会话置对账、合流未定先攒着 ---------- */
let progress = events.progress;
check(events.applyBatch(3, "nope", []) === "none" && progress.needState === true, "未知会话不动，只置对账");

const s2 = mkSession("s2");
s2.floor = 5;
check(events.applyBatch(5, "s2", [{ type: "notice", text: "旧" }]) === "none" && s2.lines.length === 0, "水位以下的批次丢掉");
check(events.applyBatch(6, "s2", [{ type: "notice", text: "新" }]) === "full" && s2.lines.length === 1, "水位之上的批次照常应用");

const s3 = mkSession("s3");
s3.hydrating = true;
check(events.applyBatch(9, "s3", [{ type: "notice", text: "早到" }]) === "none", "合流未定先返回 none");
check(s3.buffer.length === 1 && s3.buffer[0].seq === 9, "合流未定的批次先攒着");

/* ---------- 裁剪重对齐：跳号 → 水位前移、作废实时运行态、按历史重放 ---------- */
progress.seq = 2;
s2.running_known = true;
await events.trimAlign(2);
check(progress.seq === 2, "没跳号的重对齐不动水位");

await events.trimAlign(9);
check(progress.seq === 8, "裁剪后水位跳到还留着的起点之前");
check(s2.running_known === false, "裁剪作废所有会话的实时运行态");
check(calls.history === 1 && calls.renderStream === 1, "裁剪后按历史重放并重画");
check(s1.floor === 7, "重放顺带刷新合流水位");
check(s1.lines.length === 1 && s1.lines[0].text === "活过来了", "重放重建转录");

/* ---------- 批次渲染粒度：纯流式只排合并渲染，定稿整帧重建 ---------- */
const before = { tick: calls.tick, full: calls.renderAll };
await events.applyLines([{ seq: 10, sid: "s2", events: [{ type: "delta", kind: "start" }] }]);
check(calls.tick === before.tick + 1 && calls.renderAll === before.full, "纯流式增量只排合并渲染");
await events.applyLines([{ seq: 11, sid: "s2", events: [{ type: "notice", text: "定稿" }] }]);
check(calls.renderAll === before.full + 1, "定稿事件整帧重建");

/* ---------- 滚动跟随：锁存只在滚动事件推进，容差以内的上滚也算离开底部 ---------- */
check(scroll.bottomGap({ scrollHeight: 1000, scrollTop: 900, clientHeight: 100 }) === 0, "在底部时距底为 0");
check(scroll.bottomGap({ scrollHeight: 1000, scrollTop: 880, clientHeight: 100 }) === 20, "距底按内容算");
check(scroll.bottomGap({ scrollHeight: 500, scrollTop: 0, clientHeight: 500 }) === 0, "内容不满一屏算在底部");
check(scroll.bottomGap({ scrollHeight: 500, scrollTop: 30, clientHeight: 520 }) === 0, "回弹越界不产生负距底");
check(scroll.followAfterScroll({ scrollHeight: 1000, scrollTop: 900, clientHeight: 100 }) === true, "在底部就跟随");
check(scroll.followAfterScroll({ scrollHeight: 1000, scrollTop: 898, clientHeight: 100 }) === true, "分数坐标仍算在底部");
check(scroll.followAfterScroll({ scrollHeight: 1000, scrollTop: 880, clientHeight: 100 }) === false, "上滚 20px 立即解除跟随");
check(scroll.followAfterScroll({ scrollHeight: 1000, scrollTop: 888, clientHeight: 100 }) === true, "容差边界之内仍算底部");
check(scroll.followAfterScroll({ scrollHeight: 1000, scrollTop: 887, clientHeight: 100 }) === false, "越过容差即解除跟随");
check(scroll.followAfterScroll({ scrollHeight: 500, scrollTop: 0, clientHeight: 500 }) === true, "短内容始终跟随");

  if (fails.length) {
    for (const f of fails) console.log("FAIL " + f);
    console.log("TRANSCRIPT-SMOKE-FAIL");
    process.exit(1);
  }
  console.log("TRANSCRIPT-SMOKE-OK");
}

main().catch((e) => {
  console.log("FAIL 异常：" + (e && e.message));
  console.log("TRANSCRIPT-SMOKE-FAIL");
  process.exit(1);
});
