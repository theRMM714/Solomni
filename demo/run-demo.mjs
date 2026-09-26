#!/usr/bin/env node
// 演示驱动：对一个**已经跑起来**的 Solomni 转录中心执行一次完整演示——
// 投喂 demo/sample-corpus/ 里的资料 → 让它整理成报告与离线检索包 → 复核产物、再验一次检索与近似重复。
//
// 用法：
//   1) 先起产品：node start.js -webUI            （默认网页端口 3081）
//   2) 另开一个终端：node demo/run-demo.mjs
// 环境变量：
//   SOLOMNI_DEMO_BASE   转录中心地址（默认 http://127.0.0.1:3081）
//   SOLOMNI_DEMO_MODEL  指定模型 id（默认不指定 = 用核心默认模型）
//
// 只走产品自己的 HTTP 能力面（与前端同一条路），不改任何登记处；产物落在本次工作的共享区里。
// 一次问询要等模型跑完才返回（可能好几分钟），所以客户端**不设超时**；任何一步不成立都如实打印失败原因
// 并以非零退出码收场（演示失败不该被当成成功）。
import { readFileSync, existsSync, readdirSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { request } from "node:http";

const HERE = dirname(fileURLToPath(import.meta.url));
const BASE = process.env.SOLOMNI_DEMO_BASE || "http://127.0.0.1:3081";
const MODEL = process.env.SOLOMNI_DEMO_MODEL || "";
const WORK = "demo-" + Date.now();
const SAMPLE = join(HERE, "sample-corpus");
const URL_BASE = new URL(BASE);

let failed = 0;
const ok = (cond, label, extra) => {
  if (!cond) failed++;
  console.log((cond ? "PASS " : "FAIL ") + label + (cond || extra === undefined ? "" : " :: " + String(extra).slice(0, 400)));
};

/** 一次能力面调用：不设客户端超时（模型一轮可能跑几分钟），等它自己返回。 */
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

/** 等转录中心起来（最多 30 秒）。 */
async function waitReady() {
  for (let i = 0; i < 60; i++) {
    try { const s = await api("GET", "/api/state"); if (s.status === 200) return true; } catch {}
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

/** 说一句话：这一轮跑完（模型不再调用工具）才会返回，所以这里只等，不轮询。 */
async function say(sid, text) {
  const r = await api("POST", "/api/sessions/" + encodeURIComponent(sid) + "/say", { text });
  return r;
}

const toolRows = (ls) => ls.filter((l) => l.tool).map((l) => l.tool);

async function main() {
  if (!existsSync(SAMPLE)) {
    console.error("找不到示例语料：" + SAMPLE);
    return 1;
  }
  if (!(await waitReady())) {
    console.error("转录中心没起来：" + BASE + "（先跑 solomni -webUI）");
    return 1;
  }

  // ① 建工作：一个 agent、三个模块（抽取 / 呈现 / 检索各一个模块），模型用核心默认（也可用环境变量指定）。
  const agent = { name: "资料手", modules: ["harvest", "render", "indexer"] };
  if (MODEL) agent.model = MODEL;
  const created = await api("POST", "/api/sessions", { name: WORK, mode: "single", agents: [agent] });
  ok(created.status === 200, "建工作 " + WORK, created.text);
  const sid = (created.json && created.json.sid) || WORK;

  // ② 投喂示例语料（走产品的上传能力面，落进本次工作的共享区）。
  const files = readdirSync(SAMPLE).sort();
  ok(files.length > 0, "示例语料非空", files.join(" "));
  for (const name of files) {
    const up = await api("POST", "/api/sessions/" + encodeURIComponent(sid) + "/upload", {
      name,
      data_base64: readFileSync(join(SAMPLE, name)).toString("base64"),
    });
    ok(up.status === 200, "投喂 " + name, up.text);
  }

  // ③ 一句需求：把资料变成报告与检索包（用哪个工具由它自己按模块说明决定）。
  console.log("（一轮问询要等模型跑完，可能要几分钟）");
  const ask = await say(sid,
    "把共享区里的资料整理成一份能给同事看的报告，并做一个能离线检索的索引包。\n" +
    "用你手上的模块工具做完这三步：先抽语料，再出 HTML 报告，最后建索引；" +
    "做完把三个产物（语料、报告、索引）的真实绝对路径念给我。");
  ok(ask.status === 200, "发出需求", ask.text);

  const rows = toolRows(await lines(sid));
  ok(rows.length >= 3, "至少三次工具调用（抽语料 / 出报告 / 建索引）", rows.map((r) => r.name).join("、"));
  for (const r of rows) {
    console.log("   工具 " + (r.label || r.name) + (r.ok ? " 成功" : " 失败") + " → " + String(r.output || "").split("\n")[0].slice(0, 120));
  }

  // ④ 复核产物：三件都必须真的存在（不靠模型的自我陈述）。
  // 成品可以落在本工作的共享区，也可以落在某个 agent 的私有沙箱，所以每次递归找一遍。
  const root = join(process.cwd(), "session", WORK);
  const artifacts = () => {
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
  };
  const want = ["corpus.jsonl", "report.html", "index.bin"];
  // 一次问询里的工具调用有上限（超了会被要求收尾）；缺了就**如实点名**再问一轮，最多两轮，不替它编结果。
  for (let round = 0; round < 3; round++) {
    const miss = want.filter((f) => !artifacts().has(f));
    if (!miss.length) break;
    if (round === 2) {
      ok(false, "产物齐全", "缺：" + miss.join("、"));
      break;
    }
    const nudge = await say(sid, "产物还差：" + miss.join("、") + "。请把没做完的那一步接着做完，做完把路径念给我。");
    ok(nudge.status === 200, "补齐需求（第 " + (round + 1) + " 轮）", nudge.text);
    for (const r of toolRows(await lines(sid)).slice(-6)) {
      console.log("   工具 " + (r.label || r.name) + (r.ok ? " 成功" : " 失败") + " → " + String(r.output || "").split("\n")[0].slice(0, 120));
    }
  }
  const made = artifacts();
  if (want.every((f) => made.has(f))) {
    ok(true, "产物齐全（" + want.map((f) => f + "=" + statSync(made.get(f)).size + " 字节").join(" ") + "）");
  }
  for (const f of want) {
    console.log("   产物 " + f + " → " + (made.get(f) || "（没有）"));
  }

  // ⑤ 再问一次：检索 + 近似重复（这两件是只读工具，声明了可并发，核心会真并发跑）。
  const ask2 = await say(sid, "用 indexer 查一下「本地优先」，再找出语料里近似重复的文档；两条结果都贴回来。");
  ok(ask2.status === 200, "发出第二次需求（检索 + 近似重复）", ask2.text);
  const rows2 = toolRows(await lines(sid));
  const q = rows2.filter((r) => /query|dups/.test(r.name));
  ok(q.length >= 1, "检索或近似重复真的跑了", rows2.map((r) => r.name).join("、"));
  for (const r of q) {
    console.log("   " + r.name + " → " + String(r.output || "").split("\n").slice(0, 3).join(" | ").slice(0, 300));
  }

  console.log(failed ? "DEMO-FAILED failed=" + failed : "DEMO-OK（工作 " + WORK + "）");
  return failed ? 1 : 0;
}

process.exit(await main());
