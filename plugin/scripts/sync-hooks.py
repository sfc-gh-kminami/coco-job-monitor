#!/usr/bin/env python3
import json
import os
import sys

HOOKS_PATH = os.path.expanduser("~/.snowflake/cortex/hooks.json")
PLUGIN_HOOKS_PATH = os.path.expanduser(
    "~/.snowflake/cortex/plugins/coco-job-monitor/.hooks.json"
)
MARKER = "coco-job-monitor/scripts/notify.sh"


def load_json(path):
    if not os.path.exists(path):
        return {}
    with open(path, "r", encoding="utf-8") as fp:
        try:
            return json.load(fp)
        except Exception:
            return {}


def main():
    uninstall = "--uninstall" in sys.argv
    current = load_json(HOOKS_PATH)
    hooks = current.get("hooks")
    if not isinstance(hooks, dict):
        hooks = {}

    for ev in list(hooks.keys()):
        entries = hooks.get(ev) or []
        cleaned = []
        for entry in entries:
            sub = [
                h
                for h in (entry.get("hooks") or [])
                if MARKER not in (h.get("command") or "")
            ]
            if sub:
                entry["hooks"] = sub
                cleaned.append(entry)
        if cleaned:
            hooks[ev] = cleaned
        else:
            hooks.pop(ev, None)

    if not uninstall:
        plugin_cfg = load_json(PLUGIN_HOOKS_PATH).get("hooks") or {}
        for ev, entries in plugin_cfg.items():
            hooks.setdefault(ev, []).extend(entries)

    out = dict(current)
    if hooks:
        out["hooks"] = hooks
    else:
        out.pop("hooks", None)

    os.makedirs(os.path.dirname(HOOKS_PATH), exist_ok=True)
    with open(HOOKS_PATH, "w", encoding="utf-8") as fp:
        json.dump(out, fp, indent=2, ensure_ascii=False)
        fp.write("\n")

    mode = "uninstalled" if uninstall else "installed"
    print(f"coco-job-monitor hooks {mode} in {HOOKS_PATH}")


if __name__ == "__main__":
    main()
