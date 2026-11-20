---
name: job-monitor
description: CoCo Job Monitor（Rust GPUI製の最前面ジョブ監視ウィンドウ＋メニューバー）を起動・再表示・停止するスキル。「ジョブ管理ウィンドウを開いて」「Job Monitor起動」などの依頼時に使用する。
---

# CoCo Job Monitor Skill

Cortex Code Desktop の全ワークスペース横断ジョブ監視ウィンドウ（`CoCo Job Monitor.app`）を操作します。

## 起動・前面表示
以下のコマンドを実行してアプリを起動します（すでに起動中の場合はウィンドウが前面に表示されます）:

```bash
open -a "$HOME/Applications/CoCo Job Monitor.app" || "$HOME/.snowflake/cortex/plugins/coco-job-monitor/bin/coco-job-monitor" &
```

## Hook 連携の再有効化 / 解除
- 有効化: `python3 "$HOME/.snowflake/cortex/plugins/coco-job-monitor/scripts/sync-hooks.py"`
- 解除: `python3 "$HOME/.snowflake/cortex/plugins/coco-job-monitor/scripts/sync-hooks.py" --uninstall`
