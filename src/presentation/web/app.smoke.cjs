/* 前端冒烟测试（桩 DOM + 桩 fetch）：跑 app.js 的初始化路径，暴露引用错误。
 * 用法：node src/presentation/web/app.smoke.cjs   （输出 FRONTEND-INIT-OK / FAIL）
 * 为什么需要：app.js 是 no-build 原生 JS，浏览器只在运行时才发现未定义变量，
 * 这类错误会让整个界面初始化失败（例：renderSidebar 误引用外部作用域变量）。
 */
const fs = require("fs");
const path = require("path");
const vm = require("vm");
const alerts = [];
const loadErrors = [];
function el() {
  const node = {
    textContent: "", innerHTML: "", value: "", className: "", dataset: {},
    children: [],
    classList: { add() {}, remove() {} },
    addEventListener() {}, querySelector: () => el(),
    appendChild(c) { node.children.push(c); },
    onclick: null,
  };
  return node;
}
// notice/confirmBox 要能跑起来：桩一个最小 DOM（含 #notice-root）。
const noticeRoot = el();
const sandbox = {
  document: { querySelector: (s) => (s === "#notice-root" ? noticeRoot : el()), createElement: () => el(), addEventListener() {} },
  // 原生弹窗是红线：换自研弹窗之后，这里被调用一次就算失败。
  alert: (m) => alerts.push(m),
  confirm: (m) => { alerts.push(m); return true; },
  console, JSON, Promise, Map, Set, Error, Object, Array, String, Number, Boolean, Math, Date,
  setTimeout, clearTimeout,
  fetch: async (url) => {
    if (String(url).indexOf("/api/state") === 0) {
      return { ok: true, json: async () => ({ modules: [{ id: "research", brief: "调研" }], providers: [], rejected: ["broken-mod"] }) };
    }
    throw new Error("poll-not-stubbed");
  },
};
sandbox.window = sandbox;
sandbox.globalThis = sandbox;
try {
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, "md.js"), "utf8"), sandbox, { filename: "md.js" });
} catch (e) { loadErrors.push("md load error: " + e.message); }
const appPath = path.join(__dirname, "app.js");
try {
  vm.runInNewContext(fs.readFileSync(appPath, "utf8"), sandbox, { filename: "app.js" });
} catch (e) { loadErrors.push("load error: " + e.message); }
setTimeout(() => {
  // 正向观察面：自研居中弹窗真的渲染出来（只断言"原生没被调用"是不够的——那样全删掉也能过）。
  let rendered = false;
  let tierWarned = false;
  if (!loadErrors.length) {
    try {
      // 记的是虚拟机档、本机现在承载不了：打开前要给提示（不拦打开）。
      vm.runInNewContext(
        "state.history = [{ name: 'w', mode: 'single', ts: 1, done: true, tier: 'vm', tier_ready: false, tier_missing: ['本机虚拟机监视器不可用'] }];" +
          "warnIfTierUnavailable('w');",
        sandbox
      );
      tierWarned = noticeRoot.children.length > 0;
      noticeRoot.innerHTML = ''; noticeRoot.children = [];
      vm.runInNewContext("notice('测试', '内容', 'err')", sandbox);
      // 桩 DOM 不实现 innerHTML 解析，所以按"打开了 + 挂上了子节点"判断渲染。
      rendered = noticeRoot.className.indexOf("open") >= 0 && noticeRoot.children.length > 0;
    } catch (e) { loadErrors.push("notice 调用失败：" + e.message); }
  }
  const ok = alerts.length === 0 && loadErrors.length === 0 && rendered && tierWarned;
  if (!ok) {
    console.log("alerts（原生弹窗被调用的次数，应为 0）:", JSON.stringify(alerts));
    console.log("notice 渲染:", rendered, "| 虚拟机档不可用提示:", tierWarned);
    console.log("loadErrors:", JSON.stringify(loadErrors));
  }
  console.log(ok ? "FRONTEND-INIT-OK" : "FRONTEND-INIT-FAIL");
  process.exit(ok ? 0 : 1);
}, 300);