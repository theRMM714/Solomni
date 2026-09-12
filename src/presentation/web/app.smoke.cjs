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
  return {
    textContent: "", innerHTML: "", value: "", className: "", dataset: {},
    classList: { add() {}, remove() {} },
    addEventListener() {}, appendChild() {}, querySelector: () => el(),
    onclick: null,
  };
}
const sandbox = {
  document: { querySelector: () => el(), createElement: () => el(), addEventListener() {} },
  alert: (m) => alerts.push(m),
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
const appPath = path.join(__dirname, "app.js");
try {
  vm.runInNewContext(fs.readFileSync(appPath, "utf8"), sandbox, { filename: "app.js" });
} catch (e) { loadErrors.push("load error: " + e.message); }
setTimeout(() => {
  const ok = alerts.length === 0 && loadErrors.length === 0;
  if (!ok) { console.log("alerts:", JSON.stringify(alerts)); console.log("loadErrors:", JSON.stringify(loadErrors)); }
  console.log(ok ? "FRONTEND-INIT-OK" : "FRONTEND-INIT-FAIL");
  process.exit(ok ? 0 : 1);
}, 300);