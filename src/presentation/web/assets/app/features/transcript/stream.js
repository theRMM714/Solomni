/* 目的：流式区（打字指示、实时块、增量重画）与整帧重建的入口 renderStream。
 * 管：buildStreamBoxes / updateLiveGroup / renderLive / renderStream / 合并渲染节拍（scheduleLiveTick）。
 * 不管：不定义行的长相（./parts.js）、不定义气泡（./turn.js）、不决定整帧时机（./page.js）。
 * 联动：裁决门要整帧重画时经 app.renderGate 晚绑定，避免与本模块互相 import。 */

import { mdNode, toolCard, reasoningBlock, shortPath, flushScroll } from './parts.js';
import { renderDone } from './turn.js';
import { boxes } from './dom.js';
import { transcriptCopy } from './copy.js';

let g, doc, state, ui, app;

/// 选择器简写：模块内的 DOM 查找一律走注入的 doc。
const $ = (sel) => doc.querySelector(sel);

/// 打字指示器只有一个，靠 class 显隐；不参与流的 append 顺序。
function syncTyping(s) {
  if (!boxes.typing) return;
  const live = s.live || [];
  const hasLive = live.some((b) => b.kind === 'tool' || (b.segments && b.segments.length));
  const busy = app.isBusy(s);
  boxes.typing.textContent = s.working ? '正在工作：' + s.working : '正在工作…';
  boxes.typing.className = busy && !hasLive ? 'typing' : 'typing hidden';
}

/// 只有本来就在底部才自动跟随；用户往上滚时保持原位置（流式刷新不抢滚动条）。
function nearBottom(box) { return box.scrollHeight - box.scrollTop - box.clientHeight < 80; }

/// 建立两个容器 + 打字指示器（整帧重建时调用）。
function buildStreamBoxes(sid) {
  const box = $('#stream');
  box.innerHTML = '';
  boxes.done = doc.createElement('div'); boxes.done.className = 'stream-done';
  boxes.live = doc.createElement('div'); boxes.live.className = 'stream-live';
  boxes.typing = doc.createElement('div'); boxes.typing.className = 'typing hidden'; boxes.typing.textContent = 'agent 工作中';
  box.appendChild(boxes.done); box.appendChild(boxes.live); box.appendChild(boxes.typing);
  boxes.owner = sid;
}

/// 流式块：只做增量——块不存在就建节点，内容只在既有节点上更新，绝不重建。
/// **一个块最多一个思维链折叠 + 一段正文**（与定稿后的行同形）：模型会把"思维链/正文"交替吐出来，
/// 若按到达片段各建节点，流式期间就会冒出两个「思维链」、正文被切成好几段。
/// 流式 tool 块的键 = 已有权威 tool 行数 + 它在 live 里的 tool 块序号 → 与定稿后的权威行同键。
function liveTurnNode(owner) {
  const el = doc.createElement('article');
  el.className = 'turn live';
  const head = doc.createElement('div');
  head.className = 'turn-head';
  const who = doc.createElement('span');
  who.className = 'turn-who';
  who.textContent = (owner && owner.speaker) || (owner && owner.tool && owner.tool.speaker) || '';
  head.appendChild(who);
  el.appendChild(head);
  const body = doc.createElement('div');
  body.className = 'turn-body';
  el.appendChild(body);
  return { node: el, body, cot: null, textWrap: null, text: null, textValue: '', tools: null, toolList: null };
}

function updateLiveGroup(g, gi, s, showCot, streaming) {
  const ui = g.ui;
  const segs = (g.msg && g.msg.segments) || [];
  let cot = '';
  let txt = '';
  for (const seg of segs) {
    const t = String(seg.text == null ? '' : seg.text);
    if (!t) continue;
    if (seg.kind === 'reasoning') cot += t;
    else txt += t;
  }
  if (showCot && cot) {
    if (!ui.cot) {
      const node = reasoningBlock(cot, s, (g.msg && g.msg.key) || ('V' + gi));
      if (ui.textWrap) ui.body.insertBefore(node, ui.textWrap);
      else ui.body.appendChild(node);
      ui.cot = { node, text: cot };
    } else if (ui.cot.text !== cot) {
      const body = ui.cot.node.children && ui.cot.node.children[1];
      if (body) body.textContent = cot;
      ui.cot.text = cot;
    }
  }
  if (txt) {
    if (!ui.textWrap) {
      ui.textWrap = doc.createElement('div');
      ui.textWrap.className = 'turn-text';
      ui.body.appendChild(ui.textWrap);
      ui.text = mdNode(txt);
      ui.textWrap.appendChild(ui.text);
      ui.textValue = txt;
    } else if (ui.textValue !== txt) {
      if (typeof g.markdownToHtml === 'function') ui.text.innerHTML = shortPath(g.markdownToHtml(txt));
      else ui.text.textContent = txt;
      ui.textValue = txt;
    }
  }
  if (g.tools.length) {
    if (!ui.tools) {
      ui.tools = doc.createElement('details');
      ui.tools.className = 'fold turn-tools';
      const sum = doc.createElement('summary');
      ui.tools.appendChild(sum);
      ui.toolList = doc.createElement('div');
      ui.toolList.className = 'tool-list';
      ui.tools.appendChild(ui.toolList);
      ui.body.appendChild(ui.tools);
    }
    const failed = g.tools.filter((tb) => tb.tool && !tb.tool.ok).length;
    ui.tools.children[0].textContent = '工具调用 · ' + g.tools.length + (failed ? '（' + failed + ' 失败）' : '');
    for (const tb of g.tools) {
      if (tb._node) continue;
      tb._node = toolCard(tb.tool, s, 'LT' + gi + '-' + g.tools.indexOf(tb), '');
      ui.toolList.appendChild(tb._node);
    }
  }
  const visible = (showCot && !!cot) || !!txt || g.tools.length > 0;
  ui.node.className = 'turn live' + (visible ? '' : ' empty') + (streaming && visible ? ' streaming' : '');
}

/// 流式块按“一次回复”分组：消息 + 随后的工具调用合成同一个临时轮，定稿后形状不变。
function renderLive(s, full) {
  if (!boxes.live) { renderStream(true); return; }
  const live = s.live || [];
  if (full) {
    boxes.live.innerHTML = '';
    for (const b of live) { b._node = null; b._ui = null; }
  }
  const groups = [];
  for (const b of live) {
    if (b.kind === 'msg') groups.push({ msg: b, tools: [] });
    else if (b.kind === 'tool') {
      if (!groups.length) groups.push({ msg: null, tools: [] });
      groups[groups.length - 1].tools.push(b);
    }
  }
  const showCot = !!state.settings.show_reasoning;
  let lastMsg = -1;
  groups.forEach((g, gi) => { if (g.msg) lastMsg = gi; });
  groups.forEach((g, gi) => {
    const owner = g.msg || g.tools[0];
    if (!owner) return;
    if (!owner._ui) {
      owner._ui = liveTurnNode(owner);
      boxes.live.appendChild(owner._ui.node);
    }
    g.ui = owner._ui;
    if (g.msg) g.msg._node = owner._ui.node;
    updateLiveGroup(g, gi, s, showCot, gi === lastMsg);
  });
  flushScroll(s);
}

/// 整帧重建：容器与全部行都重建（折叠 / <pre> 滚动由 store 恢复）。
function renderStream(force) {
  const box = $('#stream');
  const prevTop = box.scrollTop;
  const stick = force === true || nearBottom(box);
  const s = app.activeSession();
  if (!s) {
    boxes.done = null; boxes.live = null; boxes.typing = null; boxes.owner = null;
    boxes.doneState = { owner: null, count: 0, show: null, note: '', last: null };
    box.innerHTML = '<div class="empty">' +
      '<svg class="empty-logo" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.4" aria-hidden="true">' +
      '<circle cx="12" cy="12" r="8.5"/><circle cx="12" cy="12" r="1.6" fill="currentColor" stroke="none"/></svg>' +
      '<div class="tip">' + transcriptCopy.emptyTip + '</div>' +
      '<div class="hint">' + transcriptCopy.emptyHint + '</div></div>';
    return;
  }
  const full = force === true || boxes.owner !== s.sid || !boxes.done || !boxes.live;
  if (full) {
    // 切会话 / 重置 / 开关变化：容器与全部行重建（折叠与 <pre> 滚动由 store 恢复）。
    boxes.done = null; boxes.live = null; boxes.typing = null; boxes.owner = null;
    box.innerHTML = '';
    buildStreamBoxes(s.sid);
    boxes.doneState.owner = null;
    renderDone(s, true);
    renderLive(s, true);
  } else {
    // 追加路径：只处理新到的定稿行；流式块只在"这一轮已经收尾"时撤下。
    renderDone(s, false);
    if (!s.live || !s.live.length) {
      if (boxes.live) boxes.live.innerHTML = '';
      for (const b of (s.live || [])) { b._node = null; b._cot = null; b._txt = null; }
    } else {
      renderLive(s, false);
    }
  }
  syncTyping(s);
  box.scrollTop = stick ? box.scrollHeight : prevTop;
  app.syncSendButton(s);
}

/* 流式增量按帧合并：一帧最多一次 DOM 写，高频分片不会把主线程塞满。 */

function scheduleLiveTick() {
  if (boxes.tickScheduled) return;
  boxes.tickScheduled = true;
  const run = () => {
    boxes.tickScheduled = false;
    renderLiveTick(app.activeSession());
  };
  if (typeof g.requestAnimationFrame === 'function') g.requestAnimationFrame(run);
  else g.setTimeout(run, 0);
}

/// 流式增量帧：只 append 新的流式节点，绝不重建已有节点。
function renderLiveTick(s) {
  if (!s) return;
  if (!boxes.live || boxes.owner !== s.sid) { renderStream(true); return; }
  const box = $('#stream');
  const prevTop = box.scrollTop;
  const stick = nearBottom(box);
  renderLive(s, false);
  syncTyping(s);
  box.scrollTop = stick ? box.scrollHeight : prevTop;
  app.syncSendButton(s);
  app.syncStatusbar(s);
  if (s.workDirty) { s.workDirty = false; app.loadWork(s.sid, true); }
}

export { renderStream, syncTyping, scheduleLiveTick, renderLiveTick, buildStreamBoxes };

/// 挂载：装配依赖并把本模块的入口交给组装点。
export function mount(root, deps) {
  ({ g, doc, state, ui, app } = deps);
  Object.assign(root, { renderStream, syncTyping, scheduleLiveTick, renderLiveTick, buildStreamBoxes });
}
