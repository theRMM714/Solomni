/* 目的：零依赖截图与自检工具：把真实 index.html + 合成 fixture 喂给 headless Edge，按视图与宽度出图。
 * 管：静态服务与 fetch/时钟注入、CDP 驱动、截图、--audit 的对比度/溢出/焦点/动效/表单间距/选框对齐复算。
 * 不管：视觉决策与 fixture 内容（见 fixtures.js）；不启动产品二进制、不读写 .home。
 * 用法：node tests/screenshots/shots.cjs [--audit] [--after|--tag=名字] [--only=chat,gate] [--widths=375,768,1280] [--scroll=bottom]
 * 输出：.shots/before|after/<view>-<width>.png + .shots/<mode>-audit.json
 */
"use strict";

const http = require("http");
const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawn } = require("child_process");
const fixtures = require("./fixtures.js");

const ROOT = path.join(__dirname, "..", "..");
const ASSETS = path.join(ROOT, "src", "presentation", "web", "assets");
const ARGS = process.argv.slice(2);
const AFTER = ARGS.includes("--after");
const AUDIT = ARGS.includes("--audit");
const TAG = (ARGS.find((a) => a.startsWith("--tag=")) || "").slice(6) || (AFTER ? "after" : "before");
const MODE = TAG;
const OUT = path.join(ROOT, ".shots", MODE);
const ONLY = (ARGS.find((a) => a.startsWith("--only=")) || "").slice(7).split(",").filter(Boolean);
const WIDTHS = ((ARGS.find((a) => a.startsWith("--widths=")) || "").slice(9) || "375,768,1280")
  .split(",").map(Number).filter((n) => n > 0);
// 弹层内容比视口长时，把面板滚到底再拍（用于看动作按钮行）。
const SCROLL_BOTTOM = ARGS.includes("--scroll=bottom");

const VIEWS = [
  { id: "chat", label: "会话：转录、工具回执、状态条" },
  { id: "gate", label: "会话：裁决卡挂着" },
  { id: "readonly", label: "历史回放（只读）" },
  { id: "empty", label: "空态：没有会话" },
  { id: "wizard", label: "新建工作向导" },
  { id: "settings-generate", label: "设置：生成" },
  { id: "settings-models", label: "设置：模型" },
  { id: "settings-agents", label: "设置：agent" },
  { id: "settings-storage", label: "设置：存储" },
  { id: "work", label: "共享区卡片" },
];

const MIME = {
  ".html": "text/html; charset=utf-8", ".css": "text/css; charset=utf-8",
  ".js": "application/javascript; charset=utf-8", ".mjs": "application/javascript; charset=utf-8",
  ".svg": "image/svg+xml", ".png": "image/png", ".json": "application/json; charset=utf-8",
};

function send(res, code, type, body) {
  res.writeHead(code, { "Content-Type": type, "Cache-Control": "no-store" });
  res.end(body);
}

function readAsset(urlPath) {
  const rel = decodeURIComponent(urlPath.slice("/assets/".length));
  const file = path.resolve(ASSETS, rel);
  if (!file.startsWith(ASSETS + path.sep)) return null;
  if (!fs.existsSync(file) || !fs.statSync(file).isFile()) return null;
  return file;
}

const SHIM = [
  "(function () {",
  "  var F = window.__FIXTURES;",
  "  Date.now = function () { return F.NOW; };",
  "  function answer(obj) {",
  "    return Promise.resolve(new Response(JSON.stringify(obj), {",
  "      status: 200, headers: { \"content-type\": \"application/json\" } }));",
  "  }",
  "  window.fetch = function (input) {",
  "    var url = String(input && input.url ? input.url : input);",
  "    var p = new URL(url, location.origin).pathname;",
  "    if (p === \"/api/state\") {",
  "      var empty = new URLSearchParams(location.search).get(\"empty\") === \"1\";",
  "      return answer(empty ? F.emptyState : F.state);",
  "    }",
  "    if (p.indexOf(\"/api/history/\") === 0) {",
  "      var sid = decodeURIComponent(p.slice(13));",
  "      return answer(F.histories[sid] || { meta: { name: sid }, events: [], live: [], head: 0 });",
  "    }",
  "    if (p === \"/api/events\") return new Promise(function () {});",
  "    if (p === \"/api/tiers\") return answer(F.tiers);",
  "    if (/^\\/api\\/sessions\\/[^/]+\\/work$/.test(p)) return answer(F.work);",
  "    if (/^\\/api\\/sessions\\/[^/]+\\/files$/.test(p)) return answer(F.files);",
  "    if (p === \"/api/actions\") return answer({ actions: [] });",
  "    if (p === \"/api/processes\") return answer({ processes: [] });",
  "    if (p === \"/api/storage\") return answer(F.storage);",
  "    if (p === \"/api/setup/status\") return answer({ ready: true, deps: [] });",
  "    return answer({});",
  "  };",
  "  import(\"/assets/app/main.js\").then(function (m) {",
  "    window.__solomni = m.context; window.__shotsReady = true;",
  "  }).catch(function (e) { window.__shotsError = String(e && e.stack || e); });",
  "  window.__prepare = async function (view) {",
  "    function wait(ms) { return new Promise(function (r) { setTimeout(r, ms); }); }",
  "    for (var i = 0; i < 160 && !window.__solomni; i += 1) await wait(50);",
  "    if (!window.__solomni) throw new Error(window.__shotsError || \"boot timeout\");",
  "    var c = window.__solomni, app = c.app, state = c.state, s = null;",
  "    if (view === \"empty\") { app.renderAll(); }" +
  "    else if (view === \"readonly\") { await app.openHistory(\"release-check\"); }",
  "    else if (view === \"wizard\") { app.openWizard(); }",
  "    else if (view.indexOf(\"settings-\") === 0) { app.openSettingsHub(view.slice(9)); }",
  "    else {",
  "      s = state.sessions.get(\"web-art\") || state.sessions.values().next().value;",
  "      if (s) { await app.hydrateHistory(s); app.setActive(s.sid); }",
  "      if (view === \"work\" && s) {",
  "        await app.loadWork(s.sid, true);",
  "      } else if (view === \"chat\" && s) {",
  "        s.pending = null;",
  "        app.renderAll();",
  "      }",
  "    }",
  "    await wait(300);",
  "    if (document.fonts && document.fonts.ready) await document.fonts.ready;",
  "    await new Promise(function (r) { requestAnimationFrame(function () { requestAnimationFrame(r); }); });",
  "    if (view === \"work\" && s) {",
  "      var btn = document.querySelector(\"#session-status button[aria-label=\\\"共享区提交\\\"]\");",
  "      if (btn) app.openWorkCard(s, btn);",
  "    }",
  "    if (window.__shotsScrollBottom) {",
  "      var mb = document.querySelector(\".modal-body\");",
  "      if (mb) mb.scrollTop = mb.scrollHeight;",
  "    }",
  "  };",
  "})();",
].join("\n");

function startServer() {
  const index = fs.readFileSync(path.join(ASSETS, "index.html"), "utf8");
  const injected = index.replace(
    '<script defer src="/assets/md.js"></script>',
    '<script src="/__shots/fixtures.js"></script><script src="/__shots/shim.js"></script>\n' +
    '<script defer src="/assets/md.js"></script>'
  );
  const server = http.createServer((req, res) => {
    const u = new URL(req.url, "http://127.0.0.1");
    if (u.pathname === "/") return send(res, 200, MIME[".html"], injected);
    if (u.pathname === "/__shots/fixtures.js") {
      return send(res, 200, MIME[".js"], "window.__FIXTURES = " + JSON.stringify(fixtures) + ";");
    }
    if (u.pathname === "/__shots/shim.js") {
      const flag = "window.__shotsScrollBottom = " + (SCROLL_BOTTOM ? "true" : "false") + ";\n";
      return send(res, 200, MIME[".js"], flag + SHIM);
    }
    if (u.pathname.startsWith("/assets/")) {
      const file = readAsset(u.pathname);
      if (file) return send(res, 200, MIME[path.extname(file)] || "application/octet-stream", fs.readFileSync(file));
    }
    send(res, 404, "text/plain; charset=utf-8", "not found");
  });
  return new Promise((resolve) => server.listen(0, "127.0.0.1", () => resolve(server)));
}

function edgePath() {
  if (process.env.EDGE_PATH) return process.env.EDGE_PATH;
  const cands = [];
  if (process.platform === "win32") {
    for (const k of ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"]) {
      const base = process.env[k];
      if (base) cands.push(path.join(base, "Microsoft", "Edge", "Application", "msedge.exe"));
    }
  } else if (process.platform === "darwin") {
    cands.push("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge");
  } else {
    cands.push("/usr/bin/microsoft-edge", "/usr/bin/microsoft-edge-stable");
  }
  for (const c of cands) if (fs.existsSync(c)) return c;
  return "msedge";
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function waitFile(file, timeout) {
  const end = Date.now() + timeout;
  return new Promise((resolve, reject) => {
    (function poll() {
      if (fs.existsSync(file)) return resolve(fs.readFileSync(file, "utf8"));
      if (Date.now() > end) return reject(new Error("等待 DevToolsActivePort 超时：" + file));
      setTimeout(poll, 100);
    })();
  });
}

async function getJson(url) {
  const r = await fetch(url);
  if (!r.ok) throw new Error(url + " -> " + r.status);
  return r.json();
}

class Cdp {
  constructor(ws) {
    this.ws = ws;
    this.next = 0;
    this.pending = new Map();
    this.listeners = new Map();
    ws.addEventListener("message", (ev) => {
      const m = JSON.parse(ev.data);
      if (m.id) {
        const p = this.pending.get(m.id);
        if (!p) return;
        this.pending.delete(m.id);
        if (m.error) p.reject(new Error(m.error.message));
        else p.resolve(m.result);
        return;
      }
      for (const fn of this.listeners.get(m.method) || []) fn(m.params);
    });
  }
  on(method, fn) {
    if (!this.listeners.has(method)) this.listeners.set(method, []);
    this.listeners.get(method).push(fn);
  }
  send(method, params) {
    const id = ++this.next;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.ws.send(JSON.stringify({ id, method, params: params || {} }));
    });
  }
}

async function launch() {
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), "solomni-shots-"));
  const exe = edgePath();
  const child = spawn(exe, [
    "--headless=new", "--disable-gpu", "--no-first-run", "--no-default-browser-check",
    "--disable-extensions", "--disable-background-networking", "--disable-sync",
    "--remote-debugging-port=0", "--user-data-dir=" + profile, "about:blank",
  ], { stdio: ["ignore", "ignore", "pipe"] });
  let stderr = "";
  child.stderr.on("data", (d) => { stderr += String(d); });
  const active = await waitFile(path.join(profile, "DevToolsActivePort"), 20000);
  const port = Number(active.split(/\r?\n/)[0]);
  const version = await getJson("http://127.0.0.1:" + port + "/json/version");
  let targets = await getJson("http://127.0.0.1:" + port + "/json/list");
  let page = targets.find((t) => t.type === "page");
  if (!page) {
    const r = await fetch("http://127.0.0.1:" + port + "/json/new?about:blank", { method: "PUT" });
    page = await r.json();
  }
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    ws.addEventListener("open", resolve);
    ws.addEventListener("error", () => reject(new Error("CDP 连接失败")));
  });
  return { child, profile, ws, cdp: new Cdp(ws), stderr: () => stderr, browser: version.Browser };
}
const AUDIT_FN = function () {
  function parse(c) {
    const m = String(c).match(/rgba?\(([^)]+)\)/);
    if (!m) return null;
    const p = m[1].split(",").map(Number);
    return { r: p[0], g: p[1], b: p[2], a: p.length > 3 ? p[3] : 1 };
  }
  function lum(c) {
    const f = (v) => { const x = v / 255; return x <= 0.03928 ? x / 12.92 : Math.pow((x + 0.055) / 1.055, 2.4); };
    return 0.2126 * f(c.r) + 0.7152 * f(c.g) + 0.0722 * f(c.b);
  }
  function ratio(a, b) {
    let l1 = lum(a); let l2 = lum(b);
    if (l1 < l2) { const t = l1; l1 = l2; l2 = t; }
    return (l1 + 0.05) / (l2 + 0.05);
  }
  function bgOf(el) {
    let n = el;
    while (n && n.nodeType === 1) {
      const c = parse(getComputedStyle(n).backgroundColor);
      if (c && c.a > 0.99) return c;
      n = n.parentElement;
    }
    return { r: 18, g: 17, b: 16, a: 1 };
  }
  const sels = [
    ".line .md", ".line .body", ".who", ".hname", ".hmode", ".panel-note",
    ".settings-nav-item", ".status-chip-text", ".gate-card .q", ".gate-card .hint",
    "#input", ".reg-sub", ".btn-primary", ".status-chip",
  ];
  const samples = [];
  for (const sel of sels) {
    const el = document.querySelector(sel);
    if (!el) continue;
    const cs = getComputedStyle(el);
    const fg = parse(cs.color);
    if (!fg) continue;
    const fs = parseFloat(cs.fontSize);
    const bold = parseInt(cs.fontWeight, 10) >= 600;
    const large = fs >= 24 || (bold && fs >= 18.66);
    samples.push({
      sel: sel, size: fs, need: large ? 3 : 4.5,
      ratio: Math.round(ratio(fg, bgOf(el)) * 100) / 100,
      color: cs.color, bg: "rgb(" + bgOf(el).r + ", " + bgOf(el).g + ", " + bgOf(el).b + ")",
    });
  }
  const first = document.querySelector("button:not([disabled]), [href], input, textarea, select, summary");
  let focus = null;
  if (first) {
    first.focus();
    const cs = getComputedStyle(first);
    focus = { tag: first.tagName, outline: cs.outlineStyle + " " + cs.outlineWidth, shadow: cs.boxShadow !== "none" };
  }
  const page = document.querySelector(".settings-page:not(.hidden)");
  const foot = document.querySelector(".settings-foot");
  let footerGap = null;
  if (page && foot && foot.getClientRects().length && page.lastElementChild) {
    footerGap = Math.round(foot.getBoundingClientRect().top - page.lastElementChild.getBoundingClientRect().bottom);
  }
  let checkOffset = null;
  const chk = document.querySelector(".chk");
  if (chk) {
    const box = chk.querySelector("input");
    const txt = chk.querySelector(".chk-text");
    if (box && txt) {
      const line = parseFloat(getComputedStyle(txt).lineHeight);
      const br = box.getBoundingClientRect();
      const tr = txt.getBoundingClientRect();
      checkOffset = Math.round((br.top + br.height / 2 - (tr.top + line / 2)) * 10) / 10;
    }
  }
  return {
    overflow: document.documentElement.scrollWidth - window.innerWidth,
    samples: samples,
    focus: focus,
    animations: document.getAnimations().filter((a) => a.playState === "running").length,
    footerGap: footerGap,
    checkOffset: checkOffset,
  };
};

async function main() {
  fs.mkdirSync(OUT, { recursive: true });
  const server = await startServer();
  const port = server.address().port;
  const base = "http://127.0.0.1:" + port + "/";
  const browser = await launch();
  const cdp = browser.cdp;
  console.log("Edge：" + browser.browser + "；服务：" + base + "；输出：" + path.relative(ROOT, OUT));

  const errors = [];
  cdp.on("Runtime.consoleAPICalled", (p) => {
    if (p.type !== "error") return;
    errors.push(p.args.map((a) => a.value || a.description || "").join(" "));
  });
  cdp.on("Runtime.exceptionThrown", (p) => {
    errors.push((p.exceptionDetails && p.exceptionDetails.exception && p.exceptionDetails.exception.description) || "exception");
  });
  await cdp.send("Page.enable");
  await cdp.send("Runtime.enable");

  async function evaluate(expression) {
    const r = await cdp.send("Runtime.evaluate", { expression: expression, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) {
      const d = r.exceptionDetails;
      throw new Error((d.exception && d.exception.description) || d.text || "evaluate failed");
    }
    return r.result.value;
  }

  async function ready() {
    for (let i = 0; i < 200; i += 1) {
      try { if (await evaluate("window.__shotsReady === true")) return; } catch (e) { /* 导航中 */ }
      let err = "";
      try { err = await evaluate("window.__shotsError || \"\""); } catch (e2) { err = ""; }
      if (err) throw new Error(err);
      await sleep(100);
    }
    throw new Error("页面初始化超时");
  }

  const audit = {};
  const views = ONLY.length ? VIEWS.filter((v) => ONLY.includes(v.id)) : VIEWS;
  for (const view of views) {
    for (const width of WIDTHS) {
      const mobile = width < 600;
      const height = mobile ? 812 : 900;
      await cdp.send("Emulation.setDeviceMetricsOverride", {
        width: width, height: height, deviceScaleFactor: 1, mobile: mobile,
      });
      await cdp.send("Emulation.setEmulatedMedia", {
        features: [{ name: "prefers-reduced-motion", value: "no-preference" }],
      });
      const url = base + "?view=" + encodeURIComponent(view.id) + (view.id === "empty" ? "&empty=1" : "");
      errors.length = 0;
      await cdp.send("Page.navigate", { url: url });
      await ready();
      await evaluate("window.__prepare(" + JSON.stringify(view.id) + ")");
      await sleep(150);
      const shot = await cdp.send("Page.captureScreenshot", { format: "png", fromSurface: true });
      const name = view.id + "-" + width + ".png";
      fs.writeFileSync(path.join(OUT, name), Buffer.from(shot.data, "base64"));
      const bad = errors.slice();
      if (bad.length) console.log("！" + name + " 控制台错误：" + bad.join(" | "));
      console.log("· " + name);
      if (AUDIT) {
        audit[name] = await evaluate("(" + AUDIT_FN.toString() + ")()");
        audit[name].errors = bad;
      }
    }
  }

  if (AUDIT) {
    await cdp.send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false });
    await cdp.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-reduced-motion", value: "reduce" }] });
    await cdp.send("Page.navigate", { url: base + "?view=chat" });
    await ready();
    await evaluate("window.__prepare(\"chat\")");
    const shot = await cdp.send("Page.captureScreenshot", { format: "png", fromSurface: true });
    fs.writeFileSync(path.join(OUT, "chat-1280-reduced.png"), Buffer.from(shot.data, "base64"));
    audit["chat-1280-reduced"] = await evaluate("(" + AUDIT_FN.toString() + ")()");

    const file = path.join(ROOT, ".shots", MODE + "-audit.json");
    fs.writeFileSync(file, JSON.stringify(audit, null, 2) + "\n");
    let fails = 0;
    for (const [name, a] of Object.entries(audit)) {
      const bad = a.samples.filter((s) => s.ratio < s.need);
      const msgs = [];
      if (a.overflow > 1) msgs.push("横向溢出 " + a.overflow + "px");
      if (a.footerGap !== null && a.footerGap < 12) msgs.push("表单与保存栏间距 " + a.footerGap + "px < 12px");
      if (a.checkOffset !== null && Math.abs(a.checkOffset) > 1.5) msgs.push("选框中心偏离标题行 " + a.checkOffset + "px");
      for (const s of bad) msgs.push(s.sel + " 对比度 " + s.ratio + ":1 < " + s.need);
      if (!a.focus || a.focus.outline === "none 0px") msgs.push("首个可聚焦元素没有可见轮廓");
      if (name.endsWith("-reduced") && a.animations > 0) msgs.push("reduced-motion 下仍有 " + a.animations + " 个动画");
      if (msgs.length) { fails += 1; console.log("✗ " + name + "：" + msgs.join("；")); }
    }
    console.log(fails ? "审计：" + fails + " 张有问题（见 " + path.relative(ROOT, file) + "）" : "审计：全部通过");
  }

  browser.ws.close();
  try { browser.child.kill(); } catch (e) { /* 已退出 */ }
  server.close();
  await sleep(300);
  try { fs.rmSync(browser.profile, { recursive: true, force: true }); } catch (e) { /* 占用中由系统清理 */ }
  console.log("SHOTS-OK " + views.length * WIDTHS.length);
}

main().catch((e) => { console.error("SHOTS-FAIL " + (e && e.stack || e)); process.exit(1); });
