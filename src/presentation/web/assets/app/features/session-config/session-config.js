/* 目的：会话配置弹层——模型、思考档、执行档、模块、人设。
 * 管：openConfig / loadConfig / buildConfigForm / cfgAgentBlock / cfgRuntimeBlock。
 * 不管：不写会话状态、不发事件；保存走统一动作分发。
 * 联动：模型选项来自 app.modelOptions；弹层来自 app/openModal；刷新走 app.refreshState。 */

import { thinkingLabel } from '../../stores/settings.js';
import { sessionConfigCopy } from './copy.js';

let g, doc, api, ui, app;
let areaInput, btn, checkbox, emptyHint, field, hint, numberInput, selectInput, syncSelect, textInput;
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
    sandbox: {
      policy: (cfg.sandbox && cfg.sandbox.policy) || '',
      params: Object.assign({}, (cfg.sandbox && cfg.sandbox.params) || {}),
      pins: Object.assign({}, (cfg.sandbox && cfg.sandbox.pins) || {}),
    },
    pins: {},
    permission: cfg.permission || { profile: 'offline', shared: { read: 'all', write: 'all', deny: [] } },
  };
  const pins0 = draft.sandbox.pins;
  for (const k of Object.keys(pins0)) draft.pins[k] = pins0[k];

  // 身份与冻结事实
  const head = ui.div('cfg-sec');
  head.appendChild(ui.div('cfg-sec-title', '会话 ' + (cfg.sid || sid)));
  head.appendChild(ui.div('cfg-line', '会话类型：' + app.typeLabel(cfg.kind) +
    (cfg.flows ? '（按流程推进）' : '（没有流程，直接轮循环）')));
  head.appendChild(frozen
    ? ui.detail(sessionConfigCopy.frozen)
    : ui.hint('这轮会话还没有内容：agent 名单与形态都还改得动。'));
  box.appendChild(head);

  // agent 名单：名字 / 模块 / 模型（同一模块可被多个 agent 共用）
  const secA = ui.div('cfg-sec');
  secA.appendChild(ui.div('cfg-sec-title', 'agent 名单（' + draft.agents.length + ' 个）'));
  if (!draft.agents.length) secA.appendChild(emptyHint('（这个会话没有 agent 记录）'));
  draft.agents.forEach((a, i) => secA.appendChild(cfgAgentBlock(a, i, frozen, modelSyncs)));
  box.appendChild(secA);

  // 执行环境：可选项、可用性、逐项前置与补法全部来自后端（与 /api/tiers 同源的 choices 表）；
  // 界面按表渲染，不写死任何策略名——新增一个策略只要在组合根登记，这里不用动。
  const secT = ui.div('cfg-sec');
  secT.appendChild(ui.div('cfg-sec-title', '执行环境'));
  const sandboxLocked = !!cfg.sandbox_locked;
  const choices = (cfg.choices && cfg.choices.policies) || [];
  const guests = (cfg.choices && cfg.choices.guests) || [];
  const policyNow = () => choices.find((p) => p.decl && p.decl.id === draft.sandbox.policy) || null;
  const guestInfoNow = () => guests.find((g) => g.guest === draft.sandbox.params.guest) || null;
  const needBox = ui.div('form-col');
  const paramBox = ui.div('form-col');
  if (sandboxLocked) {
    secT.appendChild(ui.hint('执行环境随主会话：子会话不能单独改策略与参数。', 'warn'));
  }
  for (const p of choices) {
    const rr = ui.radio(p.decl.name || p.decl.id, draft.sandbox.policy === p.decl.id, 'cfg-sandbox', sandboxLocked || !p.available);
    rr.box.addEventListener('change', () => {
      if (!rr.box.checked) return;
      draft.sandbox.policy = p.decl.id; draft.sandbox.params = {};
      renderSandbox();
    });
    secT.appendChild(rr.wrap);
    // 每个策略的说明由它自己的声明给（界面不编、也不写死名字）。
    if (p.decl.hint) secT.appendChild(ui.hint(p.decl.hint));
  }
  secT.appendChild(ui.detail(sessionConfigCopy.sandbox));
  secT.appendChild(needBox);
  secT.appendChild(paramBox);
  // 逐项前置照抄后端（缺哪几项、每项怎么补），界面不自己编话、也不笼统说"前置条件不具备"。
  function renderSandbox() {
    needBox.innerHTML = '';
    paramBox.innerHTML = '';
    // 前置逐项照抄后端：**每个当前不可用的策略**都列（不只选中的——选不中才更需要知道怎么修）。
    for (const p of choices) {
      const needs = p.needs || [];
      const unmet = needs.filter((n) => !n.met);
      if (!unmet.length) continue;
      needBox.appendChild(ui.hint('「' + ((p.decl && p.decl.name) || p.decl.id) + '」现在不能选：' + (p.refusal || '前置条件不具备'), 'err'));
      const box2 = ui.div('cfg-sec');
      box2.appendChild(ui.div('cfg-sec-title', '前置（' + unmet.length + ' 项未满足）'));
      for (const n of needs) {
        const line = doc.createElement('div');
        line.className = 'cfg-hint' + (n.met ? '' : ' err');
        const mark = ui.icon(n.met ? 'check' : 'close');
        mark.className += ' cfg-mark' + (n.met ? ' ok' : ' err');
        const text = doc.createElement('span');
        text.textContent = n.label + '：' + n.detail + (n.met || !n.fix || !n.fix.summary ? '' : '　怎么补：' + n.fix.summary);
        line.appendChild(mark);
        line.appendChild(text);
        box2.appendChild(line);
      }
      needBox.appendChild(box2);
      // 缺什么已经列出来了，这里给**一个动作**（不是一句命令行）；装完按新状态整份重读。
      if (p.fixable) {
        needBox.appendChild(app.setupAction(null, { policy: p.decl.id, name: p.decl.name || p.decl.id }, () => openConfig(sid)));
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
        let cur = draft.sandbox.params[name] == null ? '' : String(draft.sandbox.params[name]);
        if (!cur) { cur = opts[0].value; draft.sandbox.params[name] = cur; }
        input = selectInput(opts, cur);
      } else {
        input = textInput(label + (spec.required ? '（必填）' : ''));
        if (draft.sandbox.params[name] != null) input.value = String(draft.sandbox.params[name]);
      }
      input.disabled = sandboxLocked;
      // 下拉（取值域由后端解析，如虚拟根）换值 = 换了一台机器的事实：整段重画（含"还缺什么/能不能补"）。
      // 手填的输入框只在打字时刷提示，避免每敲一个字就重建节点。
      const onChange = () => {
        draft.sandbox.params[name] = input.value;
        if (isSelect) renderSandbox(); else renderGuestHint();
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
      paramBox.appendChild(app.setupAction(null, { policy: p.decl.id, name: p.decl.name || p.decl.id }, () => openConfig(sid)));
    }
  }
  /// 选中的虚拟根在本机的事实（装了哪些运行包、还缺什么）：按值查 guests 表，不按策略 id 分支。
  const guestHint = ui.hint('');
  function renderGuestHint() {
    const g = guestInfoNow();
    if (!g) { guestHint.textContent = ''; guestHint.className = 'cfg-hint hidden'; return; }
    guestHint.className = 'cfg-hint';
    guestHint.textContent = (g.os + ' / ' + g.arch) +
      (g.ready ? '：已装齐。' : '：还没装齐（缺 ' + (g.missing || []).join('、') + '）——点「补齐依赖…」装上再用。') +
      ((g.runtimes || []).length ? ' 自带运行能力：' + g.runtimes.join('、') + '。' : ' 它没有声明自带的运行能力。');
  }
  renderSandbox();
  box.appendChild(secT);

  // 运行能力：declared / missing / available / diagnoses / rejected / rejected_packages
  box.appendChild(cfgRuntimeBlock(rt, dir, () => {
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

  // 会话权限：网络档位（两档）+ 共享区归属（读写各三态 + 追加黑名单）。
  // 沙箱恒可读写、模块目录恒只读、工具面只由角色面决定——没有「逐工具决定」这种东西。
  const secPerm = ui.div('cfg-sec');
  secPerm.appendChild(ui.div('cfg-sec-title', '会话权限'));
  secPerm.appendChild(ui.detail(sessionConfigCopy.network));
  const perm = draft.permission || {};
  const parseLines = (text) => String(text || '')
    .split(/[\n,]/).map((s) => s.trim()).filter((s) => s.length > 0);
  const profiles = (cfg.permission && cfg.permission.profiles) || [
    { id: 'offline', label: '离线', summary: '' },
    { id: 'online', label: '联网', summary: '' },
  ];
  const curProfile = perm.profile || 'offline';
  const parentProfile = (cfg.permission && cfg.permission.parent_profile) || null;
  const parentIndex = parentProfile ? profiles.findIndex((p) => p.id === parentProfile) : profiles.length - 1;
  const prof = selectInput(profiles.map((p) => ({ value: p.id, label: p.label })), curProfile);
  Array.from(prof.children || []).forEach((o, i) => {
    if (parentProfile && i > parentIndex) o.disabled = true;
  });
  secPerm.appendChild(field('网络档位', prof));
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
  secPerm.appendChild(ui.detail('沙箱本身恒可读写；模块目录恒只读；共享区是主副本，改动经 work_commit 提交。'));
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
  secThink.appendChild(think.note ? ui.hint(think.note) : ui.detail(sessionConfigCopy.thinkNote));
  box.appendChild(secThink);

  // 上下文：过期内容替换开关与保底（工作根共享）；改了就生效，与创建向导走同一个动作。
  const secCtx = ui.div('cfg-sec');
  secCtx.appendChild(ui.div('cfg-sec-title', '上下文（工作根共享）'));
  const on = checkbox('过期内容换成占位符', !!cfg.context_freshness, sessionConfigCopy.freshness.lead);
  const keepPct = Number(cfg.freshness_keep_percent == null ? 80 : cfg.freshness_keep_percent);
  const keepIn = numberInput('保底缓存（%）', keepPct, 0, 90);
  const apply = async () => {
    const keep = Math.max(0, Math.min(90, Math.round(Number(keepIn.input.value) || 0)));
    const want = !!on.box.checked;
    try {
      await api('POST', actionUrl('set_context_freshness'), { session_id: sid, on: want, keep_percent: keep });
      await refreshState();
      c.setMsg(want ? '过期内容：已改为占位符（保底 ' + keep + '%）' : '过期内容：保持原样');
      loadConfig(sid, c, box, setSync);
    } catch (e) { c.setMsg(e.message, true); }
  };
  on.box.onchange = apply;
  keepIn.input.onchange = apply;
  secCtx.appendChild(on.wrap);
  secCtx.appendChild(keepIn.wrap);
  secCtx.appendChild(ui.detail(sessionConfigCopy.freshness.detail));
  secCtx.appendChild(ui.detail(sessionConfigCopy.freshnessKeep.detail));
  box.appendChild(secCtx);

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
      // 执行环境整份提交：策略 + 参数 + 定版（定版是会话级选择，跟着执行环境走）。
      sandbox: { policy: draft.sandbox.policy, params: draft.sandbox.params, pins: pins },
      permission: { profile: prof.value },
      shared: {
        read: getRead(),
        write: getWrite(),
        deny: parseLines(deny.value),
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
  if (frozen) row.appendChild(ui.detail(sessionConfigCopy.agentFrozen));

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
function cfgRuntimeBlock(rt, dir, rescan) {
  const sec = ui.div('cfg-sec');
  const head = ui.div('cfg-sec-head');
  head.appendChild(ui.div('cfg-sec-title', '运行能力（模块声明 / 包库可用 / 缺什么）'));
  const again = btn('重新扫描', 'btn');
  again.title = sessionConfigCopy.rescanTitle;
  again.onclick = rescan;
  head.appendChild(again);
  sec.appendChild(head);

  sec.appendChild(ui.div('cfg-line', '依赖文件夹（运行包放这里）：'));
  sec.appendChild(ui.div('cfg-dir', dir || '（后端没有给出依赖文件夹路径）'));
  sec.appendChild(ui.detail(sessionConfigCopy.savedSandbox));

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
/// 挂载：装配依赖并把本模块的入口交给组装点。
export function mount(root, deps) {
  ({ g, doc, api, ui, app } = deps);
  ({ areaInput, btn, checkbox, emptyHint, field, hint, numberInput, selectInput, syncSelect, textInput } = ui);
  Object.assign(root, { openConfig, loadConfig, buildConfigForm, cfgAgentBlock, cfgRuntimeBlock });
}
