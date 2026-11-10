/* 目的：会话的只读投影——活跃会话、运行态、父子关系与历史树，均从 state 读。
 * 管：activeSession / snapshotRunning / isBusy / generating / historyByParent / activePath / sessionParent / knownChildren。
 * 不管：不发请求、不写 state、不碰 DOM（写入只发生在 core 与 /api/state 投影里）。
 * 联动：app/core/http.js 的回包经 app/stores/state.js 投影后，功能在这里取读法。 */

let state, stores;

/* ---------- 会话历史：树 + 铅笔菜单 ----------
 * 根会话可展开子会话；活跃路径自动展开。点条目直接打开会话；改配置 / 删除走行尾铅笔的浮层菜单。
 * 子会话不单独删（删父会话会一起删）。 */
export function historyByParent() {
  const by = new Map();
  for (const h of (state.history || [])) {
    const p = h.parent || '';
    if (!by.has(p)) by.set(p, []);
    by.get(p).push(h);
  }
  return by;
}


/// 当前会话的祖先链（自己 → 父 → …）：树据此自动展开，用户不用找。
export function activePath() {
  const out = [];
  const seen = new Set();
  let cur = state.activeSid;
  while (cur && !seen.has(cur)) {
    seen.add(cur);
    out.push(cur);
    const h = (state.history || []).find((x) => x.name === cur);
    cur = h && h.parent ? h.parent : null;
  }
  return out;
}


/// 快照（/api/state）说这条会话、或它的任一子会话在跑吗。
/// **对账副本的唯一读法**：只在 isBusy 的第三档（本页没有实时知识）用。
export function snapshotRunning(sid) { return stores.snapshotRunning(state, sid); }


/// 这条会话此刻在不在干活。**运行态的唯一真相是事件**（每个生产者在起止各推一条 `Working`；
/// 刷新页面或事件台有缺口时用快照对账）——三个来源各司其职，不是三份互相打架的真话：
/// 1. `s.sending`：本页发起的命令还在途（本地事实，与服务端运行态无关）；
/// 2. `s.running` + `s.running_known`：收到过它的运行态事件 → 以事件为准，快照不得覆盖；
/// 3. 没有实时知识：用快照对账——成员回合跑在它自己的会话里，主会话整回合收不到事件，
///    只靠"有增量"猜就永远切不出「停止」按钮、也没有占位动画。
export function isBusy(s) {
  if (!s) return false;
  if (s.sending) return true;
  // ① 这条会话**自己**的运行态：有实时知识（收过它的 Working）就以事件为准，否则用快照对账。
  if (s.running_known ? s.running : snapshotRunning(s.sid)) return true;
  // ② 它的**子会话**在跑也算——成员回合、节点执行都跑在各自的子会话里，主会话整回合收不到事件，
  //    所以子会话的运行态只有快照说得清；子会话标签页开着时，它自己的实时知识比快照新，优先用它。
  const pre = s.sid + '--';
  for (const k of state.running || []) {
    if (k.indexOf(pre) !== 0) continue;
    const child = state.sessions.get(k);
    if (child && child.running_known) {
      if (child.running) return true;
      continue;
    }
    return true;
  }
  return false;
}


export function generating(sid) {
  const s = state.sessions.get(sid);
  return isBusy(s);
}


export function activeSession() { return state.sessions.get(state.activeSid); }


/* ---------- 会话标签：主 → 子 → 子子，按活跃路径逐层展开 ----------
 * 第 1 行是根（工作）；第 N 行是上一行活跃会话的孩子。只显示活跃路径上的层。
 * 孩子来源 = history（含没打开过的，弱化显示）+ 已打开的视图；点弱化的才去加载。 */
export function sessionParent(name) {
  const s = state.sessions.get(name);
  if (s && s.parent) return s.parent;
  const h = (state.history || []).find((x) => x.name === name);
  return (h && h.parent) || null;
}


export function knownChildren(parent) {
  const names = [];
  const seen = new Set();
  for (const h of (state.history || [])) {
    if ((h.parent || null) === parent && !seen.has(h.name)) { seen.add(h.name); names.push(h.name); }
  }
  for (const s of state.sessions.values()) {
    const p = s.parent || sessionParent(s.sid);
    if ((p || null) === parent && !seen.has(s.sid)) { seen.add(s.sid); names.push(s.sid); }
  }
  return names;
}


/// 挂载：装配依赖并把本模块的入口交给组装点。
export function mount(root, deps) {
  ({ state, stores } = deps);
  Object.assign(root, { activeSession, snapshotRunning, isBusy, generating, historyByParent, activePath, sessionParent, knownChildren });
}
