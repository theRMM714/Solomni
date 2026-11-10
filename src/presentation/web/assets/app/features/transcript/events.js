/* 目的：事件台到会话视图的吸收——SessionEvent 变成行 / 流式块 / 运行态，批次按水位应用。
 * 管：applySnapshot / absorb / absorbEvents / applyBatch / applyLines / trimAlign / resyncActive，
 *   以及本模块私有的水位（progress.seq）、待对账标记（progress.needState）与上次拉状态时间。
 * 不管：不碰 DOM、不渲染；需要对账或重画时交给 app 上的入口（refreshState / renderAll / renderStream）。
 * 联动：批次的传输与裁剪对齐在 app/core/events.js；行映射在 ./model.js。 */

import { lineParts } from './model.js';

let g, state, api, app;

/* 已应用到的批序号（**单调游标**）。
   事实只有一条来路（事件台）：批次按 seq 到达，游标只前进；服务端裁剪造成跳号时由
   `oldest` 触发一次历史重放重新对齐（见 trimAlign）。
   状态（侧栏/历史）可能被**别的客户端**改了：置位 needState，由轮询统一拉一次。 */
const progress = { seq: 0, needState: false, at: Date.now() };

/* 回档/重置：清空当前会话视图，并按服务端给的**活动视图**整体重建。
 * 回档会改写盘上转录；事件台上的旧事实不随文件回退，所以服务端另推一条 `rewound`
 * 快照。这里用同一段逻辑处理“动作回包”和“其它标签页收到的重置事件”。 */
function applySnapshot(s, events) {
  s.lines = [];
  s.live = [];
  s.fold = {};
  s.scroll = {};
  s.forceFull = true;
  s.pending = null;
  s.done = false;
  s.context = null; // 回档后的用量由快照里的 usage 事件重建；没有就显示"还没有调用"
  s.workDirty = true; // 回档可能也物化了共享区：状态栏重新拉一次提交
  s.readonly = false; // 回档后的会话是活动会话（历史回放被用户明确回档/继续）
  for (const ev of (events || [])) absorb(s, ev);
}

function absorb(s, ev) {
  switch (ev.type) {
    case 'notice': s.lines.push({ cls: 'sys', who: '', text: ev.text }); break;
    // 上下文用量：不是转录行、不进模型上下文；只更新状态栏（持久事件，刷新后仍在）。
    case 'usage':
      s.context = {
        prompt_tokens: ev.prompt_tokens || 0,
        completion_tokens: ev.completion_tokens || 0,
        total_tokens: ev.total_tokens || 0,
        g: ev.window || 0,
        exact: ev.exact === true,
        cache_read: typeof ev.cache_read === 'number' ? ev.cache_read : null,
        cache_creation: typeof ev.cache_creation === 'number' ? ev.cache_creation : null,
      };
      return true;
    // 留档回档：折叠分界。内容保留在文件里，可恢复。
    case 'rewind':
      if (ev.mode !== 'archive') break;
      s.lines.push({
        cls: 'sys rewind',
        who: '',
        text: ev.label
          ? '[留档] ' + ev.label + '（折叠内容仍在文件里，可恢复）'
          : '[留档] 以下内容已折叠（保留在文件里，可恢复）',
        mark: ev.mark,
      });
      break;
    // 回档成功：服务端给的是**回档后的活动视图**；清空旧视图整体重建。
    // 刷新/多端靠这条事件同步，而不是把回档前的旧事件当实时尾巴再拼一遍。
    case 'rewound':
      applySnapshot(s, ev.events || []);
      break;
    // 任务链的进展：节点开工/回报/验收/交付——主会话也要看得到，不必点进子会话。
    case 'node_started':
      // 节点是执行进度，不是转录内容：独立的卡片（有状态 / 执行人 / 进入子会话），不复用系统消息。
      if (ev.sid) state.nodeBySid.set(ev.sid, { node: ev.node || '', assignee: ev.assignee || '' });
      s.lines.push({
        cls: 'node',
        node: { id: ev.node || '', sid: ev.sid || '', assignee: ev.assignee || '' },
        state: 'started',
        report: '',
        rework: 0,
        who: '',
        text: '',
      });
      break;
    // 节点回报 = **它把活交回来了**：找到同一节点的卡标完成，不另起一行。
    // 失败的节点由核心的验收结论另行如实说明（这里只显示"交回"这个事实，不替验收下判）。
    case 'report': {
      const id = ev.id || '';
      let hit = null;
      for (let i = s.lines.length - 1; i >= 0; i -= 1) {
        const l = s.lines[i];
        if (l.cls === 'node' && l.node && l.node.id === id) { hit = l; break; }
      }
      if (hit) {
        hit.state = 'done';
        hit.report = ev.text || '';
        hit.rework = ev.rework || 0;
      } else {
        s.lines.push({
          cls: 'node',
          node: { id, sid: '', assignee: '' },
          state: 'done',
          report: ev.text || '',
          rework: ev.rework || 0,
          who: '',
          text: '',
        });
      }
      s.forceFull = true; // 卡在原位更新：报告不频繁，整帧重建比维护节点映射更稳
      break;
    }
    case 'transcript':
      // 服务端权威转录：每行带会话内稳定 id（回档按 id 定位）。
      // 权威行到达：撤掉乐观回显与流式块，改用服务端的行；带 tool 的行渲染成工具卡片。
      const hadPending = s.lines.some((x) => x.pending);
      s.lines = s.lines.filter((x) => !x.pending);
      if (hadPending) s.forceFull = true; // 乐观回显被权威行替换：整帧重建，别留下没有 id 的旧节点
      for (const l of ev.lines) {
        // 身份与样式**全读结构化字段**（种类 / 说话人 / 动词），不再从正文里抠标签（见 lineParts）。
        const parts = lineParts(l);
        for (const p of parts) { p.id = l.id; if (l.reasoning) p.reasoning = l.reasoning; }
        s.lines.push(...parts);
      }
      s.live = []; // 权威行整体替换掉流式块
      break;
    case 'delta': {
      // 流式块列表（短暂，不落盘）：一次发言可能有**多轮模型调用**（工具循环），
      // 所以按到达顺序排块——kind='start' 表示又一轮开始：封存上一块并推入新块，绝不丢弃。
      if (ev.kind === 'start' || !s.live.length || s.live[s.live.length - 1].kind !== 'msg') {
        // 每个流式轮一个**唯一键**：V{序号} 若按数组下标复用，上一轮展开的思维链会污染下一轮。
        s.liveSeq = (s.liveSeq || 0) + 1;
        s.live.push({ kind: 'msg', speaker: ev.speaker || '', segments: [], key: 'V' + s.liveSeq });
      }
      if (ev.kind === 'start') return true;
      const segs = s.live[s.live.length - 1].segments;
      const last = segs[segs.length - 1];
      if (last && last.kind === ev.kind) last.text += ev.text;
      else segs.push({ kind: ev.kind, text: ev.text });
      return true; // 短暂流式事件：只走增量渲染，不整帧重建
    }
    case 'working':
      // **权威运行态**：核心开始问某个 agent = 忙（带名字），agent=null = 这一回合收尾了。
      // 它不进转录（短暂事件）：用户看到的是占位动画与按钮切换，不是一条消息。
      // 事件即真相：agent 有名字 = 忙，null = 这一回合收尾（此后不再听快照的）。
      s.running = !!ev.agent;
      s.working = ev.agent || null;
      s.running_known = true;
      if (!ev.agent) {
        // 回合收尾却没有权威行（停止 / 错误）：未定稿的分片不得继续闪光标或冒充转录。
        s.live = [];
        return false;
      }
      return true;
    case 'tool_call':
      // 工具调用发生在轮与轮之间：按到达顺序插进流式块里（module 为空 = 内置 read/write）。
      s.live.push({ kind: 'tool', tool: ev });
      // agent 提交会改共享区 head：让状态栏 / 提交弹窗下次刷新时跟上。
      if (ev.name === 'work_commit') s.workDirty = true;
      // shell 跑完了：运行槽只经 /api/state 下发，立刻拉一次——chip 与打开的卡片不用等 3 秒轮询。
      if (ev.name === 'shell') progress.needState = true;
      return true; // 同上：卡片节点只 append，绝不重建
    case 'discussion_done':
      if (ev.over_cap) s.lines.push({ cls: 'sys', who: '', text: '讨论超轮次上限，进入裁决。' });
      break;
    case 'plan': s.lines.push({ cls: 'plan', who: '核心整理', text: ev.text }); break;
    case 'review':
      if (!ev.items || ev.items.length === 0) {
        s.lines.push({ cls: 'bad', who: '验收', text: '清单解析失败，原文：\n' + ev.raw });
      } else {
        const txt = ev.items.map((i) => '[' + i.status.toUpperCase() + '] ' + i.item + (i.note ? ' —— ' + i.note : '')).join('\n');
        s.lines.push({ cls: 'review', who: '验收清单', text: txt });
      }
      break;
    case 'delivery':
      s.lines.push(ev.ok ? { cls: 'ok', who: '交付', text: '全部通过，交付用户。' }
        : { cls: 'bad', who: '裁决', text: '返工超限仍未通过，交用户裁决。' });
      break;
    case 'decision_card':
      // **裁决卡**：与快照里的 pending 是同一个事实，只是到达得更快。
      // waiting 里是排在队首之后还在等的几张（前面还排着几条由它如实显示）。
      s.pending = ev;
      return false; // 门要整帧重画
    case 'decision_answer':
      // 一次回答的记录：这一关不再挂着（下一关的卡会随后自己推来）。
      s.pending = null;
      return false; // 门要整帧重画
    case 'decision_void':
      // **整队作废**（用户按停止 / 会话关闭 = 拒绝）：队列里没答的卡一律作废，界面撤下这张卡。
      s.pending = null;
      return false; // 门要整帧重画
    case 'ended':
      // 整场工作结束：运行态也是"没在跑"（事件给的事实，不靠快照）。
      s.done = true;
      s.live = [];
      s.running = false;
      s.working = null;
      s.running_known = true;
      break;
  }
  return false; // 定稿事件：需要整帧重建
}

/* 吸收一批事件：全是短暂流式事件 → 增量渲染；出现任何定稿事件 → 整帧重建。
 * 返回 'live' | 'full' | 'none'。 */
function absorbEvents(s, events) {
  let live = false, full = false;
  for (const ev of (events || [])) {
    if (absorb(s, ev) === true) live = true;
    else full = true;
  }
  return full ? 'full' : (live ? 'live' : 'none');
}

/** 事件流出现**补不齐的缺口**（服务端裁剪）后，按历史重放一次当前会话。
 * 为什么：spinner 与按钮都靠事件流，干等会让界面停在旧状态；重放比"攒着不显示"诚实。 */
async function resyncActive() {
  const s = app.activeSession();
  if (!s || s.readonly) return;
  try {
    const r = await api('GET', '/api/history/' + encodeURIComponent(s.sid));
    s.lines = []; s.live = [];
    // 重放的是**转录**（运行态是短暂事件，不在盘上）：这条会话的实时知识同样作废，按快照对账。
    s.running_known = false;
    // 重新合流：水位跟着刷新，之后的批次仍然只收水位之上的。
    if (typeof r.head === 'number') s.floor = r.head;
    for (const ev of (r.events || [])) absorb(s, ev);
    for (const ev of (r.live || [])) absorb(s, ev);
    app.renderStream(true);
  } catch { /* 拿不到就等下一次状态刷新 */ }
}

/** 收一批事件并应用。返回 'full' | 'live' | 'none'（渲染粒度）。 */
function applyBatch(seq, sid, events) {
  if (typeof seq === 'number') progress.seq = Math.max(progress.seq, seq);
  const s = state.sessions.get(sid);
  // **前端不认识的会话一律不动**（系统会话如核心推荐的 `#suggest` 也在内）：会话表只由
  // /api/state 给，前端只渲染它知道的会话。为事件里冒出来的 sid 就地建会话 = 前端自己造会话
  // 状态，非会话（系统会话）会因此长出多余标签页，而真会话的记录又会被建重一遍。
  // 别的客户端建的**真**会话由下一次状态刷新带进来（refreshState 建 + 合流补齐）。
  if (!s) {
    progress.needState = true;
    return 'none';
  }
  // 水位以下的批次已经在**合流**里给过（/api/history 一次给全）：丢掉，不是"没收到"。
  if (typeof seq === 'number' && typeof s.floor === 'number' && seq <= s.floor) return 'none';
  // 合流还没落定：先攒着，等水位出来再按水位筛（见 hydrateHistory）。
  if (s.hydrating) {
    if (!s.buffer) s.buffer = [];
    s.buffer.push({ seq, events });
    return 'none';
  }
  // 运行态**不从增量猜**：核心在起止各推一条 `working`（同一个批次里就在 events 里），
  // 事件按序吸收，运行态因此自己就对了——猜"有增量=在跑"只会与事件打架。
  return absorbEvents(s, events);
}

async function applyLines(lines) {
  let mode = 'none';
  for (const item of lines) {
    try {
      const m = applyBatch(item.seq, item.sid, item.events);
      if (m === 'full') mode = 'full';
      else if (m === 'live' && mode !== 'full') mode = 'live';
      // 运行态只在 applyBatch 里由事件改：这里碰它，迟到的流式增量会把「停止」误翻回「发送」。
    } catch { /* 单行损坏不拖垮轮询 */ }
  }
  // 别的客户端（演示脚本、另一个标签页）建 / 删 / 改会话时侧栏自己跟上：未知会话立刻置 progress.needState，其余靠 3 秒兜底。
  if (progress.needState || Date.now() - progress.at > 3000) {
    progress.needState = false;
    progress.at = Date.now();
    await app.refreshState();
  }
  // 纯流式增量只 append 新节点（折叠、<pre> 滚动、外层滚动都不被打断）。
  if (mode === 'full') app.renderAll();
  else if (mode === 'live') app.scheduleLiveTick();
}

/** 事件台裁剪过：since 之后有一段永久丢了。丢掉滞留，跳到还留着的起点，并按历史重放当前会话。 */
async function trimAlign(oldest) {
  if (!(oldest > 0 && oldest > progress.seq + 1)) return;
  progress.seq = oldest - 1;
  // 运行态的实时知识作废（收尾那条可能就在丢掉的那段里），退回用快照对账。
  for (const cur of state.sessions.values()) cur.running_known = false;
  progress.needState = true;
  await resyncActive();
}

/* 别的模块（输入区、裁决卡）只经这个入口置位：下一轮轮询统一拉一次 /api/state。 */
function markNeedState() { progress.needState = true; }

export { applySnapshot, absorb, absorbEvents, applyBatch, applyLines, trimAlign, resyncActive, markNeedState, progress };

/// 挂载：装配依赖并把本模块的入口交给组装点。
export function mount(root, deps) {
  ({ g, state, api, app } = deps);
  Object.assign(root, { applySnapshot, absorb, absorbEvents, applyBatch, applyLines, trimAlign, resyncActive, markNeedState, progress });
}
