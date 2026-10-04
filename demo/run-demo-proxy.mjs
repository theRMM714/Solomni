#!/usr/bin/env node
/**
 * 代理模式演示（L4 演示脚本，**在真机上跑**）：用户把决定权**整块**交给核心——
 * 他只跟核心说一句目标，核心自己挑人、建子工作、把活转达出去；子会话停下会自动叫醒核心，
 * 核心再主动倒查正文、接着安排；最后用户按「停止」，整棵子树一起停下。
 *
 * 与另外两个演示的区别（见 PRODUCT.md「代理（proxy）」）：
 *   demo/run-demo.mjs         组合式：一个 agent 装三个模块，用户直接给它活；
 *   demo/run-demo-collab.mjs  小组协作：三个 agent 各持一份能力协商，用户当裁判；
 *   本脚本                     代理：没有名单、也没有裁判——核心代用户决定（全权）。
 *
 * 用法：
 *   1) 先起产品：node start.js -webUI            （默认网页端口 3081）
 *   2) 另开一个终端：node demo/run-demo-proxy.mjs
 * 环境变量：
 *   SOLOMNI_DEMO_BASE         转录中心地址（默认 http://127.0.0.1:3081）
 *   SOLOMNI_DEMO_TIMEOUT_MIN  第一幕的收敛时限（分钟，默认 20）
 *   注意：`SOLOMNI_DEMO_MODEL` **对本脚本无效**——代理会话跑在核心默认模型上，
 *   它临时挑出来的子会话用什么模型也由核心决定（另两个演示用它给 agent 指定模型）。
 *
 * 为什么这个脚本**不进 CI**：它要真实供应商与核心默认模型（CI 上没有 .home/，会回落到内置假模型，
 * 那样验的就不是代理能力而是假脚本）。代理状态机的机器判据在 tests/cross-platform/e2e/driver.js。
 * 只走产品自己的 HTTP 能力面（与前端同一条路），**不改任何登记处**；产物落在**各子会话自己的**
 * 共享区/沙箱里——代理子工作有自己的 work/，父会话的共享区不共享给它们（见 docs/session）。
 */
import { existsSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { request } from "node:http";
import { proxyPreflight, refuseDemo } from "./preflight.mjs";

const BASE = process.env.SOLOMNI_DEMO_BASE || "http://127.0.0.1:3081";
const URL_BASE = new URL(BASE);
const WORK = "demo-proxy-" + Date.now();
const STOP_WORK = "demo-proxy-stop-" + Date.now();
const TIMEOUT_MIN = Number(process.env.SOLOMNI_DEMO_TIMEOUT_MIN || 20);
const DEADLINE_MS = (Number.isFinite(TIMEOUT_MIN) && TIMEOUT_MIN > 0 ? TIMEOUT_MIN : 20) * 60 * 1000;
const POLL_MS = 2000;
// 叫醒是**异步**的：子会话停下 → 写一条通知 → 起一个代理回合。连续几轮都安静才算收敛，
// 否则会把"刚要醒"当成"已经结束"。
const QUIET_POLLS = 5;

/** 说给代理听的目标（第一幕）：只给结果与约束，怎么挑人、怎么分工由它自己定。 */
const GOAL = [
  "把这件事办完：人怎么挑、活怎么分你自己定，不用问我；这是件小事，人手不用多。",
  "① 挑合适的人建子工作，让他在自己的共享区里写一份 notes.md（Markdown，正文不少于 300 字，说清「本地优先」的三条理由）；",
  "② 再让他基于这份笔记产出一份 card.html——自包含、能离线打开（内联样式，不许引用任何外部资源）；",
  "③ 两件产物都落盘之后，用 read_session_messages 把他的话看一遍，确认产物真的存在；",
  "④ 最后告诉我：你派了谁、建了哪些子会话、两件产物的真实绝对路径。",
].join("\n");

/** 说给第二个代理听的目标（第二幕）：要一件**要花点时间**的活，好在中途按停止。 */
const STOP_GOAL =
  "让合适的人认真写一份 800 字以上的长文《把决定权交给 AI 的边界》，写进他自己的共享区里的 long.md；" +
  "写细一点、慢慢来。人你自己挑。";

let failed = 0;
const ok = (cond, label, extra) => {
  if (!cond) failed++;
  console.log((cond ? "PASS " : "FAIL ") + label + (cond || extra === undefined ? "" : " :: " + String(extra).slice(0, 400)));
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const enc = encodeURIComponent;

/** 一次能力面调用：不设客户端超时（一轮可能跑几分钟），等它自己返回。
    长请求被网络层重置（ECONNRESET）是常态——**重试**，而不是让整个演示崩掉。 */
async function api(method, path, body, tries = 3) {
  for (let i = 1; ; i++) {
    try {
      return await once(method, path, body);
    } catch (e) {
      if (i >= tries) throw e;
      console.log("   [重试 " + i + "] " + path + "：" + (e.code || e.message));
      await sleep(2000 * i);
    }
  }
}

/** 真正发一次请求（api 负责重试）。 */
function once(method, path, body) {
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
          try { json = JSON.parse(text); } catch {}
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
    try { const s = await api("GET", "/api/state"); if (s.status === 200) return true; } catch {}
    await sleep(500);
  }
  return false;
}

/** 一次状态快照（会话视图 + 历史 + 清单都在这一个回包里）。 */
async function state() {
  const r = await api("GET", "/api/state");
  return r.json || {};
}

/** 该工作的全部事件（转录行 + 通知 + 交付）。 */
async function events(sid) {
  const r = await api("GET", "/api/history/" + enc(sid));
  return (r.json && r.json.events) || [];
}

/** 转录行（已应用回档截断）。 */
async function lines(sid) {
  const out = [];
  for (const ev of await events(sid)) {
    if (ev.type === "transcript") for (const l of ev.lines || []) out.push(l);
  }
  return out;
}

const toolRows = (ls) => ls.filter((l) => l.tool).map((l) => l.tool);

/** 本工作 + 它的**直接/间接子工作**：历史里 parent 指上来的一整棵子树。 */
function descendants(st, root) {
  const hist = (st && st.history) || [];
  const seen = new Set([root]);
  const out = [];
  let frontier = [root];
  while (frontier.length) {
    const next = [];
    for (const h of hist) {
      if (h && h.parent && frontier.includes(h.parent) && !seen.has(h.name)) {
        seen.add(h.name);
        out.push(h);
        next.push(h.name);
      }
    }
    frontier = next;
  }
  return out;
}

/** 这棵子树里此刻还在生成中的会话（含根）。 */
function subtreeRunning(st, root) {
  const names = new Set([root, ...descendants(st, root).map((h) => h.name)]);
  return ((st && st.sessions) || []).filter((v) => v && names.has(v.sid) && v.running).map((v) => v.sid);
}

/** 本工作这棵子树在盘上的目录名：`session/<工作>` 与 `session/<工作>--<子工作>`。 */
function subtreeDirs(work) {
  const base = join(process.cwd(), "session");
  if (!existsSync(base)) return [];
  return readdirSync(base, { withFileTypes: true })
    .filter((e) => e.isDirectory() && (e.name === work || e.name.startsWith(work + "--")))
    .map((e) => e.name);
}

/** 这棵子树在盘上落下的**产物文件**（会话自己的元数据不算产物）。 */
function producedFiles(work) {
  const base = join(process.cwd(), "session");
  const found = new Map();
  const walk = (dir) => {
    for (const ent of readdirSync(dir, { withFileTypes: true })) {
      const p = join(dir, ent.name);
      if (ent.isDirectory()) walk(p);
      else if (ent.name !== "meta.yaml" && ent.name !== "transcript.jsonl" && !found.has(ent.name)) found.set(ent.name, p);
    }
  };
  for (const d of subtreeDirs(work)) walk(join(base, d));
  return found;
}

async function main() {
  if (!(await waitReady())) {
    console.error("转录中心没起来：" + BASE + "（先跑 node start.js -webUI）");
    return 1;
  }
  if (process.env.SOLOMNI_DEMO_MODEL) {
    console.log("（SOLOMNI_DEMO_MODEL 对本脚本不适用：代理会话跑在核心默认模型上。）");
  }

  // 演示是**真机测试**：条件不齐就明说并退出（退出码 2），不用演示通道凑一遍。
  const block = proxyPreflight((await api("GET", "/api/state")).json);
  if (block) return refuseDemo(block);

  console.log("第一幕：把决定权整块交给核心（它自己挑人、建子工作、转达；子会话停下会叫醒它）");
  const created = await api("POST", "/api/sessions", { name: WORK, mode: "proxy" });
  ok(
    created.status === 200 && created.json && created.json.sid === WORK,
    "建代理会话（没有名单：选它就是授予全权）",
    created.text,
  );
  const sid = (created.json && created.json.sid) || WORK;

  const opened = JSON.stringify(await events(sid));
  ok(opened.includes("决定权整块交给核心"), "授权事实留在转录里（不只看 meta）", opened.slice(0, 200));

  console.log("（跟核心说一句目标；一轮要等模型跑完，可能要几分钟）");
  const said = await api("POST", "/api/sessions/" + enc(sid) + "/say", { text: GOAL });
  ok(said.status === 200, "跟核心说目标", said.text);

  // ---- 等它把活派下去、子会话跑完、它被叫醒、最后收敛 ----
  const startedAt = Date.now();
  let printedRows = 0;
  const printRows = async () => {
    const rows = toolRows(await lines(sid));
    for (const r of rows.slice(printedRows)) {
      console.log(
        "   核心工具 " + (r.label || r.name) + (r.ok ? " 成功" : " 失败") +
        " → " + String(r.output || "").split("\n")[0].slice(0, 120),
      );
    }
    printedRows = rows.length;
  };

  let lastSig = "";
  let quiet = 0;
  let converged = false;
  while (Date.now() - startedAt < DEADLINE_MS) {
    await sleep(POLL_MS);
    const st = await state();
    const kids = descendants(st, sid);
    const running = subtreeRunning(st, sid);
    const ls = await lines(sid);
    const notes = ls.filter((l) => String(l.line || "").includes("这一轮结束")).length;
    const msgs = ls.filter((l) => l.kind === "msg").length;
    const sig = kids.length + "/" + running.length + "/" + notes + "/" + msgs;
    await printRows();
    if (sig !== lastSig) {
      lastSig = sig;
      console.log(
        "   子工作 " + (kids.length ? kids.map((k) => k.name + "（" + k.mode + "）").join("、") : "（还没有）") +
        "；在跑 " + (running.length ? running.join("、") : "无") +
        "；叫醒通知 " + notes + " 条",
      );
    }
    if (running.length === 0 && (kids.length > 0 || msgs > 0)) {
      quiet++;
      if (quiet >= QUIET_POLLS) { converged = true; break; }
    } else {
      quiet = 0;
    }
  }
  if (converged) {
    console.log("   收敛：子树里已经没有在跑的生成（用时 " + Math.round((Date.now() - startedAt) / 1000) + "s）");
  } else {
    ok(false, "第一幕在时限内收敛", "超过 " + TIMEOUT_MIN + " 分钟");
  }

  // ---- 判据只认产品自己的记录与盘上的产物，不认模型的自述 ----
  const proxyLs = await lines(sid);
  const rows = toolRows(proxyLs);
  const names = rows.map((r) => r.name);
  // 代理这一回合的工具面（systools/roles.yaml 的 core_proxy）：六个代理工具 + 只读核实。
  const FACE = [
    "catalog_agents", "create_session", "send_session_message", "observe_session",
    "read_session_messages", "control_session", "read", "list", "search",
  ];
  ok(names.length > 0, "核心在代理会话里真的调了工具", names.join("、"));
  ok(names.every((n) => FACE.includes(n)), "只用它自己的工具面（模块工具一个也没发）", names.join("、"));
  ok(names.includes("catalog_agents"), "先核实清单再决定（catalog_agents）");
  ok(names.includes("create_session"), "自己建出子工作（create_session）");
  ok(names.includes("send_session_message"), "自己把活转达出去（send_session_message）");

  const st = await state();
  const kids = descendants(st, sid);
  ok(
    kids.length >= 1,
    "子工作挂在代理会话下（编排归属）",
    JSON.stringify(kids.map((k) => k.name + "<-" + (k.parent || ""))).slice(0, 300),
  );
  for (const k of kids) console.log("   子会话 " + k.name + "（mode=" + k.mode + "，父=" + (k.parent || "") + "）");

  const kidsLines = new Map();
  let worked = 0;
  for (const k of kids) {
    const kl = await lines(k.name);
    kidsLines.set(k.name, kl);
    const kt = toolRows(kl);
    if (kt.length || kl.some((l) => l.kind === "msg")) worked++;
    console.log("   " + k.name + "：工具 " + kt.length + " 次、发言 " + kl.filter((l) => l.kind === "msg").length + " 条");
    if (JSON.stringify(kl).includes("核心代理转达")) console.log("     ↳ 记录里有来源（核心代理转达，不冒充用户原话）");
  }
  ok(worked === kids.length && kids.length > 0, "每个子工作都真的跑过（有工具行或发言）", worked + "/" + kids.length);

  // 关卡不落到用户头上：代理会话自己不待裁，子会话的门由核心代答（没人等用户点头）。
  const nameSet = new Set([sid, ...kids.map((k) => k.name)]);
  const waiting = ((st && st.sessions) || []).filter((v) => v && nameSet.has(v.sid) && v.pending);
  ok(
    waiting.length === 0,
    "没有会话停在门上等用户（关卡由核心代答）",
    JSON.stringify(waiting.map((v) => v.sid + ":" + JSON.stringify(v.pending))).slice(0, 300),
  );

  const noteIdx = proxyLs.map((l, i) => i).filter((i) => String(proxyLs[i].line || "").includes("这一轮结束"));
  ok(
    noteIdx.length > 0,
    "子会话停下后核心收到通知（机制只写一条通知）",
    JSON.stringify(proxyLs.filter((l) => String(l.line || "").includes("这一轮结束")).map((l) => String(l.line).slice(0, 90))),
  );
  const woke = noteIdx.length > 0 && proxyLs.slice(noteIdx[noteIdx.length - 1] + 1).some((l) => l.kind === "msg");
  ok(woke, "被叫醒后又跑了一轮（不是靠用户再点一次）", proxyLs.slice(-3).map((l) => String(l.line || "").slice(0, 60)).join(" | "));
  ok(
    names.includes("read_session_messages") || names.includes("observe_session"),
    "核心主动倒查子会话（正文不自动送上门）",
    names.join("、"),
  );

  // 不转发转录：子会话的**工具输出**不在代理的发言里（要看只能经 read_session_messages 倒查）。
  const prose = proxyLs.filter((l) => l.kind !== "tool").map((l) => String(l.line || "")).join("\n");
  let probe = "";
  for (const [, kl] of kidsLines) {
    for (const r of toolRows(kl)) {
      const out = String(r.output || "");
      if (out.length >= 80 && out.length > probe.length) probe = out;
    }
  }
  if (probe) {
    ok(!prose.includes(probe.slice(0, 80)), "子会话的工具输出没有进代理的发言（不转发转录）", probe.slice(0, 80));
  } else {
    console.log("   （子会话没有 ≥80 字的工具输出，跳过「不转发」取样）");
  }

  // 产物：盘上真的多了东西（成品落在子会话自己的共享区/沙箱里）。
  const made = producedFiles(sid);
  console.log("   产物 " + made.size + " 件：");
  for (const [name, p] of made) console.log("     " + name + " → " + p + "（" + statSync(p).size + " 字节）");
  ok(made.size > 0, "子会话真的落下了产物文件");
  ok([...made.keys()].some((n) => /\.md$/i.test(n)), "产出了一份 Markdown 笔记");
  ok([...made.keys()].some((n) => /\.html?$/i.test(n)), "产出了一份 HTML 卡片");

  // ---- 第二幕：用户接管 = 按一次停止，整棵子树一起停 ----
  console.log("第二幕：用户按下停止 = 整棵子树一起停");
  const created2 = await api("POST", "/api/sessions", { name: STOP_WORK, mode: "proxy" });
  ok(created2.status === 200, "建第二幕的代理会话", created2.text);
  const sid2 = (created2.json && created2.json.sid) || STOP_WORK;
  const said2 = await api("POST", "/api/sessions/" + enc(sid2) + "/say", { text: STOP_GOAL });
  ok(said2.status === 200, "跟它说一件要花时间的事", said2.text);

  // 等它把活派给某个人、那个人真的在跑——这时候按停止才有级联可言。
  let caught = [];
  const until = Date.now() + 90_000;
  while (Date.now() < until && !caught.length) {
    await sleep(400);
    caught = subtreeRunning(await state(), sid2);
  }
  const kids2 = descendants(await state(), sid2);
  console.log(
    "   停止前：子会话 " + (kids2.length ? kids2.map((k) => k.name).join("、") : "（没有）") +
    "；在跑 " + (caught.length ? caught.join("、") : "无"),
  );

  const stopped = await api("POST", "/api/sessions/" + enc(sid2) + "/stop", {});
  ok(stopped.status === 200, "点停止", stopped.text);
  await sleep(1500);
  const still = subtreeRunning(await state(), sid2);
  ok(still.length === 0, "停止即全停：整棵子树都不再跑", still.join("、"));
  if (!caught.length) {
    console.log("   （停止那一刻子树里没有在跑的生成——多半已经跑完；这条只验了停止后的最终态。）");
  }

  console.log(failed ? "DEMO-FAILED failed=" + failed : "DEMO-OK（代理工作 " + WORK + "）");
  return failed ? 1 : 0;
}

main()
  .then((code) => process.exit(code))
  .catch((e) => {
    console.log("FAIL 驱动异常 :: " + (e && e.message ? e.message : e));
    process.exit(1);
  });
