/* 目的：共享区提交历史的查看与回滚：列表、提交详情、文件预览、非破坏性回滚。
 * 管：`openWorkHistory` 的弹层与分页、`previewCommitFile`、`restoreCommit`（含冲突后的覆盖确认）。
 * 不管：工作卡本身与上下文/工具浮层（见 work.js）；取数与状态落库由 work.js 的 `loadWork` 负责。
 * 联动：由 app/main.js 装配；`work.js` 经 app.openWorkHistory 晚绑定打开，弹层与提示走 ui/ 与 dialog。 */

let g, doc, state, stores, api, ui, app;
let btn;
const actionUrl = (...a) => app.actionUrl(...a);
const confirmBox = (...a) => app.confirmBox(...a);
const loadWork = (...a) => app.loadWork(...a);
const notice = (...a) => app.notice(...a);
const openModal = (...a) => app.openModal(...a);
const popoverNote = (...a) => app.popoverNote(...a);
const popoverRow = (...a) => app.popoverRow(...a);

function openWorkHistory(s, selected) {
  loadWork(s.sid, true);
  openModal('提交历史 · ' + s.sid, (c) => {
    const wrap = doc.createElement('div');
    wrap.className = 'work-history';
    const list = doc.createElement('div');
    list.className = 'work-history-list';
    const detail = doc.createElement('div');
    detail.className = 'work-detail';
    wrap.appendChild(list); wrap.appendChild(detail);
    c.body.appendChild(wrap);
    c.setMsg('回滚是非破坏性的：目标之后的提交仍保留，可再次回滚；只改共享区，不动各 agent 沙箱。');
    let page = null;
    let selectedId = selected || null;
    let loading = false;

    const paintDetail = async (id) => {
      if (!id) {
        detail.innerHTML = '<div class="work-empty">选一条提交看详情。</div>';
        return;
      }
      detail.innerHTML = '<div class="work-empty">读取中…</div>';
      let d;
      try {
        d = await api('GET', '/api/sessions/' + encodeURIComponent(s.sid) + '/work/' + encodeURIComponent(id));
      } catch (e) {
        detail.innerHTML = '';
        detail.appendChild(popoverNote('读取失败：' + e.message, 'err'));
        return;
      }
      detail.innerHTML = '';
      const head = doc.createElement('div');
      head.className = 'work-detail-head';
      const title = doc.createElement('div');
      title.className = 'work-detail-title';
      title.textContent = '#' + d.id + (d.parent ? ' ← #' + d.parent : '');
      const badge = doc.createElement('span');
      badge.className = 'badge-soft' + (s.work && s.work.head && s.work.head.id === d.id ? ' ok' : '');
      badge.textContent = s.work && s.work.head && s.work.head.id === d.id ? '当前 head' : '历史提交';
      head.appendChild(title); head.appendChild(badge);
      detail.appendChild(head);
      detail.appendChild(popoverRow('作者 / 时间', d.author + ' · ' + app.fmtDateTime(d.time)));
      const msg = doc.createElement('div');
      msg.className = 'commit-msg';
      msg.textContent = d.message;
      detail.appendChild(msg);
      const chHead = doc.createElement('div');
      chHead.className = 'work-meta';
      chHead.textContent = '改动 ' + d.changes.length + ' 处' + (d.changes_truncated ? '（已截断）' : '');
      detail.appendChild(chHead);
      const changes = doc.createElement('div');
      changes.className = 'change-list';
      for (const ch of d.changes) {
        const row = doc.createElement('button');
        row.type = 'button';
        row.className = 'change-row';
        const kind = doc.createElement('span');
        kind.className = 'change-kind ' + ch.kind;
        kind.textContent = ch.kind === 'add' ? '新增' : ch.kind === 'modify' ? '修改' : '删除';
        const path = doc.createElement('span');
        path.className = 'change-path';
        path.textContent = ch.path;
        const size = doc.createElement('span');
        size.className = 'change-size';
        size.textContent = ch.size + ' B';
        row.appendChild(kind); row.appendChild(path); row.appendChild(size);
        if (ch.kind !== 'delete') {
          row.title = '预览该提交里的这个文件';
          row.onclick = () => previewCommitFile(detail, s, d.id, ch.path);
        } else {
          row.disabled = true;
          row.title = '该提交删除了这个文件';
        }
        changes.appendChild(row);
      }
      detail.appendChild(changes);
      const foot = doc.createElement('div');
      foot.className = 'popover-foot';
      const restore = btn('回滚到此处', 'btn btn-primary');
      const blocked = s.readonly ? '历史回放只读：先启用会话' : (app.isBusy(s) ? '会话正在生成：先停止再回滚' : '');
      if (blocked) { restore.disabled = true; restore.title = blocked; }
      restore.onclick = () => restoreCommit(c, s, d);
      foot.appendChild(restore);
      detail.appendChild(foot);
    };

    const paintList = (append) => {
      if (!page) return;
      if (!append) list.innerHTML = '';
      for (const cm of page.commits) {
        if (list.querySelector('[data-commit="' + cm.id + '"]')) continue;
        const row = doc.createElement('button');
        row.type = 'button';
        row.dataset.commit = String(cm.id);
        row.className = 'work-row' + (cm.on_head ? ' current' : '');
        const rh = doc.createElement('div'); rh.className = 'work-row-head';
        const id = doc.createElement('span'); id.className = 'work-id'; id.textContent = '#' + cm.id;
        const meta = doc.createElement('span'); meta.className = 'work-meta';
        meta.textContent = (cm.on_head ? '当前线 · ' : '已离开 · ') + cm.author + ' · ' + app.timeAgo(cm.time) + ' · ' + cm.changes + ' 处';
        rh.appendChild(id); rh.appendChild(meta);
        const m = doc.createElement('div'); m.className = 'work-msg'; m.textContent = cm.message;
        row.appendChild(rh); row.appendChild(m);
        row.onclick = () => { selectedId = cm.id; paintDetail(cm.id); };
        list.appendChild(row);
      }
    };
    const loadPage = async (before) => {
      if (loading) return;
      loading = true;
      try {
        const q = '/api/sessions/' + encodeURIComponent(s.sid) + '/work?limit=50' + (before ? '&before=' + before : '');
        const r = await api('GET', q);
        page = r;
        s.work = r;
        s.workAt = Date.now();
        app.syncStatusbar(s);
        paintList(!!before);
        if (r.truncated) {
          const old = list.querySelector('.work-more');
          if (old) old.remove();
          const more = btn('加载更早', 'btn btn-ghost work-more');
          more.onclick = () => loadPage(page.commits[page.commits.length - 1].id);
          list.appendChild(more);
        }
        if (!selectedId) selectedId = r.head ? r.head.id : (r.commits[0] && r.commits[0].id);
        paintDetail(selectedId);
      } catch (e) {
        c.setMsg('读取提交历史失败：' + e.message, true);
      } finally {
        loading = false;
      }
    };
    loadPage(null);
  }, true);
}

async function previewCommitFile(holder, s, commit, path) {
  const old = holder.querySelector('.file-preview');
  if (old) old.remove();
  const box = doc.createElement('div');
  box.className = 'file-preview';
  box.appendChild(popoverNote('读取 ' + path + ' …'));
  holder.appendChild(box);
  try {
    const r = await api('GET', '/api/sessions/' + encodeURIComponent(s.sid) + '/work/' +
      encodeURIComponent(commit) + '?path=' + encodeURIComponent(path));
    box.innerHTML = '';
    box.appendChild(popoverRow('文件', r.path));
    box.appendChild(popoverRow('大小 / 类型', r.size + ' B · ' + (r.media_type || '未知')));
    if (r.too_large) box.appendChild(popoverNote('超过共享区单文件上限：只给元信息，不预览。'));
    else if (r.text == null) box.appendChild(popoverNote('二进制 / 非 UTF-8：只给元信息。'));
    else {
      const pre = doc.createElement('pre');
      pre.textContent = r.text + (r.truncated ? '\n…（预览已截断，共 ' + r.size + ' B）' : '');
      box.appendChild(pre);
    }
  } catch (e) {
    box.innerHTML = '';
    box.appendChild(popoverNote('读取失败：' + e.message, 'err'));
  }
}

async function restoreCommit(modal, s, d) {
  if (s.readonly) { notice('历史回放只读', '先启用会话，再回滚共享区。', 'info'); return; }
  if (app.isBusy(s)) { notice('会话正在生成', '先停止再回滚，避免与正在提交的 agent 交错。', 'info'); return; }
  let head = s.work && s.work.head ? s.work.head.id : 0;
  const text = '共享区将从 head #' + head + ' 回滚到提交 #' + d.id + '（' + d.author + '：' + d.message + '）。\n\n' +
    '这是非破坏性回滚：目标之后的提交仍保留在历史里，可再次回滚；只改共享区主副本，不动各 agent 沙箱，后续拉取可能遇到冲突。';
  if (!(await confirmBox('回滚共享区', text, '回滚'))) return;
  try {
    await api('POST', actionUrl('restore_work_commit'), {
      session_id: s.sid, commit: d.id, expect_head: head,
    });
    s.workAt = 0; s.workDirty = false;
    await loadWork(s.sid, true);
    ui.toast('ok', '已回滚到提交 #' + d.id);
    if (modal && modal.close) modal.close();
  } catch (e) {
    if (/被外部改动/.test(e.message) &&
        await confirmBox('覆盖工作区外部改动', e.message + '\n\n确认后会覆盖这些路径当前的内容；建议先 work_sync，保留外部修改。', '仍要覆盖')) {
      try {
        await api('POST', actionUrl('restore_work_commit'), {
          session_id: s.sid, commit: d.id, expect_head: head, force: true,
        });
        s.workAt = 0; s.workDirty = false;
        await loadWork(s.sid, true);
        ui.toast('ok', '已确认覆盖并回滚到提交 #' + d.id);
        if (modal && modal.close) modal.close();
      } catch (e2) {
        if (modal && modal.setMsg) modal.setMsg('回滚失败：' + e2.message, true);
        else notice('回滚失败', e2.message, 'err');
      }
      return;
    }
    if (modal && modal.setMsg) modal.setMsg('回滚失败：' + e.message, true);
    else notice('回滚失败', e.message, 'err');
    s.workAt = 0;
    loadWork(s.sid, true);
  }
}


export function mount(root, deps) {
  ({ g, doc, state, stores, api, ui, app } = deps);
  btn = ui.btn;
  Object.assign(root, { openWorkHistory, previewCommitFile, restoreCommit });
}
