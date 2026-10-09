# Solomni

> **中文:** [README.md](README.md) —— the same overview in Chinese.

Solomni is an **agent runtime environment (AgentOS) that runs on your own machine**: one executable plus a
`modules/` folder is the whole product. No installer, no server, no account.

## Why you need it

**① You have a tool or a workflow, and you want AI to use it.**
The usual route is to wrap it as a plugin for one specific product — and then it is tied to that product: switch agents
or platforms and you wrap it again, while the tool itself grows a shell coupled to the host. Here you **adapt it to the
contract once**: a folder plus a `module.yaml` (responsibility prompt + tool declarations), with the command it already
had; some things need a small change on their own side (putting an executable inside the module folder, agreeing on how
arguments are passed and how the receipt looks), but that change stays on their side — no restructuring, no SDK. Once
adapted, **nothing is left behind**: **outside the contract the two sides depend on nothing from each other** — take the
platform away and the tool still runs as it did; take the tool away and the platform keeps running. The same capability is
used by people through the UI and by agents through the contract from a single declaration — no two entry points to maintain.

**② Several agents working on one job.**
A shared workspace has concrete problems by default: they overwrite each other, ownership of artifacts is unclear, and
someone builds on another's unfinished output as if it were fact. Here every agent has **its own sandbox**
(`session/<name>/<agent-instance>/`), which is also its **working copy** of the shared area; the shared area `work/`
is the **main copy and read-only for agents** — pull what you need with `work_pull`, edit in the sandbox, then commit
with `work_commit`. Commits use a file-level three-way comparison, so a conflict rejects the whole commit and names
each path instead of silently overwriting; any commit point can be restored. The only two roads across agents are that
shared area and the transcript that enters the context — artifacts land in their owner's cell, so rework can find the
person; capabilities are isolated too, a module's directory belongs only to the agent that holds it.
(Finer-grained "who may write which part" is enforced by session permissions: per-agent read and commit
allow-/block-lists, module directories read-only by default — see
[docs/permission/README.md](docs/permission/README.md).)

**③ When something goes wrong, you need to know what it saw and why it decided that.**
The transcript is **append-only** (delete/restore are explicit actions that really rewrite the log). On any line,
"rewind to here" offers: **archive** — append a marker and collapse the earlier lines (all bytes kept, recoverable);
**delete** — really truncate to that point; **restore** — truncate at the marker, discarding work done since. Line ids
are monotonic and never reused, replay is reproducible, so **a rebuild matches the live run line by line**; the shared
area comes back to that moment too, via the `(agent, line)` anchor on each commit, with the whole subtree synchronized.
A delivery can be replayed from disk — what the AI did is an auditable, reproducible engineering artifact, not a chat
that is gone.

## What it is made of

| Thing | What it is |
| --- | --- |
| **Core** | The only program in the product (Rust). It provides the mechanisms: sessions and transcripts, task orchestration and acceptance, tool execution and fencing, the registry, rewind and compaction |
| **Module** | A folder `modules/<id>/`: a `module.yaml` (responsibility prompt + tool declarations) plus the tools themselves — any language, anything that runs |
| **Agent** | A name + the modules it holds + a model + its own sandbox. Whichever agent a module is loaded into, it speaks in that agent's name |
| **Users** | People and agents. People use the interfaces (terminal / local web UI), agents use the module contract — the same modules, two doors |

## What works today

- **Three forms** (chosen per piece of work):
  - **single agent**: one AI holding several module capabilities, working directly;
  - **group collaboration**: several agents each holding one capability — form-up → discussion → a task chain
    is drafted → **review gate** (nothing starts until you approve) → execution per stage with concurrency →
    one acceptance pass per stage → final acceptance → delivery; on failure only the named nodes go back;
  - **proxy**: hand the whole decision to the core, which picks people and creates child sessions; entered
    from the API / scripts today.
- **Modules are drop-in**: a folder with a valid `module.yaml` under `modules/` appears; remove it and it is
  gone — no registration, no build, no restart.
- **Sessions and history**: one directory per piece of work (`session/<name>/`); "rewind to here" on any line —
  **archive** (recoverable) / **delete** (real truncation) / **restore** — and the shared area comes back with it,
  anchored by commit line; transcripts are replayable, context is compactable; child sessions live under the
  parent's `children/` (nesting is allowed) and the whole tree shares **one** work area.
- **Three isolation layers**: path checks inside the built-in tools → every tool process runs behind the gate
  process (environment allowlist / process tree / timeout kills the whole tree) → platform fences
  (Windows AppContainer, Linux Landlock, macOS seatbelt). If a mechanism cannot be installed, the capability
  grade is reported honestly instead of pretending. Fence grant targets are split into **required** and
  **optional**: an optional target that cannot be granted is only recorded, while a **required** one is never
  degraded silently — it raises a card offering "run this once unfenced / drop the call", and with no front end
  able to answer, the call is refused (the command is never started).
- **Registry**: four YAML files for providers / models / agents / settings, all under `.home/`, managed from
  the UI. Keys live only in the local registry and in the core's outbound calls (boundaries in
  [REGISTRY_SPEC.md](REGISTRY_SPEC.md)).
- **Two interfaces**: the terminal transcript center (default) and the local web UI (`-webUI`, binds
  `127.0.0.1` only, port 3081 by default).
- **One decision channel**: everything that needs your call — the core's gates (roster / start / plan review /
  failed node), tool-level confirmation, and questions raised by the tool layer itself — uses the same card:
  **message + options** (the option id is the contract, wording belongs to the UI), answered by one command.
  It **blocks**: nothing moves until you answer; pressing stop voids the whole queue and releases waiters as
  "denied". Cards and answers are persisted with the session (already answered ones are never re-asked); when no
  option can actually be executed, no card is shown — the session is stopped and a warning is recorded.

## Install and run

Requirements: **Node** (to run the launcher) and **Rust** (to build the core).

```bash
node env.js setup        # prepare the environment only: toolchain into platform/<os>/ and .tools/
node start.js            # any platform; on Windows also start.bat, on macOS / Linux also ./start.sh
node start.js -webUI     # go straight to the local web UI (127.0.0.1:3081)
```

- The environment is its own layer ([env.js](env.js)): path conventions (`platform/<os>/`, `.tools/`), toolchain
  detection, environment composition and installation all live there. The launcher only orchestrates build/run, and
  `run-tests.js` resolves its environment from the same place — **switching dev environments means editing one file**.
  `node env.js` shows the resolved environment; `node env.js --print-env` prints it machine-readable.
- The toolchain stays **inside the project**: if Rust is missing it asks for consent before installing and never
  touches the system; with **no interactive terminal** it skips the install, prints the manual steps, and exits.
- **You can run it without a provider**: it walks the flow with the built-in fake model and says so plainly.
- Common flags: `--root <dir>` (product root), `--web-port <port>`, `--release`.
- Commands at the prompt: `single [agent…]` / `collab [agent…|?]` / `proxy` (hand the whole decision to the core) / `module [module-id.tool [json]]` (run a module tool directly, without AI) / `webui`.

### Add a provider and a model

In the UI, add a provider (endpoint + key) and models, and pick the core default model. Keys are written only
to `.home/providers.yaml` and are never echoed back.

## Module contract

A module is a folder plus a `module.yaml`, and it **depends on nothing in this project**; the core does not
depend on the module's implementation either — both sides depend only on this contract (full fields and rules
in [MODULE_SPEC.md](MODULE_SPEC.md)):

```yaml
id: research                 # globally unique, = the folder name
brief: Research and comparison.   # short capability note: what the core picks people from
system: You take care of research...  # responsibility prompt: scope + how your tools are used
runtimes: [python]           # optional: runtime capabilities the tools need (versions are chosen per session)
tools:                       # optional: external tool table
  read_txt:
    command: python tools/read_txt.py   # working directory = the module root
    params:
      path: { type: string, required: true, desc: real absolute path to read }
```

- **One calling convention**: a start command + arguments + a stdin/stdout receipt. Scripts, skills, MCP servers
  and other harnesses are all instances of it.
- A tool process can reach: that agent's private sandbox + its own module directory; the shared main copy is not
  among them (pull before editing, commit when done); it accepts **real absolute paths** only, and anything outside is refused.
- **Every module keeps a cross-task private area** `<module>/userdata/`: it is ensured to exist when the
  module is loaded (directory only, idempotent; failure never blocks loading, it is just reported — that seat then
  has no private landing). Module directories are read-only by default, `userdata/` excepted.
- If it runs, it is a valid module; if it does not, the core reports the reason plainly — no guessing, no fallback.

### Run the demo

The demo is a **real-machine test**: it runs against a real model — it feeds real files, calls real tools and
checks real artifacts. So it verifies the preconditions first. The first two scripts need the three modules in
the roster, the indexer built, and a provider plus a usable model (`SOLOMNI_DEMO_MODEL`, or the core default);
the **proxy** script needs a provider, a model and a **core default model**, plus at least one module or a
stored agent.
**When a precondition is missing it prints `DEMO-SKIPPED` with how to fix it and exits with code 2** — it never
runs the flow on the built-in demo channel just to look successful.

```bash
node start.js -webUI
node demo/run-demo.mjs         # composite: one agent holding three modules (python / node / C++)
node demo/run-demo-collab.mjs  # collaboration: three agents with separate powers, end to end
node demo/run-demo-proxy.mjs   # proxy: talk to the core only; it picks people, one stop stops the tree
```

The C++ module has to be compiled once (the artifact is not committed); the command and the reason are in
[modules/indexer/README.md](modules/indexer/README.md) — the preflight check confirms it for you.

## Not wired up yet

- **The guest body for the VM tier**: selection, diagnostics, the mount plan and the per-item pre-checks are in
  place, but the guest itself is not wired in, so the **VM tier cannot be selected at all** right now (the UI
  lists what is missing and how to fix each item).
- Credential custody for external tool services (module tools must not depend on credentials today), module
  distribution and a marketplace, and wider concurrent dispatch.
- Every unfinished item, how to fix it and its acceptance criteria have exactly one authority:
  [tests/gaps.yaml](tests/gaps.yaml).

## What it is not

- Not a model or a provider: channels and keys are product resources, and models are replaceable.
- Not a runtime that schedules resident processes: modules never sit around waiting; they work once per task.
- Not a server: it binds `127.0.0.1` only — no server side, no account.

**Quality**: `node run-tests.js` is the single entry point — T0 (format / compile / clippy / duplicate deps /
structural review, zero tolerance) + unit + cross-platform integration + frontend smoke + end-to-end; the
real-machine fence probes run on three CI platforms (see [TESTING.md](TESTING.md)).

## Document guide

| I want to… | Read |
| --- | --- |
| See the overview and quick start | This file (Chinese: [README.md](README.md)) |
| See what this project is meant to become (direction and commitments) | [PHILOSOPHY.md](PHILOSOPHY.md) |
| Use the product / see user journeys and interfaces | [PRODUCT.md](PRODUCT.md) |
| Write a module (YAML contract and module tools) | [MODULE_SPEC.md](MODULE_SPEC.md) |
| System tools, roles and the path model (reports and acceptance too) | [SYSTOOL.md](SYSTOOL.md) |
| The planned core-proxy system tools | [systool_gaps.yaml](systool_gaps.yaml) (planned, not current capability) |
| Build a runtime package (`package.yaml`) | [RUNTIME_SPEC.md](RUNTIME_SPEC.md) |
| Manage providers / models / agents / settings (and key boundaries) | [REGISTRY_SPEC.md](REGISTRY_SPEC.md) |
| Change code (layers, ports, logging, prompts, on-disk contracts) | [ARCHITECTURE.md](ARCHITECTURE.md) |
| Write tests and acceptance (levels, doubles, gates, gaps, CI) | [TESTING.md](TESTING.md) |
| Repository rules and document routing | [AGENTS.md](AGENTS.md) |

> **Two layers**: the repository root holds **portals** (positioning + references), `docs/` holds the **details**
> — every fact has exactly one authority. Full routing is in [AGENTS.md](AGENTS.md) under "Document routing".

Apache License 2.0 · see [LICENSE](LICENSE)
