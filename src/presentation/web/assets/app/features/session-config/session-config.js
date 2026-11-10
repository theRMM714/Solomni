/* 目的：会话配置弹层——模型、思考档、执行档、模块、人设。
 * 管：openConfig / loadConfig / buildConfigForm / cfgAgentBlock / cfgRuntimeBlock / guestPicker。
 * 不管：不写会话状态、不发事件；保存走统一动作分发。
 * 联动：模型选项来自 app.modelOptions；弹层来自 app/openModal；刷新走 app.refreshState。 */

import { thinkingLabel } from '../../stores/settings.js';

let g, doc, api, ui, app;
let areaInput, btn, emptyHint, field, hint, selectInput, syncSelect, textInput;
const actionUrl = (...a) => app.actionUrl(...a);
const modelOptions = (...a) => app.modelOptions(...a);
const refreshState = (...a) => app.refreshState(...a);

/// 从会话列表的「编辑」进入：面板自己拉配置、自己重绘（保存 / 重新扫描都走这里）。
function openConfig(sid) {
  ui.openModal('配置：' + sid, (c) => {
    const box = doc.createElement('div'); box.className = 'cfg';
    c.body.appendChild(box);
    // 登记处变了只同步模型下拉；重读配置会换掉 DOM，所以这里只订阅一次，由 syncModels 指向最新一份。
    let syncModels = () => {};
    c.onRegistryChange(() => syncModels());
    loadConfig(sid, c, box, (fn) => { syncModels = fn; });
  }, true);
}

async function loadConfig(sid, c, box, setSync) {
  box.innerHTML = '';
  box.appendChild(emptyHint('正在读取配置…'));
  let cfg = null;
  try {
    const r = await api('GET', '/api/sessions/' + encodeURIComponent(sid) + '/config');
    cfg = r.config || null;
  } catch (e) {
    box.innerHTML = '';
    box.appendChild(ui.hint('读不到这个会话的配置：' + e.message, 'err'));
    return;
  }
  box.innerHTML = '';
  if (!cfg) { box.appendChild(ui.hint('后端没有返回 config', 'err')); return; }
  buildConfigForm(sid, cfg, c, box, setSync);
}

function buildConfigForm(sid, cfg, c, box, setSync) {
  const frozen = !!cfg.started;
  // 模型下拉的同步器：openConfig 的登记处订阅只调最新这一份。
  const modelSyncs = [];
  if (setSync) setSync(() => { for (const fn of modelSyncs) fn(); });
  const rt = cfg.runtime || {};
  const dir = cfg.runtimes_dir || '';
  // 表单草稿：保存从这里读；「重新扫描」整块重建，未保存的改动随之丢弃。
  const draft = {
    agents: (cfg.agents || []).map((a) => ({
      name: a.name || '', modules: (a.modules || []).slice(), model: a.model || '', chara: a.chara || '',
    })),
    tier: cfg.tier === 'vm' ? 'vm' : 'host',
    guest: cfg.guest || '',
    pins: {},
    permission: cfg.permission || { profile: 'workspace', shared: { read: 'all', write: 'all', deny: [] }, tools: {} },
  };
  const pins0 = cfg.pins || {};
  for (const k of Object.keys(pins0)) draft.pins[k] = pins0[k];

  // 身份与冻结事实
  const head = ui.div('cfg-sec');
  head.appendChild(ui.div('cfg-sec-title', '会话 ' + (cfg.sid || sid)));
  head.appendChild(ui.div('cfg-line', '形态：' + (cfg.mode === 'collab'
    ? '协作（N 个 agent，各自独立沙箱）'
    : '单 agent（恰好 1 个 agent，模块数不限）')));
  head.appendChild(frozen
    ? ui.hint('这轮会话已经开过：agent 名单与形态冻结 —— 名字只读、不能增删 agent（要换人请新建会话）。模块与模型仍可改。', 'warn')
    : ui.hint('这轮会话还没有内容：agent 名单与形态都还改得动。'));
  box.appendChild(head);

  // agent 名单：名字 / 模块 / 模型（同一模块可被多个 agent 共用）
  const secA = ui.div('cfg-sec');
  secA.appendChild(ui.div('cfg-sec-title', 'agent 名单（' + draft.agents.length + ' 个）'));
  if (!draft.agents.length) secA.appendChild(emptyHint('（这个会话没有 agent 记录）'));
  draft.agents.forEach((a, i) => secA.appendChild(cfgAgentBlock(a, i, frozen, modelSyncs)));
  box.appendChild(secA);

  // 执行档位：本机 / 虚拟机（如实说明各自是什么）
  const secT = ui.div('cfg-sec');
  secT.appendChild(ui.div('cfg-sec-title', '执行档位'));
  // 虚拟机档的**承载**由后端判定（与「开始」/保存同一把尺子）：前置条件不具备时禁用，不让选。
  // 与「未接入」是两回事：guest 本体尚未接入这一点写在下面的提示里，两条都如实说。
  const vmOk = cfg.vm_available !== false;
  const tierLocked = !!cfg.tier_locked;
  const hostR = ui.radio('本机档 —— 脚本直接在宿主上跑：按宿主目标装载 runtimes/ 的包，基础工具链 / 宿主 PATH 回退；隔离就是宿主本身（快，但风险也在宿主上）。', draft.tier === 'host', 'cfg-tier');
  const vmR = ui.radio('虚拟机档 —— 一整套 guest，脚本在 guest 里跑：按模块声明的运行能力装载运行包；隔离更强，代价是更重、依赖运行包。', draft.tier === 'vm', 'cfg-tier', !vmOk);
  if (tierLocked) {
    hostR.box.disabled = true;
    vmR.box.disabled = true;
    secT.appendChild(ui.hint('执行档位随主会话：子会话不能单独改档位与虚拟根。', 'warn'));
  }
  // 虚拟机档选的是**虚拟根**（清单里的一个虚拟机），不是路径：本机有哪些由后端给。
  let guestInfo = (cfg.guests || []).find((g) => g.guest === draft.guest) || null;
  let syncGuestSetup = () => {};
  const pick = guestPicker(cfg.guests, draft.guest, (v, g) => {
    draft.guest = v; guestInfo = g; syncGuestSetup();
  });
  // 机器前置齐、但选中的虚拟根没装齐时，这里给同一个补齐动作（与上面的机器前置入口不重复）。
  const guestSetup = app.setupAction(null, () => openConfig(sid));
  guestSetup.classList.add('hidden');
  syncGuestSetup = () => {
    const show = vmOk && draft.tier === 'vm' && (!guestInfo || !guestInfo.ready);
    if (show) guestSetup.classList.remove('hidden');
    else guestSetup.classList.add('hidden');
  };
  const syncTier = () => {
    const vm = draft.tier === 'vm';
    const show = vm && !tierLocked;
    pick.wrap.className = show ? 'wf-field' : 'wf-field hidden';
    pick.hint.className = 'cfg-hint' + (show ? '' : ' hidden');
    pick.sync();
    syncGuestSetup();
  };
  hostR.box.addEventListener('change', () => { if (hostR.box.checked) { draft.tier = 'host'; syncTier(); } });
  vmR.box.addEventListener('change', () => { if (vmR.box.checked) { draft.tier = 'vm'; syncTier(); } });
  secT.appendChild(hostR.wrap); secT.appendChild(vmR.wrap);
  // 前置逐项照抄后端（缺哪几项、每项怎么补），界面不自己编话、也不笼统说"前置条件不具备"。
  const reqs = cfg.vm_requirements || [];
  if (!vmOk) {
    secT.appendChild(ui.hint('虚拟机档现在不能选：' + (cfg.vm_unavailable_reason || '前置条件不具备'), 'err'));
    if (reqs.length) {
      const box2 = ui.div('cfg-sec');
      box2.appendChild(ui.div('cfg-sec-title', '虚拟机档前置（' + reqs.filter((r) => !r.met).length + ' 项未满足）'));
      for (const r of reqs) {
        const line = doc.createElement('div');
        line.className = 'cfg-hint' + (r.met ? '' : ' err');
        line.textContent =
          (r.met ? '✓ ' : '✗ ') + r.detail + (r.met ? '' : '　怎么补：' + r.how);
        box2.appendChild(line);
      }
      secT.appendChild(box2);
    }
    // 缺什么已经列出来了，这里给**一个动作**（不是一句命令行）。
    // 装完重开配置面板：补齐弹层会把 #modal-root 顶掉，旧面板的 DOM 已经不在了——
    // 与其往脱离文档的节点上重绘，不如按新状态整份重读（未保存的改动会丢，和「重新扫描」同一个代价）。
    secT.appendChild(app.setupAction(null, () => openConfig(sid)));
  }
  secT.appendChild(pick.wrap); secT.appendChild(pick.hint); secT.appendChild(guestSetup);
  box.appendChild(secT);
  syncTier();

  // 运行能力：declared / missing / available / diagnoses / rejected / rejected_packages
  box.appendChild(cfgRuntimeBlock(rt, dir, draft.tier, () => {
    c.setMsg('正在重新扫描模块清单与依赖文件夹…');
    loadConfig(sid, c, box, setSync);
  }));

  // 定版：只对「有多个可用版本」的能力给选择
  const secP = ui.div('cfg-sec');
  secP.appendChild(ui.div('cfg-sec-title', '定版（同一能力有多个可用版本时才需要选）'));
  const avail = rt.available || {};
  const multiCaps = Object.keys(avail).filter((cap) => (avail[cap] || []).length > 1).sort();
  const singleCaps = Object.keys(avail).filter((cap) => (avail[cap] || []).length === 1).sort();
  if (!multiCaps.length) {
    secP.appendChild(ui.hint(singleCaps.length
      ? '（' + singleCaps.join('、') + ' 只有一个可用版本，不需要定版）'
      : '（还没有可用的多版本能力：依赖文件夹里的能力都只有一个版本，或包库里还没有运行包）'));
  }
  for (const cap of multiCaps) {
    const vs = (avail[cap] || []).slice();
    const cur = draft.pins[cap] || '';
    const opts = [{ value: '', label: '（不定版：用包库里第一个 ' + (vs[0] || '') + '）' }]
      .concat(vs.map((v) => ({ value: v, label: v })));
    if (cur && vs.indexOf(cur) < 0) opts.push({ value: cur, label: cur + '（包库里现在没有这个版本）' });
    const sel = selectInput(opts, cur);
    sel.addEventListener('change', () => { draft.pins[cap] = sel.value; });
    secP.appendChild(field(cap + ' 的版本', sel));
  }
  // 单版本 / 看不见的能力：不给选择，但要如实说清现状（不静默丢掉已有的定版）
  for (const cap of singleCaps) {
    if (draft.pins[cap]) {
      secP.appendChild(ui.hint('已定版：' + cap + ' = ' + draft.pins[cap] + '（这个能力只有一个可用版本，不需要选）'));
    }
  }
  for (const cap of Object.keys(draft.pins)) {
    if (!avail[cap]) {
      secP.appendChild(ui.hint('已定版：' + cap + ' = ' + draft.pins[cap] + '（包库里现在看不到这个能力，确认版本或重新扫描）', 'warn'));
    }
  }
  box.appendChild(secP);

  // 会话权限：四档 + 共享区三态 + 逐工具决定（网络是全局开关，不在这里）
  const secPerm = ui.div('cfg-sec');
  secPerm.appendChild(ui.div('cfg-sec-title', '会话权限'));
  secPerm.appendChild(ui.hint('网络由全局 sandbox.policy.network 控制；本会话不可单独改。子会话只能换更低档、只能收窄。'));
  const perm = draft.permission || {};
  const parseLines = (text) => String(text || '')
    .split(/[\n,]/).map((s) => s.trim()).filter((s) => s.length > 0);
  const profiles = (cfg.permission && cfg.permission.profiles) || [
    { id: 'view', label: '仅可查看', summary: '' },
    { id: 'workspace', label: '工作区内修改', summary: '' },
    { id: 'external', label: '外部可达（逐次确认）', summary: '' },
    { id: 'all_reach', label: '全可达（自动放行）', summary: '' },
  ];
  const curProfile = perm.profile || 'workspace';
  const parentProfile = (cfg.permission && cfg.permission.parent_profile) || null;
  const parentIndex = parentProfile ? profiles.findIndex((p) => p.id === parentProfile) : profiles.length - 1;
  const prof = selectInput(profiles.map((p) => ({ value: p.id, label: p.label })), curProfile);
  Array.from(prof.children || []).forEach((o, i) => {
    if (parentProfile && i > parentIndex) o.disabled = true;
  });
  secPerm.appendChild(field('权限档位', prof));
  const profHint = ui.hint('');
  secPerm.appendChild(profHint);
  const syncProf = () => {
    const p = profiles.find((x) => x.id === prof.value);
    profHint.textContent = p ? p.summary : '';
  };
  prof.addEventListener('change', syncProf);
  syncProf();
  const makeList = (label, value, hint) => {
    const t = areaInput('每行一条');
    t.value = Array.isArray(value) ? value.join('\n') : '';
    t.addEventListener('input', () => { if (hint) hint.textContent = ''; });
    secPerm.appendChild(field(label, t));
    return t;
  };
  const scopeEditor = (label, value) => {
    const mode = Array.isArray(value) ? 'paths' : (value === 'none' ? 'none' : 'all');
    const sel = selectInput([
      { value: 'all', label: '整棵共享区' },
      { value: 'none', label: '无权限' },
      { value: 'paths', label: '指定路径' },
    ], mode);
    const area = areaInput('每行一条（相对共享区根）');
    area.value = Array.isArray(value) ? value.join('\n') : '';
    const wrap = ui.div('cfg-sec');
    wrap.appendChild(field(label, sel));
    const areaField = field(label + '：路径', area);
    wrap.appendChild(areaField);
    const sync = () => { areaField.style.display = sel.value === 'paths' ? '' : 'none'; };
    sel.addEventListener('change', sync);
    sync();
    secPerm.appendChild(wrap);
    return () => (sel.value === 'all' ? 'all' : sel.value === 'none' ? 'none' : parseLines(area.value));
  };
  const getRead = scopeEditor('共享区读范围', (perm.shared && perm.shared.read) || 'all');
  const getWrite = scopeEditor('共享区写范围', (perm.shared && perm.shared.write) || 'all');
  const deny = makeList('追加黑名单（读写都禁；全局黑名单是硬底线，这里只能追加）', (perm.shared && perm.shared.deny) || []);
  const toolsArea = areaInput('每行一条：工具名=allow/ask/deny');
  toolsArea.value = Object.entries(perm.tools || {}).map(([k, v]) => k + '=' + v).join('\n');
  secPerm.appendChild(field('逐工具决定（可选；子会话不能改）', toolsArea));
  const parseTools = () => {
    const out = {};
    for (const line of parseLines(toolsArea.value)) {
      const i = line.indexOf('=');
      if (i < 0) continue;
      const k = line.slice(0, i).trim();
      const v = line.slice(i + 1).trim();
      if (k && ['allow', 'ask', 'deny'].includes(v)) out[k] = v;
    }
    return out;
  };
  box.appendChild(secPerm);

  // 思考强度：工作根一个档（子会话共享）；改了就生效，与状态栏 chip 走同一个动作。
  const secThink = ui.div('cfg-sec');
  secThink.appendChild(ui.div('cfg-sec-title', '思考强度（工作根共享）'));
  const think = cfg.thinking || { level: null, default_level: null, label: '未设置', options: [], note: '' };
  const thinkOpts = (think.options || []).map((o) => ({ value: o.id, label: o.label }));
  const current = think.level || think.default_level || '';
  // 只有没有统一默认档时，才需要「跟随默认」这一项；有具体默认档时它本身就是一项。
  if (!think.default_level) thinkOpts.unshift({ value: '', label: '（跟随默认）' });
  const known = (think.options || []).some((o) => o.id === current);
  if (current && !known) {
    thinkOpts.unshift({ value: current, label: (think.label || current) + '（当前档不可用）' });
  }
  const thinkSel = selectInput(thinkOpts, current);
  if (current && !known && thinkSel.children[0]) thinkSel.children[0].disabled = true;
  thinkSel.onchange = async () => {
    const picked = thinkSel.value;
    const level = picked === think.default_level ? '' : picked;
    try {
      await api('POST', actionUrl('set_thinking'), { session_id: sid, level });
      await refreshState();
      c.setMsg(level ? '思考强度：' + thinkingLabel(level) : '思考强度：跟随模型默认');
      loadConfig(sid, c, box, setSync);
    } catch (e) { c.setMsg(e.message, true); }
  };
  secThink.appendChild(field('思考强度', thinkSel));
  secThink.appendChild(ui.hint(think.note || '只作用于对话生成；核心内部编排保持供应商默认。只列这组模型都登记了的档；选中共同默认档 = 跟随模型默认。'));
  box.appendChild(secThink);

  // 动作
  const acts = ui.div('cfg-actions');
  const save = btn('保存', 'btn btn-primary');
  const cancel = btn('取消', 'btn btn-ghost');
  cancel.onclick = ui.closeModal;
  save.onclick = async () => {
    const pins = {}; // 空串 = 不定版：不提交（提交空串会被当成"定版到空版本"）
    for (const cap of Object.keys(draft.pins)) {
      const v = draft.pins[cap];
      if (v) pins[cap] = v;
    }
    const body = {
      agents: draft.agents.map((a) => ({ name: a.name.trim(), modules: a.modules.slice(), model: a.model || '', chara: a.chara || '' })),
      tier: draft.tier,
      guest: draft.tier === 'vm' ? (draft.guest || null) : null,
      pins: pins,
      permission: {
        profile: prof.value,
        shared: {
          read: getRead(),
          write: getWrite(),
          deny: parseLines(deny.value),
        },
        tools: (cfg.permission && cfg.permission.narrowed) ? {} : parseTools(),
      },
    };
    save.disabled = true;
    c.setMsg('保存中…');
    try {
      await api('POST', actionUrl('edit_session'), Object.assign({ session_id: sid }, body));
      await refreshState();
      c.setMsg('已保存：下一次发言按新配置生效');
      loadConfig(sid, c, box, setSync); // 重读一遍：把落盘后的样子如实显示出来
    } catch (e) {
      save.disabled = false;
      c.setMsg('保存失败：' + e.message, true); // 后端 error 原文，不吞
    }
  };
  acts.appendChild(save); acts.appendChild(cancel);
  box.appendChild(acts);
}

/// 一个 agent 的编辑块：名字（冻结时只读）/ 模块 chips（选项来自 /api/state 的 modules）/ 人设 / 模型下拉。
function cfgAgentBlock(a, idx, frozen, modelSyncs) {
  const row = ui.div('cfg-agent');
  row.appendChild(ui.div('cfg-agent-head', 'agent ' + (idx + 1) + '：' + (a.name || '（未命名）')));

  const nameIn = textInput('agent 名字（也会当它的沙箱目录名）');
  nameIn.value = a.name || '';
  nameIn.disabled = frozen;
  nameIn.addEventListener('input', () => { a.name = nameIn.value; });
  row.appendChild(field(frozen ? '名字（只读）' : '名字', nameIn));
  if (frozen) row.appendChild(ui.hint('会话已经开过：名单与形态冻结 —— 名字改不了（要换人请新建会话）；模块与模型仍可改。', 'warn'));

  const chips = app.moduleChips({
    selected: () => a.modules,
    onChange: (ids) => { a.modules = ids; },
  });
  row.appendChild(field('模块（这个 agent 的能力；同一模块可被多个 agent 共用）', chips.node));
  const charaIn = areaInput('人设 / 身份（只影响这个会话里的这份 copy，不写回登记处）');
  charaIn.value = a.chara || '';
  charaIn.addEventListener('input', () => { a.chara = charaIn.value; });
  row.appendChild(field('人设 / 身份', charaIn));
  const cur = a.model || '';
  const sel = selectInput(modelOptions(cur, true), cur);
  sel.addEventListener('change', () => { a.model = sel.value; });
  // 登记处变了就换选项，保留这一席当前选择（值没了就如实显示"登记处已无此模型"）。
  if (modelSyncs) modelSyncs.push(() => syncSelect(sel, modelOptions(a.model || '', true), a.model || ''));
  row.appendChild(field('模型（留空 = 用核心默认）', sel));
  return row;
}

/// 运行能力区：声明 / 缺口 / 可用 / 虚拟机档诊断 / 两类拒收，全部如实呈现。
function cfgRuntimeBlock(rt, dir, savedTier, rescan) {
  const sec = ui.div('cfg-sec');
  const head = ui.div('cfg-sec-head');
  head.appendChild(ui.div('cfg-sec-title', '运行能力（模块声明 / 包库可用 / 缺什么）'));
  const again = btn('重新扫描', 'btn');
  again.title = '重新读取这个会话的配置（模块清单与依赖文件夹每次读取都重扫，放入即出现）；未保存的改动会随之丢弃';
  again.onclick = rescan;
  head.appendChild(again);
  sec.appendChild(head);

  sec.appendChild(ui.div('cfg-line', '依赖文件夹（运行包放这里）：'));
  sec.appendChild(ui.div('cfg-dir', dir || '（后端没有给出依赖文件夹路径）'));
  sec.appendChild(ui.hint(savedTier === 'host'
    ? '已保存的是本机档：运行包按宿主目标装载，包 bin/ 进本次执行的 PATH；缺包或缺本机载荷 = 该模块的工具不执行（会话照样进行，如实说明缺什么）。'
    : '已保存的是虚拟机档：按模块声明的运行能力装载运行包；缺包 = 该模块的工具不执行（会话照样进行，如实说明缺什么）。'));

  const declared = rt.declared || {};
  const dIds = Object.keys(declared);
  sec.appendChild(ui.div('cfg-sub', '模块声明的运行能力'));
  if (!dIds.length) sec.appendChild(ui.div('cfg-line', '（没有模块声明运行能力：都不需要运行包）'));
  for (const id of dIds) sec.appendChild(ui.div('cfg-line', '· 模块 ' + id + ' 需要：' + (declared[id] || []).join('、')));

  const missing = rt.missing || {};
  const mIds = Object.keys(missing);
  sec.appendChild(ui.div('cfg-sub', '缺口（本档位用不了的能力）'));
  if (!mIds.length) {
    sec.appendChild(ui.div('cfg-line cfg-ok', '没有缺口：模块声明的运行能力在依赖文件夹里都能找到。'));
  } else {
    for (const id of mIds) sec.appendChild(ui.div('cfg-line cfg-bad', '· 模块 ' + id + ' 缺：' + (missing[id] || []).join('、')));
    sec.appendChild(ui.hint('把适用于本档位的运行包放进 ' + (dir || '依赖文件夹 runtimes/') + '，再点上面的「重新扫描」（放入即出现；活动会话重新启用后生效）。', 'warn'));
  }

  const avail = rt.available || {};
  const aCaps = Object.keys(avail);
  sec.appendChild(ui.div('cfg-sub', '包库里可用'));
  if (!aCaps.length) sec.appendChild(ui.div('cfg-line', '（依赖文件夹里没有可用的运行包）'));
  for (const cap of aCaps) sec.appendChild(ui.div('cfg-line', '· ' + cap + '：' + (avail[cap] || []).join('、')));

  const diags = rt.diagnoses || [];
  if (diags.length) {
    sec.appendChild(ui.div('cfg-sub', '运行能力诊断（选型不成立的原因）'));
    for (const d of diags) sec.appendChild(ui.div('cfg-line cfg-bad', '· ' + app.diagnosisText(d)));
  }

  const rej = rt.rejected || [];
  const rejPk = rt.rejected_packages || [];
  sec.appendChild(ui.div('cfg-sub', '被拒收的模块 / 运行包（原因原文）'));
  if (!rej.length && !rejPk.length) sec.appendChild(ui.div('cfg-line', '没有被拒收的模块或运行包。'));
  for (const x of rej) sec.appendChild(ui.div('cfg-line cfg-bad', '· 模块：' + x));
  for (const x of rejPk) sec.appendChild(ui.div('cfg-line cfg-bad', '· 运行包：' + x));
  return sec;
}

/* ---------- 通用小构件：组件库在 app/ui/（btn / field / 各类输入见文件顶部的绑定） ---------- */
/* 虚拟根选择器：选的是**虚拟机**（清单里的一个虚拟根），不是路径——界面从不让人填目录。
 * 每一项如实标注：没装齐（缺哪几条）、各自是什么系统——界面照抄后端事实，不自己编话。 */
function guestPicker(guests, current, onChange) {
  const list = guests || [];
  const opts = list.map((g) => ({
    value: g.guest,
    label: g.label + (g.guest !== g.label ? "（" + g.guest + "）" : "") +
      (g.ready ? "" : " —— 未装齐，先补齐依赖"),
  }));
  if (current && !opts.some((o) => o.value === current)) {
    opts.push({ value: current, label: current + "（本机现在没有这个虚拟根）" });
  }
  if (!opts.length) opts.push({ value: "", label: "（本机没有可选虚拟根：先补齐依赖）" });
  const sel = selectInput(opts, current || opts[0].value);
  const wrap = field("虚拟机（虚拟根）", sel);
  const hint = ui.hint("");
  const sync = () => {
    const g = list.find((x) => x.guest === sel.value);
    hint.textContent = !g
      ? "这个虚拟根在本机找不到：点「补齐虚拟机依赖」装进项目内，或换一个。"
      : (g.os + " / " + g.arch) +
        (g.ready ? "：已装齐。" : "：还没装齐（缺 " + (g.missing || []).join("、") + "）——点「补齐虚拟机依赖」装上再用。") +
        ((g.runtimes || []).length
          ? " 自带运行能力：" + g.runtimes.join("、") + "。"
          : " 它没有声明自带的运行能力。");
    if (onChange) onChange(sel.value, g);
  };
  sel.addEventListener("change", sync);
  sync();
  return { wrap: wrap, sel: sel, hint: hint, sync: sync };
}

/// 挂载：装配依赖并把本模块的入口交给组装点。
export function mount(root, deps) {
  ({ g, doc, api, ui, app } = deps);
  ({ areaInput, btn, emptyHint, field, hint, selectInput, syncSelect, textInput } = ui);
  Object.assign(root, { openConfig, loadConfig, buildConfigForm, cfgAgentBlock, cfgRuntimeBlock, guestPicker });
}
