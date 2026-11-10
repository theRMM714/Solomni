/* 目的：裁决门与裁决卡、任务链的进度行、输入取值。
 * 管：renderGate / renderCard / answerCard / gateCard / updateTaskFlow / updateTask / takeInput。
 * 不管：卡上的四格全部来自后端（信封 / 消息 / 选项），前端不认识业务含义、不按 kind 猜按钮。
 * 联动：提交走 /api/actions；回答后重画经 app.renderStream / app.renderAll，置位经 app.markNeedState。 */

let doc, api, ui, app;

/// 选择器简写：模块内的 DOM 查找一律走注入的 doc。
const $ = (sel) => doc.querySelector(sel);

/* 裁决门：**按后端给的消息与选项渲染**（前端不认识业务含义，也不按 kind 猜按钮）。
   改需求**不是**门——它是会话级按钮（见 syncSendButton / updateTaskFlow）。 */
function renderGate(s) {
  const gate = $('#gate');
  gate.innerHTML = '';
  if (!s || s.done || s.readonly) return;
  const p = s.pending;
  if (app.isBusy(s) && !p) return;
  if (s.awaiting === 'task' && !app.isBusy(s)) {
    gate.appendChild(gateCard('请提交本次协作需求：', [
      ['提交', async () => { const v = takeInput(); if (v) await app.act('task', v); }],
    ]));
    return;
  }
  // **卡片照画**：工具级确认发生在生成中（生成正停下来等这一答），核心关卡的卡也一样——
  // 前端只认后端给的四格（信封 / 消息 / 选项），不按 kind 猜按钮、也不在忙时把它藏起来。
  if (!p) return;
  gate.appendChild(renderCard(p, s));
}

/// 裁决卡：信封 / 消息（标题 / 正文 / 详情）/ 选项——四格都由后端给，前端照画（**扁平**，见契约）。
function renderCard(p, s) {
  const busy = !!(s && s.sending);
  const msg = p.message || {};
  const el = doc.createElement('div');
  el.className = 'gate-card';
  const who = doc.createElement('div');
  who.className = 'who';
  who.textContent = (p.envelope && p.envelope.name) || '';
  if (who.textContent) el.appendChild(who);
  // **谁在等、前面还排着几条**：队首之后还在等的那几张如实列出来（只有队首能答）。
  const waiting = p.waiting || [];
  if (waiting.length) {
    const wq = doc.createElement('div');
    wq.className = 'queue';
    wq.textContent =
      '后面还排着 ' + waiting.length + ' 张（先答上面这张）：' +
      waiting.map((w) => ((w.envelope && w.envelope.name) || '') + ' 在等 ' + (w.title || '')).join('；');
    el.appendChild(wq);
  }
  const q = doc.createElement('div');
  q.className = 'q';
  q.textContent = msg.title || '';
  el.appendChild(q);
  if (msg.body) {
    const bd = doc.createElement('div');
    bd.className = 'hint';
    bd.textContent = msg.body;
    el.appendChild(bd);
  }
  if (msg.detail) {
    const dt = doc.createElement('div');
    dt.className = 'advice';
    dt.textContent = msg.detail;
    el.appendChild(dt);
  }
  const inp = doc.createElement('input');
  inp.className = 'decision-input';
  inp.placeholder = '附言（可空；要你回话 / 先说一句的那一关必填）…';
  inp.disabled = busy;
  el.appendChild(inp);
  const bs = doc.createElement('div');
  bs.className = 'btns';
  for (const o of (p.options || [])) {
    const bt = doc.createElement('button');
    bt.className = 'btn btn-primary';
    bt.textContent = o.label;
    bt.disabled = busy;
    bt.onclick = () => answerCard(p.id, o.id, inp.value.trim());
    bs.appendChild(bt);
  }
  el.appendChild(bs);
  return el;
}

/* 回答一张裁决卡：**选项 id 是契约**（前端只把它发回去），附言是用户想说的话。
   回答只有这一条命令（卡片 id + 选项 id），校验属于当时那张卡由后端做。 */
async function answerCard(card, option, note) {
  const s = app.activeSession();
  if (!s || s.readonly || s.sending) return;
  if (note) s.lines.push({ cls: 'user', who: '用户', text: note, pending: true });
  s.sending = true;
  app.renderStream();
  renderGate(s);
  try {
    await api('POST', app.actionUrl('answer_card'), { session_id: s.sid, card: card, option: option, note: note });
    app.renderAll();
  } catch (err) {
    s.lines = s.lines.filter((x) => !x.pending);
    s.forceFull = true;
    s.lines.push({ cls: 'bad', who: '错误', text: err.message });
    app.renderAll();
  } finally {
    s.sending = false;
    app.markNeedState();
    app.renderAll();
  }
}

/* 改需求：**用户自己点的动作**（不是核心推的门）。
   服务端清掉旧需求行、在末尾追加新需求（不回档、不动文件），返回完整重放，前端整体重建。 */
async function updateTaskFlow() {
  const s = app.activeSession();
  if (!s || app.isBusy(s) || !s.can_update_task) return;
  const v = takeInput();
  if (!v) {
    ui.notice('改需求', '先在输入框写下新的本次需求，再点「改需求」。', 'info');
    return;
  }
  await updateTask(v);
}

async function updateTask(text) {
  const s = app.activeSession();
  if (!s || app.isBusy(s)) return;
  try {
    const r = await api('POST', app.actionUrl('update_task'), { session_id: s.sid, text });
    s.lines = []; s.pending = null; s.readonly = false; s.done = false;
    s.forceFull = true;
    for (const ev of (r.events || [])) app.absorb(s, ev);
    app.renderAll();
    ui.toast('ok', '已改需求');
  } catch (err) { ui.notice('操作失败', err.message, 'err'); }
}

function gateCard(q, btns) {
  const el = doc.createElement('div');
  el.className = 'gate-card';
  const qq = doc.createElement('div'); qq.className = 'q'; qq.textContent = q; el.appendChild(qq);
  const bs = doc.createElement('div'); bs.className = 'btns';
  for (const [label, fn] of btns) {
    const b = doc.createElement('button'); b.className = 'btn'; b.textContent = label;
    b.onclick = () => fn().catch((err) => ui.notice('操作失败', err.message, 'err'));
    bs.appendChild(b);
  }
  el.appendChild(bs);
  return el;
}

function takeInput() {
  const v = $('#input').value.trim();
  $('#input').value = '';
  app.atClose();
  app.autoGrow();
  return v;
}

export { renderGate, renderCard, answerCard, gateCard, updateTaskFlow, updateTask, takeInput };

/// 挂载：装配依赖并把本模块的入口交给组装点。
export function mount(root, deps) {
  ({ doc, api, ui, app } = deps);
  Object.assign(root, { renderGate, renderCard, answerCard, gateCard, updateTaskFlow, updateTask, takeInput });
}
