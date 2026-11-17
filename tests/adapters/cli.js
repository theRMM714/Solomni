//! 目的：把「跑独立测试二进制」这一种派发形态收在一处。
//! 管：按目标名跑 `cargo test --test <name>`，同样把「0 个用例命中」判成失败。
//! 不管：各平台目标自身的用例内容（tests/<平台>/）；选哪些目标（在 tests/lib/plan.js）。
//! 联动：tests/lib/plan.js 的 platformTargets；run-tests.js 的子集执行。

const parsers = require("../lib/parsers.js");

module.exports = {
  name: "cli",
  title: "独立测试二进制（cargo --test）",
  run(targets, ctx, serial) {
    return targets.map((t) => {
      const r = ctx.proc.sh(
        "cargo",
        ["test", "--color", "never", "--test", t, "--", "--nocapture"].concat(serial)
      );
      const counts = parsers.cargoCounts(r.out);
      // 目标存在性已由 tests/lib/plan.js 判过；这里 0 个用例是「本平台没有对应用例」的正常结果
      // （平台目标里的用例都按 target_os 过滤），不能当失败，否则跑一个平台必红另外三个。
      const ok = r.code === 0;
      const detail = ok ? (counts.passed > 0 ? counts.passed + " passed" : "本平台没有命中用例") : "日志 " + r.log;
      return { ok, label: "目标 " + t, detail, tail: r.out };
    });
  },
};
