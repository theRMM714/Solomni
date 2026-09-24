/* 前端判据：弹窗真的建得起来 + 行解析/按序应用这两条纯逻辑。
 * 用法：node src/presentation/web/modals.smoke.cjs   （输出 FRONTEND-MODALS-OK / FAIL）
 * 为什么需要：连续几个 bug 都在前端渲染层，而 Rust 侧与 HTTP 能力面碰不到 DOM——只能靠肉眼发现。
 * 这里的桩 DOM **会拒绝非节点**（真实 DOM 的 appendChild 就会抛）：
 * numberInput 返回的是 {input, wrap}，误把它当元素传给 field() 会让整个表单建不起来——
 * 真机上就是"点编辑下面无法展开修改"。桩 DOM 若不校验，这类误用就永远照不出来。
 */
const fs = require("fs");
const path = require("path");
const vm = require("vm");
const loadErrors = [];

function el(tag) {
  const node = {
    tagName: (tag || "div").toUpperCase(),
    textContent: "", value: "", className: "", dataset: {}, style: {},
    type: "", checked: false, disabled: false, title: "", placeholder: "",
    children: [],
    classList: { add() {}, remove() {}, contains: () => false },
    addEventListener() {},
    querySelector: () => el(),
    appendChild(c) {
      // 真实 DOM 会抛：appendChild 只接受节点。桩必须一样严，否则误用照不出来。
      if (!c || typeof c.appendChild !== "function") {
        throw new Error("appendChild 收到非节点（真实 DOM 会抛）：" + JSON.stringify(c === null ? null : Object.keys(c)));
      }
      node.children.push(c);
      return c;
    },
    onclick: null,
  };
  // 真实 DOM：innerHTML = '' 会清空子节点。桩不模拟这一条，第二次打开弹窗时旧内容还在，
  // 判据就会读到上一个弹窗（测试自己发现过这个不忠实）。
  let html = "";
  Object.defineProperty(node, "innerHTML", {
    get: () => html,
    set: (v) => {
      html = v;
      if (v === "") node.children = [];
    },
  });
  return node;
}

const roots = new Map();
const sandbox = {
  document: {
    querySelector: (s) => {
      if (!roots.has(s)) roots.set(s, el());
      return roots.get(s);
    },
    createElement: (t) => el(t),
    addEventListener() {},
  },
  alert: () => { throw new Error("原生 alert 是红线"); },
  confirm: () => { throw new Error("原生 confirm 是红线"); },
  console, JSON, Promise, Map, Set, Error, Object, Array, String, Number, Boolean, Math, Date, RegExp,
  setTimeout, clearTimeout,
  fetch: async (url) => {
    if (String(url).indexOf("/api/state") === 0) {
      return {
        ok: true,
        json: async () => ({
          modules: [{ id: "render", brief: "出报告" }],
          providers: [{ id: "p1", base_url: "http://x" }],
          models: [{ id: "m1", name: "M", api_model: "m", provider: "p1", note: "", tools: "native", context: 32000, is_core: true }],
          agents: [],
          settings: { streaming: true, show_reasoning: true, llm_timeout_secs: 300, discuss_remind_cap: 3, compact_at_percent: 70 },
          history: [],
        }),
      };
    }
    return { ok: true, json: async () => ({}) };
  },
};
sandbox.window = sandbox;
sandbox.globalThis = sandbox;

function load(file) {
  try {
    vm.runInNewContext(fs.readFileSync(path.join(__dirname, file), "utf8"), sandbox, { filename: file });
  } catch (e) { loadErrors.push(file + " 载入失败：" + e.message); }
}
load("md.js");
load("app.js");

// 递归收集树里的节点（判"表单真的建出来了"）。
function walk(node, out) {
  out.push(node);
  for (const c of node.children || []) walk(c, out);
  return out;
}
const inputsIn = (node) => walk(node, []).filter((n) => n.tagName === "INPUT");

const results = [];
function check(name, fn) {
  try {
    const ok = fn();
    results.push([name, !!ok, ""]);
  } catch (e) {
    results.push([name, false, e.message]);
  }
}

if (!loadErrors.length) {
  // ① 模型登记弹窗：建得起来，且有"上下文窗口"数字输入（此前误用对象让整个表单炸掉）。
  check("模型登记弹窗建得起来且含窗口输入", () => {
    vm.runInNewContext("openModelsModal()", sandbox);
    const body = roots.get("#modal-root").children[0].children[0].children[1];
    const nums = inputsIn(body).filter((i) => i.type === "number");
    return nums.length >= 1;
  });
  // ② 基本设置弹窗：超时 / 提醒次数 / 压缩阈值三项都在（都是数字输入）。
  //（调用次数上限已撤掉：模型调用没有次数上限，只有提醒次数。）
  check("基本设置弹窗含超时、提醒与压缩三项", () => {
    vm.runInNewContext("openSettingsModal()", sandbox);
    const body = roots.get("#modal-root").children[0].children[0].children[1];
    const nums = inputsIn(body).filter((i) => i.type === "number");
    return nums.length >= 3;
  });
  // ③ 回合分隔行要可见：解析成系统分隔行，而不是 who=标签、text 为空的空行。
  check("回合分隔行解析成系统行", () => {
    const out = vm.runInNewContext('parseLine("[回合 t2｜第 0 轮]", false)', sandbox);
    return out.length === 1 && out[0].cls === "sys system" && out[0].text.indexOf("回合") >= 0;
  });
  // ④ 轮次分隔行同样（两种标签都要认）。
  check("轮次分隔行解析成系统行", () => {
    const out = vm.runInNewContext('parseLine("[轮次 2]", false)', sandbox);
    return out.length === 1 && out[0].cls === "sys system";
  });
  // ⑤ 乱序批按序补洞：先到 seq=2、后到 seq=1，两条都要应用且顺序正确。
  check("乱序批按 seq 补洞后按序应用", () => {
    const got = vm.runInNewContext(`
      (() => {
        state.sessions.set("s1", { sid: "s1", lines: [], live: [], pending: null, busy: false, done: false, fold: {}, scroll: {} });
        appliedSeq = 0;
        pendingBatches.clear();
        applyBatch(2, "s1", [{ type: "notice", text: "第二" }]);
        const afterTwo = state.sessions.get("s1").lines.length;
        applyBatch(1, "s1", [{ type: "notice", text: "第一" }]);
        const lines = state.sessions.get("s1").lines.map((l) => l.text);
        return { afterTwo, lines, applied: appliedSeq };
      })()
    `, sandbox);
    // seq=2 先到时不能应用（缺口没补），补上 seq=1 后两条按序都进来。
    return got.afterTwo === 0 && got.applied === 2 &&
      got.lines.length === 2 && got.lines[0].indexOf("第一") >= 0 && got.lines[1].indexOf("第二") >= 0;
  });
  // ⑥ 节点回报要显示成一行"完成"（此前只有"开工"，用户看不到节点交回来了）。
  check("节点回报显示成完成行", () => {
    const snippet = [
      "(() => {",
      '  state.sessions.set("s2", { sid: "s2", lines: [], live: [], pending: null, busy: false, done: false, fold: {}, scroll: {} });',
      '  absorb(state.sessions.get("s2"), { type: "report", id: "n1", rework: 0, text: "语料抽好了\\n第二行不该显示" });',
      '  return state.sessions.get("s2").lines.map((l) => l.text);',
      "})()",
    ].join("\n");
    const got = vm.runInNewContext(snippet, sandbox);
    return got.length === 1 && got[0].indexOf("[节点] 完成：n1") === 0 &&
      got[0].indexOf("语料抽好了") > 0 && got[0].indexOf("第二行") < 0;
  });
}

const failed = results.filter((r) => !r[1]);
for (const [name, ok, why] of results) {
  console.log((ok ? "PASS " : "FAIL ") + name + (ok ? "" : " :: " + why));
}
if (loadErrors.length) console.log("loadErrors:", JSON.stringify(loadErrors));
const ok = failed.length === 0 && loadErrors.length === 0;
console.log(ok ? "FRONTEND-MODALS-OK" : "FRONTEND-MODALS-FAIL");
process.exit(ok ? 0 : 1);