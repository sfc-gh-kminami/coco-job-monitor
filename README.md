# CoCo Job Monitor

macOS native **"Factory Andon + RunCat"** job monitor for parallel Coding Agent sessions, built in Rust (`gpui-kit` + `objc2`).

Zero LLM tokens, <0.2% idle CPU, instant Hook + FSEvents state tracking, animated pixel-art menu bar sprite with dynamic FPS, and virtual-audio-safe system alert chimes.

---

## Key Features

- **RunCat-style Animated Menu Bar (`NSStatusItem`)**
  - Gothic Crow Hunter pixel-art GIFs (`idle.gif` / `run.gif` / `wait.gif` / `trouble.gif` / `done.gif`)
  - **Dynamic Run Speed**: `130ms/frame` (1 agent) → `85ms/frame` (2 agents) → `55ms/frame` (~18fps sprint for 3+ parallel agents or active subagents)
  - **Hot-Reload Custom GIFs**: Drop your own `64×36px` transparent GIFs (or sequential PNG folders) into `~/.snowflake/cortex/job-monitor/icons/` — reloads automatically within 3s without restarting
- **Andon Notification Discipline**
  - Silent when submitting prompts or while agents are making progress
  - Chimes **only** when the ball returns to the human (`PermissionRequest` / `ask_user_question` → `Ping`, `Completed` → `Glass`, `Error` → `Sosumi`)
  - Routes audio via macOS `AudioToolbox` (`AudioServicesPlayAlertSound`) so alerts still play through Mac speakers even when `ZoomAudioDevice` hijacks default output
- **Inbox Zero Auto-Dismiss**
  - Shows only **unseen** completed sessions; automatically clears once viewed in Cortex Code for 4 seconds
- **4 Synchronized Views**
  - Snap-docked Side Panel (`368×580px`) following the Cortex Code window
  - 1-Line Mini Pill (`368×54px`)
  - 3-Column Kanban Detail Board (`1240×780px`)
  - Live Web Dashboard (`http://127.0.0.1:47823/`)

---

## Quick Start (macOS)

Requires Rust toolchain (`cargo`) and `jq`.

```bash
sh build.sh
open -a "$HOME/Applications/CoCo Job Monitor.app"
```

`./build.sh` compiles the release binary, creates `~/Applications/CoCo Job Monitor.app`, installs default GIF sprites into `~/.snowflake/cortex/job-monitor/icons/`, and registers lifecycle hooks via `sync-hooks.py`.

---

## Documentation & Interactive Visualizers

- [`SPECIFICATION.md`](./SPECIFICATION.md) — Full architecture, state machine, and Universal Agent Andon wrapper specification
- [`coding-agent-anatomy.html`](./coding-agent-anatomy.html) — Interactive 10-step Coding Agent harness & SSE protocol debugger
- [`crow-hunter-preview.html`](./crow-hunter-preview.html) — Menu bar sprite & dynamic FPS simulator
