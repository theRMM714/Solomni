# Solomni

> **中文版:** [README.md](README.md) —— the same overview in Chinese.

Solomni is an **Agent OS** that runs entirely on your own machine.

## What it is, in one sentence

**One executable plus a `modules/` folder is the whole product.** No installer, no server, no account.

This project is **not about attaching tools to an agent — it gives a tool or skill an agent**:

- **A module is a folder** = a charter prompt + optional external tools + a private workspace.
  Drop it into `modules/` and it appears; take it out and it is gone — no registration, no build, no restart.
- **A module can be anything**: a prompt, a Python / Node / C++ script, an MCP, a skill.
  It only needs a `module.yaml` written in YAML to follow the contract, and it **imports no code or
  dependency from this project**. Your tools stay yours: without this product they still run.
- **Agents speak, not modules**: an agent = a name + the modules it holds + its model + its own sandbox.
  Whichever agent a module is loaded into, that agent speaks in its name.
- **The form follows the task**: the same commons of capabilities can project as one AI holding several
  capabilities (single agent), or as several AIs negotiating with separate powers (collaboration).
  When the task ends the projection dissolves; the capabilities still belong to the modules.
- **Local by design**: the web UI binds `127.0.0.1` only; keys live in the local registry and in outbound
  calls only, and never enter prompts, transcripts, logs or module workspaces.
- **The transcript is the context**: what you see and what enters the model's context are the *same thing*;
  an agent's words are always data, never instructions.

For the design philosophy and invariants, read [PHILOSOPHY.md](PHILOSOPHY.md).

## Where the project stands

**Working today**:

- **Two forms**: single agent (direct / composite) and group collaboration — form-up → discussion →
  a task chain is drafted → **review gate** (nothing starts until you approve) → execution per stage with
  concurrency inside a stage → **one acceptance pass per stage** → final acceptance → delivery. On failure,
  only the named nodes are sent back (nodes already passed in that stage stay passed); you control the flow
  with Stop / Continue.
- **Two interfaces**: the terminal transcript center (default) and the local web UI (`-webUI`, binds
  `127.0.0.1` only, port 3081 by default).
- **Sessions and history**: one directory per piece of work (`session/<name>/`), append-only records,
  rewind to any line, and context compaction.
- **Three isolation layers**: path checks inside the built-in tools → every tool process runs behind the
  gate process (environment allowlist / process tree / timeout kills the whole tree) → platform fences
  (Windows AppContainer, Linux Landlock, macOS seatbelt). If a mechanism cannot be installed, the startup
  report says so instead of pretending.
- **Registry**: four YAML files for providers / models / agents / settings, all under `.home/`, managed
  from the UI; keys are never echoed back.

**Not wired up yet (stated plainly)**:

- **The guest body for the VM tier**: selection, diagnostics, the mount plan and the per-item pre-checks are
  in place, but the guest itself is not wired up, so the VM tier **cannot be selected at all** right now
  (the UI lists exactly what is missing and how to provide it).
- **Credential custody for external tool services** (module tools must not depend on credentials today),
  module packaging/marketplace, and further parallel dispatch at execution time.
- Every unfinished item, how to finish it and its acceptance criteria live in exactly one place:
  [tests/gaps.yaml](tests/gaps.yaml).

**Quality**: `node run-tests.js` is the single entry point — locally it runs T0 (format / compile / clippy /
duplicate dependencies, all zero-tolerance) plus unit, cross-platform integration, frontend smoke and
end-to-end tests; real-machine fence probes run in three-platform CI (see [TESTING.md](TESTING.md)).

## How to use it

### 1) Start the product

```bash
start.bat                 # Windows (double-click or command line)
node start.js             # any platform (macOS/Linux: ./start.sh)
node start.js -webUI      # go straight to the local web UI; or type webui in the menu
```

The first run needs no manual setup: the launcher keeps the toolchain inside the project
(`platform/`, `.tools/`) and asks for consent before installing Rust if it is missing.
**It also runs without any provider configured** — a built-in fake model demonstrates the flow and says so.

### 2) Register a channel and models (optional)

Add a provider (endpoint + key) and models in the UI, and pick the core default model.
Keys are written only to `.home/providers.yaml`.

### 3) Create a piece of work

Name it → pick a form (single agent / collaboration) → pick agents (the core can recommend a roster, but
**you confirm it**) → for collaboration, write down the task. Then watch it discuss, execute and review:
you can Stop / Continue at any time, and speak inside any sub-session.

### Run the demo

A real provider is required; without one the built-in fake model keeps the flow running but calls no tools
and produces no artifacts.

```bash
node start.js -webUI
node demo/run-demo.mjs         # composite: one agent holding three modules (python / node / C++)
node demo/run-demo-collab.mjs  # collaboration: three agents with separate powers, end to end
```

The C++ module has to be compiled once; the command and the reason are in
[modules/indexer/README.md](modules/indexer/README.md).

## What it is not

- Not a model or a provider: channels and keys are product resources, and models are replaceable.
- Not a runtime that schedules resident processes: modules never sit around waiting; they work once per task.
- It does not favour either extreme: centralised and decentralised live on one orchestration plane, and the
  user picks the form.

## Document guide

| I want to… | Read |
| --- | --- |
| A project overview and quick start | this file (Chinese: [README.md](README.md)) |
| Understand *why* it is designed this way | [PHILOSOPHY.md](PHILOSOPHY.md) |
| Use the product / follow the user journey | [PRODUCT.md](PRODUCT.md) |
| Write a module (YAML contract, tools, path model) | [MODULE_SPEC.md](MODULE_SPEC.md) |
| Build a runtime package (`package.yaml`) | [RUNTIME_SPEC.md](RUNTIME_SPEC.md) |
| Manage providers / models / agents / settings (and key boundaries) | [REGISTRY_SPEC.md](REGISTRY_SPEC.md) |
| Change the code (layering, ports, logging, prompts, persistence) | [ARCHITECTURE.md](ARCHITECTURE.md) |
| Write tests and acceptance (levels, doubles, gates, gaps, CI) | [TESTING.md](TESTING.md) |
| Repo collaboration rules and the document routing table | [AGENTS.md](AGENTS.md) |

> **Two layers**: the repository root holds the **portals** (positioning and references) and `docs/` holds the
> **details** — every fact has exactly one home. Details: `docs/testing/` (levels, doubles and the port matrix,
> quality and isolation, entry points and CI, gaps and acceptance, module delivery) and `docs/architecture/`
> (module map, inbound contract and route table, system tools and roles, task chain and sub-sessions, session
> model, prompt booklet). The full routing table is in [AGENTS.md](AGENTS.md).

Apache License 2.0 · see [LICENSE](LICENSE)
