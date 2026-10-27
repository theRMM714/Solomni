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
    children: [], _h: {},
    classList: { add() {}, remove() {}, contains: () => false },
    addEventListener(t, f) { (node._h[t] = node._h[t] || []).push(f); },
    setAttribute() {},
    querySelector: () => el(),
    appendChild(c) {
      // 真实 DOM 会抛：appendChild 只接受节点。桩必须一样严，否则误用照不出来。
      if (!c || typeof c.appendChild !== "function") {
        throw new Error("appendChild 收到非节点（真实 DOM 会抛）：" + JSON.stringify(c === null ? null : Object.keys(c)));
      }
      c.parentNode = node;
      node.children.push(c);
      return c;
    },
    removeChild(c) {
      const i = node.children.indexOf(c);
      if (i >= 0) node.children.splice(i, 1);
      if (c) c.parentNode = null;
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
    if (String(url).indexOf("/api/processes") === 0) {
      return {
        ok: true,
        json: async () => ({
          procs: [
            { pid: 4242, alive: true, purpose: "guest", owner: "w-1", since: 1 },
            { pid: 4343, alive: false, purpose: "share:virtiofs", owner: "/work", since: 2 },
          ],
        }),
      };
    }
    if (String(url).indexOf("/api/state") === 0) {
      return {
        ok: true,
        json: async () => ({
          modules: [{ id: "render", brief: "出报告" }],
          providers: [{ id: "p1", base_url: "http://x" }],
          models: [{ id: "m1", name: "M", api_model: "m", provider: "p1", note: "", tools: "native", context: 32000, is_core: true }],
          agents: [],
          settings: { streaming: true, show_reasoning: true, llm_timeout_secs: 300, discuss_remind_cap: 3, compact_at_percent: 70, max_guests: 2, sandbox: { policy: { network: false, enforce: { filesystem: "require", network: "require", process_tree: "require", env_whitelist: "require" }, on_unavailable: "ask", windows: { write_acl: false }, host_read_roots: [], deny: [] }, defaults: { allow_read: [], allow_write: [], granularity: "ask", ask: [] } } },
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
load("ui.js");
load("app.js");

// 递归收集树里的节点（判"表单真的建出来了"）。
function walk(node, out) {
  out.push(node);
  for (const c of node.children || []) walk(c, out);
  return out;
}
const inputsIn = (node) => walk(node, []).filter((n) => n.tagName === "INPUT");
const hubBody = () => walk(roots.get("#modal-root"), []).find((n) => String(n.className) === "settings-hub");
const openPage = (id) => {
  const hub = hubBody();
  const nav = walk(hub, []).find((n) => String(n.className) === "settings-nav");
  const btn = walk(nav, []).find((n) => n.tagName === "BUTTON" && n.dataset && n.dataset.id === id);
  if (btn) btn.onclick();
  return walk(hub, []).find((n) => String(n.className).indexOf("settings-page") === 0 && n.dataset && n.dataset.id === id);
};
const optionValues = (page) => walk(page, []).filter((n) => n.tagName === "OPTION").map((n) => n.value);
// 每个同步用例从同一份登记处起步（前一个用例可能改过全局 state）。
function seedRegistry() {
  vm.runInNewContext(`
    state.providers = [{ id: "p1", base_url: "http://x" }];
    state.models = [{ id: "m1", name: "M", api_model: "m", provider: "p1", note: "", tools: "native", context: 0, is_core: true }];
    state.core = "m1";
    state.agents = [];
    notifyRegistryChange();
  `, sandbox);
}

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
  // ② 设置中心：分页后生成页只有三项数字（超时 / 提醒 / 压缩），同时上台数在资源页。
  check("设置中心：生成页含超时、提醒、压缩", () => {
    vm.runInNewContext("openSettingsModal()", sandbox);
    const page = openPage("generate");
    const nums = inputsIn(page).filter((i) => i.type === "number");
    return nums.length >= 3;
  });
  // ⑥ 同时上台数要随保存一起提交（后端 set_settings 收这个字段）；"现在在跑什么"同页。
  check("设置中心：资源页含同时上台数与进程区块", () => {
    vm.runInNewContext("openSettingsModal()", sandbox);
    const page = openPage("resources");
    const nums = inputsIn(page).filter((i) => i.type === "number");
    const nodes = walk(page, []);
    const title = nodes.find((n) => n.className === "wf-label" && n.textContent === "现在在跑什么");
    const log = nodes.find((n) => n.className === "cfg-log");
    return nums.length >= 1 && !!title && !!log;
  });
  check("设置中心：底栏保存接线", () => {
    vm.runInNewContext("openSettingsModal()", sandbox);
    openPage("generate");
    const hub = hubBody();
    const save = walk(hub, []).find((n) => n.tagName === "BUTTON" && n.textContent === "保存");
    return !!save && typeof save.onclick === "function";
  });
  check("设置中心：沙箱页含硬约束与网络围栏", () => {
    vm.runInNewContext("openSettingsModal()", sandbox);
    const page = openPage("sandbox");
    const text = walk(page, []).map((n) => String(n.textContent || "")).join(" ");
    return text.indexOf("全局硬约束") >= 0 && text.indexOf("网络围栏") >= 0;
  });
  // 登记处同步：页面已建好并缓存后，新登记的东西必须立刻出现在它的列表 / 下拉里。
  check("模型页先建好，登记供应商后下拉与发现区即时更新", () => {
    seedRegistry();
    vm.runInNewContext("openSettingsModal()", sandbox);
    const page = openPage("models");
    vm.runInNewContext(
      'state.providers = state.providers.concat([{ id: "p2", base_url: "http://y" }]); notifyRegistryChange();',
      sandbox
    );
    const text = walk(page, []).map((n) => String(n.textContent || "")).join(" ");
    return optionValues(page).indexOf("p2") >= 0 && text.indexOf("p2") >= 0;
  });
  check("模型页先建好，登记模型后列表与核心下拉即时更新", () => {
    seedRegistry();
    vm.runInNewContext("openSettingsModal()", sandbox);
    const page = openPage("models");
    vm.runInNewContext(
      'state.models = state.models.concat([{ id: "m2", name: "M2", api_model: "m2", provider: "p1", note: "", tools: "envelope", context: 0, is_core: false }]); notifyRegistryChange();',
      sandbox
    );
    const text = walk(page, []).map((n) => String(n.textContent || "")).join(" ");
    return optionValues(page).indexOf("m2") >= 0 && text.indexOf("M2") >= 0;
  });
  check("设为核心后模型行的「核心」徽标即时移动", () => {
    seedRegistry();
    vm.runInNewContext("openSettingsModal()", sandbox);
    const page = openPage("models");
    vm.runInNewContext(
      'state.models = state.models.concat([{ id: "m2", name: "M2", api_model: "m2", provider: "p1", note: "", tools: "envelope", context: 0, is_core: false }]); state.core = "m2"; state.models = state.models.map((m) => ({ ...m, is_core: m.id === "m2" })); notifyRegistryChange();',
      sandbox
    );
    const idNode = walk(page, []).find((n) => n.className === "reg-id" && String(n.textContent) === "m2");
    return !!idNode && (idNode.children || []).some((c) => c.className === "reg-core");
  });
  check("登记处同步不重建表单（正在填的输入不丢）", () => {
    seedRegistry();
    vm.runInNewContext("openSettingsModal()", sandbox);
    const page = openPage("models");
    const idInput = walk(page, []).find((n) => n.tagName === "INPUT" && n.placeholder === "id");
    idInput.value = "正在填的 id";
    vm.runInNewContext(
      'state.providers = state.providers.concat([{ id: "p3", base_url: "http://z" }]); notifyRegistryChange();',
      sandbox
    );
    return idInput.value === "正在填的 id";
  });
  check("agent 页新建弹层：登记模型后默认模型下拉即时更新", () => {
    seedRegistry();
    vm.runInNewContext("openSettingsModal()", sandbox);
    const page = openPage("agents");
    const add = walk(page, []).find((n) => n.tagName === "BUTTON" && n.textContent === "+ 新建 agent");
    if (!add) return false;
    add.onclick();
    vm.runInNewContext(
      'state.models = state.models.concat([{ id: "m3", name: "M3", api_model: "m3", provider: "p1", note: "", tools: "envelope", context: 0, is_core: false }]); notifyRegistryChange();',
      sandbox
    );
    return optionValues(roots.get("#modal-root")).indexOf("m3") >= 0;
  });
  check("模型表单说明 0 = 不填且自动压缩按 128K 兜底", () => {
    vm.runInNewContext("openModelsModal()", sandbox);
    const body = roots.get("#modal-root").children[0].children[0].children[1];
    const text = walk(body, []).map((n) => String(n.textContent || "")).join(" ");
    return text.indexOf("0 或留空 = 不填") >= 0 && text.indexOf("128K") >= 0;
  });
  // ③ 回合分隔行要可见：解析成系统分隔行，而不是 who=标签、text 为空的空行。
  check("回合分隔行解析成系统行", () => {
    const out = vm.runInNewContext(
      "lineParts({ kind: 'system', line: '[回合 t2｜第 0 轮]' })",
      sandbox
    );
    return out.length === 1 && out[0].cls === "sys system" && out[0].text.indexOf("回合") >= 0;
  });
  // ④ 轮次分隔行同样（两种标签都要认）。
  check("轮次分隔行解析成系统行", () => {
    const out = vm.runInNewContext(
      "lineParts({ kind: 'round', speaker: '轮次', line: '2' })",
      sandbox
    );
    return out.length === 1 && out[0].cls === "sys system";
  });
  // ⑤ 批次按 seq 应用，游标**单调前进**（事实只有一条来路：事件台）。
  check("批次按 seq 应用，游标只前进", () => {
    const got = vm.runInNewContext(`
      (() => {
        state.sessions.set("s1", { sid: "s1", lines: [], live: [], pending: null, sending: false, done: false, fold: {}, scroll: {} });
        appliedSeq = 0;
        applyBatch(1, "s1", [{ type: "notice", text: "第一" }]);
        applyBatch(2, "s1", [{ type: "notice", text: "第二" }]);
        const lines = state.sessions.get("s1").lines.map((l) => l.text);
        return { lines, applied: appliedSeq };
      })()
    `, sandbox);
    return got.applied === 2 && got.lines.length === 2 &&
      got.lines[0].indexOf("第一") >= 0 && got.lines[1].indexOf("第二") >= 0;
  });
  // ⑥ 节点回报要与"开工"配成同一张完成卡（不再是一条系统文本行）。
  check("节点回报配成完成卡", () => {
    const snippet = [
      "(() => {",
      '  state.sessions.set("s2", { sid: "s2", lines: [], live: [], pending: null, sending: false, done: false, fold: {}, scroll: {} });',
      '  absorb(state.sessions.get("s2"), { type: "node_started", node: "n1", sid: "w--a", assignee: "甲" });',
      '  absorb(state.sessions.get("s2"), { type: "report", id: "n1", rework: 2, text: "语料抽好了\\n第二行也留着" });',
      '  const l = state.sessions.get("s2").lines[0] || {};',
      '  return { n: state.sessions.get("s2").lines.length, cls: l.cls, state: l.state, id: l.node && l.node.id, sid: l.node && l.node.sid, rework: l.rework, report: l.report };',
      "})()",
    ].join("\n");
    const got = vm.runInNewContext(snippet, sandbox);
    return got.n === 1 && got.cls === "node" && got.state === "done" &&
      got.id === "n1" && got.sid === "w--a" && got.rework === 2 &&
      String(got.report).indexOf("语料抽好了") === 0;
  });
  // ⑦ 模块与服务面板：只列模块；服务带开关与运行态；功能给「使用」，参数在子卡片里。
  check("模块与服务面板只列模块、功能走「使用」子卡片", () => {
    const got = vm.runInNewContext(`
      (() => {
        const ctx = openModal('测试', () => {});
        renderModulesPanel(ctx,
          [{ id: 'research', brief: '调研', enabled: true, services: [{ name: 'fs', enabled: false }] },
           { id: 'lonely', brief: '只有 system', enabled: false, services: [] }],
          [{ module: 'research', name: 'fs', adapter: 'mcp', state: '未启动' }],
          [{ id: 'set_module_enabled', desc: '模块开关', params: [] },
           { id: 'control_resident', desc: '服务开关', params: [] },
           { id: 'module.research.grep', desc: '搜索', params: [] }],
          async () => {});
        const walk = (n, out) => { out.push(n); for (const c of (n.children || [])) walk(c, out); return out; };
        const nodes = walk($('#modal-root'), []);
        const texts = nodes.map((n) => n.textContent || '');
        const labels = nodes.filter((n) => n.tagName === 'BUTTON').map((n) => n.textContent);
        const listed = { use: labels.indexOf('使用') >= 0, run: labels.indexOf('运行') >= 0 };
        openUseCard({ id: 'module.research.grep', desc: '搜索', params: [{ name: 'q', ty: 'string' }] });
        const after = walk($('#modal-root'), []).filter((n) => n.tagName === 'BUTTON').map((n) => n.textContent);
        return {
          research: texts.some((t) => t.indexOf('research') >= 0),
          service: texts.some((t) => t.indexOf('fs · mcp · 未启动') >= 0),
          use: listed.use,
          noRunInList: !listed.run,
          noSysTools: texts.every((t) => t.indexOf('set_module_enabled') < 0 && t.indexOf('control_resident') < 0),
          lonely: texts.some((t) => t.indexOf('只能经 AI 使用') >= 0),
          subRun: after.indexOf('运行') >= 0,
        };
      })()
    `, sandbox);
    return got.research && got.service && got.use && got.noRunInList && got.noSysTools && got.lonely && got.subRun;
  });
  // ⑧ 核心默认块：设置模型页顶部有下拉与保存，容器须给二者留间距（保存按钮紧贴下拉的回归）。
  check("核心默认块：下拉与保存按钮在设置模型页且有间距", () => {
    seedRegistry();
    vm.runInNewContext("openSettingsModal()", sandbox);
    const page = openPage("models");
    const core = walk(page, []).find((n) => String(n.className) === "settings-core");
    const save = core && walk(core, []).find((n) => n.tagName === "BUTTON" && n.textContent === "保存");
    const label = core && walk(core, []).find((n) => n.className === "wf-label" && n.textContent === "核心 AI 使用的模型");
    const css = fs.readFileSync(path.join(__dirname, "style.css"), "utf8");
    const rule = (css.match(/\.settings-core\s*\{[^}]*\}/) || [""])[0];
    const spaced = /display:\s*flex/.test(rule) && /flex-direction:\s*column/.test(rule) &&
      /gap:\s*(12px|var\(--sp-3\))/.test(rule);
    return !!(core && save && label && spaced);
  });
  // 弹层栈：关上层保留下层；关全部才清空（向导里开 agent / 模块选择器不丢状态的基础）。
  check("弹层叠层：关上层保留下层，关全部才清空", () => {
    const got = vm.runInNewContext(`
      (() => {
        closeModal();
        const root = document.querySelector('#modal-root');
        openModal('下层', (c) => { c.body.appendChild(ui.el('div', 'lower', 'LOWER')); });
        const top = openModalLayer('上层', (c) => { c.body.appendChild(ui.el('div', 'upper', 'UPPER')); });
        const walk = (n, out) => { out.push(n); for (const c of (n.children || [])) walk(c, out); return out; };
        const lowerBefore = walk(root, []).some((n) => n.className === 'lower');
        const upperBefore = walk(root, []).some((n) => n.className === 'upper');
        top.close();
        const lowerAfter = walk(root, []).some((n) => n.className === 'lower');
        const upperAfter = walk(root, []).some((n) => n.className === 'upper');
        closeModal();
        const cleared = root.children.length === 0;
        return { lowerBefore, upperBefore, lowerAfter, upperAfter, cleared };
      })()
    `, sandbox);
    return got.lowerBefore && got.upperBefore && got.lowerAfter && !got.upperAfter && got.cleared;
  });
  // 模块搜索只认名字（id）与简介（brief），不串 runtimes 等字段。
  check("模块搜索只匹配名字与简介", () => {
    const got = vm.runInNewContext(`
      (() => {
        state.modules = [
          { id: 'research', brief: '调研', runtimes: ['python'] },
          { id: 'db', brief: '数据库', runtimes: ['ruby'] }
        ];
        return {
          byId: moduleMatches(state.modules[0], 'res'),
          byBrief: moduleMatches(state.modules[1], '数据'),
          notRuntime: moduleMatches(state.modules[0], 'python'),
          multi: moduleMatches(state.modules[0], 'res 调研'),
          blank: moduleMatches(state.modules[0], '')
        };
      })()
    `, sandbox);
    return got.byId && got.byBrief && !got.notRuntime && got.multi && got.blank;
  });
  // 模块选择器：已加入置顶、不参与搜索；可选列表排除已加入；点卡片可来回切换。
  check("模块选择器：已加入置顶且可选列表排除已加入", () => {
    const got = vm.runInNewContext(`
      (() => {
        state.modules = [
          { id: 'research', brief: '调研' },
          { id: 'db', brief: '数据库' }
        ];
        closeModal();
        openModulePicker({ selected: ['research'], onDone: () => {} });
        const root = document.querySelector('#modal-root');
        const walk = (n, out) => { out.push(n); for (const c of (n.children || [])) walk(c, out); return out; };
        const tileOf = (id) => {
          const idNode = walk(root, []).find((n) => String(n.className) === 'mod-tile-id' && String(n.textContent) === id);
          let t = idNode;
          while (t && String(t.className).split(' ').indexOf('mod-tile') < 0) t = t.parentNode;
          return t;
        };
        const isAdded = (t) => !!t && String(t.className).split(' ').indexOf('is-added') >= 0;
        const before = {
          added: isAdded(tileOf('research')),
          availableHasDb: !!tileOf('db'),
          availableHasResearch: !isAdded(tileOf('research')) && !!tileOf('research')
        };
        const search = walk(root, []).find((n) => n.tagName === 'INPUT');
        search.value = 'research';
        (search._h.input || []).forEach((f) => f());
        const afterSearch = {
          addedStill: isAdded(tileOf('research')),
          dbHidden: !tileOf('db')
        };
        tileOf('research').onclick();
        const afterRemove = {
          addedGone: !isAdded(tileOf('research')),
          backToAvailable: !!tileOf('research') && !isAdded(tileOf('research'))
        };
        closeModal();
        return { before, afterSearch, afterRemove };
      })()
    `, sandbox);
    return got.before.added && got.before.availableHasDb && !got.before.availableHasResearch
      && got.afterSearch.addedStill && got.afterSearch.dbHidden
      && got.afterRemove.addedGone && got.afterRemove.backToAvailable;
  });
  // 临时 agent 弹层必须有：人设输入、模块加号、加入按钮。
  check("新建临时 agent 弹层含人设 / 模块加号", () => {
    const got = vm.runInNewContext(`
      (() => {
        closeModal();
        openTempAgentSheet({ onAdd: () => {} });
        const root = document.querySelector('#modal-root');
        const walk = (n, out) => { out.push(n); for (const c of (n.children || [])) walk(c, out); return out; };
        const nodes = walk(root, []);
        return {
          add: nodes.some((n) => n.tagName === 'BUTTON' && n.textContent === '加入本次（临时）'),
          modAdd: nodes.some((n) => n.tagName === 'BUTTON' && n.textContent === '+ 添加模块'),
          chara: nodes.some((n) => n.tagName === 'TEXTAREA' && String(n.placeholder).indexOf('人设') >= 0)
        };
      })()
    `, sandbox);
    vm.runInNewContext("closeModal()", sandbox);
    return got.add && got.modAdd && got.chara;
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