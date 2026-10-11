#!/usr/bin/env node
// 演示驱动：对一个**已经跑起来**的 Solomni 转录中心做一次 MCP 真机演示——
// 拉起 mcp-toolbox 常驻服务 → 建一个只装这一个模块的单 agent 工作 → 一句需求让它用 MCP 操作
// （算 SHA-256、取 UUID 与当前时间、写入并取回便签）→ 从落盘转录逐条对账，而不是信它的自我陈述。
//
// 用法：
//   1) 先起产品：node start.js -webUI            （默认网页端口 3081）
//   2) 另开一个终端：node demo/run-demo-mcp.mjs
// 环境变量：
//   SOLOMNI_DEMO_BASE   转录中心地址（默认 http://127.0.0.1:3081）
//   SOLOMNI_DEMO_MODEL  指定模型 id（默认不指定 = 用核心默认模型）
//
// **为什么先起服务再建工作**：单 agent 会话的工具面在**建工作时**装配一次（started_service_ops），
// 之后才启动的服务不会补进这条会话——顺序反了这次演示就验不出东西。这条约束写在模块 README 里。
// 只走产品自己的 HTTP 能力面（与前端同一条路），不改任何登记处；任何一步不成立都如实打印并以非零退出码收场。
import { createHash } from "node:crypto";
import { request } from "node:http";
import { mcpPreflight, refuseDemo } from "./preflight.mjs";

const BASE = process.env.SOLOMNI_DEMO_BASE || "http://127.0.0.1:3081";
const MODEL = process.env.SOLOMNI_DEMO_MODEL || "";
const URL_BASE = new URL(BASE);

const MODULE = "mcp-toolbox";
const SERVICE = "toolbox";
const AGENT = "工具手";
const WORK = "demo-mcp-" + Date.now();
const HASH_TEXT = "solomni";
const EXPECTED_SHA = createHash("sha256").update(HASH_TEXT).digest("hex");
const NOTES = [["项目", "solomni"], ["角色", "mcp-toolbox"], ["口令", "sesame-42"]];
const OPS = ["now", "uuid", "sha256", "remember", "recall"];

let failed = 0;
const ok = (cond, label, extra) => {
  if (!cond) failed++;
  console.log((cond ? "PASS " : "FAIL ") + label + (cond || extra === undefined ? "" : " :: " + String(extra).slice(0, 400)));
};

/** 比对人念回来的值：忽略大小写与空白（模型可能换行或大写，值本身没变）。 */
const flat = (s) => String(s).toLowerCase().replace(/\s+/g, "");

/** 一次能力面调用：不设客户端超时（模型一轮可能跑几分钟），等它自己返回。 */
const actUrl = (id) => "/api/actions/" + id;
function api(method, path, body) {
  return new Promise((resolve, reject) => {
    const payload = body === undefined ? null : Buffer.from(JSON.stringify(body), "utf8");
    const headers = payload ? { "Content-Type": "application/json", "Content-Length": payload.length } : undefined;
    const req = request(
      { hostname: URL_BASE.hostname, port: URL_BASE.port, path, method, headers },
      (res) => {
        const chunks = [];
        res.on("data", (c) => chunks.push(c));
        res.on("end", () => {
          const text = Buffer.concat(chunks).toString("utf8");
          let json = null;
          try { json = JSON.parse(text); } catch (e) { void e; }
          resolve({ status: res.statusCode, json, text });
        });
      },
    );
    req.setTimeout(0);
    req.on("error", reject);
    if (payload) req.write(payload);
    req.end();
  });
}

/** 等转录中心起来（最多 30 秒）。 */
async function waitReady() {
  for (let i = 0; i < 60; i++) {
    try { const s = await api("GET", "/api/state"); if (s.status === 200) return true; } catch (e) { void e; }
    await new Promise((r) => setTimeout(r, 500));
  }
  return false;
}

/** 该工作此刻的转录行（从落盘流水取，已应用回档截断）。 */
async function lines(sid) {
  const r = await api("GET", "/api/history/" + encodeURIComponent(sid));
  const out = [];
  for (const ev of (r.json && r.json.events) || []) {
    if (ev.type === "transcript") for (const l of ev.lines || []) out.push(l);
  }
  return out;
}

/** 说一句话：这一轮跑完（模型不再调用工具）才会返回。 */
async function say(sid, text) {
  return api("POST", actUrl("send_message"), { session_id: sid, text });
}

/** 常驻服务生命周期（唯一入口，人经呈现层用）。 */
async function resident(action, extra) {
  const body = Object.assign({ module: MODULE, name: SERVICE, action }, extra || {});
  return api("POST", actUrl("control_resident"), body);
}

const toolRows = (ls) => ls.filter((l) => l.tool).map((l) => l.tool);
/** 线上名可能是 toolbox.now、mcp-toolbox_toolbox_now——取最后一段当操作名。 */
const leaf = (name) => String(name).split(/[._]/).pop();
const rowsOf = (rows, op) => rows.filter((r) => leaf(r.name) === op);
const parse = (text) => { try { return JSON.parse(text); } catch (e) { void e; return null; } };
const answerOf = (ls) => ls.filter((l) => l.kind === "msg").map((l) => String(l.line || "")).join("\n");

function taskText() {
  return [
    "你手上只有一个模块 mcp-toolbox，它声明了一个 MCP 常驻服务 toolbox，操作有 toolbox.now / toolbox.uuid / toolbox.sha256 / toolbox.remember / toolbox.recall。",
    "请在这一次回复里依次做完，每一步都必须真的调用对应操作，不要凭记忆编造：",
    "1) 用 toolbox.sha256 算文本 \"" + HASH_TEXT + "\" 的 SHA-256 摘要；",
    "2) 用 toolbox.uuid 取一个随机 UUID；",
    "3) 用 toolbox.now 取当前 UTC 时间；",
    "4) 用 toolbox.remember 依次记住三条：" + NOTES.map((kv) => kv[0] + "=" + kv[1]).join("、") + "；",
    "5) 用 toolbox.recall 依次取回这三条。",
    "最后把 1-5 每一步的原始回执原样念给我（一行一条，写明操作名与回执内容）。",
  ].join("\n");
}

async function main() {
  if (!(await waitReady())) {
    console.error("转录中心没起来：" + BASE + "（先跑 node start.js -webUI）");
    return 1;
  }
  const pre = await api("GET", "/api/state");
  const block = mcpPreflight(pre.json, MODEL);
  if (block) return refuseDemo(block);

  // ① 先起服务：enable 打开开关，start 拉起并发现操作（这一步就验了 MCP 握手 + tools/list）。
  const en = await resident("enable");
  ok(en.status === 200, "启用常驻服务 " + MODULE + "." + SERVICE, en.text);
  const st = await resident("start");
  const ops = (st.json && st.json.operations) || [];
  ok(st.status === 200 && OPS.every((o) => ops.includes(o)), "启动服务并发现操作（MCP 握手 + tools/list）", ops.join("、") || st.text);

  // ② 再建工作：一个 agent，只装这一个模块。
  const agent = { name: AGENT, modules: [MODULE] };
  if (MODEL) agent.model = MODEL;
  const created = await api("POST", actUrl("create_session"), { name: WORK, mode: "single", agents: [agent] });
  ok(created.status === 200, "建工作 " + WORK, created.text);
  const sid = (created.json && created.json.sid) || WORK;

  try {
    // ③ 一句需求：全部走 MCP 操作，并把原始回执念回来。缺操作就**如实点名再问一轮**（最多两轮）。
    console.log("（一轮问询要等模型跑完，可能要几分钟）");
    let rows = [];
    for (let round = 0; round < 2; round++) {
      const ask = await say(sid, round === 0 ? taskText() : (
        "还差这些操作没有成功调用：" + OPS.filter((op) => !rowsOf(rows, op).some((r) => r.ok)).join("、") +
        "。请补齐，并把每一步的原始回执原样念给我。"
      ));
      ok(ask.status === 200, round === 0 ? "发出需求" : "补齐需求", ask.text);
      rows = toolRows(await lines(sid));
      const missing = OPS.filter((op) => !rowsOf(rows, op).some((r) => r.ok));
      if (!missing.length) break;
      if (round === 1) ok(false, "五个操作都成功调用", "缺：" + missing.join("、"));
    }

    // ④ 对账：工具行是服务的真回执，模型的发言要带回这些值。
    for (const r of rows) {
      console.log("   工具 " + (r.label || r.name) + (r.ok ? " 成功" : " 失败") + " → " + String(r.output || "").split("\n")[0].slice(0, 160));
    }
    for (const op of OPS) {
      ok(rowsOf(rows, op).some((r) => r.ok), "调用过并成功：" + op, rowsOf(rows, op).map((r) => (r.ok ? "ok" : "fail")).join(","));
    }
    const answer = answerOf(await lines(sid));

    const shaRows = rowsOf(rows, "sha256").filter((r) => r.ok);
    ok(
      shaRows.some((r) => (parse(r.output) || {}).sha256 === EXPECTED_SHA),
      "sha256 回执与本地计算一致（" + EXPECTED_SHA.slice(0, 12) + "…）",
      shaRows.map((r) => r.output).join(" | "),
    );
    ok(flat(answer).includes(EXPECTED_SHA), "模型的发言带回了这个摘要", answer.slice(0, 300));

    const uuids = rowsOf(rows, "uuid").filter((r) => r.ok).map((r) => (parse(r.output) || {}).uuid).filter(Boolean);
    ok(uuids.some((u) => flat(answer).includes(flat(u))), "模型的发言带回了服务给的确切 UUID", uuids.join("、"));

    const recall = rowsOf(rows, "recall").filter((r) => r.ok).map((r) => parse(r.output) || {});
    const got = recall.filter((r) => r.found).map((r) => String(r.value));
    const want = NOTES.map((kv) => kv[1]);
    ok(want.every((v) => got.includes(v)), "recall 取回了写入的三条便签", got.join("、"));
    ok(want.every((v) => flat(answer).includes(flat(v))), "模型的发言带回了三条便签的值", answer.slice(0, 300));

    const skews = rowsOf(rows, "now").filter((r) => r.ok)
      .map((r) => Math.abs(Date.now() - Number((parse(r.output) || {}).epoch_ms)))
      .filter((n) => Number.isFinite(n));
    ok(skews.length > 0 && Math.min.apply(null, skews) < 10 * 60 * 1000, "now 回执落在本次演示时刻附近", JSON.stringify(skews));

    console.log(failed ? "DEMO-FAILED failed=" + failed : "DEMO-OK（工作 " + WORK + "）");
    return failed ? 1 : 0;
  } finally {
    const stop = await resident("stop");
    console.log("[收尾] 停止常驻服务 " + MODULE + "." + SERVICE + " → " + (stop.status === 200 ? "ok" : stop.text));
  }
}

process.exit(await main());
