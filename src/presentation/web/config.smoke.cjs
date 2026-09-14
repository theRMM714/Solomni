/* 前端配置视图冒烟测试（桩 DOM + 桩 fetch）：跑会话列表的「打开 / 编辑」与配置面板。
 * 用法：node src/presentation/web/config.smoke.cjs   （输出 CONFIG-SMOKE-OK / CONFIG-SMOKE-FAIL）
 * 与 app.smoke.cjs 同一套写法的独立文件；同目录的 smoke.cjs 跑手会自动带上本文件。
 * 覆盖：条目三按钮（打开 / 编辑 / 删除）与「点条目不进会话」、正在生成中的编辑禁用与理由、
 *      冻结（started=true）与未冻结两种配置、档位 radio 切换与 base 显隐、运行能力六项事实的中文呈现、
 *      只有多版本能力才出现定版选择、保存成功与失败（失败显示后端 error 原文）、提交体形状、
 *      「重新扫描」重新 GET 并重绘、点「打开」真的进入会话视图。
 * 为什么需要：app.js 是 no-build 原生 JS，这些交互只有跑起来才发现引用错误与文案缺失。
 */
const fs = require("fs");
const path = require("path");
const vm = require("vm");
const checks = [];
const loadErrors = [];
const alerts = [];

/* ---------- 桩 DOM：只用 app.js 真正用到的 API ---------- */
class N {
  constructor(tag) {
    this.tag = tag; this.children = []; this.textContent = ""; this.className = "";
    this.dataset = {}; this._h = {}; this.value = ""; this.checked = false; this._html = "";
    this.style = {};
  }
  appendChild(c) { this.children.push(c); return c; }
  addEventListener(t, f) { (this._h[t] = this._h[t] || []).push(f); }
  querySelector() { return new N("div"); }
  querySelectorAll() { return []; }
  get classList() {
    const self = this;
    return {
      add(c) { self.className = (self.className + " " + c).trim(); },
      remove(c) { self.className = self.className.split(" ").filter((x) => x !== c).join(" "); },
    };
  }
  set innerHTML(v) { this.children = []; this._html = v; }
  get innerHTML() { return this._html; }
  focus() {} setAttribute() {} getAttribute() { return null; }
}
const reg = new Map(); // 选择器 -> 同一个节点（app.js 靠 $ 复用容器）
const document = {
  querySelector(s) { if (!reg.has(s)) reg.set(s, new N("div")); return reg.get(s); },
  createElement(t) { return new N(t); },
  addEventListener() {},
};

/* ---------- 节点查找小工具 ---------- */
function walk(n, out) { out.push(n); for (const c of n.children) walk(c, out); return out; }
function allText(n) { return walk(n, []).map((x) => (x.textContent == null ? "" : String(x.textContent))).join("\n"); }
function findByClass(n, cls) { return walk(n, []).filter((x) => String(x.className).split(" ").indexOf(cls) >= 0); }
function findButton(n, label) { return walk(n, []).find((x) => x.tag === "button" && x.textContent === label); }
function baseField(n) {
  return walk(n, []).find((x) => String(x.className).indexOf("wf-field") === 0
    && (x.children || []).some((c) => String(c.textContent).indexOf("基础根") >= 0));
}
function check(name, cond, extra) {
  checks.push((cond ? "PASS " : "FAIL ") + name + (cond ? "" : "  <<< " + (extra === undefined ? "" : extra)));
}

/* ---------- 固定夹具：与 /api/sessions/{sid}/config 的 JSON 形状同形 ---------- */
/* 路径用中性书写形式（斜杠 + 无盘符），只验呈现，不代表任何机器路径。 */
const CFG = {
  sid: "w", mode: "single", started: true,
  agents: [{ name: "调研员", modules: ["research"], model: "m1" }],
  tier: "vm", base: "base-linux", net: true,
  pins: { python: "3.12", ruby: "3.2" },
  runtime: {
    tier: "vm",
    declared: { research: ["python"] },
    available: { python: ["3.11", "3.12"], node: ["20.11.0"] },
    missing: { research: ["python"] },
    diagnoses: [
      { Ambiguous: { capability: "python", versions: ["3.11", "3.12"] } },
      { Missing: { module: "research", capability: "python" } },
      { UnknownPin: { capability: "ruby", version: "3.2" } },
      { Conflict: { path: "opt/rt/bin", a: "p1", b: "p2" } },
    ],
    rejected: ["broken-mod：缺 module.yaml"],
    rejected_packages: ["runtimes/bad：manifest 非法"],
  },
  runtimes_dir: "/runtimes",
};
const RAW = { // 还没开过的协作会话：名字可改、本机档
  sid: "raw", mode: "collab", started: false,
  agents: [{ name: "x", modules: ["research"], model: "" }, { name: "y", modules: ["research"], model: "m2" }],
  tier: "host", base: null, net: false, pins: {},
  runtime: { tier: "host", declared: {}, available: {}, missing: {}, diagnoses: [], rejected: [], rejected_packages: [] },
  runtimes_dir: "/runtimes",
};
const calls = [];
const jsonRes = (v, ok, status) => ({ ok: ok !== false, status: status || 200, json: async () => v });
const sandbox = {
  document,
  alert: (m) => alerts.push(m),
  confirm: () => true,
  console, JSON, Promise, Map, Set, Error, Object, Array, String, Number, Boolean, Math, Date,
  setTimeout, clearTimeout,
  fetch: async (url, opts) => {
    calls.push({ url: String(url), method: (opts && opts.method) || "GET", body: opts && opts.body });
    if (url === "/api/state") {
      return jsonRes({
        modules: [{ id: "research", brief: "调研" }, { id: "db", brief: "数据库" }],
        providers: [], models: [{ id: "m1", name: "模型一" }, { id: "m2", name: "模型二" }],
        core: "m1", rejected: [], agents: [],
        settings: { streaming: true, show_reasoning: true },
        sessions: [],
        history: [
          { name: "w", mode: "single", ts: 1, done: true },
          { name: "busy-one", mode: "collab", ts: 2, done: false },
        ],
      });
    }
    if (String(url).indexOf("/api/events") === 0) throw new Error("poll-not-stubbed"); // 轮询不参与本冒烟
    if (/\/api\/history\/[^/]+$/.test(url)) return jsonRes({ meta: { mode: "single" }, events: [] });
    if (/\/files$/.test(url)) return jsonRes({ work: [], agents: [], roots: { work: "/session/w/work", agents: [] } });
    if (/\/api\/sessions\/w\/config$/.test(url)) return jsonRes({ config: CFG });
    if (/\/api\/sessions\/raw\/config$/.test(url)) return jsonRes({ config: RAW });
    if (/\/api\/sessions\/raw\/edit$/.test(url)) return jsonRes({ ok: true });
    if (/\/api\/sessions\/w\/edit$/.test(url)) {
      // 后端在生成中拒绝编辑：原文必须原样显示出来（前端不许兜底改写）
      return jsonRes({ error: "该会话正在生成中：先「停止」或等它结束，再改配置" }, false, 400);
    }
    throw new Error("not-stubbed: " + url);
  },
};
sandbox.window = sandbox; sandbox.globalThis = sandbox;

/* 顶层 const 进的是上下文的词法环境：要拿 state 与内部函数，必须在同一个 context 里再求值。 */
const ctx = vm.createContext(sandbox);
try {
  vm.runInContext(fs.readFileSync(path.join(__dirname, "md.js"), "utf8"), ctx, { filename: "md.js" });
} catch (e) { loadErrors.push("md load error: " + e.message); }
try {
  vm.runInContext(fs.readFileSync(path.join(__dirname, "app.js"), "utf8"), ctx, { filename: "app.js" });
} catch (e) { loadErrors.push("load error: " + e.message); }
let state = null, renderHistory = null, openConfig = null, closeModalIfAny = null;
if (!loadErrors.length) {
  state = vm.runInContext("state", ctx);
  renderHistory = vm.runInContext("renderHistory", ctx);
  openConfig = vm.runInContext("openConfig", ctx);
  closeModalIfAny = () => vm.runInContext("closeModal()", ctx);
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

(async () => {
  if (loadErrors.length) return finish();

  await sleep(80); // 让启动路径的 refreshState 落地（renderHistory 用的是桩历史）
  state.sessions.set("busy-one", { sid: "busy-one", busy: true, mode: "collab" }); // 本标签页正在生成
  renderHistory();

  /* ---- 会话列表：三个按钮 + 点条目不进会话 + 生成中禁用「编辑」 ---- */
  const list = reg.get("#history-list");
  const items = findByClass(list, "history-item");
  check("history: 两项都渲染", items.length === 2, "items=" + items.length);
  check("history: 条目本身没有 onclick（点条目不进会话）", items.every((it) => !it.onclick));
  const opened = findButton(items[0], "打开");
  const edited = findButton(items[0], "编辑");
  check("history: 打开/编辑按钮存在", !!opened && !!edited);
  check("history: 删除按钮仍在", !!findButton(items[0], "✕"));
  check("history: 打开按钮接线到 openHistory", typeof opened.onclick === "function");
  check("history: 非生成中会话的编辑按钮可用", edited.disabled !== true && typeof edited.onclick === "function");
  check("history: 删除按钮接线", typeof findButton(items[0], "✕").onclick === "function");
  const busyItem = items.find((it) => allText(it).indexOf("busy-one") >= 0);
  const busyEdit = findButton(busyItem, "编辑");
  check("history: 生成中会话的编辑按钮被禁用", busyEdit.disabled === true, "disabled=" + busyEdit.disabled);
  check("history: 生成中的禁用理由写在 title 上", String(busyEdit.title).indexOf("正在生成中") >= 0, busyEdit.title);
  check("history: 生成中会话的编辑按钮没有接线", typeof busyEdit.onclick !== "function");
  check("history: 状态文案保留（·进行中）", allText(items[0]).indexOf("·进行中") < 0 && allText(busyItem).indexOf("·进行中") >= 0);

  /* ---- 配置面板（started=true、虚拟机档）：冻结、模块、模型、档位、运行能力、定版、网络 ---- */
  openConfig("w");
  await sleep(80);
  const modal = reg.get("#modal-root");
  const t = allText(modal);
  check("config: 标题带会话名", t.indexOf("配置：w") >= 0);
  check("config: 冻结说明（名单与形态）", t.indexOf("名单与形态冻结") >= 0, t.slice(0, 200));
  check("config: 形态说明", t.indexOf("单 agent（恰好 1 个 agent，模块数不限）") >= 0);
  check("config: agent 名字只读", t.indexOf("名字（只读）") >= 0);
  check("config: 模块选项来自 /api/state", t.indexOf("research") >= 0 && t.indexOf("db") >= 0);
  const selects = walk(modal, []).filter((x) => x.tag === "select");
  const modelSel = selects.find((s) => (s.children || []).some((o) => String(o.textContent).indexOf("核心默认") >= 0));
  check("config: 模型下拉含核心默认空选项", !!modelSel);
  check("config: 档位二选一（本机/虚拟机各自说明）",
    t.indexOf("本机档 —— 脚本直接在宿主上跑") >= 0 && t.indexOf("虚拟机档 —— 一整套 guest") >= 0);
  const radios = walk(modal, []).filter((x) => x.type === "radio");
  check("config: 两个档位 radio 同名互斥且选中当前档位",
    radios.length === 2 && radios[0].name === radios[1].name && radios[1].checked === true,
    JSON.stringify(radios.map((r) => [r.name, r.checked])));
  check("config: 虚拟机档给出 base 输入", t.indexOf("虚拟机基础根 base（可选）") >= 0);
  check("config: 依赖文件夹真实路径（照它去放包）", t.indexOf("/runtimes") >= 0);
  check("config: declared 呈现", t.indexOf("模块 research 需要：python") >= 0);
  check("config: missing 呈现 + 放进 runtimes_dir 后重新扫描",
    t.indexOf("模块 research 缺：python") >= 0 && t.indexOf("把对应的运行包放进 /runtimes") >= 0 && t.indexOf("重新扫描") >= 0);
  check("config: available 呈现", t.indexOf("· python：3.11、3.12") >= 0 && t.indexOf("· node：20.11.0") >= 0);
  check("config: 四类 diagnosis 都译成中文",
    ["有多个版本", "在包库里没有", "不在包库里", "都要写进 opt/rt/bin"].every((k) => t.indexOf(k) >= 0), t);
  check("config: 模块/运行包拒收原因原文",
    t.indexOf("模块：broken-mod：缺 module.yaml") >= 0 && t.indexOf("运行包：runtimes/bad：manifest 非法") >= 0);
  const pinSelects = selects.filter((s) => (s.children || []).some((o) => ["3.11", "3.12"].indexOf(String(o.textContent)) >= 0));
  check("config: 多版本能力给出版本选择", pinSelects.length === 1, "pinSelects=" + pinSelects.length);
  check("config: 版本选项齐全 + 含不定版选项",
    pinSelects[0] && pinSelects[0].children.map((o) => o.textContent).join("|") === "（不定版：用包库里第一个 3.11）|3.11|3.12",
    pinSelects[0] && pinSelects[0].children.map((o) => o.textContent).join("|"));
  check("config: 单版本能力不显示选择", !selects.some((s) => (s.children || []).some((o) => String(o.textContent) === "20.11.0")));
  check("config: 单版本已有定版如实说明", t.indexOf("包库里现在看不到这个能力") >= 0);
  check("config: 网络开关", t.indexOf("放行出站网络") >= 0);
  check("config: 保存/取消按钮", !!findButton(modal, "保存") && !!findButton(modal, "取消"));

  /* ---- 保存失败：后端 error 原文 ---- */
  const save = findButton(modal, "保存");
  save.onclick();
  await sleep(60);
  const msg = findByClass(modal, "modal-msg")[0];
  check("config: 保存失败显示后端 error 原文",
    msg && String(msg.textContent).indexOf("保存失败：该会话正在生成中：先「停止」或等它结束，再改配置") >= 0, msg && msg.textContent);
  check("config: 失败后保存按钮可再点", save.disabled === false);
  const editCall = calls.find((c) => /\/edit$/.test(c.url));
  const sent = JSON.parse(editCall.body);
  check("config: 提交体形状 {agents,tier,base,pins,net}",
    JSON.stringify(Object.keys(sent).sort()) === JSON.stringify(["agents", "base", "net", "pins", "tier"]), editCall.body);
  check("config: 提交体内容（名单/模块/模型/档位/base/pins/网络）",
    sent.tier === "vm" && sent.base === "base-linux" && sent.net === true
    && sent.agents[0].name === "调研员" && JSON.stringify(sent.agents[0].modules) === JSON.stringify(["research"])
    && sent.agents[0].model === "m1" && JSON.stringify(sent.pins) === JSON.stringify({ python: "3.12", ruby: "3.2" }),
    editCall.body);

  /* ---- 取消定版 = 空串不提交 ---- */
  pinSelects[0].value = "";
  pinSelects[0]._h.change[0]();
  findButton(modal, "保存").onclick();
  await sleep(60);
  const sent2 = JSON.parse(calls.filter((c) => /\/edit$/.test(c.url)).pop().body);
  check("config: 取消定版后不从 pins 提交空串", !("python" in sent2.pins) && sent2.pins.ruby === "3.2", JSON.stringify(sent2.pins));

  /* ---- 重新扫描 = 重新 GET config 并重绘 ---- */
  const before = calls.filter((c) => /\/config$/.test(c.url)).length;
  findButton(modal, "重新扫描").onclick();
  await sleep(60);
  check("config: 重新扫描再来一次 GET config", calls.filter((c) => /\/config$/.test(c.url)).length === before + 1);
  check("config: 重新扫描后仍渲染出内容", allText(reg.get("#modal-root")).indexOf("模块 research 缺：python") >= 0);

  /* ---- 档位切换：虚拟机档显示 base，本机档隐藏且提交 base=null ---- */
  // 「重新扫描」会整块重建表单：radio 节点要重新取（旧节点上的处理器属于旧表单）
  check("config: 虚拟机档 base 字段可见", String(baseField(reg.get("#modal-root")).className) === "wf-field",
    baseField(reg.get("#modal-root")).className);
  const radios2 = walk(reg.get("#modal-root"), []).filter((x) => x.type === "radio");
  radios2[1].checked = false;
  radios2[0].checked = true;
  radios2[0]._h.change[0]();
  check("config: 切到本机档后 base 字段隐藏", String(baseField(reg.get("#modal-root")).className).indexOf("hidden") >= 0,
    baseField(reg.get("#modal-root")).className);
  findButton(reg.get("#modal-root"), "保存").onclick();
  await sleep(60);
  const sent3 = JSON.parse(calls.filter((c) => /\/edit$/.test(c.url)).pop().body);
  check("config: 本机档提交 tier=host 且 base=null", sent3.tier === "host" && sent3.base === null, JSON.stringify(sent3));

  /* ---- 未开过的会话（started=false）：名字可改、无冻结、跨 agent 重复模块实时提示、保存成功 ---- */
  openConfig("raw");
  await sleep(60);
  const m2 = reg.get("#modal-root");
  const t2 = allText(m2);
  check("config(raw): 没有冻结提示", t2.indexOf("名单与形态冻结") < 0);
  const nameInputs = walk(m2, []).filter((n) => n.tag === "input" && String(n.placeholder).indexOf("agent 名字") >= 0);
  check("config(raw): 名字可编辑（会话还没开过）", nameInputs.length === 2 && nameInputs.every((n) => n.disabled !== true));
  check("config(raw): 协作形态说明", t2.indexOf("协作（N 个 agent，各自独立沙箱）") >= 0);
  check("config(raw): 本机档不显示 base 字段",
    t2.indexOf("虚拟机基础根 base（可选）") >= 0 && String(baseField(m2).className).indexOf("hidden") >= 0, baseField(m2).className);
  const dupHints = findByClass(m2, "cfg-hint").map((x) => x.textContent)
    .filter((x) => String(x).indexOf("同一模块只能属于一个 agent") >= 0);
  check("config(raw): 跨 agent 重复模块实时提示",
    dupHints.length === 2 && dupHints.every((x) => x.indexOf("research") >= 0), JSON.stringify(dupHints));
  findButton(m2, "保存").onclick();
  await sleep(80);
  const msg2 = findByClass(reg.get("#modal-root"), "modal-msg")[0];
  check("config(raw): 保存成功后如实提示", msg2 && String(msg2.textContent).indexOf("已保存") >= 0, msg2 && msg2.textContent);
  const sentRaw = JSON.parse(calls.filter((c) => /\/raw\/edit$/.test(c.url)).pop().body);
  check("config(raw): 名字改动随提交送出", sentRaw.agents.map((a) => a.name).join(",") === "x,y", JSON.stringify(sentRaw.agents));
  check("config(raw): 无模型以空串送出", sentRaw.agents[0].model === "" && sentRaw.agents[1].model === "m2", JSON.stringify(sentRaw.agents));

  /* ---- 「打开」接线：点了真的进入会话视图 ---- */
  closeModalIfAny();
  let openErrored = null;
  try {
    findButton(findByClass(reg.get("#history-list"), "history-item")[0], "打开").onclick({ stopPropagation() {} });
    await sleep(80);
  } catch (e) { openErrored = e && e.message; }
  check("open: 点「打开」进入会话视图不报错", !openErrored, String(openErrored));
  check("open: 会话被激活、按历史只读回放", state.activeSid === "w" && !!state.sessions.get("w"), "activeSid=" + state.activeSid);
  check("open: 条目本身仍不是入口", typeof findByClass(reg.get("#history-list"), "history-item")[0].onclick !== "function");

  finish();
})().catch((e) => { loadErrors.push("harness error: " + ((e && e.stack) || e)); finish(); });

function finish() {
  if (process.env.SMOKE_VERBOSE) console.log(checks.join("\n"));
  const failedLines = checks.filter((c) => c.indexOf("FAIL ") === 0);
  if (failedLines.length) console.log(failedLines.join("\n"));
  if (loadErrors.length) console.log("loadErrors:", JSON.stringify(loadErrors));
  if (alerts.length) console.log("alerts:", JSON.stringify(alerts));
  const bad = failedLines.length + loadErrors.length + alerts.length;
  console.log("config.smoke: " + (checks.length - failedLines.length) + "/" + checks.length + " 断言通过");
  console.log(bad ? "CONFIG-SMOKE-FAIL" : "CONFIG-SMOKE-OK");
  process.exit(bad ? 1 : 0);
}
