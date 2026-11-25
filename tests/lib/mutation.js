/**
 * 突变范围的唯一声明处：各单元 `tests/specs/<unit>.yaml` 里的 `mutation` 块。
 * 目的：谁需要突变谁自己登记，门禁与 CI 都从这里读；不再维护第二份手写清单（旧 `tests/mutation-scope.json` 已删）。
 * 声明只是菜单，不是闸门：每周只跑 `schedule: weekly` 的单元，手动抽查用 `--unit` 指定。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { loadSpecs } = require("./spec.js");

const SCHEDULES = ["weekly", "none"];
/** 路径里出现这些字符说明写成了 glob：突变范围要具体文件，路径一搬走门禁才会判红。 */
const GLOB = /[*?[\]]/;

/** 单元管到哪棵子树：kernel 单元对 `src/kernel/`，其它单元对自己的能力目录。 */
function unitPrefix(unit) {
  return unit === "kernel" ? "src/kernel/" : "src/units/" + unit + "/";
}

/**
 * 校验全部 mutation 声明，返回问题列表（空数组=通过）。
 * 只判声明自身是否自洽：schedule 合法、文件存在、落在本单元、不重复、不写 glob、有 case 兜底。
 */
function validate(root) {
  const { specs, problems } = loadSpecs(root);
  const out = problems.slice();
  const owner = new Map();
  for (const spec of specs) {
    const m = spec.mutation;
    if (!m) continue;
    const name = spec.file;
    if (!SCHEDULES.includes(m.schedule)) out.push(name + "：mutation.schedule 只认 weekly / none，收到 " + m.schedule);
    if (!spec.cases.length) out.push(name + "：声明了 mutation 却没有任何 case（突变要有测试兜底，否则变异体全是 survived）");
    for (const file of m.files) {
      if (GLOB.test(file)) {
        out.push(name + "：mutation.files 不许写 glob —— `" + file + "`（列具体文件，路径漂移才会被判红）");
        continue;
      }
      if (!file.startsWith(unitPrefix(spec.unit))) {
        out.push(name + "：mutation.files 不在本单元范围（应为 " + unitPrefix(spec.unit) + "…）：`" + file + "`");
      }
      if (file.startsWith("src/tests/")) {
        out.push(name + "：mutation.files 不许指向 src/tests/（测试不给自己做突变）：`" + file + "`");
      }
      const prev = owner.get(file);
      if (prev) {
        if (prev !== spec.unit) out.push("mutation.files 被两个单元重复声明：`" + file + "`（" + prev + " / " + spec.unit + "）");
      } else owner.set(file, spec.unit);
      if (!fs.existsSync(path.join(root, file))) {
        out.push(name + "：mutation.files 指向不存在的文件：`" + file + "`（搬文件时同次改声明）");
      }
    }
  }
  return out;
}

/**
 * 收集一次突变运行的范围。
 * scope=weekly 取所有 `schedule: weekly` 的单元；scope=unit 取指定单元（不论 schedule）。
 * 返回 { problems, scope, unit, units: [{ unit, files }], files }；problems 非空时不应开跑。
 */
function collect(root, opts) {
  const options = opts || {};
  const scope = String(options.scope || "weekly");
  const unit = String(options.unit || "");
  const empty = () => ({ problems: [], scope, unit, units: [], files: [] });
  const fail = (problems) => Object.assign(empty(), { problems });
  if (scope !== "weekly" && scope !== "unit") {
    return fail(["scope 只认 weekly / unit，收到：" + scope + (scope === "capability" ? "（旧能力抽查改成 --scope unit --unit <单元名>）" : "")]);
  }
  if (scope === "unit" && !unit) {
    return fail(["scope=unit 需要 --unit <单元名>：可选值见 tests/specs/*.yaml 的 unit 行"]);
  }
  const problems = validate(root);
  if (problems.length) return fail(problems);
  const { specs } = loadSpecs(root);
  let picked;
  if (scope === "weekly") {
    picked = specs.filter((s) => s.mutation && s.mutation.schedule === "weekly");
  } else {
    const spec = specs.find((s) => s.unit === unit);
    if (!spec) return fail(["未知单元：" + unit + "（tests/specs/ 下没有 " + unit + ".yaml）"]);
    if (!spec.mutation) return fail(["单元 " + unit + " 没有 mutation 声明（tests/specs/" + unit + ".yaml 里加 mutation 块）"]);
    picked = [spec];
  }
  const units = picked.map((s) => ({ unit: s.unit, files: s.mutation.files.slice() }));
  const files = [];
  for (const u of units) for (const f of u.files) files.push(f);
  if (!files.length) {
    return fail([(scope === "weekly" ? "没有任何单元声明 `mutation.schedule: weekly`" : "单元 " + unit + " 的 mutation.files 是空的") + "：范围为空，拒绝空跑"]);
  }
  return { problems: [], scope, unit, units, files };
}

module.exports = { SCHEDULES, unitPrefix, validate, collect };
