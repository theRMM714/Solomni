/* 目的：新建工作向导——选目录、起会话、切活跃、登记根路径。
 * 管：openWizard / pickWorkspaceDir / startSession / setActive / applyPathRoots / loadPathRoots。
 * 不管：不渲染转录、不管历史树；起完会话只提交给 app 的刷新入口。
 * 联动：agent 表单与模块选择来自 app/features/agents；会话配置来自 session-config。 */

import { wizardCopy } from './copy.js';

let g, doc, state, api, ui, app;
let areaInput, btn, busyButton, checkbox, emptyHint, field, fold, hint, numberInput, selectInput, textInput;
const actionUrl = (...a) => app.actionUrl(...a);
const atClose = (...a) => app.atClose(...a);
const atLoad = (...a) => app.atLoad(...a);
const buildRootList = (...a) => app.buildRootList(...a);
const hydrateHistory = (...a) => app.hydrateHistory(...a);
const loadWork = (...a) => app.loadWork(...a);
const refreshState = (...a) => app.refreshState(...a);
const renderAll = (...a) => app.renderAll(...a);
const renderGate = (...a) => app.renderGate(...a);
const renderStream = (...a) => app.renderStream(...a);
const renderTabs = (...a) => app.renderTabs(...a);
const syncComposer = (...a) => app.syncComposer(...a);

/// 选择器简写：模块内的 DOM 查找一律走注入的 doc。
const $ = (sel) => doc.querySelector(sel);

/* ---------- 新建工作向导 ---------- */
async function openWizard(draft) {
  // draft = 上一次没提交的草稿：补齐依赖后要把向导按原样重开，别把用户已经填的东西丢掉。
  // onclick 直接挂本函数时第一个参数是事件对象，所以按**标记**认草稿，不按真值认。
  const resumed = !!(draft && draft.__wizardDraft);
  // 类型表由后端给（systools/profiles.yaml）：界面只按它渲染名字与差别，不写死类型名。
  // 有名单的类型共用一份 agent 卡片列表：单人的恰好 0/1 个，多人的可多个。
  const w = resumed ? draft : {
    kind: '', agents: [], task: '', sandbox: 'host', guest: '', permission: 'offline',
    context_freshness: false, freshness_keep_percent: 80, __wizardDraft: true,
  };
  if (!Array.isArray(w.agents)) w.agents = [];
  // 名称与需求是**弹层里**输入框的现值（在本作用域之外）：由弹层装好后把取值钩子挂上来。
  let snapshotDraft = () => {};
  // 执行环境：可选项、可用性、逐项前置与"能不能补"全部来自后端（GET /api/tiers 的 policies 表）；
  // 界面按表渲染，不写死任何策略名——新增一个策略只要在组合根登记，这里不用动。
  let sandboxChoices = null;
  try { const r = await api('GET', '/api/tiers'); sandboxChoices = (r && r.tiers) || null; } catch (e) { sandboxChoices = null; }
  const policies = (sandboxChoices && sandboxChoices.policies) || [];
  if (!resumed && sandboxChoices && sandboxChoices.default) w.sandbox = sandboxChoices.default;
  if (!w.sandboxParams || typeof w.sandboxParams !== 'object') w.sandboxParams = {};
  const policyNow = () => policies.find((p) => p.decl && p.decl.id === w.sandbox) || null;
  const guestList = (sandboxChoices && sandboxChoices.guests) || [];
  // 参数里选了虚拟根时，它的装机事实按值去 guests 表里查（表是引擎给的，不按策略 id 分支）。
  const guestInfoNow = () => guestList.find((g) => g.guest === w.sandboxParams.guest) || null;
  const sandboxField = doc.createElement('div'); sandboxField.className = 'wf-field';
  const sandboxLabel = doc.createElement('div'); sandboxLabel.className = 'wf-label'; sandboxLabel.textContent = '执行环境';
  const sandboxGroup = doc.createElement('div'); sandboxGroup.className = 'form-col';
  for (const p of policies) {
    const rr = ui.radio(p.decl.name || p.decl.id, w.sandbox === p.decl.id, 'wf-sandbox', !p.available);
    rr.box.addEventListener('change', () => {
      if (!rr.box.checked) return;
      w.sandbox = p.decl.id; w.sandboxParams = {};
      renderSandbox();
    });
    sandboxGroup.appendChild(rr.wrap);
    // 每个策略的说明由它自己的声明给（界面不编、也不写死名字）。
    if (p.decl.hint) sandboxGroup.appendChild(ui.hint(p.decl.hint));
  }
  sandboxField.appendChild(sandboxLabel); sandboxField.appendChild(sandboxGroup);
  ui.attachHelp(sandboxField, wizardCopy.sandbox);
  // 上下文：过期内容替换与保底。挂在形态与执行环境之后：先定形态与环境，再定这轮的上下文策略。
  const ctxField = doc.createElement('div'); ctxField.className = 'wf-field';
  const ctxLabel = doc.createElement('div'); ctxLabel.className = 'wf-label'; ctxLabel.textContent = '上下文';
  const freshOn = checkbox('过期内容换成占位符', !!w.context_freshness, wizardCopy.context.lead);
  const keepNow = () => Math.max(0, Math.min(90, Math.round(Number(w.freshness_keep_percent)) || 0));
  const freshKeep = numberInput('保底缓存（前多少内容原样保留，%）', keepNow(), 0, 90);
  const syncCtx = () => { freshKeep.input.disabled = !w.context_freshness; };
  freshOn.box.addEventListener('change', () => { w.context_freshness = !!freshOn.box.checked; syncCtx(); });
  freshKeep.input.addEventListener('change', () => {
    w.freshness_keep_percent = Math.max(0, Math.min(90, Math.round(Number(freshKeep.input.value) || 0)));
    freshKeep.input.value = String(w.freshness_keep_percent);
  });
  ctxField.appendChild(ctxLabel); ctxField.appendChild(freshOn.wrap); ctxField.appendChild(freshKeep.wrap);
  ui.attachHelp(ctxField, wizardCopy.context);
  syncCtx();
  // 网络档位：只影响出站网络；执行环境自己的围栏另算。
  const profileField = doc.createElement('div'); profileField.className = 'wf-field';
  const profileLabel = doc.createElement('div'); profileLabel.className = 'wf-label'; profileLabel.textContent = '网络档位';
  const profileSel = selectInput([
    { value: 'offline', label: '离线（默认）' },
    { value: 'online', label: '联网' },
  ], w.permission || 'offline');
  profileSel.addEventListener('change', () => { w.permission = profileSel.value; });
  profileField.appendChild(profileLabel); profileField.appendChild(profileSel);
  profileField.appendChild(ui.hint(wizardCopy.permission));
  // 选中策略的现状与补法：缺项逐条列现状，能补就给一个入口（补法由后端说，界面不编话）。
  const needBox = doc.createElement('div'); needBox.className = 'form-col';
  // 选中策略的参数：取值域由后端解析好（如虚拟根清单），界面只按下拉/输入框画。
  const paramBox = doc.createElement('div'); paramBox.className = 'form-col';
  const gapHint = ui.hint('');
  /// 选中参数值在本机的事实（如虚拟根装了哪些运行能力、还缺什么）：按值查表，不按策略名分支。
  const guestHint = ui.hint('');
  function renderGuestHint() {
    const g = guestInfoNow();
    if (!g) { guestHint.textContent = ''; guestHint.className = 'wf-hint hidden'; return; }
    guestHint.className = 'wf-hint';
    guestHint.textContent = (g.os + ' / ' + g.arch) +
      (g.ready ? '：已装齐。' : '：还没装齐（缺 ' + (g.missing || []).join('、') + '）——点「补齐依赖…」装上再用。') +
      ((g.runtimes || []).length ? ' 自带运行能力：' + g.runtimes.join('、') + '。' : ' 它没有声明自带的运行能力。');
  }
  function syncGap() {
    const g = guestInfoNow();
    w.guestInfo = g;
    if (!g || !g.ready) { gapHint.textContent = ''; gapHint.className = 'wf-hint hidden'; return; }
    const covered = {};
    (g.runtimes || []).forEach((c) => { covered[c] = true; });
    Object.keys((sandboxChoices && sandboxChoices.capabilities) || {}).forEach((c) => { covered[c] = true; });
    const ids = w.agents.reduce((acc, a) => acc.concat(a.modules || []), []);
    const lack = [];
    for (const id of ids) {
      const m = state.modules.find((x) => x.id === id);
      if (!m) continue;
      const miss = (m.runtimes || []).filter((c) => !covered[c]);
      if (miss.length) lack.push(id + ' 需要 ' + miss.join('/'));
    }
    gapHint.className = 'wf-hint' + (lack.length ? '' : ' hidden');
    gapHint.textContent = lack.length
      ? wizardCopy.gapLead + lack.join('；') + '。' + wizardCopy.gapDetail
      : '';
  }
  /// 按当前选中的策略重画：前置（缺什么、怎么补）+ 参数（取值域由后端给）。
  function renderSandbox() {
    needBox.innerHTML = '';
    paramBox.innerHTML = '';
    // 前置逐项照抄后端：**每个当前不可用的策略**都列（不只选中的——选不中才更需要知道怎么修）。
    for (const p of policies) {
      const unmet = (p.needs || []).filter((n) => !n.met);
      if (!unmet.length) continue;
      needBox.appendChild(ui.hint(
        '「' + (p.decl.name || p.decl.id) + '」现在不能选：' + (p.refusal || '本机不具备它的前置条件') +
        '\n' + unmet.map((n) => '· ' + n.label + '：' + n.detail + (n.fix && n.fix.summary ? '（' + n.fix.summary + '）' : '')).join('\n'),
        'err'
      ));
      if (p.fixable) {
        needBox.appendChild(app.setupAction(snapshotDraft, { policy: p.decl.id, name: p.decl.name || p.decl.id }, () => openWizard(w)));
      }
    }
    const p = policyNow();
    const params = (p && p.decl && p.decl.params) || {};
    for (const name of Object.keys(params)) {
      const spec = params[name] || {};
      const label = spec.desc || name;
      let input;
      const isSelect = Array.isArray(spec.values) && spec.values.length > 0;
      if (isSelect) {
        const opts = spec.values.map((v) => ({ value: v, label: v }));
        let cur = w.sandboxParams[name] == null ? '' : String(w.sandboxParams[name]);
        if (!cur) { cur = opts[0].value; w.sandboxParams[name] = cur; }
        input = selectInput(opts, cur);
      } else {
        input = textInput(label + (spec.required ? '（必填）' : ''));
        if (w.sandboxParams[name] != null) input.value = String(w.sandboxParams[name]);
      }
      // 下拉（取值域由后端解析，如虚拟根）换值 = 换了一台机器的事实：整段重画（含"还缺什么/能不能补"）。
      const onChange = () => {
        w.sandboxParams[name] = input.value;
        if (isSelect) renderSandbox(); else syncGap();
      };
      input.addEventListener('change', onChange);
      if (input.tagName === 'INPUT') input.addEventListener('input', onChange);
      paramBox.appendChild(field(label, input));
    }
    paramBox.appendChild(guestHint);
    renderGuestHint();
    // 选中的虚拟根还没装齐：补法是同一个（策略自己登记的补齐器），这里给一个入口而不是让用户猜。
    const gNow = guestInfoNow();
    if (gNow && !gNow.ready && p && p.fixable) {
      paramBox.appendChild(app.setupAction(snapshotDraft, { policy: p.decl.id, name: p.decl.name || p.decl.id }, () => openWizard(w)));
    }
    syncGap();
  }
/* 选择工作区目录：目录列表来自本机 Web 的 /api/dirs；只选路径，不开文件。 */
function pickWorkspaceDir(onPick) {
  let parentPath = null;
  // 叠在“新建工作”之上开一层：关掉选择器后创建表单还在。
  ui.openModalLayer('选择工作区目录', (c) => {
    const pathIn = textInput('当前目录');
    pathIn.readOnly = true;
    const rootsRow = doc.createElement('div');
    rootsRow.style.display = 'flex';
    rootsRow.style.gap = '6px';
    rootsRow.style.flexWrap = 'wrap';
    const list = doc.createElement('div');
    list.style.maxHeight = '40vh';
    list.style.overflow = 'auto';
    const up = btn('上一级', 'btn btn-block');
    const pick = btn('选这个目录', 'btn btn-primary btn-block');
    const cancel = btn('取消', 'btn btn-block');
    async function load(p) {
      const r = await api('GET', '/api/dirs' + (p ? '?path=' + encodeURIComponent(p) : ''));
      pathIn.value = r.path || '';
      parentPath = r.parent || null;
      up.disabled = !parentPath;
      rootsRow.innerHTML = '';
      for (const root of (r.roots || [])) {
        const rb = btn(root, 'btn btn-ghost');
        rb.onclick = () => load(root).catch((e) => c.setMsg(e.message, true));
        rootsRow.appendChild(rb);
      }
      list.innerHTML = '';
      for (const d of (r.dirs || [])) {
        const b = btn(d.name, 'btn btn-block');
        b.style.textAlign = 'left';
        b.onclick = () => load(d.path).catch((e) => c.setMsg(e.message, true));
        list.appendChild(b);
      }
    }
    up.onclick = () => { if (parentPath) load(parentPath).catch((e) => c.setMsg(e.message, true)); };
    pick.onclick = () => { if (pathIn.value) { onPick(pathIn.value); c.close(); } };
    cancel.onclick = () => c.close();
    c.body.appendChild(pathIn);
    c.body.appendChild(rootsRow);
    c.body.appendChild(up);
    c.body.appendChild(list);
    c.body.appendChild(pick);
    c.body.appendChild(cancel);
    load(null).catch((e) => c.setMsg(e.message, true));
  }, true);
}

  ui.openModal('新建工作', (c) => {
    const nameIn = textInput('工作名称（必填，会话落盘目录名）');
    if (w.name) nameIn.value = w.name;   // 草稿重开：把用户填过的带回来
    // 可手填：选择器只列本机盘符，网络共享（UNC）要自己写——虚拟机档接受，本机档由后端如实拒绝。
    const workspaceIn = textInput('默认（会话内 work/）；也可填绝对路径或 UNC：\\\\server\\share\\目录');
    if (w.workspace) workspaceIn.value = w.workspace;
    // 手填也算草稿：重开时带回来的是最后填的写法。
    workspaceIn.addEventListener('input', () => { w.workspace = workspaceIn.value; });
    const wsPick = btn('选择目录', 'btn btn-ghost');
    const wsDefault = btn('默认', 'btn btn-ghost');
    wsPick.onclick = () => pickWorkspaceDir((p) => {
      workspaceIn.value = p; w.workspace = p;
    });
    wsDefault.onclick = () => {
      workspaceIn.value = ''; w.workspace = '';
    };
    const workspaceRow = doc.createElement('div');
    workspaceRow.style.display = 'flex';
    workspaceRow.style.gap = '8px';
    workspaceRow.appendChild(workspaceIn);
    workspaceRow.appendChild(wsPick);
    workspaceRow.appendChild(wsDefault);
    const workspaceWrap = doc.createElement('div');
    workspaceWrap.appendChild(workspaceRow);
    // 会话类型是可扩展的数据：选项、名字与差别都来自后端（app.typeOf），这里不复制一份类型知识。
    const types = state.types || [];
    const typeSel = selectInput(
      types.map((t) => ({ value: t.kind, label: t.name || t.kind })),
      w.kind || (types[0] && types[0].kind) || ''
    );
    if (!w.kind && types[0]) w.kind = types[0].kind;
    // 当前类型的形状：roster 假 = 决定权整块交给核心（不问名单）；flows 假 = 没有流程（单 agent 轮循环）。
    const typeNow = () => app.typeOf(w.kind) || null;
    const typeHint = doc.createElement('div'); typeHint.className = 'wf-hint';
    function typeHintText() {
      const t = typeNow();
      return (t && t.hint) || '';
    }
    const partWrap = doc.createElement('div'); partWrap.className = 'wf-field';
    const modelWrap = doc.createElement('div'); modelWrap.className = 'wf-models';
    const taskIn = areaInput('本次需求（协作必填；也可写上，核心据此推荐）');
    if (w.task) taskIn.value = w.task;   // 草稿重开：同上
    // 上面那个「补齐虚拟机依赖」按钮在本作用域之外：把抄草稿的钩子交给它，点击时才读现值。
    snapshotDraft = () => { w.name = nameIn.value; w.task = taskIn.value; };
    // 需求那一格与推荐按钮都是**非代理形态**才问的：形态一换就收起来（见 renderParts）。
    const taskField = field('本次需求', taskIn);
    const recBtn = btn('让核心推荐', 'btn btn-lead');
    const createBtn = btn('创建并开始', 'btn btn-primary');
    const cancelBtn = btn('取消', 'btn');
    // 没有可用模型就**不允许创建**：后端也会拒绝，这里先把原因说在前面。
    function createBlocked() {
      if (!state.models.length) {
        return '未登记任何模型，不能创建会话：先到「设置 → 模型与核心默认」登记供应商与模型';
      }
      const t = typeNow();
      if (!t) {
        return '还没取到会话类型表（systools/profiles.yaml）：刷新页面再试';
      }
      if (!t.roster) {
        return state.core ? '' : '代理会话必须设定核心默认模型：先到「设置 → 模型与核心默认」选定';
      }
      if (!w.agents.length) {
        return t.flows ? '先加入至少 1 个 agent' : '先选或新建一个 agent';
      }
      const noModel = w.agents.find((a) => !a.model);
      if (noModel && !state.core) {
        return 'agent「' + noModel.name + '」没有默认模型，且核心默认未设定：给它指定一个模型，或先选定核心默认';
      }
      return '';
    }
    function syncCreateButton() {
      const why = createBlocked();
      createBtn.disabled = !!why;
      createBtn.title = why;
    }
    cancelBtn.onclick = ui.closeModal;

    // 向导里的 agent 卡片：新建临时 / 从登记处加入；编辑已加入的卡片。
    function agentActions(single, hasAgent) {
      const row = doc.createElement('div'); row.className = 'wf-inline';
      const temp = btn(hasAgent ? '+ 更换为临时 agent' : '+ 新建临时 agent', 'btn');
      temp.onclick = () => {
        app.openTempAgentSheet({
          onAdd: (a) => { if (single) w.agents = [a]; else w.agents.push(a); renderAll(); },
          onSaveAsAgent: (a) => { if (single) w.agents = [a]; else w.agents.push(a); renderAll(); },
        });
      };
      const reg = btn(hasAgent ? '从登记处更换' : '从登记处加入', 'btn');
      reg.onclick = () => {
        app.openRegistryAgentPicker({
          single,
          taken: single ? [] : w.agents.map((a) => a.name),
          onPick: (a) => { if (single) w.agents = [a]; else w.agents.push(a); renderAll(); },
        });
      };
      row.appendChild(temp); row.appendChild(reg);
      if (!state.agents.length) {
        const hint = doc.createElement('div'); hint.className = 'wf-hint';
        hint.textContent = '（登记处还没有 agent：可以直接新建临时 agent）';
        row.appendChild(hint);
      }
      return row;
    }

    function saveCard(i) {
      const a = w.agents[i];
      if (!a || !a.transient) return;
      const name = a.name.trim();
      if (!name) { c.setMsg('agent 名字不能为空', true); return; }
      return (async () => {
        try {
          if (state.agents.some((x) => x.name === name)) {
            const ok = await ui.confirmBox('同名 agent', '登记处已有「' + name + '」，保存会更新它。继续？', '更新');
            if (!ok) return;
          }
          await c.mutate('upsert_agent', {
            name,
            modules: a.modules.slice(),
            model: a.model || undefined,
            note: (a.note || '').trim(),
            chara: (a.chara || '').trim(),
          });
          a.name = name; a.transient = false; a.reuse = true;
          renderAll(); c.setMsg('已保存 agent：' + name);
        } catch (e) { c.setMsg(e.message, true); }
      })();
    }

    function cardHandlers(i) {
      return {
        onEdit: (a) => {
          if (a.transient) {
            app.openTempAgentSheet({
              agent: a,
              title: '编辑临时 agent',
              onAdd: (next) => { w.agents[i] = next; renderAll(); },
              onSaveAsAgent: (next) => { w.agents[i] = next; renderAll(); },
            });
          } else {
            // 登记处 agent 的本次 copy：名字与模块由登记处决定，人设 / 模型可就本工作调整。
            app.openTempAgentSheet({
              agent: a,
              title: '编辑本次 agent（不改登记处）',
              lockedName: true,
              lockedModules: true,
              allowSaveAsAgent: false,
              onAdd: (next) => { w.agents[i] = next; renderAll(); },
            });
          }
        },
        onSave: () => saveCard(i),
        onRemove: () => { w.agents.splice(i, 1); renderAll(); },
      };
    }

    function agentSection() {
      const box = doc.createElement('div'); box.className = 'form-col';
      if (!(typeNow() || {}).flows) {
        if (w.agents.length) box.appendChild(app.agentCard(w.agents[0], cardHandlers(0)));
        else box.appendChild(emptyHint('（还没有选 agent）'));
        box.appendChild(agentActions(true, w.agents.length > 0));
        return box;
      }
      const list = doc.createElement('div'); list.className = 'agent-list';
      if (!w.agents.length) list.appendChild(emptyHint('（还没加入 agent）'));
      w.agents.forEach((a, i) => list.appendChild(app.agentCard(a, cardHandlers(i))));
      box.appendChild(list);
      box.appendChild(agentActions(false, false));
      return box;
    }

    function renderParts() {
      partWrap.innerHTML = '';
      const text = typeHintText();
      typeHint.innerHTML = '';
      if (text && typeof text === 'object') typeHint.appendChild(ui.detail(text));
      else typeHint.textContent = text;
      // 决定权整块交给核心的类型没有名单也没有模型要选：把该说的说清，把不该问的收起来。
      const t = typeNow() || {};
      const proxy = !t.roster;
      taskField.className = proxy ? 'wf-field hidden' : 'wf-field';
      recBtn.className = proxy ? 'btn btn-lead hidden' : 'btn btn-lead';
      if (proxy) {
        partWrap.appendChild(emptyHint(
          '（代理形态不需要选人）核心会用登记处里当前有效的 agent 与模块自己组队，' +
          '按你的目标建出一个或多个子工作，并代你回答它们的关卡。'
        ));
        return;
      }
      const lab = doc.createElement('div'); lab.className = 'wf-label';
      lab.textContent = t.flows ? 'agent 名单' : 'agent';
      partWrap.appendChild(lab);
      partWrap.appendChild(agentSection());
      const rule = doc.createElement('div'); rule.className = 'wf-hint';
      rule.textContent = t.flows ? '可多个，各自独立沙箱。' : '恰好 1 个；模块数不限。';
      partWrap.appendChild(rule);
    }

    function renderModels() {
      modelWrap.innerHTML = '';
      if (!(typeNow() || {}).roster) {
        modelWrap.appendChild(emptyHint('（代理会话走核心默认通道：模型按登记处的核心默认）'));
        if (!state.core) {
          modelWrap.appendChild(ui.hint('（核心默认模型未设定：代理会话不能创建；先到「设置 → 模型与核心默认」选定）', 'warn'));
        }
        return;
      }
      if (!state.models.length) {
        modelWrap.appendChild(ui.hint('（未登记任何模型：不能创建会话。先到「设置 → 模型与核心默认」登记。）', 'warn'));
        return;
      }
      if (!state.core && w.agents.some((a) => !a.model)) {
        modelWrap.appendChild(ui.hint(wizardCopy.noCore, 'warn'));
      }
    }

    function renderAll() { renderParts(); renderModels(); syncGap(); syncCreateButton(); }

    typeSel.addEventListener('change', () => {
      w.kind = typeSel.value;
      const t = typeNow() || {};
      // 决定权交给核心的类型默认显式选联网（用户仍可在下拉里改；不再由后端硬编码）。
      if (!t.roster) {
        w.permission = 'online';
        profileSel.value = 'online';
      }
      // 从多人切回单人：只保留第一张卡片（不静默替换成别的人）。
      if (!t.flows && w.agents.length > 1) w.agents = [w.agents[0]];
      renderAll();
    });

    recBtn.onclick = async () => {
      const t = typeNow() || {};
      if (!t.roster) { c.setMsg('这类会话不需要名单：核心自己挑人', true); return; }
      const task = taskIn.value.trim();
      if (!task) { c.setMsg('先写下本次需求，核心才能据此推荐', true); return; }
      c.setMsg('核心根据需求推荐中…');
      try {
        const r = await api('POST', '/api/suggest-models', { task, kind: w.kind });
        const rec = (r.agents || []).filter((a) => a && (a.name || (a.modules || []).length));
        if (!rec.length) { c.setMsg('核心未给出可用建议', true); return; }
        const asItem = (a) => {
          const reg = a.reuse === true ? state.agents.find((x) => x.name === a.name) : null;
          return {
            name: a.name || (a.modules && a.modules[0]) || 'agent',
            transient: a.reuse !== true,
            reuse: a.reuse === true,
            modules: ((reg && reg.modules) ? reg.modules : (a.modules || [])).slice(),
            model: a.model || (reg && reg.model) || '',
            chara: a.reuse === true && reg ? (reg.chara || a.chara || '') : (a.chara || ''),
            note: reg ? (reg.note || '') : '',
            why: a.why || null,
          };
        };
        const items = rec.map(asItem);
        const one = !t.flows;
        w.agents = one ? [items[0]] : items;
        renderAll();
        c.setMsg(one
          ? '核心建议 agent「' + items[0].name + '」（见下方卡片；名字、人设、模块、模型都可改）'
          : '核心已给出建议（见下方卡片；每条都可改）');
      } catch (e) { c.setMsg(e.message, true); }
    };

    // 真正下发（重名时先经 choiceModal 让用户裁决，绝不用原生 confirm）
    // 要等：虚拟机档在这一步里起 guest（拉 QEMU + 等 agent，首次几十秒到几分钟）——按钮转圈 + 说清在等什么。
    async function submit(body) {
      const release = busyButton(createBtn, '正在创建…');
      cancelBtn.disabled = true;
      c.setMsg(wizardCopy.creating);
      try {
        await startSession(body);
        ui.closeModal();
      } catch (e) {
        c.setMsg('创建失败：' + e.message, true);
      } finally {
        release();
        cancelBtn.disabled = false;
      }
    }

    createBtn.onclick = async () => {
      const blocked = createBlocked();
      if (blocked) { c.setMsg(blocked, true); return; }
      const name = nameIn.value.trim();
      if (!name) { c.setMsg('工作名称必填', true); return; }
      const task = taskIn.value.trim();
      // 策略声明的必填参数（虚拟根这类）：缺了先在这里拦，说法照抄声明，不按策略名编话。
      const pNow = policyNow();
      const paramsNow = (pNow && pNow.decl && pNow.decl.params) || {};
      for (const key of Object.keys(paramsNow)) {
        const spec = paramsNow[key] || {};
        if (spec.required && !String(w.sandboxParams[key] || '').trim()) {
          c.setMsg('执行环境「' + ((pNow.decl && pNow.decl.name) || w.sandbox) + '」要填参数 ' + key + (spec.desc ? '：' + spec.desc : ''), true);
          return;
        }
      }
      const gNow = guestInfoNow();
      if (gNow && !gNow.ready) {
        const miss = (gNow.missing || []).join('、');
        c.setMsg('这个虚拟根还没装齐' + (miss ? '（缺 ' + miss + '）' : '') + '：先点「补齐依赖…」装上再用。', true);
        return;
      }
      const sandboxDecl = { policy: w.sandbox, params: w.sandboxParams, pins: {} };
      let agents;
      const t = typeNow() || {};
      if (!t.roster) {
        // 决定权交给核心的类型：没有名单、没有需求——选这个类型就是授予全权（档位由上面的选择显式落盘）。
        const proxyBody = { name, kind: w.kind, agents: [], sandbox: sandboxDecl, permission: w.permission || 'online' };
        proxyBody.context_freshness = !!w.context_freshness;
        proxyBody.freshness_keep_percent = keepNow();
        const proxyWs = workspaceIn.value.trim();
        if (proxyWs) {
          proxyBody.workspace = proxyWs;
        }
        await submit(proxyBody);
        return;
      }
      if (!t.flows) {
        if (!w.agents.length) { c.setMsg('先选或新建一个 agent', true); return; }
        const a = w.agents[0];
        const mods = (a.modules || []).slice();
        const nm = (a.name || '').trim() || mods[0] || '';
        if (!nm) { c.setMsg('给这个 agent 一个名字（或至少 1 个模块）', true); return; }
        agents = [{
          name: nm,
          transient: !!a.transient,
          modules: mods,
          model: a.model || undefined,
          chara: (a.chara || '').trim(),
        }];
      } else {
        if (!w.agents.length) { c.setMsg('请先加入至少 1 个 agent', true); return; }
        if (!task) { c.setMsg('有流程的会话必须填写本次需求', true); return; }
        agents = w.agents.map((a) => ({
          name: a.name.trim(),
          transient: !!a.transient,
          modules: a.modules.slice(),
          model: a.model || undefined,
          chara: (a.chara || '').trim(),
        }));
        if (agents.some((a) => !a.name)) { c.setMsg('agent 名字不能为空', true); return; }
      }
      const names = agents.map((a) => a.name);
      const dup = names.filter((n, i) => names.indexOf(n) !== i)[0];
      const body = { name, kind: w.kind, agents, sandbox: sandboxDecl, permission: w.permission || 'offline' };
      body.context_freshness = !!w.context_freshness;
      body.freshness_keep_percent = keepNow();
      const bodyWs = workspaceIn.value.trim();
      if (bodyWs) {
        body.workspace = bodyWs;
      }
      if (task) body.task = task;
      if (dup) {
        app.choiceModal('agent 重名', '本次工作里有同名 agent「' + dup + '」。', [
          ['按现名继续（后端自动加尾号 -2…）', 'btn btn-primary', () => submit(body)],
          ['回去改名', 'btn btn-ghost', () => {}],
        ]);
        return;
      }
      await submit(body);
    };

    c.body.appendChild(field('工作名称', nameIn));
    c.body.appendChild(field('工作区（可选）', workspaceWrap, wizardCopy.workspace));
    c.body.appendChild(field('会话类型', typeSel));
    c.body.appendChild(typeHint);
    c.body.appendChild(sandboxField);
    c.body.appendChild(ctxField);
    c.body.appendChild(profileField);
    c.body.appendChild(needBox);
    c.body.appendChild(paramBox);
    c.body.appendChild(gapHint);
    c.body.appendChild(partWrap);
    c.body.appendChild(modelWrap);
    c.body.appendChild(taskField);
    const actions = doc.createElement('div');
    actions.className = 'form-actions';
    actions.appendChild(recBtn);
    actions.appendChild(cancelBtn);
    actions.appendChild(createBtn);
    // 有 foot 就固定在底部；没有（老调用）仍随正文走。
    (c.foot || c.body).appendChild(actions);

    renderParts(); renderModels(); renderSandbox(); syncCreateButton();
    // 别的标签页 / 设置在轮询后改了登记处：只换模型提示与创建按钮，不重建名字 / 需求输入框。
    c.onRegistryChange(() => { renderModels(); syncCreateButton(); });
  }, true);
}

/* ---------- 会话 ---------- */
async function startSession(body) {
  const r = await api('POST', actionUrl('create_session'), body);
  const sid = r.sid;
  // 新会话的形状按类型表判：有流程的会排队推进，决定权交给核心的不收追问。
  const t = app.typeOf(body.kind) || {};
  const s = {
    sid, kind: body.kind, flows: !!t.flows, auto: !t.roster, title: body.name || sid, run: 'active',
    lines: [], live: [], pending: null, sending: false,
    running: false, working: null, running_known: false,
    context: null, shell: null, work: null, workAt: 0, workDirty: false, workError: '',
    context_freshness: body.context_freshness === true, freshness_keep_percent: Number(body.freshness_keep_percent == null ? 80 : body.freshness_keep_percent),
    done: false, awaiting: null, fold: {}, scroll: {},
  };
  // 开场事实由 /api/sessions 那一刻就推给事件台了（回包只给头部序号），所以这里**合流**一次：
  // 盘上转录 + 事件台尾巴一次给全，水位也随之下定（此后的批次按水位筛）。
  s.hydrated = false;
  s.buffer = [];
  state.sessions.set(sid, s);
  hydrateHistory(s);
  // 开场事实不随回包走：它们已经在事件台上，长轮询会照 seq 补进来。
  setActive(sid);
  renderAll();
  // 会话历史随建随现，但不拿它当关弹层的门槛：派一次就够，慢或失败只让侧栏晚一点刷出来。
  refreshState().catch(() => {});
  return s;
}

function setActive(sid) {
  atClose(); // 换会话 = 换一份文件清单，菜单先收起来
  state.activeSid = sid;
  // 换会话先清掉上一份根（免得拿别人的根去缩）；缓存里有就同步先缩，避免先长后短的闪烁。
  state.pathRoots = null;
  const cached = app.filesCache.get(sid);
  if (cached) state.pathRoots = buildRootList(cached.roots);
  const s = state.sessions.get(sid);
  // 流程视图跟着活跃会话走：它是整页的一部分，漏画就会把上一个会话的链留在状态栏上方。
  renderTabs(); renderStream(true); renderGate(s); app.renderFlow(s);
  if (s) { syncComposer(s); loadWork(sid); }
  if (!s || !s.readonly) $('#input').focus();
  if (!cached) loadPathRoots(sid); // 没缓存就去拉一次（失败静默），拿到后补一次重渲染
}

/* 把 roots 应用到当前活动会话；返回是否变化（变了才值得重渲染）。 */
function applyPathRoots(sid, data) {
  if (state.activeSid !== sid) return false;
  const next = buildRootList(data && data.roots);
  const same = JSON.stringify(next) === JSON.stringify(state.pathRoots);
  state.pathRoots = next;
  return !same;
}

/* 会话激活时就把路径根拉到手（复用 @ 菜单那份缓存）；失败静默，不影响会话本身。 */
async function loadPathRoots(sid) {
  const data = await atLoad(sid);
  if (!data) return;
  if (applyPathRoots(sid, data)) renderStream(true); // 根到手 → 重渲染一次，长路径立刻变短
}

/// 挂载：装配依赖并把本模块的入口交给组装点。
export function mount(root, deps) {
  ({ g, doc, state, api, ui, app } = deps);
  ({ areaInput, btn, busyButton, checkbox, emptyHint, field, fold, hint, numberInput, selectInput, textInput } = ui);
  Object.assign(root, { openWizard, startSession, setActive, applyPathRoots, loadPathRoots });
}
