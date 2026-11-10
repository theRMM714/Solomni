/* 目的：新建工作向导——选目录、起会话、切活跃、登记根路径。
 * 管：openWizard / pickWorkspaceDir / startSession / setActive / applyPathRoots / loadPathRoots。
 * 不管：不渲染转录、不管历史树；起完会话只提交给 app 的刷新入口。
 * 联动：agent 表单与模块选择来自 app/features/agents；会话配置来自 session-config。 */

let g, doc, state, api, ui, app;
let areaInput, btn, busyButton, emptyHint, field, fold, hint, selectInput, textInput;
const actionUrl = (...a) => app.actionUrl(...a);
const atClose = (...a) => app.atClose(...a);
const atLoad = (...a) => app.atLoad(...a);
const buildRootList = (...a) => app.buildRootList(...a);
const cfgRadio = (...a) => app.cfgRadio(...a);
const closeModal = (...a) => app.closeModal(...a);
const confirmBox = (...a) => app.confirmBox(...a);
const guestPicker = (...a) => app.guestPicker(...a);
const hydrateHistory = (...a) => app.hydrateHistory(...a);
const loadWork = (...a) => app.loadWork(...a);
const openModal = (...a) => app.openModal(...a);
const openModalLayer = (...a) => app.openModalLayer(...a);
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
  // 两种形态共用一份 agent 卡片列表：single 恰好 0/1 个；collab 可多个。
  const w = resumed ? draft : {
    mode: 'single', agents: [], task: '', tier: 'host', guest: '', permission: 'workspace', __wizardDraft: true,
  };
  if (!Array.isArray(w.agents)) w.agents = [];
  // 名称与需求是**弹层里**输入框的现值（在本作用域之外）：由弹层装好后把取值钩子挂上来。
  let snapshotDraft = () => {};
  // 档位选择：默认档与虚拟机档可用性由后端给（GET /api/tiers，与「开始」的校验同源）。
  let tiers = null;
  try { const r = await api('GET', '/api/tiers'); tiers = (r && r.tiers) || null; } catch (e) { tiers = null; }
  if (!resumed && tiers && tiers.default) w.tier = tiers.default;
  // 档位那一格：虚拟机档不可用时禁用，并逐项说明缺什么、怎么补（照抄后端事实，不自己编话）。
  const tierField = doc.createElement('div'); tierField.className = 'wf-field';
  const tierLabel = doc.createElement('div'); tierLabel.className = 'wf-label'; tierLabel.textContent = '执行档位';
  const hostR = cfgRadio('本机档 —— 脚本直接在宿主上跑：宿主自备解释器；隔离就是宿主本身（默认）。', w.tier !== 'vm', 'wf-tier');
  const vmOk = !!(tiers && tiers.vm_available);
  const vmR = cfgRadio('虚拟机档 —— 一整套 guest，隔离更强；按模块声明装载运行包，依赖 guest 本体。', w.tier === 'vm', 'wf-tier', !vmOk);
  hostR.box.addEventListener('change', () => { if (hostR.box.checked) { w.tier = 'host'; syncWizardTier(); } });
  vmR.box.addEventListener('change', () => { if (vmR.box.checked) { w.tier = 'vm'; syncWizardTier(); } });
  tierField.appendChild(tierLabel); tierField.appendChild(hostR.wrap); tierField.appendChild(vmR.wrap);
  const profileField = doc.createElement('div'); profileField.className = 'wf-field';
  const profileLabel = doc.createElement('div'); profileLabel.className = 'wf-label'; profileLabel.textContent = '权限档位';
  const profileSel = selectInput([
    { value: 'view', label: '仅可查看' },
    { value: 'workspace', label: '工作区内修改（默认）' },
    { value: 'external', label: '外部可达（逐次确认）' },
    { value: 'all_reach', label: '全可达（自动放行）' },
  ], w.permission || 'workspace');
  profileSel.addEventListener('change', () => { w.permission = profileSel.value; });
  profileField.appendChild(profileLabel); profileField.appendChild(profileSel);
  profileField.appendChild(ui.hint('档位只决定能到哪、要不要问；创建后仍可在会话配置里收窄。代理形态默认全可达。'));
  if (!vmOk) {
    const reqs = (tiers && tiers.vm_requirements) || [];
    // 只列**现状**：怎么补这件事由下面的按钮承担（把指引和动作分成两件事，用户不会只看到话看不到路）。
    const lines = reqs.filter((r) => !r.met).map((r) => '· ' + r.detail);
    tierField.appendChild(ui.hint(
      '虚拟机档现在不能选：' + ((tiers && tiers.vm_unavailable_reason) || '本机不具备虚拟机档的前置条件') +
      (lines.length ? '\n' + lines.join('\n') : ''),
      'err'
    ));
    // 装完把向导按草稿重开：判据是后端按新状态算的，界面不自己改结论。
    // 先把用户填的名称与需求抄进草稿（它们只在弹层里读得到），否则重开会把它们清掉。
    tierField.appendChild(app.setupAction(snapshotDraft, () => openWizard(w)));
  }

  // 虚拟机档在**创建时**就要选一个虚拟根（会话级选择；机器能力已在上面判过）。
  let syncGuestSetup = () => {};
  const pick = guestPicker(tiers && tiers.guests, w.guest, (v, g) => {
    w.guest = v; w.guestInfo = g; syncGuestSetup();
  });
  // 机器前置齐、但选中的虚拟根没装齐时，虚拟根选择器下面给同一个补齐动作（与机器前置入口不重复）。
  const guestSetup = app.setupAction(snapshotDraft, () => openWizard(w));
  guestSetup.classList.add('hidden');
  syncGuestSetup = () => {
    const show = vmOk && w.tier === 'vm' && (!w.guestInfo || !w.guestInfo.ready);
    if (show) guestSetup.classList.remove('hidden');
    else guestSetup.classList.add('hidden');
  };
  // 预判：这个虚拟根（+ 包库）满足不了本次点到的哪些模块——**只是提示**，判据仍以创建时的后端校验为准。
  const gapHint = ui.hint('');
  function syncGuestGap() {
    if (w.tier !== 'vm') { gapHint.textContent = ''; return; }
    const g = w.guestInfo;
    if (!g || !g.ready) { gapHint.textContent = ''; return; }
    const covered = {};
    (g.runtimes || []).forEach((c) => { covered[c] = true; });
    Object.keys((tiers && tiers.available) || {}).forEach((c) => { covered[c] = true; });
    const ids = w.agents.reduce((acc, a) => acc.concat(a.modules || []), []);
    const lack = [];
    for (const id of ids) {
      const m = state.modules.find((x) => x.id === id);
      if (!m) continue;
      const miss = (m.runtimes || []).filter((c) => !covered[c]);
      if (miss.length) lack.push(id + ' 需要 ' + miss.join('/'));
    }
    gapHint.textContent = lack.length
      ? '这个虚拟机可能要缺：' + lack.join('；') + '。可选：把运行包放进 runtimes/、换一个虚拟机、或改用本机档（创建时后端会按同一份事实再判一次）。'
      : '';
  }
  function syncWizardTier() {
    pick.wrap.className = w.tier === 'vm' ? 'wf-field' : 'wf-field hidden';
    pick.hint.className = 'wf-hint' + (w.tier === 'vm' ? '' : ' hidden');
    gapHint.className = 'wf-hint' + (w.tier === 'vm' ? '' : ' hidden');
    pick.sync();
    syncGuestSetup();
    syncGuestGap();
  }
/* 选择工作区目录：目录列表来自本机 Web 的 /api/dirs；只选路径，不开文件。 */
function pickWorkspaceDir(onPick) {
  let parentPath = null;
  // 叠在“新建工作”之上开一层：关掉选择器后创建表单还在。
  openModalLayer('选择工作区目录', (c) => {
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

  openModal('新建工作', (c) => {
    const nameIn = textInput('工作名称（必填，会话落盘目录名）');
    if (w.name) nameIn.value = w.name;   // 草稿重开：把用户填过的带回来
    // 可手填：选择器只列本机盘符，网络共享（UNC）要自己写——虚拟机档接受，本机档由后端如实拒绝。
    const workspaceIn = textInput('默认（会话内 work/）；也可填绝对路径或 UNC：\\\\server\\share\\目录');
    if (w.workspace) workspaceIn.value = w.workspace;
    // 手填也算草稿：重开时带回来的是最后填的写法。
    workspaceIn.addEventListener('input', () => { w.workspace = workspaceIn.value; });
    const wsPick = btn('选择目录', 'btn btn-ghost');
    const wsDefault = btn('默认', 'btn btn-ghost');
    wsPick.onclick = () => pickWorkspaceDir((p) => { workspaceIn.value = p; w.workspace = p; });
    wsDefault.onclick = () => { workspaceIn.value = ''; w.workspace = ''; };
    const workspaceRow = doc.createElement('div');
    workspaceRow.style.display = 'flex';
    workspaceRow.style.gap = '8px';
    workspaceRow.appendChild(workspaceIn);
    workspaceRow.appendChild(wsPick);
    workspaceRow.appendChild(wsDefault);
    const wsHint = doc.createElement('div'); wsHint.className = 'wf-hint';
    wsHint.textContent = '本机档只接受本机目录（网络工作区会被拒绝，容器围栏访问不了私有网络）；' +
      '虚拟机档可填 UNC 共享下的目录，凭据用当前登录用户。';
    const workspaceWrap = doc.createElement('div');
    workspaceWrap.appendChild(workspaceRow);
    workspaceWrap.appendChild(wsHint);
    const modeSel = selectInput([
      { value: 'single', label: '单 agent（1 个或多个模块）' },
      { value: 'collab', label: '协作（多个 agent）' },
      { value: 'proxy', label: '代理（把决定权整块交给核心）' },
    ], 'single');
    // 形态说明随选择走（三种形态的差别就在这一句里说清）。
    const modeHint = doc.createElement('div'); modeHint.className = 'wf-hint';
    function modeHintText() {
      if (w.mode === 'proxy') {
        return '代理：没有名单要选——核心自己挑人、建子工作并代你决定（选这个形态就是授予全权）。\n' +
          '它的子会话不会把整份对话推给它；它靠观察与倒查消息决定下一步。想接管就按「停止」：相关的会话会一起停下。';
      }
      if (w.mode === 'collab') {
        return '协作：多个 agent 各自独立沙箱，先分权协商、再按任务链执行，由核心统一验收。';
      }
      return '单 agent：选 1 个模块 = 直连式；选多个 = 组合式。';
    }
    const partWrap = doc.createElement('div'); partWrap.className = 'wf-field';
    const modelWrap = doc.createElement('div'); modelWrap.className = 'wf-models';
    const taskIn = areaInput('本次需求（协作必填；也可写上，核心据此推荐）');
    if (w.task) taskIn.value = w.task;   // 草稿重开：同上
    // 上面那个「补齐虚拟机依赖」按钮在本作用域之外：把抄草稿的钩子交给它，点击时才读现值。
    snapshotDraft = () => { w.name = nameIn.value; w.task = taskIn.value; };
    // 需求那一格与推荐按钮都是**非代理形态**才问的：形态一换就收起来（见 renderParts）。
    const taskField = field('本次需求', taskIn);
    const recBtn = btn('让核心推荐', 'btn btn-block');
    const createBtn = btn('创建并开始', 'btn btn-primary btn-block');
    const cancelBtn = btn('取消', 'btn btn-block');
    // 没有可用模型就**不允许创建**：后端也会拒绝，这里先把原因说在前面。
    function createBlocked() {
      if (!state.models.length) {
        return '未登记任何模型，不能创建会话：先到「设置 → 模型与核心默认」登记供应商与模型';
      }
      if (w.mode === 'proxy') {
        return state.core ? '' : '代理会话必须设定核心默认模型：先到「设置 → 模型与核心默认」选定';
      }
      if (!w.agents.length) {
        return w.mode === 'single' ? '单 agent：先选或新建一个 agent' : '协作：先加入至少 1 个 agent';
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
    cancelBtn.onclick = closeModal;

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
            const ok = await confirmBox('同名 agent', '登记处已有「' + name + '」，保存会更新它。继续？', '更新');
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
      if (w.mode === 'single') {
        if (w.agents.length) box.appendChild(app.agentCard(w.agents[0], cardHandlers(0)));
        else box.appendChild(emptyHint('（还没有选 agent：新建临时 agent，或从登记处加入）'));
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
      modeHint.textContent = modeHintText();
      // 代理形态没有名单也没有模型要选：把该说的说清，把不该问的收起来。
      const proxy = w.mode === 'proxy';
      taskField.className = proxy ? 'wf-field hidden' : 'wf-field';
      recBtn.className = proxy ? 'btn btn-block hidden' : 'btn btn-block';
      if (proxy) {
        partWrap.appendChild(emptyHint(
          '（代理形态不需要选人）核心会用登记处里当前有效的 agent 与模块自己组队，' +
          '按你的目标建出一个或多个子工作，并代你回答它们的关卡。'
        ));
        return;
      }
      const lab = doc.createElement('div'); lab.className = 'wf-label';
      lab.textContent = w.mode === 'single'
        ? 'agent（单 agent：恰好 1 个；模块数不限）'
        : 'agent 名单（协作：可多个，各自独立沙箱）';
      partWrap.appendChild(lab);
      partWrap.appendChild(agentSection());
    }

    function renderModels() {
      modelWrap.innerHTML = '';
      if (w.mode === 'proxy') {
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
        modelWrap.appendChild(ui.hint('（核心默认未设定：没指定模型的 agent 不能创建；在卡片里给它选一个模型，或先选定核心默认）', 'warn'));
      }
    }

    function renderAll() { renderParts(); renderModels(); syncGuestGap(); syncCreateButton(); }

    modeSel.addEventListener('change', () => {
      w.mode = modeSel.value;
      // 代理形态默认显式选全可达（用户仍可在下拉里改；不再由后端硬编码）。
      if (w.mode === 'proxy') {
        w.permission = 'all_reach';
        profileSel.value = 'all_reach';
      }
      // 从协作切回单 agent：只保留第一张卡片（不静默替换成别的人）。
      if (w.mode === 'single' && w.agents.length > 1) w.agents = [w.agents[0]];
      renderAll();
    });

    recBtn.onclick = async () => {
      if (w.mode === 'proxy') { c.setMsg('代理形态不需要名单：核心自己挑人', true); return; }
      const task = taskIn.value.trim();
      if (!task) { c.setMsg('先写下本次需求，核心才能据此推荐', true); return; }
      c.setMsg('核心根据需求推荐中…');
      try {
        const r = await api('POST', '/api/suggest-models', { task, mode: w.mode });
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
        w.agents = w.mode === 'single' ? [items[0]] : items;
        renderAll();
        c.setMsg(w.mode === 'single'
          ? '核心建议 agent「' + items[0].name + '」（见下方卡片；名字、人设、模块、模型都可改）'
          : '核心已给出建议（见下方卡片；每条都可改）');
      } catch (e) { c.setMsg(e.message, true); }
    };

    // 真正下发（重名时先经 choiceModal 让用户裁决，绝不用原生 confirm）
    // 要等：虚拟机档在这一步里起 guest（拉 QEMU + 等 agent，首次几十秒到几分钟）——按钮转圈 + 说清在等什么。
    async function submit(body) {
      const release = busyButton(createBtn, '正在创建…');
      cancelBtn.disabled = true;
      c.setMsg(w.tier === 'vm'
        ? '正在创建：虚拟机档要先起一台 guest（首次可能要几十秒到几分钟）；缺编译产物时还会先在 guest 里编一次…'
        : '正在创建：缺编译产物时先补编一次，编完就开工…');
      try {
        await startSession(body);
        closeModal();
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
      const vmGuest = (w.guest || '').trim();
      if (w.tier === 'vm' && !vmGuest) {
        c.setMsg('虚拟机档要先选一个虚拟机（虚拟根）：点上面的「补齐虚拟机依赖」装上，或换一个已装好的。', true);
        return;
      }
      if (w.tier === 'vm' && w.guestInfo && !w.guestInfo.ready) {
        const miss = (w.guestInfo.missing || []).join('、');
        c.setMsg('这个虚拟根还没装齐' + (miss ? '（缺 ' + miss + '）' : '') + '：先点「补齐虚拟机依赖…」装上再用。', true);
        return;
      }
      let agents;
      if (w.mode === 'proxy') {
        // 代理：没有名单、没有需求——选这个形态就是授予全权（档位由上面的选择显式落盘）。
        const proxyBody = { name, mode: 'proxy', agents: [], tier: w.tier, guest: w.tier === 'vm' ? vmGuest : null, permission: w.permission || 'all_reach' };
        if (workspaceIn.value.trim()) proxyBody.workspace = workspaceIn.value.trim();
        await submit(proxyBody);
        return;
      }
      if (w.mode === 'single') {
        if (!w.agents.length) { c.setMsg('单 agent：先选或新建一个 agent', true); return; }
        const a = w.agents[0];
        const mods = (a.modules || []).slice();
        const nm = (a.name || '').trim() || mods[0] || '';
        if (!nm) { c.setMsg('单 agent：给这个 agent 一个名字（或至少 1 个模块）', true); return; }
        agents = [{
          name: nm,
          transient: !!a.transient,
          modules: mods,
          model: a.model || undefined,
          chara: (a.chara || '').trim(),
        }];
      } else {
        if (!w.agents.length) { c.setMsg('请先加入至少 1 个 agent', true); return; }
        if (!task) { c.setMsg('协作模式必须填写本次需求', true); return; }
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
      const body = { name, mode: w.mode, agents, tier: w.tier, permission: w.permission || 'workspace' };
      if (workspaceIn.value.trim()) body.workspace = workspaceIn.value.trim();
      if (w.tier === 'vm') body.guest = vmGuest;
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
    c.body.appendChild(field('工作区（可选）', workspaceWrap));
    c.body.appendChild(field('形态', modeSel));
    c.body.appendChild(modeHint);
    c.body.appendChild(tierField);
    c.body.appendChild(profileField);
    c.body.appendChild(pick.wrap);
    c.body.appendChild(pick.hint);
    c.body.appendChild(guestSetup);
    c.body.appendChild(gapHint);
    c.body.appendChild(partWrap);
    c.body.appendChild(modelWrap);
    c.body.appendChild(taskField);
    c.body.appendChild(recBtn);
    c.body.appendChild(createBtn);
    c.body.appendChild(cancelBtn);

    renderParts(); renderModels(); syncWizardTier(); syncCreateButton();
    // 别的标签页 / 设置在轮询后改了登记处：只换模型提示与创建按钮，不重建名字 / 需求输入框。
    c.onRegistryChange(() => { renderModels(); syncCreateButton(); });
  }, true);
}

/* ---------- 会话 ---------- */
async function startSession(body) {
  const r = await api('POST', actionUrl('create_session'), body);
  const sid = r.sid;
  const s = {
    sid, mode: body.mode, title: body.name || sid, run: 'active',
    lines: [], live: [], pending: null, sending: false,
    running: false, working: null, running_known: false,
    context: null, shell: null, work: null, workAt: 0, workDirty: false, workError: '',
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
  await refreshState(); // 会话历史随建随现
  return s;
}

function setActive(sid) {
  atClose(); // 换会话 = 换一份文件清单，菜单先收起来
  state.activeSid = sid;
  // 换会话先清掉上一份根（免得拿别人的根去缩）；缓存里有就同步先缩，避免先长后短的闪烁。
  state.pathRoots = null;
  const cached = app.filesCache.get(sid);
  if (cached) state.pathRoots = buildRootList(cached.roots);
  renderTabs(); renderStream(true); renderGate(state.sessions.get(sid));
  const s = state.sessions.get(sid);
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
  ({ areaInput, btn, busyButton, emptyHint, field, fold, hint, selectInput, textInput } = ui);
  Object.assign(root, { openWizard, startSession, setActive, applyPathRoots, loadPathRoots });
}
