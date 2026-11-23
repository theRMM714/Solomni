/* 目的：流式区（打字指示、实时块、增量重画）与整帧重建的入口 renderStream。
 * 管：buildStreamBoxes / updateLiveGroup / renderLive / renderStream / 合并渲染节拍（scheduleLiveTick）/
 *   跟随锁存与「回到底部」浮标。
 * 不管：不定义行的长相（./parts.js）、不定义气泡（./turn.js）、不决定整帧时机（./page.js）、
 *   跟随判定的边界（./scroll.js 的纯函数）。
 * 联动：裁决门要整帧重画时经 app.renderGate 晚绑定，避免与本模块互相 import；
 *   新增浮标节点要随 buildStreamBoxes 一起挂，否则整帧重建会把它清掉。 */

import { mdNode, toolCard, reasoningBlock, shortPath, flushScroll } from './parts.js';
import { renderDone } from './turn.js';
import { boxes } from './dom.js';
import { transcriptCopy } from './copy.js';
import { followAfterScroll } from './scroll.js';

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

/// 跟随锁存：只由 #stream 的滚动事件推进（滚离底部解除、回到底部恢复）。
/// 渲染路径只认这个锁存，不能每帧再按"距底多近"重判——容差以内的轻微上滚会被下一帧
/// 立刻吃掉，用户就永远逃不出底部（边界算法见 ./scroll.js）。
let follow = true;

/// 浮标显隐：不跟随时才出现；显式 fresh 才点亮「新消息」（跟随期间清掉）。
function syncJump(fresh) {
  const btn = boxes.jump;
  if (!btn) return;
  const show = !follow && !!boxes.owner;
  if (show) btn.classList.remove('hidden');
  else btn.classList.add('hidden');
  if (!show) btn.classList.remove('fresh');
  else if (fresh) btn.classList.add('fresh');
}

/// 滚回底部：同一把锁存，所以之后的流式增量会继续跟随。
function jumpToBottom() {
  const box = $('#stream');
  follow = true;
  box.scrollTop = box.scrollHeight;
  syncJump(false);
}

/// 滚动事件是锁存的唯一来源；程序写入 scrollTop 也会触发，回到底部时自然恢复跟随。
function onStreamScroll() {
  follow = followAfterScroll($('#stream'));
  syncJump(false);
}

/// 浮标节点建一次、随容器重建搬过去（不重建，免得丢掉点击处理）。
function makeJump() {
  const btn = doc.createElement('button');
  btn.type = 'button';
  btn.className = 'jump-bottom hidden';
  btn.title = transcriptCopy.jumpBottom;
  btn.setAttribute('aria-label', transcriptCopy.jumpBottom);
  btn.appendChild(ui.icon('chevron'));
  const fresh = doc.createElement('span');
  fresh.className = 'jump-fresh';
  fresh.textContent = transcriptCopy.jumpFresh;
  btn.appendChild(fresh);
  btn.addEventListener('click', jumpToBottom);
  return btn;
}

/// 建立两个容器 + 打字指示器（整帧重建时调用）。
function buildStreamBoxes(sid) {
  const box = $('#stream');
  box.innerHTML = '';
  boxes.done = doc.createElement('div'); boxes.done.className = 'stream-done';
  boxes.live = doc.createElement('div'); boxes.live.className = 'stream-live';
  boxes.typing = doc.createElement('div'); boxes.typing.className = 'typing hidden'; boxes.typing.textContent = 'agent 工作中';
  boxes.jump = boxes.jump || makeJump();
  box.appendChild(boxes.done); box.appendChild(boxes.live); box.appendChild(boxes.typing);
  box.appendChild(boxes.jump);
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

function updateLiveGroup(grp, gi, s, showCot, streaming) {
  const ui = grp.ui;
  const segs = (grp.msg && grp.msg.segments) || [];
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
      const node = reasoningBlock(cot, s, (grp.msg && grp.msg.key) || ('V' + gi));
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
  if (grp.tools.length) {
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
    const failed = grp.tools.filter((tb) => tb.tool && !tb.tool.ok).length;
    ui.tools.children[0].textContent = '工具调用 · ' + grp.tools.length + (failed ? '（' + failed + ' 失败）' : '');
    for (const tb of grp.tools) {
      if (tb._node) continue;
      tb._node = toolCard(tb.tool, s, 'LT' + gi + '-' + grp.tools.indexOf(tb), '');
      ui.toolList.appendChild(tb._node);
    }
  }
  const visible = (showCot && !!cot) || !!txt || grp.tools.length > 0;
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
    if (b.kind === 'msg') groups.push({ msg: b, tools: [], ui: null });
    else if (b.kind === 'tool') {
      if (!groups.length) groups.push({ msg: null, tools: [], ui: null });
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
  const prevCount = boxes.doneState.count;
  const s = app.activeSession();
  if (!s) {
    boxes.done = null; boxes.live = null; boxes.typing = null; boxes.owner = null;
    follow = true; syncJump(false);
    boxes.doneState = { owner: null, count: 0, show: null, note: '', last: null };
    box.innerHTML = '<div class="empty">' +
      '<svg class="empty-logo" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.4" aria-hidden="true">' +
      '<circle cx="12" cy="12" r="8.5"/><circle cx="12" cy="12" r="1.6" fill="currentColor" stroke="none"/></svg>' +
      '<div class="tip">' + transcriptCopy.emptyTip + '</div>' +
      '<div class="hint">' + transcriptCopy.emptyHint + '</div></div>';
    const empty = box.firstChild;
    const cta = ui.btn(transcriptCopy.emptyCta, 'btn btn-primary');
    cta.onclick = () => app.openWizard();
    if (empty) empty.appendChild(cta);
    return;
  }
  if (boxes.owner !== s.sid) follow = true; // 切会话：从最新处看起
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
  if (follow) box.scrollTop = box.scrollHeight;
  else if (full) box.scrollTop = prevTop; // 重建后保持视口，别把回看的人弹走
  syncJump(boxes.doneState.count !== prevCount);
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
  const prevHeight = box.scrollHeight;
  renderLive(s, false);
  syncTyping(s);
  if (follow) box.scrollTop = box.scrollHeight;
  syncJump(follow ? false : box.scrollHeight !== prevHeight);
  app.syncSendButton(s);
  app.syncStatusbar(s);
  if (s.workDirty) { s.workDirty = false; app.loadWork(s.sid, true); }
}

export { renderStream, syncTyping, scheduleLiveTick, renderLiveTick, buildStreamBoxes };

/// 挂载：装配依赖，挂滚动监听（锁存的唯一来源），并把本模块的入口交给组装点。
export function mount(root, deps) {
  ({ g, doc, state, ui, app } = deps);
  const box = $('#stream');
  if (box && box.addEventListener) box.addEventListener('scroll', onStreamScroll, { passive: true });
  Object.assign(root, { renderStream, syncTyping, scheduleLiveTick, renderLiveTick, buildStreamBoxes });
}
