#!/usr/bin/env node
/**
 * 虚拟机档演示（L4 演示脚本，**在真机上跑**）：单 agent 跑在 **guest 里**（不是宿主）——
 * 把共享区的资料抽成语料，产物经**实时共享**直接落进宿主的会话目录，没有"执行完再回拷"这一步。
 *
 * 与其它演示的区别（见 docs/vm/README.md 与 RUNTIME_SPEC.md §五）：
 *   demo/run-demo.mjs        组合式：一个 agent 装三个模块，本机档；
 *   demo/run-demo-collab.mjs 小组协作；
 *   demo/run-demo-mcp.mjs    MCP 常驻服务；
 *   demo/run-demo-proxy.mjs  代理形态；
 *   本脚本                    虚拟机档：模块命令送进 guest，guest 自带的 python 满足选型预检。
 *
 * 用法：
 *   1) 先起产品：node start.js -webUI            （默认网页端口 3081）
 *   2) 另开一个终端：node demo/run-demo-vm.mjs
 * 环境变量：
 *   SOLOMNI_DEMO_BASE      转录中心地址（默认 http://127.0.0.1:3081）
 *   SOLOMNI_DEMO_MODEL     指定模型 id（默认不指定 = 用核心默认模型）
 *   SOLOMNI_DEMO_VM_BASE   虚拟机档基础根（默认 .vm/guest/base）
 *
 * 只走产品自己的 HTTP 能力面（与前端同一条路），不改任何登记处；任何一步不成立都如实打印并以非零退出码收场。
 */
import { readFileSync, existsSync, readdirSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { request } from "node:http";
import { vmPreflight, refuseDemo } from "./preflight.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const BASE = process.env.SOLOMNI_DEMO_BASE || "http://127.0.0.1:3081";
const MODEL = process.env.SOLOMNI_DEMO_MODEL || "";
const VM_BASE = process.env.SOLOMNI_DEMO_VM_BASE || ".vm/guest/base";
const WORK = "demo-vm-" + Date.now();
const SAMPLE = join(HERE, "sample-corpus");
const OVERLAY_DIR = join(process.cwd(), ".vm", "overlays");
const URL_BASE = new URL(BASE);

let failed = 0;
const ok = (cond, label, extra) => {
  if (!cond) failed++;
  console.log((cond ? "PASS " : "FAIL ") + label + (cond || extra === undefined ? "" : " :: " + String(extra).slice(0, 400)));
};

/* 动作只有一个入口 /api/actions/{id}：声明与校验都在核心。 */
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

const toolRows = (ls) => ls.filter((l) => l.tool).map((l) => l.tool);

/** 会话目录里找产物（会话自己的元数据不算产物）。 */
function artifacts(work) {
  const root = join(process.cwd(), "session", work);
  const found = new Map();
  const walk = (dir) => {
    for (const ent of readdirSync(dir, { withFileTypes: true })) {
      const p = join(dir, ent.name);
      if (ent.isDirectory()) { if (ent.name !== ".work") walk(p); }
      else if (ent.name !== "meta.yaml" && ent.name !== "transcript.jsonl" && !found.has(ent.name)) found.set(ent.name, p);
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
    console.error("转录中心没起来：" + BASE + "（先跑 node start.js -webUI）");
    return 1;
  }
  // 演示是**真机测试**：条件不齐就明说并退出（退出码 2），不用演示通道凑一遍。
  const pre = await api("GET", "/api/state");
  const block = vmPreflight(pre.json, MODEL, VM_BASE);
  if (block) return refuseDemo(block);

  // ① 建虚拟机档工作：单 agent、harvest（python）。核心在创建那一刻做静态预检——
  //    机器能力 × base × guest 自带运行能力 × 模块 × 包库，缺任何一项就拒绝、不拉 guest。
  const agent = { name: "资料手", modules: ["harvest"] };
  if (MODEL) agent.model = MODEL;
  const created = await api("POST", actUrl("create_session"), {
    name: WORK, mode: "single", tier: "vm", base: VM_BASE, agents: [agent],
  });
  if (created.status !== 200) {
    ok(false, "建虚拟机档工作 " + WORK, created.text);
    return failed ? 1 : 0;
  }
  ok(true, "建虚拟机档工作 " + WORK);
  const sid = (created.json && created.json.sid) || WORK;

  // ② 会话配置必须如实认成虚拟机档，且选型预检通过（guest 自带的 python 不算缺）。
  const cfg = await api("GET", "/api/sessions/" + encodeURIComponent(sid) + "/config");
  const c = (cfg.json && cfg.json.config) || {};
  ok(c.tier === "vm", "会话记的是虚拟机档（tier=vm）", JSON.stringify(c.tier));
  ok(
    !(c.runtime && c.runtime.missing && c.runtime.missing.harvest),
    "选型预检通过：harvest 的 python 由 guest 自带（不报缺包）",
    JSON.stringify((c.runtime && c.runtime.missing) || {}),
  );
  // 每租约可写层：会话活着时它在场（基础镜像只读，guest 的写落在这层上）。
  const overlay = join(OVERLAY_DIR, sid + ".qcow2");
  ok(existsSync(overlay), "每租约可写层已建（基础镜像只读）", overlay);

  // ③ 投喂示例语料（走产品的上传能力面，落进本次工作的共享区）。
  const files = readdirSync(SAMPLE).sort();
  ok(files.length > 0, "示例语料非空", files.join(" "));
  for (const name of files) {
    const up = await api("POST", actUrl("upload"), {
      session_id: sid,
      name,
      data_base64: readFileSync(join(SAMPLE, name)).toString("base64"),
    });
    ok(up.status === 200, "投喂 " + name, up.text);
  }

  // ④ 一句需求：拉进沙箱、抽成语料、提交回共享区。缺产物就如实点名再问一轮（最多两轮），不替它编结果。
  console.log("（一轮问询要等模型跑完，可能要几分钟）");
  const ask = await api("POST", actUrl("send_message"), {
    session_id: sid,
    text: "共享区对你只读：先用 work_pull 把资料拉进你的沙箱，再用 harvest 的 scan 把语料抽成 corpus.jsonl，"
      + "最后用 work_commit 把 corpus.jsonl 提交回共享区；做完把它的路径念给我。",
  });
  ok(ask.status === 200, "发出需求", ask.text);
  for (let round = 0; round < 2 && !artifacts(sid).has("corpus.jsonl"); round++) {
    const nudge = await api("POST", actUrl("send_message"), {
      session_id: sid,
      text: "还差 corpus.jsonl：接着把没做完的那一步做完，做完把路径念给我。",
    });
    ok(nudge.status === 200, "补齐需求（第 " + (round + 1) + " 轮）", nudge.text);
  }

  const rows = toolRows(await lines(sid));
  for (const r of rows) {
    console.log("   工具 " + (r.label || r.name) + (r.ok ? " 成功" : " 失败") + " → " + String(r.output || "").split("\n")[0].slice(0, 120));
  }
  ok(rows.some((r) => /scan/.test(r.name) && r.ok), "harvest 在 guest 里真跑了（scan 成功）", rows.map((r) => r.name).join("、"));
  ok(rows.some((r) => r.name === "work_commit" && r.ok), "产物经 work_commit 提交回共享区", rows.map((r) => r.name).join("、"));

  // ⑤ 复核产物：不靠模型自述——盘上真的多了 corpus.jsonl（agent 沙箱是宿主目录，guest 的直接写落在这上面）。
  const made = artifacts(sid);
  const corpus = made.get("corpus.jsonl");
  ok(!!corpus, "语料产物真的存在（实时共享，没有回拷这一步）", [...made.keys()].join("、"));
  if (corpus) {
    console.log("   产物 corpus.jsonl → " + corpus + "（" + statSync(corpus).size + " 字节）");
    ok(statSync(corpus).size > 0, "产物非空");
  }
  const shared = join(process.cwd(), "session", sid, "work", "corpus.jsonl");
  ok(existsSync(shared), "提交后共享区里有它（work/）", shared);

  // ⑥ 删会话 = 销毁 guest 与它的可写层（租约生命周期）。
  const del = await api("POST", "/api/history/" + encodeURIComponent(sid) + "/delete", {});
  ok(del.status === 200, "删除会话", del.text);
  await new Promise((r) => setTimeout(r, 1000));
  ok(!existsSync(join(process.cwd(), "session", sid)), "删除会话后目录清掉");
  ok(!existsSync(overlay), "删除会话后每租约可写层一起删掉", overlay);

  console.log(failed ? "DEMO-FAILED failed=" + failed : "DEMO-OK（虚拟机档工作 " + WORK + "）");
  return failed ? 1 : 0;
}

process.exit(await main());