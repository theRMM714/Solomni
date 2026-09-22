#!/usr/bin/env node
/**
 * 协作演示（L4 演示脚本，**在真机上跑**）：三个 agent 各持一个模块，走完整协作六阶段，
 * 把共享区里的资料变成"语料 + 报告 + 离线索引"三件产物。
 *
 * 与 demo/run-demo.mjs 的区别：那个是**组合式**（一个 agent 装三个模块），这个走**小组协作**
 * （N 个 agent 分权协商：讨论 → 整理出任务链 → **审查关卡** → 链驱动（子会话）→ 节点验收 → 总验收）。
 * 三个模块仍是三种语言：harvest=python、render=node、indexer=C++。
 * 细则见 docs/architecture/task-chain.md。
 *
 * 用法：
 *   1) 先起产品：node start.js -webUI            （默认网页端口 3081）
 *   2) 另开一个终端：node demo/run-demo-collab.mjs
 * 环境变量：
 *   SOLOMNI_DEMO_BASE   转录中心地址（默认 http://127.0.0.1:3081）
 *   SOLOMNI_DEMO_MODEL  指定模型 id（默认不指定 = 用核心默认模型）
 *
 * 为什么这个脚本**不进 CI**：它要真实供应商（CI 上没有 .home/，会回落到内置假模型，
 * 那样验的就不是协作能力而是假脚本）。协作状态机的机器判据在 tests/cross-platform/e2e/。
 * 只走产品自己的 HTTP 能力面（与前端同一条路）；产物落在本次工作的共享区里。
 */
import { readFileSync, existsSync, readdirSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { request } from "node:http";

const HERE = dirname(fileURLToPath(import.meta.url));
const BASE = process.env.SOLOMNI_DEMO_BASE || "http://127.0.0.1:3081";
const MODEL = process.env.SOLOMNI_DEMO_MODEL || "";
const WORK = "demo-collab-" + Date.now();
const SAMPLE = join(HERE, "sample-corpus");
const URL_BASE = new URL(BASE);

let failed = 0;
const ok = (cond, label, extra) => {
  if (!cond) failed++;
  console.log((cond ? "PASS " : "FAIL ") + label + (cond || extra === undefined ? "" : " :: " + String(extra).slice(0, 400)));
};

/** 一次能力面调用：不设客户端超时（一轮可能跑几分钟），等它自己返回。 */
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

async function waitReady() {
  for (let i = 0; i < 60; i++) {
    try { const s = await api("GET", "/api/state"); if (s.status === 200) return true; } catch {}
    await new Promise((r) => setTimeout(r, 500));
  }
  return false;
}

/** 该工作的全部事件（转录行 + 通知 + 交付）。 */
async function events(sid) {
  const r = await api("GET", "/api/history/" + encodeURIComponent(sid));
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

/** 本工作 + 它**子会话**的全部转录行：节点跑在自己的子会话里，工具调用落在那边。 */
async function allLines(sid) {
  const out = await lines(sid);
  const st = await api("GET", "/api/state");
  const kids = ((st.json && st.json.history) || []).filter((h) => h.parent === sid);
  for (const k of kids) out.push(...(await lines(k.name)));
  return out;
}

/** 本工作共享区/沙箱里找产物（成品可以落在共享区，也可以落在某个 agent 的私有沙箱）。 */
function artifacts(work) {
  const root = join(process.cwd(), "session", work);
  const found = new Map();
  const walk = (dir) => {
    for (const ent of readdirSync(dir, { withFileTypes: true })) {
      const p = join(dir, ent.name);
      if (ent.isDirectory()) walk(p);
      else if (!found.has(ent.name)) found.set(ent.name, p);
    }
  };
  if (existsSync(root)) walk(root);
  return found;
}

async function main() {
  if (!existsSync(SAMPLE)) {
    console.error("找不到示例语料：" + SAMPLE);
    return 1;
  }
  if (!(await waitReady())) {
    console.error("转录中心没起来：" + BASE + "（先跑 solomni -webUI）");
    return 1;
  }

  // ① 建协作工作：三个 agent 各持一个模块（三种语言），需求一句话。
  const task = "把共享区里的资料变成一份能给同事看的报告，再做一个能离线检索的索引包。"
    + "分工：先把资料抽成语料，再出 HTML 报告，最后建索引并当场检索一次证明可用；"
    + "做完把三件产物（语料、报告、索引）的真实绝对路径念给我。";
  const agents = [
    { name: "资料手", modules: ["harvest"] },
    { name: "呈现手", modules: ["render"] },
    { name: "检索手", modules: ["indexer"] },
  ];
  if (MODEL) for (const a of agents) a.model = MODEL;
  const created = await api("POST", "/api/sessions", { name: WORK, mode: "collab", agents, task });
  ok(created.status === 200, "建协作工作 " + WORK + "（3 个 agent / 3 个模块 / 3 种语言）", created.text);
  const sid = (created.json && created.json.sid) || WORK;

  // ② 投喂示例语料。
  const files = readdirSync(SAMPLE).sort();
  ok(files.length > 0, "示例语料非空", files.join(" "));
  for (const name of files) {
    const up = await api("POST", "/api/sessions/" + encodeURIComponent(sid) + "/upload", {
      name,
      data_base64: readFileSync(join(SAMPLE, name)).toString("base64"),
    });
    ok(up.status === 200, "投喂 " + name, up.text);
  }

  // ③ 开始讨论（不带 allow：需要用户裁决时如实停下来问你，而不是替你决定）。
  console.log("（协作一轮要等模型跑完，可能要几分钟）");
  const begin = await api("POST", "/api/sessions/" + encodeURIComponent(sid) + "/begin", { text: "yes" });
  ok(begin.status === 200, "开始讨论", begin.text);

  // ④ 轮询：协作是拉模式——有 pending 就回答，没 pending 就继续推进，直到交付。
  // 每一步都如实打印发生了什么（讨论轮次 / 方案 / 回报 / 验收 / 返工 / 交付）。
  let delivered = false;
  for (let step = 0; step < 40; step++) {
    const p = await api("POST", "/api/sessions/" + encodeURIComponent(sid) + "/pending", {});
    const pending = p.json && p.json.pending;
    if (pending && pending.type === "ask") {
      console.log("   [待裁决] " + String(pending.member || "") + " 提问：" + String(pending.question || "").slice(0, 200));
      const ans = await api("POST", "/api/sessions/" + encodeURIComponent(sid) + "/answer", { text: "按你的判断做" });
      ok(ans.status === 200, "回答 agent 的提问", ans.text);
      continue;
    }
    // **审查关卡**：整理完不自动开工——方案与任务链先给用户看，点「同意」才推进。
    if (pending && pending.type === "plan_review") {
      console.log("   [审查关卡] 方案与任务链已备好，点「同意」开工");
      const a = await api("POST", "/api/sessions/" + encodeURIComponent(sid) + "/approve-plan", {});
      ok(a.status === 200, "审查关卡：同意方案开工", a.text);
      continue;
    }
    // **节点验收没过**：如实报告是哪几个节点，然后点「继续」重派它们。
    if (pending && pending.type === "node_blocked") {
      console.log("   [节点验收] 没通过：" + JSON.stringify(pending.nodes || []));
      const c = await api("POST", "/api/sessions/" + encodeURIComponent(sid) + "/continue", {});
      ok(c.status === 200, "重派没通过的节点", c.text);
      continue;
    }
    const ev = await events(sid);
    if (ev.some((e) => e.type === "ended")) { delivered = true; break; }
    const c = await api("POST", "/api/sessions/" + encodeURIComponent(sid) + "/continue", {});
    if (c.status !== 200) { ok(false, "推进协作", c.text); break; }
  }

  // ⑤ 复核：不靠模型自述——阶段事件、产物、报告的自包含性都自己查一遍。
  const ev = await events(sid);
  const kinds = ev.map((e) => e.type);
  ok(kinds.includes("discussion_done"), "讨论收敛（discussion_done）", kinds.join(","));
  ok(kinds.includes("plan"), "整理出方案（plan）", kinds.join(","));
  ok(kinds.includes("plan_review"), "审查关卡：整理完停在待审（plan_review）", kinds.join(","));
  ok(kinds.includes("node_started"), "就绪节点各起了子会话（node_started）", kinds.join(","));
  ok(kinds.includes("report"), "各 agent 交了回报（report）", kinds.join(","));
  ok(kinds.includes("review"), "核心逐项验收（review）", kinds.join(","));
  const delivery = ev.filter((e) => e.type === "delivery").pop();
  ok(!!delivery, "交付（delivery）", JSON.stringify(delivery || {}));
  if (delivery) console.log("   交付结论：ok=" + delivery.ok + " over_rework=" + delivery.over_rework);

  const made = artifacts(WORK);
  const want = ["corpus.jsonl", "report.html", "index.bin"];
  for (const f of want) {
    const p = made.get(f);
    console.log("   产物 " + f + " → " + (p ? p + "（" + statSync(p).size + " 字节）" : "（没有）"));
  }
  ok(want.every((f) => made.has(f)), "三件产物都真的存在", want.filter((f) => !made.has(f)).join("、") || "");

  // 报告必须自包含：离线打开不许引用任何外部资源。
  if (made.has("report.html")) {
    const html = readFileSync(made.get("report.html"), "utf8");
    const external = /(?:src|href)\s*=\s*["']https?:/i.test(html) || /<script[^>]+src=/i.test(html);
    ok(!external, "报告自包含（无外部 src/href、无外部 script）", html.slice(0, 200));
  }

  // 检索真的可用：至少有一次成功的 query，且结果里有命中。
  // 子会话真的建出来了：侧栏据此把子会话缩进挂在父会话下（history.parent）。
  const st = await api("GET", "/api/state");
  const kids = ((st.json && st.json.history) || []).filter((h) => h.parent === sid);
  ok(kids.length > 0, "子会话挂在父会话下（history.parent）", JSON.stringify(kids.map((k) => k.name)));

  const rows = toolRows(await allLines(sid));
  console.log("   工具调用：" + (rows.map((r) => (r.label || r.name) + (r.ok ? "✓" : "✗")).join("、") || "（没有）"));
  const q = rows.filter((r) => /query/.test(r.name) && r.ok).pop();
  ok(!!q, "索引建好后当场检索过", JSON.stringify(rows.map((r) => [r.name, r.ok])));

  console.log(failed ? "DEMO-FAILED failed=" + failed : "DEMO-OK（工作 " + WORK + "，三 agent 协作）");
  return failed ? 1 : 0;
}

process.exit(await main());
