/* 前端配置视图冒烟测试（桩 DOM + 桩 fetch）：跑会话列表的「打开 / 编辑」与配置面板。
 * 用法：node src/presentation/web/config.smoke.cjs   （输出 CONFIG-SMOKE-OK / CONFIG-SMOKE-FAIL）
 * 与 app.smoke.cjs 同一套写法的独立文件；同目录的 smoke.cjs 跑手会自动带上本文件。
 * 覆盖：条目三按钮（打开 / 编辑 / 删除）与「点条目不进会话」、正在生成中的编辑禁用与理由、
 *      冻结（started=true）与未冻结两种配置、档位 radio 切换与虚拟根选择器显隐、运行能力六项事实的中文呈现、
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
  appendChild(c) { if (c) c.parentNode = this; this.children.push(c); return c; }
  removeChild(c) { const i = this.children.indexOf(c); if (i >= 0) this.children.splice(i, 1); if (c) c.parentNode = null; return c; }
  contains(n) { return this.children.indexOf(n) >= 0; }
  getBoundingClientRect() { return { left: 0, top: 0, right: 0, bottom: 0 }; }
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
function guestField(n) {
  return walk(n, []).find((x) => String(x.className).indexOf("wf-field") === 0
    && (x.children || []).some((c) => String(c.textContent).indexOf("虚拟机（虚拟根）") >= 0));
}
function check(name, cond, extra) {
  checks.push((cond ? "PASS " : "FAIL ") + name + (cond ? "" : "  <<< " + (extra === undefined ? "" : extra)));
}

/* ---------- 固定夹具：与 /api/sessions/{sid}/config 的 JSON 形状同形 ---------- */
/* 路径用中性书写形式（斜杠 + 无盘符），只验呈现，不代表任何机器路径。 */
const CFG = {
  sid: "w", mode: "single", started: true,
  agents: [{ name: "调研员", modules: ["research"], model: "m1" }],
  tier: "vm", guest: "debian", net: true,
  permissions: { allow_read: [], allow_write: ["docs"], deny: [], granularity: "ask", ask: ["research.grep"] },
  guests: [
    { guest: "debian", label: "debian-13-genericcloud-amd64", os: "linux", arch: "amd64", ready: true, missing: [], install: ".vm/guest/debian", runtimes: ["python"] },
    { guest: "windows", label: "debian-13-genericcloud-amd64", os: "linux", arch: "amd64", ready: false, missing: ["vm.guest-runtime.windows"], install: ".vm/guest/windows" },
  ],
  pins: { python: "3.12", ruby: "3.2" },
  vm_available: true, vm_unavailable_reason: "", vm_requirements: [],
  tier_ready: true, tier_missing: [],
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
  tier: "host", guest: null, guests: [], net: false, pins: {},
  // 这台机器**不具备**虚拟机档的前置条件：界面必须禁用虚拟机档并给出后端原话（本机档不受影响）。
  vm_available: false, vm_unavailable_reason: "虚拟机档现在不可用（guest 本体尚未接入）",
  vm_requirements: [
    { id: "hypervisor", met: false, detail: "本机虚拟机监视器不可用（要启用「虚拟机平台」组件）", how: "启用「虚拟机平台」组件，然后重启" },
    { id: "guest", met: false, detail: "guest 本体尚未接入（工具进程仍在宿主上跑，没有真正的 guest）", how: "等产品的 guest 接入" },
    { id: "qemu", met: false, detail: "没有找到 qemu-system-x86_64.exe（按PATH找过）", how: "在创建向导或配置界面点「补齐虚拟机依赖」把它下载进项目内；也可自行安装 QEMU 并放进 PATH" },
    { id: "base", met: false, detail: "没有选定虚拟根", how: "在创建向导或配置界面点「补齐虚拟机依赖」把虚拟根装进项目内" },
  ],
  tier_ready: true, tier_missing: [],
  runtime: { tier: "host", declared: {}, available: {}, missing: {}, diagnoses: [], rejected: [], rejected_packages: [] },
  runtimes_dir: "/runtimes",
};
const calls = [];
let setupPolls = 0;
let setupStarted = false;
let tiersVmAvailable = false; // 先在"虚拟机档不可用"的一侧验补齐入口，再切到可用侧验创建忙碌态
let tiersGuestReady = true; // 机器前置齐时，虚拟根是否装齐是另一条事实（决定虚拟根下方的补齐入口）
const jsonRes = (v, ok, status) => ({ ok: ok !== false, status: status || 200, json: async () => v });
const sandbox = {
  document,
  // 原生弹窗是红线：换自研弹窗之后，这里被调用一次就算失败。
  alert: (m) => alerts.push(m),
  confirm: (m) => { alerts.push(m); return true; },
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
    // 依赖补齐：起作业前 = 缺一条；起作业后前两次长轮询还在跑，之后就结束（pending 清空）。
    if (/^\/api\/setup\/status/.test(String(url))) {
      const wait = String(url).indexOf("wait=1") >= 0;
      if (wait) setupPolls++;
      const running = setupStarted && (!wait || setupPolls < 2);
      const pending = running ? ["vm.virtiofsd.windows"] : (setupStarted ? [] : ["vm.virtiofsd.windows"]);
      return jsonRes({
        deps: [{ id: "vm.virtiofsd.windows", guest: null, present: !pending.length, install: ".vm/virtiofsd",
          probe: ".vm/virtiofsd/virtiofsd.exe", source: "release:virtiofsd", url: "https://example.invalid/v.zip",
          size_mb: 3, note: "virtiofsd 便携包" }],
        job: { running, current: "", seq: setupPolls, lines: ["vm.virtiofsd.windows：下载 https://example.invalid/v.zip"], error: null },
        missing: pending.length,
        pending,
      });
    }
    // 建会话（虚拟机会慢）：故意拖一会儿，好在请求在飞的时候验"按钮在转圈"。
    if (url === "/api/actions/create_session") {
      await new Promise((r) => setTimeout(r, 60));
      return jsonRes({ sid: "new-1", agents: ["research"] });
    }
    if (url === "/api/setup/ensure") {
      setupStarted = true;
      return jsonRes({ ok: true, deps: ["vm.virtiofsd.windows"] });
    }
    // 创建向导要读一次档位选择（默认档 + 虚拟机档可用性与逐项前置 + 可选虚拟根）。
    if (url === "/api/tiers") {
      return jsonRes({ tiers: {
        default: "host", vm_available: tiersVmAvailable,
        vm_unavailable_reason: tiersVmAvailable ? "" : "虚拟机档现在不可用（没有找到 qemu-system-x86_64（按PATH找过））",
        vm_requirements: tiersVmAvailable ? [] : RAW.vm_requirements,
        guests: tiersVmAvailable
          ? [{ guest: "windows", label: "debian-13", os: "linux", arch: "amd64", ready: tiersGuestReady,
              missing: tiersGuestReady ? [] : ["vm.guest-runtime.linux"], install: ".vm/guest/windows", runtimes: ["python"] }]
          : [],
        available: {},
      } });
    }
    if (/\/api\/history\/[^/]+$/.test(url)) return jsonRes({ meta: { mode: "single" }, events: [] });
    if (/\/files$/.test(url)) return jsonRes({ work: [], agents: [], roots: { work: "/session/w/work", agents: [] } });
    if (/\/api\/sessions\/w\/config$/.test(url)) return jsonRes({ config: CFG });
    if (/\/api\/sessions\/raw\/config$/.test(url)) return jsonRes({ config: RAW });
    if (url === "/api/actions/edit_session") {
      let sid = "";
      try { sid = String((JSON.parse(opts.body || "{}") || {}).session_id || ""); } catch (e) { sid = ""; }
      if (sid === "raw") return jsonRes({ ok: true });
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
  vm.runInContext(fs.readFileSync(path.join(__dirname, "ui.js"), "utf8"), ctx, { filename: "ui.js" });
} catch (e) { loadErrors.push("ui load error: " + e.message); }
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
  state.sessions.set("busy-one", { sid: "busy-one", sending: false, running: true, running_known: true, mode: "collab" }); // 本标签页收到了它的运行态事件
  renderHistory();

  /* ---- 会话列表：铅笔菜单 + 点条目不进会话 + 生成中禁用「编辑配置」 ---- */
  const list = reg.get("#history-list");
  const items = findByClass(list, "history-item");
  check("history: 两项都渲染", items.length === 2, "items=" + items.length);
  check("history: 条目本身可点击打开", items.every((it) => typeof it.onclick === "function"));
  const pencils = findByClass(list, "hpen");
  check("history: 每行一个铅笔", pencils.length === items.length, "pencils=" + pencils.length);
  const menuOf = (pencil) => {
    pencil.onclick({ stopPropagation() {} });
    const root = reg.get("#menu-root");
    return root.children[root.children.length - 1];
  };
  const m0 = menuOf(pencils[0]);
  const labels0 = walk(m0, []).filter((x) => x.tag === "button").map((x) => allText(x).trim());
  check("history: 菜单有打开/编辑/删除",
    labels0.indexOf("打开") >= 0 && labels0.indexOf("编辑配置") >= 0 && labels0.indexOf("删除") >= 0,
    labels0.join("|"));
  const openBtn = walk(m0, []).find((x) => x.tag === "button" && allText(x).trim() === "打开");
  const editBtn = walk(m0, []).find((x) => x.tag === "button" && allText(x).trim() === "编辑配置");
  check("history: 打开接线到 openHistory", openBtn && typeof openBtn.onclick === "function");
  check("history: 非生成中会话的编辑可用", editBtn && editBtn.disabled !== true && typeof editBtn.onclick === "function");
  const busyItem = items.find((it) => allText(it).indexOf("busy-one") >= 0);
  const busyPencil = findByClass(busyItem, "hpen")[0];
  const mb = menuOf(busyPencil);
  const busyEdit = walk(mb, []).find((x) => x.tag === "button" && allText(x).trim() === "编辑配置");
  check("history: 生成中会话的编辑被禁用", busyEdit && busyEdit.disabled === true, "disabled=" + (busyEdit && busyEdit.disabled));
  check("history: 生成中的禁用理由写在 title 上", busyEdit && String(busyEdit.title).indexOf("正在生成中") >= 0, busyEdit && busyEdit.title);
  check("history: 生成中会话的编辑没有接线", busyEdit && typeof busyEdit.onclick !== "function");
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
  // 承载判定归后端：本机具备前置条件时虚拟机档可选，且 guest 未接入这一点如实标注。
  check("config: 本机具备前置条件时虚拟机档可选", radios[1].disabled !== true);
  check("config: 虚拟机档可选时不渲染前置清单（那是禁用态的事）",
    t.indexOf("虚拟机档前置") < 0);
  check("config: 虚拟机档给出虚拟根选择器", t.indexOf("虚拟机（虚拟根）") >= 0 && t.indexOf("debian-13-genericcloud-amd64") >= 0);
check("config: 虚拟根状态说清装齐与自带能力（不写「已就绪」这种会被读成「能用」的词）",
  t.indexOf("已装齐") >= 0 && t.indexOf("自带运行能力：python") >= 0 && t.indexOf("已就绪") < 0);
  check("config: 依赖文件夹真实路径（照它去放包）", t.indexOf("/runtimes") >= 0);
  check("config: declared 呈现", t.indexOf("模块 research 需要：python") >= 0);
  check("config: missing 呈现 + 放进 runtimes_dir 后重新扫描",
    t.indexOf("模块 research 缺：python") >= 0 && t.indexOf("把适用于本档位的运行包放进 /runtimes") >= 0 && t.indexOf("重新扫描") >= 0);
  check("config: available 呈现", t.indexOf("· python：3.11、3.12") >= 0 && t.indexOf("· node：20.11.0") >= 0);
  check("config: 四类 diagnosis 都译成中文",
    ["有多个版本", "在本档位没有", "不在本档位可用的包里", "都要写进 opt/rt/bin"].every((k) => t.indexOf(k) >= 0), t);
  check("config: 模块/运行包拒收原因原文",
    t.indexOf("模块：broken-mod：缺 module.yaml") >= 0 && t.indexOf("运行包：runtimes/bad：manifest 非法") >= 0);
  const pinSelects = selects.filter((s) => (s.children || []).some((o) => ["3.11", "3.12"].indexOf(String(o.textContent)) >= 0));
  check("config: 多版本能力给出版本选择", pinSelects.length === 1, "pinSelects=" + pinSelects.length);
  check("config: 版本选项齐全 + 含不定版选项",
    pinSelects[0] && pinSelects[0].children.map((o) => o.textContent).join("|") === "（不定版：用包库里第一个 3.11）|3.11|3.12",
    pinSelects[0] && pinSelects[0].children.map((o) => o.textContent).join("|"));
  check("config: 单版本能力不显示选择", !selects.some((s) => (s.children || []).some((o) => String(o.textContent) === "20.11.0")));
  check("config: 单版本已有定版如实说明", t.indexOf("包库里现在看不到这个能力") >= 0);
  check("config: 会话权限段", t.indexOf("会话权限") >= 0 && t.indexOf("网络由全局") >= 0);
  check("config: 保存/取消按钮", !!findButton(modal, "保存") && !!findButton(modal, "取消"));

  /* ---- 保存失败：后端 error 原文 ---- */
  const save = findButton(modal, "保存");
  save.onclick();
  await sleep(60);
  const msg = findByClass(modal, "modal-msg")[0];
  check("config: 保存失败显示后端 error 原文",
    msg && String(msg.textContent).indexOf("保存失败：该会话正在生成中：先「停止」或等它结束，再改配置") >= 0, msg && msg.textContent);
  check("config: 失败后保存按钮可再点", save.disabled === false);
  const editCall = calls.find((c) => /\/api\/actions\/edit_session$/.test(c.url));
  const sent = JSON.parse(editCall.body);
  check("config: 提交体形状 {session_id,agents,tier,guest,pins,permissions}",
    JSON.stringify(Object.keys(sent).sort()) === JSON.stringify(["agents", "guest", "permissions", "pins", "session_id", "tier"])
      && sent.session_id === "w", editCall.body);
  check("config: 提交体内容（名单/模块/模型/档位/虚拟根/pins/会话权限）",
    sent.tier === "vm" && sent.guest === "debian"
    && sent.agents[0].name === "调研员" && JSON.stringify(sent.agents[0].modules) === JSON.stringify(["research"])
    && sent.agents[0].model === "m1" && JSON.stringify(sent.pins) === JSON.stringify({ python: "3.12", ruby: "3.2" })
    && JSON.stringify(sent.permissions.allow_write) === JSON.stringify(["docs"])
    && !("module_write" in sent.permissions),
    editCall.body);

  /* ---- 取消定版 = 空串不提交 ---- */
  pinSelects[0].value = "";
  pinSelects[0]._h.change[0]();
  findButton(modal, "保存").onclick();
  await sleep(60);
  const sent2 = JSON.parse(calls.filter((c) => /\/api\/actions\/edit_session$/.test(c.url)).pop().body);
  check("config: 取消定版后不从 pins 提交空串", !("python" in sent2.pins) && sent2.pins.ruby === "3.2", JSON.stringify(sent2.pins));

  /* ---- 重新扫描 = 重新 GET config 并重绘 ---- */
  const before = calls.filter((c) => /\/config$/.test(c.url)).length;
  findButton(modal, "重新扫描").onclick();
  await sleep(60);
  check("config: 重新扫描再来一次 GET config", calls.filter((c) => /\/config$/.test(c.url)).length === before + 1);
  check("config: 重新扫描后仍渲染出内容", allText(reg.get("#modal-root")).indexOf("模块 research 缺：python") >= 0);

  /* ---- 档位切换：虚拟机档显示虚拟根选择器，本机档隐藏且提交 guest=null ---- */
  // 「重新扫描」会整块重建表单：radio 节点要重新取（旧节点上的处理器属于旧表单）
  check("config: 虚拟机档虚拟根选择器可见", String(guestField(reg.get("#modal-root")).className) === "wf-field",
    guestField(reg.get("#modal-root")).className);
  const radios2 = walk(reg.get("#modal-root"), []).filter((x) => x.type === "radio");
  radios2[1].checked = false;
  radios2[0].checked = true;
  radios2[0]._h.change[0]();
  check("config: 切到本机档后虚拟根选择器隐藏", String(guestField(reg.get("#modal-root")).className).indexOf("hidden") >= 0,
    guestField(reg.get("#modal-root")).className);
  findButton(reg.get("#modal-root"), "保存").onclick();
  await sleep(60);
  const sent3 = JSON.parse(calls.filter((c) => /\/api\/actions\/edit_session$/.test(c.url)).pop().body);
  check("config: 本机档提交 tier=host 且 guest=null", sent3.tier === "host" && sent3.guest === null, JSON.stringify(sent3));

  /* ---- 未开过的会话（started=false）：名字可改、无冻结、跨 agent 共用模块可保存 ---- */
  openConfig("raw");
  await sleep(60);
  const m2 = reg.get("#modal-root");
  const t2 = allText(m2);
  check("config(raw): 没有冻结提示", t2.indexOf("名单与形态冻结") < 0);
  const nameInputs = walk(m2, []).filter((n) => n.tag === "input" && String(n.placeholder).indexOf("agent 名字") >= 0);
  check("config(raw): 名字可编辑（会话还没开过）", nameInputs.length === 2 && nameInputs.every((n) => n.disabled !== true));
  check("config(raw): 协作形态说明", t2.indexOf("协作（N 个 agent，各自独立沙箱）") >= 0);
  check("config(raw): 本机档不显示虚拟根选择器",
    t2.indexOf("虚拟机（虚拟根）") >= 0 && String(guestField(m2).className).indexOf("hidden") >= 0, guestField(m2).className);
  // 前置条件不具备 = 虚拟机档禁用（不让选），并把后端给的原因原样呈现；本机档照旧可选。
  const radiosRaw = walk(m2, []).filter((x) => x.type === "radio");
  check("config(raw): 虚拟机档不可用时禁用且本机档仍可选",
    radiosRaw.length === 2 && radiosRaw[1].disabled === true && radiosRaw[0].disabled !== true,
    JSON.stringify(radiosRaw.map((r) => [r.name, r.checked, r.disabled])));
  check("config(raw): 虚拟机档不可用的原因如实呈现",
    t2.indexOf("虚拟机档现在不能选") >= 0 && t2.indexOf("本机虚拟机监视器不可用") >= 0);
  // 逐项前置照抄后端：缺哪几项、每项怎么补（不笼统说"前置条件不具备"）。
  check("config(raw): 虚拟机档前置逐项列出并给怎么补",
    t2.indexOf("虚拟机档前置（") >= 0 && t2.indexOf("guest 本体尚未接入") >= 0 &&
    t2.indexOf("怎么补：") >= 0 && t2.indexOf("自行安装 QEMU") >= 0 && t2.indexOf("没有选定虚拟根") >= 0);
  const dupHints = findByClass(m2, "cfg-hint").map((x) => x.textContent)
    .filter((x) => String(x).indexOf("同一模块") >= 0);
  check("config(raw): 跨 agent 共用模块不再提示拒绝",
    dupHints.length === 0, JSON.stringify(dupHints));
  findButton(m2, "保存").onclick();
  await sleep(80);
  const msg2 = findByClass(reg.get("#modal-root"), "modal-msg")[0];
  check("config(raw): 保存成功后如实提示", msg2 && String(msg2.textContent).indexOf("已保存") >= 0, msg2 && msg2.textContent);
  const sentRaw = JSON.parse(calls.filter((c) => /\/api\/actions\/edit_session$/.test(c.url)).pop().body);
  check("config(raw): 名字改动随提交送出", sentRaw.agents.map((a) => a.name).join(",") === "x,y", JSON.stringify(sentRaw.agents));
  check("config(raw): 无模型以空串送出", sentRaw.agents[0].model === "" && sentRaw.agents[1].model === "m2", JSON.stringify(sentRaw.agents));

  /* ---- 依赖补齐：不可用时给**一个动作**，而不是一句命令行（"用户没有办法装上默认环境"的回归锚） ---- */
  const setupBtn = findButton(m2, "补齐虚拟机依赖…");
  check("config(raw): 虚拟机档不可用时给出补齐动作", !!setupBtn && typeof setupBtn.onclick === "function");
  setupBtn.onclick();
  await sleep(100);
  const t3 = allText(reg.get("#modal-root"));
  check("setup: 列出要装的条目（含体积与来源说明）",
    t3.indexOf("vm.virtiofsd.windows") >= 0 && t3.indexOf("约 3 MB") >= 0, t3.slice(0, 400));
  check("setup: 说明只写项目内", t3.indexOf("只写项目内") >= 0);
  const goBtn = findButton(reg.get("#modal-root"), "开始下载安装");
  check("setup: 先看清单、再确认下载（两步）", !!goBtn);
  goBtn.onclick();
  await sleep(150);
  const sentEnsure = calls.filter((c) => /\/api\/setup\/ensure$/.test(c.url)).pop();
  check("setup: 确认后 POST ensure（带 confirm 与后端给的 pending id）",
    !!sentEnsure && String(sentEnsure.body).indexOf('"confirm":true') >= 0 &&
    String(sentEnsure.body).indexOf("vm.virtiofsd.windows") >= 0, sentEnsure && sentEnsure.body);
  check("setup: 进度走长轮询（since + wait=1）",
    calls.some((c) => /\/api\/setup\/status\?since=\d+&wait=1/.test(c.url)));
  const t4 = allText(reg.get("#modal-root"));
  check("setup: 装完说清「立刻生效、不用重启」",
    t4.indexOf("已经生效") >= 0 && t4.indexOf("不需要重启") >= 0, t4.slice(-300));
  check("setup: 下载过程有据可查（进度行留在弹层里）",
    t4.indexOf("下载 https://example.invalid/v.zip") >= 0, t4.slice(-300));
  check("setup: 没有用原生 alert/confirm", alerts.length === 0, JSON.stringify(alerts));
  closeModalIfAny();

  /* ---- 创建向导里的补齐动作：点了必须**有反应**（作用域写错就会静默什么都不发生） ---- */
  closeModalIfAny();
  setupStarted = false; setupPolls = 0;
  let wizardErrored = null;
  try { await vm.runInContext('openWizard()', ctx); await sleep(80); } catch (e) { wizardErrored = e && e.message; }
  check('wizard: 打得开且说明虚拟机档不可用', !wizardErrored && allText(reg.get('#modal-root')).indexOf('虚拟机档现在不能选') >= 0, String(wizardErrored));
  const wizSetup = findButton(reg.get('#modal-root'), '补齐虚拟机依赖…');
  check('wizard: 不可用时给出补齐按钮', !!wizSetup && typeof wizSetup.onclick === 'function');
  let clickErrored = null;
  try { wizSetup.onclick(); await sleep(120); } catch (e) { clickErrored = e && e.message; }
  const wizText = allText(reg.get('#modal-root'));
  check('wizard: 点补齐真的打开补齐弹层（列出要装的条目）',
    !clickErrored && wizText.indexOf('vm.virtiofsd.windows') >= 0 && wizText.indexOf('只写项目内') >= 0,
    'clickErrored=' + String(clickErrored) + ' text=' + wizText.slice(0, 200));
  closeModalIfAny();

  /* ---- 机器前置齐、虚拟根未装齐：向导也要给出补齐动作，创建前直接拦住 ---- */
  closeModalIfAny();
  setupStarted = false; setupPolls = 0;
  tiersVmAvailable = true; tiersGuestReady = false;
  let guestErrored = null;
  try { await vm.runInContext('openWizard()', ctx); await sleep(80); } catch (e) { guestErrored = e && e.message; }
  const wizGuestRoot = reg.get('#modal-root');
  check('wizard: 虚拟根未装齐时可打开向导', !guestErrored, String(guestErrored));
  const wizGuestRadios = walk(wizGuestRoot, []).filter((n) => n.type === 'radio' && n.name === 'wf-tier');
  wizGuestRadios[1].checked = true;
  (wizGuestRadios[1]._h.change || []).forEach((f) => f());
  const wizGuestSetup = findButton(wizGuestRoot, '补齐虚拟机依赖…');
  check('wizard: 虚拟根未装齐时给出补齐按钮', !!wizGuestSetup && typeof wizGuestSetup.onclick === 'function');
  // 填个名字、勾个模块，点创建：未装齐应当在本地被拦住，不发 create_session。
  const guestNameInp = walk(wizGuestRoot, []).find((n) => n.tag === 'input' && String(n.placeholder).indexOf('工作名称') >= 0);
  guestNameInp.value = 'guest-not-ready';
  const guestModLab = walk(wizGuestRoot, []).find((n) => n.tag === 'label'
    && (n.children || []).some((c) => String(c.textContent || '') === 'research'));
  const guestModCb = guestModLab && (guestModLab.children || []).find((n) => n.type === 'checkbox');
  guestModCb.checked = true; (guestModCb._h.change || []).forEach((f) => f());
  const createsBefore = calls.filter((c) => /\/api\/actions\/create_session$/.test(c.url)).length;
  findButton(wizGuestRoot, '创建并开始').onclick();
  await sleep(30);
  check('wizard: 未装齐时创建被本地拦住（不发请求）',
    calls.filter((c) => /\/api\/actions\/create_session$/.test(c.url)).length === createsBefore);
  const guestBlockMsg = findByClass(reg.get('#modal-root'), 'modal-msg')[0];
  check('wizard: 拦住时说明缺哪几条',
    !!guestBlockMsg && String(guestBlockMsg.textContent).indexOf('还没装齐') >= 0
    && String(guestBlockMsg.textContent).indexOf('vm.guest-runtime.linux') >= 0,
    guestBlockMsg && guestBlockMsg.textContent);
  let guestClickErrored = null;
  try { wizGuestSetup.onclick(); await sleep(120); } catch (e) { guestClickErrored = e && e.message; }
  const guestSetupText = allText(reg.get('#modal-root'));
  check('wizard: 点虚拟根旁的补齐真的打开补齐弹层',
    !guestClickErrored && guestSetupText.indexOf('vm.virtiofsd.windows') >= 0 && guestSetupText.indexOf('只写项目内') >= 0,
    'clickErrored=' + String(guestClickErrored) + ' text=' + guestSetupText.slice(0, 200));
  closeModalIfAny();
  tiersGuestReady = true;


  /* ---- 创建时的忙碌态：虚拟机档要在这一步起 guest（要等），按钮必须转圈并说清在等什么 ---- */
  closeModalIfAny();
  tiersVmAvailable = true; // 这一段要验虚拟机档的创建（它才要等 guest 起来）
  await vm.runInContext('openWizard()', ctx);
  await sleep(80);
  const wizRoot = reg.get('#modal-root');
  const wizRadios = walk(wizRoot, []).filter((n) => n.type === 'radio' && n.name === 'wf-tier');
  const wizVm = wizRadios[1];
  check('busy: 虚拟机档可用时可选', !!wizVm && wizVm.disabled !== true);
  wizVm.checked = true; (wizVm._h.change || []).forEach((f) => f());
  const nameInp = walk(wizRoot, []).find((n) => n.tag === 'input' && String(n.placeholder).indexOf('工作名称') >= 0);
  nameInp.value = 'busy-check';
  const modLab = walk(wizRoot, []).find((n) => n.tag === 'label'
    && (n.children || []).some((c) => String(c.textContent || '') === 'research'));
  const modCb = modLab && (modLab.children || []).find((n) => n.type === 'checkbox');
  check('busy: 向导里能勾到模块', !!modCb);
  modCb.checked = true; (modCb._h.change || []).forEach((f) => f());
  const createBtn = findButton(wizRoot, '创建并开始');
  check('busy: 创建按钮在场', !!createBtn);
  const pending = createBtn.onclick();
  await sleep(10);
  check('busy: 点下去立刻置忙（禁用 + 文案改口）',
    createBtn.disabled === true && createBtn.textContent === '正在创建…',
    'disabled=' + createBtn.disabled + ' text=' + createBtn.textContent);
  check('busy: 忙碌时按钮带转圈类', String(createBtn.className).indexOf('busy') >= 0, createBtn.className);
  const busyMsg = findByClass(reg.get('#modal-root'), 'modal-msg')[0];
  check('busy: 说清在等什么（虚拟机要先起 guest）',
    !!busyMsg && String(busyMsg.textContent).indexOf('正在创建') >= 0
    && String(busyMsg.textContent).indexOf('guest') >= 0, busyMsg && busyMsg.textContent);
  await pending;
  await sleep(120);
  check('busy: 建完之后进入会话、弹层关掉', state.activeSid === 'new-1', 'activeSid=' + state.activeSid);

  /* ---- 「打开」接线：点了真的进入会话视图 ---- */
  closeModalIfAny();
  let openErrored = null;
  try {
    const firstItem = findByClass(reg.get("#history-list"), "history-item")[0];
    firstItem.onclick();
    await sleep(80);
  } catch (e) { openErrored = e && e.message; }
  check("open: 点「打开」进入会话视图不报错", !openErrored, String(openErrored));
  check("open: 会话被激活、按历史只读回放", state.activeSid === "w" && !!state.sessions.get("w"), "activeSid=" + state.activeSid);
  check("open: 条目本身仍是入口", typeof findByClass(reg.get("#history-list"), "history-item")[0].onclick === "function");


  /* ---- 机器前置齐、虚拟根未装齐：配置面板也要给出补齐动作 ---- */
  closeModalIfAny();
  openConfig("w");
  await sleep(80);
  const cfgUnreadyRoot = reg.get("#modal-root");
  const cfgGuestSel = walk(guestField(cfgUnreadyRoot), []).find((n) => n.tag === "select");
  check("config: 未装齐用例能拿到虚拟根选择器", !!cfgGuestSel);
  cfgGuestSel.value = "windows";
  (cfgGuestSel._h.change || []).forEach((f) => f());
  const cfgGuestSetup = findButton(cfgUnreadyRoot, "补齐虚拟机依赖…");
  check("config: 虚拟根未装齐时给出补齐按钮",
    !!cfgGuestSetup && typeof cfgGuestSetup.onclick === "function");
  cfgGuestSetup.onclick();
  await sleep(120);
  check("config: 点虚拟根旁的补齐打开同一个弹层",
    allText(reg.get("#modal-root")).indexOf("vm.virtiofsd.windows") >= 0);
  closeModalIfAny();

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
