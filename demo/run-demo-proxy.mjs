#!/usr/bin/env node
/**
 * 代理模式演示（L4 演示脚本，**在真机上跑**）：用户把决定权**整块**交给核心——
 * 他只跟核心说一句目标，核心自己挑人、建子工作、把活转达出去；子会话停下会自动叫醒核心，
 * 核心再主动倒查正文、接着安排。
 *
 * 本脚本跑三种**会话拼装**（每一幕从零建一个代理会话，再由核心建一个子会话）：
 *   1) 只执行：临时拼装一条 execute 流程（planner 拟计划 → 审批 → 派节点 → 验收 → 交付）；
 *   2) 只讨论：临时拼装一条 discuss 流程（≥2 位 discussant 收敛 → 核心写总结，不派活）；
 *   3) 会议：会话类型表里的「会议」(collab) = 讨论 + 执行两段串起来（总结就是执行段的需求）。
 * 另外（默认不跑，`SOLOMNI_DEMO_STOP=1` 打开）：用户按「停止」整棵子树停住、按「继续」解冻。
 *
 * 与另外两个演示的区别（见 PRODUCT.md「代理（proxy）」）：
 *   demo/run-demo.mjs         组合式：一个 agent 装三个模块，用户直接给它活；
 *   demo/run-demo-collab.mjs  小组协作：三个 agent 各持一份能力协商，用户当裁判；
 *   本脚本                     代理：没有名单、也没有裁判——核心代用户决定（全权）。
 *
 * 用法：
 *   1) 先起产品：node start.js --web            （默认网页端口 3081）
 *   2) 另开一个终端：node demo/run-demo-proxy.mjs
 * 环境变量：
 *   SOLOMNI_DEMO_BASE         转录中心地址（默认 http://127.0.0.1:3081）
 *   SOLOMNI_DEMO_TIMEOUT_MIN  每一幕的收敛时限（分钟，默认 15）
 *   SOLOMNI_DEMO_ACTS         只跑哪几幕（逗号分隔：execute,discuss,meeting；默认三幕都跑）
 *   SOLOMNI_DEMO_STOP         设为 1 时加跑「停止 / 继续」那一幕（默认不跑，省时间）
 *   注意：`SOLOMNI_DEMO_MODEL` **对本脚本无效**——代理会话跑在核心默认模型上，
 *   它临时挑出来的子会话用什么模型也由核心决定（另两个演示用它给 agent 指定模型）。
 *
 * 为什么这个脚本**不进 CI**：它要真实供应商与核心默认模型（CI 上没有 .home/，会回落到内置假模型，
 * 那样验的就不是代理能力而是假脚本）。流程状态机的机器判据在 tests/cross-platform/e2e/driver.js。
 * 只走产品自己的 HTTP 能力面（与前端同一条路），**不改任何登记处**。
 */
import { existsSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { request } from "node:http";
import { proxyPreflight, refuseDemo } from "./preflight.mjs";

const BASE = process.env.SOLOMNI_DEMO_BASE || "http://127.0.0.1:3081";
const URL_BASE = new URL(BASE);
const TIMEOUT_MIN = Number(process.env.SOLOMNI_DEMO_TIMEOUT_MIN || 15);
const DEADLINE_MS = (Number.isFinite(TIMEOUT_MIN) && TIMEOUT_MIN > 0 ? TIMEOUT_MIN : 15) * 60 * 1000;
const POLL_MS = 2000;
// 叫醒是**异步**的：子会话停下 → 写一条通知 → 起一个代理回合。连续几轮都安静才算收敛，
// 否则会把"刚要醒"当成"已经结束"。
const QUIET_POLLS = 5;

/** 只执行那一幕的说法：明确只建一条执行流程，免得核心顺手把讨论也带上。 */
const EXECUTE_GOAL = [
  "这件事只建一个**只有执行流程**的子会话，不要先讨论；人手一个就够，怎么派活由你和它定。",
  "用 create_session 这样建（省略 kind，用 flows 临时拼装）：",
  "flows = [\"execute\"]，agents 只给一个零模块的人（名字你定）。",
  "task 用下面这段需求：",
  "—— 让执行席在自己沙箱里写一份 notes.md（Markdown，正文不少于 200 字，说清「改配置前为什么要先备份」），写完用 submit_report 把结果回报出来。",
  "它停在方案审查那一关时你代答放行（选项 id 见卡片）。交付后告诉我：子会话名、产物路径、它的回报。",
].join("\n");

/** 只讨论那一幕的说法：只要讨论与总结，不拟计划、不派活。 */
const DISCUSS_GOAL = [
  "只建一个**只有讨论流程**的子会话：不拟计划、不派活、不建任务链。",
  "用 create_session 建（省略 kind，用 flows）：",
  "flows = [\"discuss\"]，agents 给两个零模块的人（名字你定）。",
  "task 用：",
  "「讨论：一个本地优先的笔记工具，最该先做哪三个功能？各自说理由，收敛出三条结论。」",
  "讨论收敛后核心会写一份总结，流程到此为止。它停下就告诉我：子会话名、总结里的那三条是什么。",
].join("\n");

/** 会议那一幕：直接点会话类型表里的 collab，让讨论与执行两段串起来。 */
const MEETING_GOAL = [
  "用会话类型表里的「会议」（kind 给 collab）建一个子会话，让它先讨论、再执行；agents 给两个零模块的人（名字你定）。",
  "task 用：",
  "「讨论出给团队做一次 30 分钟分享的三个要点，然后让执行席在沙箱里写一份 plan.md（三节，每节一句话说明），最后交付。」",
  "方案审查那一关你代答放行。交付后告诉我：子会话名、产物路径。",
].join("\n");

/** 停止 / 继续那一幕：要一件**要花点时间**的活，好在中途按停止。 */
const STOP_GOAL =
  "让合适的人认真写一份 800 字以上的长文《把决定权交给 AI 的边界》，写进他自己的沙箱里的 long.md；" +
  "写细一点、慢慢来。人你自己挑。";

/** 三幕：各自的目标、要建成的类型、该有与不该有的事件、该落下的产物。 */
const ACTS = [
  {
    id: "execute",
    title: "第一幕：只执行（临时拼装 execute：拟计划 → 审批 → 派节点 → 验收 → 交付）",
    goal: EXECUTE_GOAL,
    kind: "custom",
    terminal: ["delivery"],
    expect: ["plan", "plan_review", "node_started", "report", "delivery"],
    forbid: ["discussion_done", "summary"],
    artifact: /^notes\.md$/i,
  },
  {
    id: "discuss",
    title: "第二幕：只讨论（临时拼装 discuss：≥2 位 discussant 收敛 → 核心写总结）",
    goal: DISCUSS_GOAL,
    kind: "custom",
    terminal: ["discussion_done", "summary"],
    expect: ["discussion_done", "summary"],
    forbid: ["plan", "node_started", "delivery"],
    artifact: null,
  },
  {
    id: "meeting",
    title: "第三幕：会议（类型表里的 collab：讨论 → 总结 → 执行 → 交付）",
    goal: MEETING_GOAL,
    kind: "collab",
    terminal: ["delivery"],
    expect: ["discussion_done", "summary", "plan", "plan_review", "node_started", "report", "delivery"],
    forbid: [],
    artifact: /^plan\.md$/i,
  },
];

let failed = 0;
const ok = (cond, label, extra) => {
  if (!cond) failed++;
  console.log((cond ? "PASS " : "FAIL ") + label + (cond || extra === undefined ? "" : " :: " + String(extra).slice(0, 400)));
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const enc = encodeURIComponent;

/** 一次能力面调用：不设客户端超时（一轮可能跑几分钟），等它自己返回。
    长请求被网络层重置（ECONNRESET）是常态——**重试**，而不是让整个演示崩掉。 */
/* 动作只有一个入口 /api/actions/{id}：声明与校验都在核心。 */
const actUrl = (id) => "/api/actions/" + id;
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

/** 这棵子树在盘上落下的**产物文件**（会话自己的元数据不算产物）。 */
function producedFiles(work) {
  const base = join(process.cwd(), "session");
  const found = new Map();
  const walk = (dir) => {
    for (const ent of readdirSync(dir, { withFileTypes: true })) {
      const p = join(dir, ent.name);
      if (ent.isDirectory()) { if (ent.name !== ".work") walk(p); }
      else if (ent.name !== "meta.yaml" && ent.name !== "transcript.jsonl" && !found.has(ent.name)) found.set(ent.name, p);
    }
  };
  if (existsSync(join(base, work))) walk(join(base, work));
  return found;
}

/** 走一场戏：建代理会话 → 说目标 → 等子树安静 → 断言这一段的流程真的跑完。 */
async function runAct(act) {
  const work = "demo-proxy-" + act.id + "-" + Date.now();
  console.log("\n" + act.title);

  const created = await api("POST", actUrl("create_session"), { name: work, kind: "proxy" });
  ok(
    created.status === 200 && created.json && created.json.sid === work,
    "建代理会话（没有名单：选它就是授予全权）",
    created.text,
  );
  const sid = (created.json && created.json.sid) || work;
  const opened = JSON.stringify(await events(sid));
  ok(opened.includes("决定权整块交给核心"), "授权事实留在转录里（不只看 meta）", opened.slice(0, 200));

  console.log("（跟核心说一句目标；一轮要等模型跑完，可能要几分钟）");
  const said = await api("POST", actUrl("send_message"), { session_id: sid, text: act.goal });
  ok(said.status === 200, "跟核心说目标", said.text);

  // ---- 等核心建出子会话、把它跑到收敛（有终点事件 + 整棵子树安静） ----
  const startedAt = Date.now();
  let printedRows = 0;
  let printedKids = 0;
  let quiet = 0;
  let converged = false;
  let kid = null;
  let types = [];
  while (Date.now() - startedAt < DEADLINE_MS) {
    await sleep(POLL_MS);
    const st = await state();
    const rows = toolRows(await lines(sid));
    for (const r of rows.slice(printedRows)) {
      console.log(
        "   核心工具 " + (r.label || r.name) + (r.ok ? " 成功" : " 失败") +
        " → " + String(r.output || "").split("\n")[0].slice(0, 120),
      );
    }
    printedRows = rows.length;

    const direct = (st.history || []).filter((h) => h.parent === sid);
    const running = subtreeRunning(st, sid);
    if (direct.length !== printedKids) {
      printedKids = direct.length;
      console.log("   子会话 " + (direct.length ? direct.map((k) => k.name + "（kind=" + k.kind + "）").join("、") : "（还没有）"));
    }
    if (direct.length) {
      kid = direct[0];
      const evs = await events(kid.name);
      types = evs.map((e) => e.type);
      const reached = act.terminal.every((t) => types.includes(t));
      if (reached && running.length === 0) {
        quiet++;
        if (quiet >= QUIET_POLLS) { converged = true; break; }
      } else {
        quiet = 0;
      }
    }
  }
  if (converged) {
    console.log("   收敛：" + act.terminal.join(" + ") + " 都到了，子树里已没有在跑的生成（用时 " + Math.round((Date.now() - startedAt) / 1000) + "s）");
  } else {
    ok(false, "这一幕在时限内收敛", "超过 " + TIMEOUT_MIN + " 分钟；已见到事件 " + types.join(","));
  }

  // ---- 判据只认产品自己的记录，不认模型的自述 ----
  const st = await state();
  const direct = (st.history || []).filter((h) => h.parent === sid);
  ok(direct.length === 1, "核心只建了一个子会话（三种拼装各一场）", JSON.stringify(direct.map((k) => k.name + "<-" + (k.parent || ""))));
  const child = direct[0];
  if (!child) return;
  const kids = descendants(st, sid);
  for (const k of kids) console.log("   子树 " + k.name + "（kind=" + k.kind + "，父=" + (k.parent || "") + "）");
  ok(child.flows === true, "子会话是有流程的（按流程编排跑，不是单 agent）", String(child.flows));
  ok(child.kind === act.kind, "子会话类型是 " + act.kind, child.kind);

  const proxyLs = await lines(sid);
  const names = toolRows(proxyLs).map((r) => r.name);
  const FACE = [
    "catalog_agents", "create_session", "send_session_message", "observe_session",
    "read_session_messages", "control_session", "read", "list", "grep", "shell",
  ];
  ok(names.length > 0, "核心在代理会话里真的调了工具", names.join("、"));
  ok(names.every((n) => FACE.includes(n)), "只用它自己的工具面（模块工具一个也没发）", names.join("、"));
  ok(names.includes("create_session"), "自己建出子会话（create_session）");

  const evs = await events(child.name);
  const seen = [...new Set(evs.map((e) => e.type))];
  for (const t of act.expect) ok(seen.includes(t), "子会话有 " + t + " 事件", seen.join(","));
  for (const t of act.forbid) ok(!seen.includes(t), "子会话没有 " + t + " 事件", seen.join(","));

  const childLs = await lines(child.name);
  const childRows = toolRows(childLs);
  const spokes = childLs.filter((l) => l.kind === "msg").length;
  console.log(
    "   " + child.name + "：工具 " + childRows.length + " 次、发言 " + spokes + " 条、总结 " +
    (childLs.some((l) => l.kind === "system" && l.speaker === "总结") ? "有" : "无"),
  );
  ok(childLs.some((l) => l.kind === "user" && l.verb === "需求"), "子会话的开头是本次需求", JSON.stringify(childLs.filter((l) => l.kind === "user").map((l) => l.line).slice(0, 1)));
  ok(childRows.length > 0 || spokes > 0, "子会话真的跑过（有工具行或发言）");
  if (act.id === "discuss") {
    ok(childLs.some((l) => l.kind === "system" && l.speaker === "总结"), "讨论收敛后核心写了总结（一条 总结 行）");
    ok(spokes >= 2, "至少两位 discussant 真的发过言", spokes);
  } else {
    // 执行席的活干在它自己的会话里：流程主会话只看得见核心的核验行，替它回报的那条在成员会话。
    const memberRows = [];
    for (const k of kids.filter((x) => x.name.startsWith(child.name + "--"))) {
      memberRows.push(...toolRows(await lines(k.name)));
    }
    ok(
      memberRows.some((r) => r.name === "submit_report" || r.name === "work_commit"),
      "执行席交过回报或提交过产物",
      JSON.stringify(memberRows.map((r) => r.name).slice(0, 12)),
    );
    ok(names.includes("send_session_message"), "核心代答了子会话的关卡（send_session_message）", names.join("、"));
  }

  // 关卡不落到用户头上（代理会话自己不待裁，子会话的门由核心代答）。
  const nameSet = new Set([sid, ...kids.map((k) => k.name)]);
  const waiting = ((st && st.sessions) || []).filter((v) => v && nameSet.has(v.sid) && v.pending);
  ok(
    waiting.length === 0,
    "没有会话停在门上等用户（关卡由核心代答）",
    JSON.stringify(waiting.map((v) => v.sid + ":" + JSON.stringify(v.pending))).slice(0, 300),
  );

  // 产物：盘上真的多了东西（各 agent 的成品在它自己的沙箱里）。
  const made = producedFiles(sid);
  console.log("   产物 " + made.size + " 件：");
  for (const [name, p] of made) console.log("     " + name + " → " + p + "（" + statSync(p).size + " 字节）");
  if (act.artifact) {
    ok([...made.keys()].some((n) => act.artifact.test(n)), "落下了 " + act.artifact.source, [...made.keys()].join("、"));
  }
  console.log("   这一场：代理会话 " + sid + "，子会话 " + child.name);
}

/** 停止 / 继续：用户接管 = 按一次停止，整棵子树一起停；继续再解冻。 */
async function stopAct() {
  const work = "demo-proxy-stop-" + Date.now();
  console.log("\n第四幕：用户按下停止 = 整棵子树一起停");
  const created = await api("POST", actUrl("create_session"), { name: work, kind: "proxy" });
  ok(created.status === 200, "建这一场的代理会话", created.text);
  const sid = (created.json && created.json.sid) || work;
  const said = await api("POST", actUrl("send_message"), { session_id: sid, text: STOP_GOAL });
  ok(said.status === 200, "跟它说一件要花时间的事", said.text);

  // 等它把活派给某个人、那个人真的在跑——这时候按停止才有级联可言。
  let caught = [];
  const until = Date.now() + 90_000;
  while (Date.now() < until && !caught.length) {
    await sleep(400);
    caught = subtreeRunning(await state(), sid);
  }
  const kids = (await state()).history.filter((h) => h.parent === sid);
  console.log(
    "   停止前：子会话 " + (kids.length ? kids.map((k) => k.name).join("、") : "（没有）") +
    "；在跑 " + (caught.length ? caught.join("、") : "无"),
  );

  const stopped = await api("POST", actUrl("control_session"), { session_id: sid, action: "stop" });
  ok(stopped.status === 200, "点停止", stopped.text);
  // 停止是**持久事实**（run=stopped），不是只中断这一刻：多等几轮，之后的叫醒也会被闸门拒掉。
  let still = [];
  for (let i = 0; i < 5; i++) {
    await sleep(1500);
    const running = subtreeRunning(await state(), sid);
    if (running.length) { still = running; break; }
  }
  ok(still.length === 0, "停止即全停：整棵子树都不再跑（且保持）", still.join("、"));
  const stoppedTree = (await state()).history.filter((h) => h.name === sid || h.parent === sid);
  ok(
    stoppedTree.length > 0 && stoppedTree.every((h) => h.run === "stopped"),
    "整棵子树落盘为已停止（run=stopped）",
    JSON.stringify(stoppedTree.map((h) => h.name + ":" + h.run)),
  );

  console.log("   继续：解冻并接着走");
  const resumed = await api("POST", actUrl("control_session"), { session_id: sid, action: "continue" });
  ok(resumed.status === 200, "点继续（解冻并接着走）", resumed.text);
  const activeTree = (await state()).history.filter((h) => h.name === sid || h.parent === sid);
  ok(
    activeTree.some((h) => h.run === "active") && activeTree.every((h) => h.run !== "stopped"),
    "继续后整棵子树回到可被驱动（不再是 stopped）",
    JSON.stringify(activeTree.map((h) => h.name + ":" + h.run)),
  );
  if (!caught.length) console.log("   （停止那一刻子树里没有在跑的生成——多半已经跑完；这条只验了停止后的最终态。）");
}

async function main() {
  if (!(await waitReady())) {
    console.error("转录中心没起来：" + BASE + "（先跑 node start.js --web）");
    return 1;
  }
  if (process.env.SOLOMNI_DEMO_MODEL) {
    console.log("（SOLOMNI_DEMO_MODEL 对本脚本不适用：代理会话跑在核心默认模型上。）");
  }

  // 演示是**真机测试**：条件不齐就明说并退出（退出码 2），不用演示通道凑一遍。
  const block = proxyPreflight((await api("GET", "/api/state")).json);
  if (block) return refuseDemo(block);

  const want = (process.env.SOLOMNI_DEMO_ACTS || "execute,discuss,meeting")
    .split(",").map((s) => s.trim()).filter(Boolean);
  const picked = ACTS.filter((a) => want.includes(a.id));
  if (!picked.length) {
    console.log("没有要跑的一幕：SOLOMNI_DEMO_ACTS=" + want.join(",") + "（可选 " + ACTS.map((a) => a.id).join(" / ") + "）");
    return 1;
  }
  for (const act of picked) await runAct(act);
  if (process.env.SOLOMNI_DEMO_STOP === "1") await stopAct();

  console.log(failed ? "DEMO-FAILED failed=" + failed : "DEMO-OK（三幕：只执行 / 只讨论 / 会议）");
  return failed ? 1 : 0;
}

main()
  .then((code) => process.exit(code))
  .catch((e) => {
    console.log("FAIL 驱动异常 :: " + (e && e.message ? e.message : e));
    process.exit(1);
  });
