/* 目的：`/api/state` 快照在前端的投影（会话表、登记处、设置、围栏）与登记处变化通知。
 * 管：建初始状态；把快照写进状态（只为「后端说在跑」的会话建条目，已开的条目对齐快照）；
 *   运行态快照的唯一读法；登记处指纹变化时通知订阅者。
 * 不管：拉取快照（调用方用 `app/core/http.js`）、补历史、渲染；不为事件流里冒出的 sid 造会话。
 *   长路径缩写的根列表 pathRoots 也放这里（向导写入、转录读），它是界面状态，不对应任何接口。
 * 联动：会话表与运行态规则见 `docs/presentation/contracts.md` 的「顺序与去重」与
 *   `docs/session/session-model.md`；`app/main.js` 把它交给界面层。
 *   回执说明的词表收到后交给 `../core/i18n.js`（工具卡片按码渲染，不在前端复制中文表）。 */

import { setCatalog } from '../core/i18n.js';

const DEFAULT_SETTINGS = {
  streaming: true, show_reasoning: true, llm_timeout_secs: 300,
  discuss_remind_cap: 3, compact_at_percent: 70, compact_keep_percent: 15, compact_model: '',
};

export function createState() {
  return {
    modules: [], providers: [], models: [], core: null, rejected: [], agents: [],
    // 会话类型表（后端按 systools/profiles.yaml 给）：向导与侧栏的名字都查它。
    types: [],
    history: [],
    settings: { ...DEFAULT_SETTINGS },
    fence: null,
    toolchains: null,
    i18n: { lang: 'zh-CN', messages: {} },
    sessions: new Map(),
    activeSid: null,
    treeOpen: new Set(),
    autoOpenedFor: null,
    rail: false,
    nodeBySid: new Map(),
    pathRoots: null,
  };
}

/** 快照（/api/state）说这条会话、或它的任一子会话在跑吗：只在本页没有实时知识时用。 */
export function snapshotRunning(state, sid) {
  const run = state.running;
  if (!run || !sid) return false;
  if (run.has(sid)) return true;
  const pre = sid + '--';
  for (const k of run) if (k.indexOf(pre) === 0) return true;
  return false;
}

/** 目的：按会话类型 id 取类型表里的那一条（没有就是 null）。 */
export function typeOf(state, kind) {
  return (state.types || []).find((t) => t.kind === kind) || null;
}

/** 目的：界面显示的类型名：表里有就用 name，没有（例如类型表删了这条）就退回 id。 */
export function typeLabel(state, kind) {
  const t = typeOf(state, kind);
  return (t && t.name) || kind || '';
}

function newSession(v, h) {
  return {
    sid: v.sid, kind: v.kind || '', flows: !!v.flows, auto: !!v.auto,
    title: (h && h.name) || v.sid, run: v.run || 'active',
    parent: (h && h.parent) || null,
    lines: [], live: [], pending: v.pending || null, sending: false, answering: false,
    running: false, working: null, running_known: false,
    can_update_task: !!v.can_update_task,
    continue_action: v.continue || null,
    tier: v.tier, tier_ready: v.tier_ready, tier_missing: v.tier_missing || [],
    permission_profile: v.permission_profile || '', permission_label: v.permission_label || '',
    thinking: v.thinking || null,
    permission_narrowed: !!v.permission_narrowed, permission_parent: v.permission_parent || null,
    activating: false,
    done: !!(h && h.done), awaiting: null, fold: {}, scroll: {},
    context: v.context || null,
    usage_totals: v.usage_totals || null,
    shell: v.shell || null,
    work: null, workAt: 0, workDirty: false, workError: '',
    readonly: false, hydrated: false, buffer: [],
    workspace: v.workspace || (h && h.workspace) || null,
    workspace_missing: !!(v.workspace_missing || (h && h.workspace_missing)),
  };
}

function align(state, cur, v) {
  cur.can_update_task = !!v.can_update_task;
  cur.run = v.run || 'active';
  // 类型与流程是落盘事实：变了就跟着变（类型表改名不影响已开会话的 kind）。
  if (v.kind !== undefined) cur.kind = v.kind;
  if (v.flows !== undefined) cur.flows = !!v.flows;
  if (v.auto !== undefined) cur.auto = !!v.auto;
  cur.pending = v.pending || null;
  // 只读视图不显示上下文继续（只有启用）。
  if (!cur.readonly) cur.continue_action = v.continue || null;
  cur.tier = v.tier;
  cur.tier_ready = v.tier_ready;
  cur.tier_missing = v.tier_missing || [];
  cur.permission_profile = v.permission_profile || cur.permission_profile;
  cur.permission_label = v.permission_label || cur.permission_label;
  if (v.thinking !== undefined) cur.thinking = v.thinking || null;
  if (v.context_freshness !== undefined) cur.context_freshness = !!v.context_freshness;
  if (v.freshness_keep_percent !== undefined) cur.freshness_keep_percent = Number(v.freshness_keep_percent);
  cur.freshness_stats = v.freshness_stats || null;
  cur.permission_narrowed = !!v.permission_narrowed;
  cur.permission_parent = v.permission_parent || null;
  if (v.context !== undefined) cur.context = v.context || null;
  if (v.usage_totals !== undefined) cur.usage_totals = v.usage_totals || null;
  // 活动会话以服务端运行槽为准（null = 被回档截掉）；历史回放保留 /api/history 的派生值。
  if (!cur.readonly && v.shell !== undefined) cur.shell = v.shell || null;
  // 运行态只对账、不覆盖：有实时知识时快照更旧，迟到的轮询不能把已收尾的回合标回「在跑」。
  if (!cur.running_known) {
    cur.running = snapshotRunning(state, cur.sid);
    if (!cur.running) cur.working = null;
  }
}

/**
 * 把一份快照写进状态。
 * @returns {object[]} 本次新建的会话条目（调用方负责补盘上转录）。
 */
export function projectSnapshot(state, s) {
  state.modules = s.modules || [];
  state.providers = s.providers || [];
  state.models = s.models || [];
  state.core = s.core || null;
  state.rejected = s.rejected || [];
  state.agents = s.agents || [];
  state.types = s.session_types || [];
  state.history = s.history || [];
  state.views = new Map((s.sessions || []).map((v) => [v.sid, v]));
  state.running = new Set(
    [].concat(
      (s.sessions || []).filter((v) => v.running).map((v) => v.sid),
      (state.history || []).filter((h) => h.running).map((h) => h.name),
    ),
  );
  const opened = [];
  for (const v of state.views.values()) {
    if (state.sessions.has(v.sid) || !state.running.has(v.sid)) continue;
    const h = (state.history || []).find((x) => x.name === v.sid);
    const cur = newSession(v, h);
    state.sessions.set(v.sid, cur);
    opened.push(cur);
  }
  for (const cur of state.sessions.values()) {
    const v = state.views.get(cur.sid);
    if (v) align(state, cur, v);
  }
  state.settings = s.settings || { ...DEFAULT_SETTINGS };
  state.fence = s.fence || null;
  state.toolchains = s.toolchains || null;
  // 回执说明的词表随快照下来：工具卡片按码取文本，不自己复制一份中文表。
  if (s.i18n) { state.i18n = s.i18n; setCatalog(s.i18n); }
  return opened;
}

/**
 * 登记处变化通知：订阅者只同步列表与下拉选项，不重建表单主体（轮询每 3 秒都会走到这里）。
 * `check()` 在快照写入后调用：指纹变了才通知，返回是否变了。
 */
export function createRegistryWatch(state, onListenerError) {
  const listeners = new Set();
  let print = '';
  const fingerprint = () => JSON.stringify([
    state.providers, state.models, state.agents, state.core,
    state.settings && state.settings.thinking,
  ]);
  return {
    on(fn) {
      listeners.add(fn);
      return () => listeners.delete(fn);
    },
    notify() {
      for (const fn of Array.from(listeners)) {
        try { fn(); } catch (e) { onListenerError(e); }
      }
    },
    check() {
      const next = fingerprint();
      const changed = next !== print;
      print = next;
      return changed;
    },
  };
}
