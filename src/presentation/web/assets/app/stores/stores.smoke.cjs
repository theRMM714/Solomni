/* 冒烟：app/stores/state.js 的快照投影、运行态对账与登记处变化通知。
 * 用法：node src/presentation/web/assets/app/stores/stores.smoke.cjs   （输出 STORES-SMOKE-OK / FAIL）
 * 盯的契约：会话表只由后端给（只为在跑的会话建条目）；有实时知识时快照不覆盖运行态；
 *   登记处指纹变了才通知，一个订阅者出错不拖垮其它订阅者。 */
const s = require("./state.js");

const fails = [];
const check = (ok, name) => { if (!ok) fails.push(name); };

const st = s.createState();
check(st.sessions instanceof Map && st.activeSid === null && st.settings.compact_keep_percent === 15, "初始状态形状");

const snap = {
  modules: [{ id: "m" }],
  sessions: [
    { sid: "w1", running: true, mode: "collab", continue: { id: "c" } },
    { sid: "w2", running: false },
  ],
  history: [{ name: "w1", parent: null, done: false }, { name: "w3--甲", running: true }],
};
const opened = s.projectSnapshot(st, snap);
check(opened.length === 1 && opened[0].sid === "w1" && opened[0].mode === "collab", "只为在跑的会话建条目");
check(st.sessions.has("w1") && !st.sessions.has("w2"), "不在跑的会话不建条目");
check(opened[0].continue_action && opened[0].continue_action.id === "c", "上下文继续取自后端视图");
check(opened[0].running === true && opened[0].running_known === false, "新条目按快照对账运行态");
check(s.snapshotRunning(st, "w3") === true, "子会话在跑也算父会话在跑");
check(s.snapshotRunning(st, "w2") === false && s.snapshotRunning(st, "") === false, "未在跑 / 空 sid");
check(st.settings.streaming === true && st.settings.compact_keep_percent === 15 && st.settings.compact_model === "", "缺 settings 时退回默认值（含压缩保留与总结模型）");
check(s.projectSnapshot(st, snap).length === 0, "已建的条目不重复建");

const w1 = st.sessions.get("w1");
w1.running_known = true; w1.running = false; w1.working = "x";
s.projectSnapshot(st, snap);
check(w1.running === false && w1.working === "x", "有实时知识时快照不覆盖运行态");
w1.running_known = false;
s.projectSnapshot(st, { sessions: [{ sid: "w1", running: false }] });
check(w1.running === false && w1.working === null, "没有实时知识时快照说没跑就清掉 working");
w1.readonly = true; w1.continue_action = null;
s.projectSnapshot(st, { sessions: [{ sid: "w1", continue: { id: "c" } }] });
check(w1.continue_action === null, "只读视图不显示上下文继续");

const errs = [];
const reg = s.createRegistryWatch(st, (e) => errs.push(e.message));
let hits = 0;
reg.on(() => { throw new Error("坏订阅者"); });
const off = reg.on(() => { hits += 1; });
check(reg.check() === true, "第一次检查算变化");
check(reg.check() === false, "指纹没变不算变化");
st.models = [{ id: "m1" }];
check(reg.check() === true, "登记处变了算变化");
reg.notify();
check(hits === 1 && errs.length === 1 && errs[0] === "坏订阅者", "订阅者出错不拖垮其它订阅者");
off();
reg.notify();
check(hits === 1, "退订后不再通知");

if (fails.length) {
  for (const f of fails) console.log("FAIL " + f);
  console.log("STORES-SMOKE-FAIL");
  process.exit(1);
}
console.log("STORES-SMOKE-OK");
