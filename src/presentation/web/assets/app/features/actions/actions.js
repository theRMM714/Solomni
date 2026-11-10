/* 目的：会话动作的唯一入口：把按钮意图变成 POST，再把回包按事件里的语义应用（乐观回显、回档、撤回同意、激活、继续）。
 * 管：动作 id 与 URL、乐观回显行、失败行的清场、回档/重置后重建视图、继续当前需求。
 * 不管：按钮长在哪、事件流怎么取（见 features/transcript/events.js 与 shell.js 的长轮询）。
 * 联动：由 app/main.js 装配；渲染经 app.renderAll / app.markNeedState 晚绑定，避免与转录模块互相 import。 */

let api, ui, app;

/* 本地名 → 动作 id 的映射：动作目录与校验都在核心（systools/tools.yaml），这里只做转写。 */
const ACT_IDS = { say: 'send_message', task: 'set_task' };

/* 动作只有一个入口 /api/actions/{id}：声明、授权、执行与审计都在核心那一处。 */
function actionUrl(id) { return '/api/actions/' + id; }

async function act(action, text) {
  const s = app.activeSession();
  if (!s) return;
  if (app.isBusy(s)) return;
  if (s.readonly) { ui.notice('历史回放为只读', '点「启用会话」后才能发送消息。', 'info'); return; }
  // 本地事实：我的命令在途（不是服务端运行态）——按钮立刻切「停止」，避免重复提交。
  s.sending = true;
  // 乐观回显：自己的发言立刻可见；服务端权威行到达时自动替换（见 app.absorb）。
  if (text && (action === 'say' || action === 'task' || action === 'answer')) {
    s.lines.push({ cls: 'user', who: '用户', text, pending: true });
  }
  app.renderStream();
  try {
    // 命令回包只有头部序号：事实（含自己那条发言的权威行）由事件流补进来。
    await api('POST', actionUrl(ACT_IDS[action] || action), { session_id: s.sid, text });
    s.awaiting = null;
    app.renderAll();
  } catch (err) {
    s.lines = s.lines.filter((x) => !x.pending);
    s.forceFull = true;
    s.lines.push({ cls: 'bad', who: '错误', text: err.message });
    app.renderAll();
  } finally {
    s.sending = false;
    // 回包只给头部序号；事实由事件流补进来。顺手要一次快照**对账**：命令已结束但它的
    // 收尾事件可能还在路上，运行态由事件（或这轮快照）说了算，不由"我在不在途"说了算。
    app.markNeedState();
    app.renderAll();
  }
}

/* 回档：留档（默认，可恢复）或删除（真的截断）；服务端回重放后的完整事件流，前端整体重建。 */
function rewindTo(id) {
  const s = app.activeSession();
  if (!s || app.isBusy(s)) return;
  app.choiceModal('回档到此处', '留档：保留内容、折叠成标记，可随时恢复，工作区文件（共享区与各 agent 沙箱）也一起记入档案；删除：真的删掉这一行及其后的全部，工作区文件一起回到该行（未提交内容进 lost 备份），删除点之后的留档一并作废，不可恢复。', [
    ['留档', 'btn', () => applyRewind(s, { id, mode: 'archive' })],
    ['删除', 'btn btn-danger', () => applyRewind(s, { id, mode: 'delete' })],
    ['取消', 'btn btn-ghost', () => {}],
  ]);
}

/* 恢复某次留档：删掉该标记及其后的内容（含留档期间的新工作）。 */
function restoreMark(mark) {
  const s = app.activeSession();
  if (!s || app.isBusy(s)) return;
  app.choiceModal('恢复这次留档', '恢复到留档之前：该标记及其后的全部内容（含留档期间的新工作）会被删除；工作区文件（共享区与各 agent 沙箱）一起回到留档点，未提交的改动进 lost 备份。', [
    ['恢复', 'btn btn-danger', () => applyRewind(s, { id: mark, mode: 'restore' })],
    ['取消', 'btn btn-ghost', () => {}],
  ]);
}

/* 回档请求的统一收尾：服务端回重放事件，前端整体重建。 */
async function applyRewind(s, body) {
  try {
    const r = await api('POST', actionUrl('rewind'), Object.assign({ session_id: s.sid }, body));
    if (typeof r.head === 'number') s.floor = r.head;
    // 行整体重建，折叠状态一并重来（避免旧键被新行复用）；与事件台上的 `rewound` 同一套逻辑。
    app.applySnapshot(s, (r.events || []).concat(r.live || []));
    s.workDirty = true;
    app.renderAll();
    ui.toast('ok', '已回档');
  } catch (err) {
    if (/被外部改动/.test(err.message) &&
        await ui.confirmBox('覆盖工作区外部改动', err.message + '\n\n确认后会先采纳这些外部改动，再把工作区回档到目标提交；未跟踪文件不受影响。', '仍要回档')) {
      try {
        const r = await api('POST', actionUrl('rewind'), Object.assign({ session_id: s.sid, force: true }, body));
        if (typeof r.head === 'number') s.floor = r.head;
        app.applySnapshot(s, (r.events || []).concat(r.live || []));
        s.workDirty = true;
        app.renderAll();
        ui.toast('ok', '已确认覆盖并回档');
      } catch (e2) { ui.notice('操作失败', e2.message, 'err'); }
      return;
    }
    if (/个留档/.test(err.message) &&
        await ui.confirmBox('删除会作废留档', err.message + '\n\n确认后会连同共享区与各 agent 沙箱一起回到该行；这些留档作废后不可恢复。', '仍要删除')) {
      try {
        const r = await api('POST', actionUrl('rewind'), Object.assign({ session_id: s.sid, force: true }, body));
        if (typeof r.head === 'number') s.floor = r.head;
        app.applySnapshot(s, (r.events || []).concat(r.live || []));
        s.workDirty = true;
        app.renderAll();
        ui.toast('ok', '已确认删除');
      } catch (e2) { ui.notice('操作失败', e2.message, 'err'); }
      return;
    }
    ui.notice('操作失败', err.message, 'err');
  }
}

/* 撤回某 agent 的同意（转录追加撤回行，协作才有意义）；值 = agent 实例名。 */
function withdrawAgree(agent) {
  const s = app.activeSession();
  if (!s || app.isBusy(s)) return;
  app.choiceModal('撤回同意', '撤回「' + agent + '」的同意？继续时会按剩余转录重新判定。', [
    ['撤回', 'btn btn-danger', async () => {
      try {
        await api('POST', actionUrl('withdraw'), { session_id: s.sid, agent });
        app.renderAll();
        ui.toast('ok', '已撤回「' + agent + '」的同意');
      } catch (err) { ui.notice('操作失败', err.message, 'err'); }
    }],
    ['取消', 'btn btn-ghost', () => {}],
  ]);
}

async function activateFlow() {
  const s = app.activeSession();
  if (s && s.workspace_missing) { ui.notice('工作区已不存在', '这条会话绑定的工作区已不可达：恢复目录或删除会话后再启用。', 'err'); return; }
  if (!s || !s.readonly || s.activating) return;
  s.activating = true;
  app.renderAll();
  try {
    const r = await api('POST', actionUrl('activate_session'), { session_id: s.sid });
    applyActivation(s, (r && r.session) || {});
    s.readonly = false;
    app.renderAll();
    ui.toast('ok', '会话已启用');
  } catch (err) {
    ui.notice('启用失败', err.message, 'err');
  } finally {
    s.activating = false;
    app.renderAll();
  }
}

function applyActivation(s, v) {
  s.mode = v.mode || s.mode;
  s.run = v.run || s.run || 'active';
  s.running = !!v.running;
  s.running_known = true;
  s.done = !!v.done;
  s.can_update_task = !!v.can_update_task;
  s.pending = v.pending || null;
  s.continue_action = v.continue || null;
  s.tier = v.tier || s.tier;
  s.tier_ready = v.tier_ready;
  s.tier_missing = v.tier_missing || [];
  // 启用返回的 SessionView 已经带生效权限：和会话状态一起贴回，别等下一次 /api/state 轮询。
  s.permission_profile = v.permission_profile || s.permission_profile;
  s.permission_label = v.permission_label || s.permission_label;
  s.permission_narrowed = !!v.permission_narrowed;
  s.permission_parent = v.permission_parent || null;
  // 启用时后端已按转录回填 shell 快照：贴回来，不用等下一次 /api/state。
  if (v.shell) s.shell = v.shell;
}

async function continueContext() {
  const s = app.activeSession();
  if (!s || s.readonly || app.isBusy(s) || !s.continue_action) return;
  const hint = s.continue_action;
  s.sending = true;
  s.continue_action = null; // 先收起；以事件与快照为准，失败时状态刷新会再给
  app.renderAll();
  try {
    await api('POST', actionUrl('control_session'), {
      session_id: s.sid,
      action: 'continue',
      reason: hint.reason || hint.label || '',
    });
  } catch (err) {
    s.lines.push({ cls: 'bad', who: '错误', text: err.message });
    app.renderAll();
  } finally {
    s.sending = false;
    app.markNeedState(); // 运行态以事件为准，快照只对账
    app.renderAll();
  }
}

export function mount(root, deps) {
  ({ api, ui, app } = deps);
  Object.assign(root, { ACT_IDS, actionUrl, act, rewindTo, restoreMark, applyRewind, withdrawAgree, activateFlow, applyActivation, continueContext });
}
