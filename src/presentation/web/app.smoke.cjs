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
let eventPolls = 0;
// 应用侧异常一律走 console.error（app.js 的 eventError）：这里记下来，当成硬失败。
// 为什么必须有它：连接状态已经不看应用异常了，没有这一条，"渲染里抛异常"就会悄悄溜过去。
const consoleErrors = [];
// /api/state 的桩：默认没有进行中的会话；运行态那条用例会临时塞一条进去。
const stateStub = { sessions: [] };
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
// 选择器结果**按选择器记忆**：setConn 写过的元素要能读回来（否则测不出"状态点被误置红"）。
const noticeRoot = el();
const els = new Map();
const sandbox = {
  document: {
    querySelector: (s) => {
      if (s === "#notice-root") return noticeRoot;
      if (!els.has(s)) els.set(s, el());
      return els.get(s);
    },
    createElement: () => el(),
    addEventListener() {},
  },
  // 原生弹窗是红线：换自研弹窗之后，这里被调用一次就算失败。
  alert: (m) => alerts.push(m),
  confirm: (m) => { alerts.push(m); return true; },
  console: {
    log: console.log,
    warn: console.warn,
    error: (...a) => { consoleErrors.push(a.map((x) => String((x && x.stack) || x)).join(" ")); console.error(...a); },
  },
  JSON, Promise, Map, Set, Error, Object, Array, String, Number, Boolean, Math, Date,
  setTimeout, clearTimeout,
  fetch: async (url) => {
    const u = String(url);
    if (u.indexOf("/api/state") === 0) {
      return {
        ok: true,
        json: async () => ({
          modules: [{ id: "research", brief: "调研" }],
          providers: [], rejected: ["broken-mod"], agents: [], history: [],
          settings: { streaming: true, show_reasoning: true, llm_timeout_secs: 300, discuss_remind_cap: 3, compact_at_percent: 70 },
          sessions: stateStub.sessions,
        }),
      };
    }
    if (u.indexOf("/api/events") === 0) {
      eventPolls += 1;
      if (eventPolls === 1) {
        // 一批**定稿**事件：轮询要能应用它，并且仍然显示"已连接"。
        return {
          ok: true,
          json: async () => ({
            lines: [{ seq: 1, sid: "smoke-w", events: [{ type: "transcript", lines: [{ id: 0, line: "[甲:say] 完整一句" }] }] }],
            head: 1, oldest: 0,
          }),
        };
      }
      return new Promise(() => {}); // 长轮询挂着等新事件（不空转）
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
setTimeout(async () => {
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
  // 每条消息都要显示身份（说话人 · 动词）：正文**追加**，不许把身份标题一起抹掉。
  // 真实 DOM 里 el.innerHTML = … 会清空子节点——桩 DOM 默认是一根普通属性，测不出这个错，
  // 所以这里用**会清空 children 的**严格元素来验（agree 这类非 line 类的消息曾经因此丢了说话人）。
  let identityKept = false;
  if (!loadErrors.length) {
    try {
      const parts = vm.runInNewContext("parseLine('[资料手:agree] 同意', false)", sandbox);
      const p = parts[0];
      const strict = { children: [], className: "", appendChild(c) { strict.children.push(c); } };
      Object.defineProperty(strict, "innerHTML", {
        set() { strict.children = []; },
        get() { return ""; },
      });
      const who = { textContent: p.who };
      strict.appendChild(who);
      vm.runInNewContext("appendBody", sandbox)(strict, p.cls, p.text);
      identityKept = String(p.who).indexOf("资料手 · agree") >= 0 && strict.children.indexOf(who) >= 0;
    } catch (e) { loadErrors.push("身份标题检查失败：" + e.message); }
  }
  // **轮询取到事件后状态点必须是"已连接"**：这条钉住一个真实缺陷——轮询的状态刷新分支调了
  // 一个作用域外的函数（agent 登记弹窗内部的 renderList），每轮都抛 ReferenceError，
  // 被同一个 catch 当成"断线"，状态点因此一直红着，而服务其实好好的。
  let pollConn = "";
  let pollApplied = false;
  if (!loadErrors.length) {
    try {
      pollConn = String(els.get("#conn-text").textContent || "");
      pollApplied = vm.runInNewContext("(state.sessions.get('smoke-w') || { lines: [] }).lines.length", sandbox) > 0;
    } catch (e) { loadErrors.push("轮询检查失败：" + e.message); }
  }
  // **权威行到达后流式块必须被替换**：否则那一行会一直挂着闪烁光标（"已落盘的还在流式"）。
  let liveReplaced = false;
  let liveClearedOnIdle = false;
  let toolReasoningRendered = false;
  if (!loadErrors.length) {
    try {
      const r = vm.runInNewContext(
        "(function () { const s = { sid: 'x', lines: [], live: [], fold: {}, scroll: {} };" +
          "absorb(s, { type: 'delta', kind: 'text', speaker: '甲', text: '半截' }); const afterDelta = s.live.length;" +
          "absorb(s, { type: 'transcript', lines: [{ id: 1, line: '[甲:say] 完整一句' }] });" +
          "return { afterDelta: afterDelta, afterTranscript: s.live.length, lines: s.lines.length }; })()",
        sandbox
      );
      liveReplaced = r.afterDelta === 1 && r.afterTranscript === 0 && r.lines === 1;
      if (!liveReplaced) loadErrors.push("流式替换检查：delta 后 live=" + r.afterDelta + "、定稿后 live=" + r.afterTranscript + "、行数=" + r.lines);
    } catch (e) { loadErrors.push("流式替换检查失败：" + e.message); }
  }

  if (!loadErrors.length) {
    try {
      const r = vm.runInNewContext(
        "(function () { const s = { sid: 'x', lines: [], live: [], fold: {}, scroll: {} };" +
          "absorb(s, { type: 'delta', kind: 'text', speaker: '甲', text: '半截' });" +
          "const before = s.live.length; absorb(s, { type: 'working', agent: null });" +
          "return { before: before, after: s.live.length, running: s.running === true, working: s.working }; })()",
        sandbox
      );
      liveClearedOnIdle = r.before === 1 && r.after === 0 && !r.running && r.working === null;
      if (!liveClearedOnIdle) loadErrors.push("空闲收尾检查：idle 前=" + r.before + "、idle 后=" + r.after + "、running=" + r.running);
    } catch (e) { loadErrors.push("空闲收尾检查失败：" + e.message); }
  }
  if (!loadErrors.length) {
    try {
      const r = vm.runInNewContext(
        "(function () { const s = { sid: 'x', lines: [], live: [], fold: {}, scroll: {} };" +
          "absorb(s, { type: 'transcript', lines: [{ id: 1, line: '[甲:say] 完整一句', reasoning: '过程说明' }] });" +
          "return { reasoning: s.lines[0].reasoning, show: state.settings.show_reasoning }; })()",
        sandbox
      );
      toolReasoningRendered = r.reasoning === '过程说明' && r.show === true;
    } catch (e) { loadErrors.push("定稿思维链检查失败：" + e.message); }
  }
  // **改需求按钮**：没有能力位就根本不渲染（不是灰着）；会话工作时不可点。
  let taskButtonRule = false;
  if (!loadErrors.length) {
    try {
      const r = vm.runInNewContext(
        "(function () {" +
          "function st(can, busy) { return { sid: 'x', can_update_task: can, running: busy, running_known: true, sending: false, done: false, readonly: false, lines: [], live: [] }; }" +
          "syncSendButton(st(false, false)); const off = document.querySelector('#btn-update-task').className;" +
          "syncSendButton(st(true, false)); const on = document.querySelector('#btn-update-task').className;" +
          "syncSendButton(st(true, true)); const busyHidden = document.querySelector('#btn-update-task').disabled;" +
          "return { off: off, on: on, busyDisabled: busyHidden }; })()",
        sandbox
      );
      taskButtonRule = r.off.indexOf("hidden") >= 0 && r.on.indexOf("hidden") < 0 && r.busyDisabled === true;
      if (!taskButtonRule) loadErrors.push("改需求按钮规则：无权=" + r.off + "、有权=" + r.on + "、工作中 disabled=" + r.busyDisabled);
    } catch (e) { loadErrors.push("改需求按钮检查失败：" + e.message); }
  }
  // **裁决卡**：自由文本那类渲染成"说明 + 建议 + 问题 + 输入框 + 提交"；二选一仍给按钮。
  let decisionCardRule = false;
  if (!loadErrors.length) {
    try {
      const r = vm.runInNewContext(
        "(function () {" +
          "const s = { sid: 'd', lines: [], live: [], sending: false, done: false, readonly: false, fold: {}, scroll: {}," +
          "  pending: { type: 'decision', kind: 'plan_review', summary: 'S', advice: 'A', question: 'Q', payload: {} } };" +
          "renderGate(s); const g = document.querySelector('#gate');" +
          "const card = g.children[0] || { children: [] };" +
          "const cls = card.children.map(function (c) { return c.className; });" +
          "const slate = { sid: 'd2', lines: [], live: [], sending: false, done: false, readonly: false, fold: {}, scroll: {}," +
          "  pending: { type: 'decision', kind: 'confirm_slate', summary: 'S2' } };" +
          "renderGate(slate); const g2 = document.querySelector('#gate');" +
          "return { hasInput: cls.indexOf('decision-input') >= 0, hasBtns: cls.indexOf('btns') >= 0, slateKids: g2.children.length }; })()",
        sandbox
      );
      decisionCardRule = r.hasInput && r.hasBtns && r.slateKids > 0;
      if (!decisionCardRule) loadErrors.push("裁决卡检查：输入框=" + r.hasInput + "、按钮=" + r.hasBtns + "、二选一卡=" + r.slateKids);
    } catch (e) { loadErrors.push("裁决卡检查失败：" + e.message); }
  }
  // **只有正在传的那一块带光标**：一轮开始后，前一块不再像"还在流式"（否则看着像已落盘还在流）。
  let onlyLastStreams = false;
  if (!loadErrors.length) {
    try {
      const r = vm.runInNewContext(
        "(function () {" +
          "const s = { sid: 'lv', lines: [], live: [], sending: true, done: false, readonly: false, fold: {}, scroll: {} };" +
          "state.sessions.set('lv', s); state.activeSid = 'lv';" +
          "absorb(s, { type: 'delta', kind: 'start', speaker: 'a', text: '' });" +
          "absorb(s, { type: 'delta', kind: 'text', speaker: 'a', text: '第一轮' });" +
          "absorb(s, { type: 'delta', kind: 'start', speaker: 'a', text: '' });" +
          "absorb(s, { type: 'delta', kind: 'text', speaker: 'a', text: '第二轮' });" +
          "renderStream(true);" +
          "return { n: s.live.length, first: s.live[0]._node.className, last: s.live[s.live.length - 1]._node.className }; })()",
        sandbox
      );
      onlyLastStreams = r.n === 2 && r.first.indexOf("streaming") < 0 && r.last.indexOf("streaming") >= 0;
      if (!onlyLastStreams) loadErrors.push("流式光标检查：块数=" + r.n + "、首块=" + r.first + "、末块=" + r.last);
    } catch (e) { loadErrors.push("流式光标检查失败：" + e.message); }
  }
  // **运行态归一**：事件是唯一真相，/api/state 的 running 只是**对账副本**。
  //  ① 有实时知识的会话：快照说在跑也不能覆盖（收尾事件到了就是收尾）；
  //  ② 没有实时知识的会话：快照补齐（刚刷新页面照样显示"正在工作"），快照说没跑就清掉本地遗留；
  //  ③ 本页发出的命令在途 = 忙（本地事实，与服务端运行态是两回事）。
  let runningRules = false;
  if (!loadErrors.length) {
    try {
      stateStub.sessions = [{ sid: 'rec', running: true }, { sid: 'fresh', running: true }, { sid: 'stale', running: false }];
      const r = await vm.runInNewContext(
        "(function () {" +
          " const rec = { sid: 'rec', lines: [], live: [], running: false, running_known: true, sending: false };" +
          " const fresh = { sid: 'fresh', lines: [], live: [], running: false, running_known: false, sending: false };" +
          " const stale = { sid: 'stale', lines: [], live: [], running: true, working: '甲', running_known: false, sending: false };" +
          " const mine = { sid: 'mine', lines: [], live: [], running: false, running_known: true, sending: true };" +
          " state.sessions.set('rec', rec); state.sessions.set('fresh', fresh);" +
          " state.sessions.set('stale', stale); state.sessions.set('mine', mine); state.activeSid = null;" +
          " return refreshState().then(function () {" +
          "   return { rec: { run: rec.running, busy: isBusy(rec) }, fresh: { run: fresh.running, busy: isBusy(fresh) }," +
          "            stale: { run: stale.running, who: stale.working, busy: isBusy(stale) }, mine: isBusy(mine) };" +
          " }); })()",
        sandbox
      );
      runningRules = r.rec.run === false && r.rec.busy === false
        && r.fresh.run === true && r.fresh.busy === true
        && r.stale.run === false && r.stale.who === null && r.stale.busy === false
        && r.mine === true;
      if (!runningRules) loadErrors.push("运行态归一：有实时知识 rec=" + JSON.stringify(r.rec) + "、无知识 fresh=" + JSON.stringify(r.fresh) + "、遗留 stale=" + JSON.stringify(r.stale) + "、在途=" + r.mine);
    } catch (e) { loadErrors.push("运行态归一检查失败：" + e.message); }
    stateStub.sessions = [];
  }
  const ok = alerts.length === 0 && loadErrors.length === 0 && rendered && tierWarned && identityKept
    && pollConn === "已连接" && pollApplied && liveReplaced && liveClearedOnIdle && toolReasoningRendered
    && taskButtonRule && decisionCardRule && onlyLastStreams && runningRules && consoleErrors.length === 0;
  if (!ok) {
    console.log("alerts（原生弹窗被调用的次数，应为 0）:", JSON.stringify(alerts));
    console.log("notice 渲染:", rendered, "| 虚拟机档不可用提示:", tierWarned, "| 身份标题保留:", identityKept);
    console.log("轮询状态点:", JSON.stringify(pollConn), "| 事件已应用:", pollApplied, "| 流式被替换:", liveReplaced, "| 空闲清理:", liveClearedOnIdle, "| 思维链:", toolReasoningRendered);
    console.log("运行态归一（事件为准 / 快照只对账）:", runningRules);
    console.log("应用侧 console.error:", JSON.stringify(consoleErrors.slice(0, 3)));
    console.log("loadErrors:", JSON.stringify(loadErrors));
  }
  console.log(ok ? "FRONTEND-INIT-OK" : "FRONTEND-INIT-FAIL");
  process.exit(ok ? 0 : 1);
}, 300);