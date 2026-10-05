// 演示是**真机测试**：条件不齐就明说并拒绝跑，绝不用内置演示通道演一遍。
//
// 条件全部取自产品自己的能力面（`GET /api/state`），不猜：
//   ① 三个模块都在清单里：harvest / render / indexer；indexer 是 C++，要先编译出 build/ 里的可执行文件；
//   ② 登记处里至少有一个供应商（端点 + 密钥）与一个模型；
//   ③ 本次要用的模型定得下来：`SOLOMNI_DEMO_MODEL` 指定的在册，或有一个在册的核心默认模型。
// 三条全过 = 返回 null；否则返回 { reason, hints }，调用方打印后以退出码 2 收场（条件不足 ≠ 演示失败）。
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const NEEDED_MODULES = ["harvest", "render", "indexer"];

/** indexer 的构建产物在不在（两个平台的名字都认）。 */
function indexerBuilt() {
  const build = join(HERE, "..", "modules", "indexer", "build");
  return existsSync(join(build, "indexer.exe")) || existsSync(join(build, "indexer"));
}

export function demoPreflight(state, model) {
  const modules = (state && state.modules) || [];
  const providers = (state && state.providers) || [];
  const models = (state && state.models) || [];
  const core = (state && state.core) || "";
  const missing = NEEDED_MODULES.filter((id) => !modules.some((m) => m && m.id === id));
  if (missing.length) {
    return {
      reason: "模块清单里缺 " + missing.join("、"),
      hints: [
        "模块放进 modules/ 就出现（清单是目录扫描的纯函数）",
        "indexer 是 C++：按 modules/indexer/README.md 的命令编译一次",
      ],
    };
  }
  if (!indexerBuilt()) {
    return {
      reason: "indexer 还没构建（找不到 modules/indexer/build/ 下的可执行文件）",
      hints: ["按 modules/indexer/README.md 的命令编译一次（产物不入库，要本机自己建）"],
    };
  }
  if (!providers.length) {
    return {
      reason: "登记处里没有供应商",
      hints: ["界面「设置 → 供应商」加一个端点 + 密钥（密钥只写进 .home/providers.yaml，不回显）"],
    };
  }
  if (!models.length) {
    return {
      reason: "登记处里没有模型",
      hints: ["界面「设置 → 模型」加一个模型（选供应商 + 写实际模型串）"],
    };
  }
  const known = models.map((m) => m && m.id).filter(Boolean);
  if (model) {
    if (!known.includes(model)) {
      return {
        reason: "SOLOMNI_DEMO_MODEL 指定的模型不在登记处：" + model,
        hints: ["在册的模型：" + known.join("、")],
      };
    }
    return null;
  }
  if (!core || !known.includes(core)) {
    return {
      reason: "没有可用的核心默认模型",
      hints: [
        "在界面「设置」里给核心选一个默认模型，或用 SOLOMNI_DEMO_MODEL=<模型 id> 指定",
        "在册的模型：" + known.join("、"),
      ],
    };
  }
  return null;
}

/**
 * 代理演示（`demo/run-demo-proxy.mjs`）的前置：与前两个演示不同，这里**不指定模块**——
 * 挑人是核心自己的事（核心可以临时组装零模块的 agent：只用内建文件工具）。
 *
 * 代理会话跑在**核心默认模型**上（代理工具的执行者是核心自己）：没有核心默认模型就会回落到内置
 * 演示通道——那条通道不会原生调工具，验不出代理能力，所以这里与其余前置一样**如实拒跑**。
 * 也因此本演示**不认 `SOLOMNI_DEMO_MODEL`**（那是给 agent 指定模型用的）。
 */
export function proxyPreflight(state) {
  const providers = (state && state.providers) || [];
  const models = (state && state.models) || [];
  const core = (state && state.core) || "";
  if (!providers.length) {
    return {
      reason: "登记处里没有供应商",
      hints: ["界面「设置 → 供应商」加一个端点 + 密钥（密钥只写进 .home/providers.yaml，不回显）"],
    };
  }
  if (!models.length) {
    return {
      reason: "登记处里没有模型",
      hints: ["界面「设置 → 模型」加一个模型（选供应商 + 写实际模型串）"],
    };
  }
  const known = models.map((m) => m && m.id).filter(Boolean);
  if (!core || !known.includes(core)) {
    return {
      reason: "没有可用的核心默认模型（核心代理跑在核心通道上）：" + (core || "（没设）"),
      hints: [
        "在界面「设置」里给核心选一个默认模型——代理会话与它临时挑出来的子会话都走这条通道",
        "在册的模型：" + known.join("、"),
      ],
    };
  }
  return null;
}

/** 打印条件不足的原因与出路；退出码 2 = 条件不足（不是演示失败）。 */
export function refuseDemo(block) {
  console.error("DEMO-SKIPPED：" + block.reason);
  for (const h of block.hints) console.error("  · " + h);
  console.error("  （演示走真实模型：条件不齐就不跑，不用演示通道凑一遍。）");
  return 2;
}
