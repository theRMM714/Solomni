/* 目的：把本地文件上传进当前会话的 work/：选择、命名、重名替换与失败提示。
 * 管：上传交互与 /api/sessions/{sid}/files 的调用；filesCache 记每个会话的文件清单。
 * 不管：输入区的 @ 菜单渲染、上传后的全局刷新（经 app.refreshState）。
 * 联动：侧栏的上传按钮、模块与服务页、向导都调用 pickUploadFile。
 */
let g, doc, api, ui, app;
let btn, textInput, field;
/** 挂载：把本功能的入口登记到 app；跨功能调用一律经 app 晚绑定。 */
export function mount(root, deps) {
  ({ g, doc, api, ui, app } = deps);
  ({ btn, textInput, field } = ui);
  Object.assign(root, { filesCache, pickUploadFile, suggestAltName, sendUpload, conflictUpload, renameUpload });
}
/* ---------- 输入框 @ 引用（工作区 / 沙箱里的文件） ---------- */
/* 数据 = GET /api/sessions/{sid}/files，按会话缓存；上传成功即失效；切换会话即清菜单。
 * 菜单只列已存在的文件（上传仍走输入区那个 ＋ 按钮）；前端只负责**插入与渲染**，
 * 不做任何路径改写——@work:… / @sandbox:… 由核心在入转录前改写成 work:/… / sandbox:/…。 */
const filesCache = new Map();

/* ---------- 上传文件到本次工作的 work/ ---------- */
/* 文件选择器按需创建：模块求值时 document 还没注入（mount 时才拿到）。 */
let uploadPicker = null;
function ensurePicker() {
  if (uploadPicker) return uploadPicker;
  const picker = doc.createElement('input');
  picker.type = 'file';
  picker.addEventListener('change', () => {
    const f = picker.files && picker.files[0];
    if (!f) return;
    const s = app.activeSession();
    if (!s) { app.notice('还不能上传', '先打开或新建一个工作，再上传文件到它的 work/。'); picker.value = ''; return; }
    const reader = new FileReader();
    reader.onload = async () => {
      const b64 = String(reader.result || '').split(',')[1] || '';
      try {
        const r = await api('POST', app.actionUrl('upload'), { session_id: s.sid, name: f.name, data_base64: b64 });
        if (r.uploaded === false) {
          conflictUpload(s.sid, f.name, b64);
        } else {
          filesCache.delete(s.sid); // 文件清单变了，@ 菜单下次重拉
          ui.toast('ok', '已上传到 work/：' + f.name);
        }
      } catch (err) {
        app.notice('操作失败', err.message, 'err');
      }
      picker.value = '';
    };
    reader.readAsDataURL(f);
  });
  uploadPicker = picker;
  return picker;
}

function pickUploadFile() { ensurePicker().click(); }

/* a.txt -> a-2.txt（用于「改名」建议） */
function suggestAltName(name) {
  const dot = name.lastIndexOf('.');
  if (dot <= 0) return name + '-2';
  return name.slice(0, dot) + '-2' + name.slice(dot);
}

async function sendUpload(sid, name, b64, overwrite) {
  try {
    const body = { session_id: sid, name, data_base64: b64 };
    if (overwrite) body.overwrite = true;
    const r = await api('POST', app.actionUrl('upload'), body);
    if (r.uploaded === false) {
      app.notice('同名文件已存在', '目标里已经有这个名字，请改名或确认覆盖。', 'err');
      return;
    }
    filesCache.delete(sid); // 文件清单变了，@ 菜单下次重拉
    app.loadWork(sid, true); // 上传 = 一次提交，状态栏的 head 要跟着变
    ui.toast('ok', '已上传：' + name);
  } catch (e) { app.notice('操作失败', e.message, 'err'); }
}

function conflictUpload(sid, name, b64) {
  const alt = suggestAltName(name);
  app.choiceModal('文件同名', '本次工作的 work/ 里已有「' + name + '」，怎么办？', [
    ['覆盖', 'btn btn-danger', () => sendUpload(sid, name, b64, true)],
    ['改名…', 'btn', () => renameUpload(sid, name, b64, alt)],
    ['取消', 'btn btn-ghost', () => {}],
  ]);
}

function renameUpload(sid, name, b64, alt) {
  app.openModal('改文件名上传', (c) => {
    const inp = textInput('新文件名'); inp.value = alt;
    const go = btn('上传', 'btn btn-primary btn-block');
    go.onclick = async () => {
      const v = inp.value.trim();
      if (!v) { c.setMsg('文件名不能为空', true); return; }
      try {
        const r = await api('POST', app.actionUrl('upload'), { session_id: sid, name: v, data_base64: b64 });
        if (r.uploaded === false) { c.setMsg('还是同名，请换一个名字', true); return; }
        filesCache.delete(sid); // 文件清单变了，@ 菜单下次重拉
        app.loadWork(sid, true);
        app.closeModal(); ui.toast('ok', '已上传：' + v);
      } catch (e) {
        c.setMsg(e.message, true);
      }
    };
    c.body.appendChild(field('文件名', inp));
    c.body.appendChild(go);
  });
}