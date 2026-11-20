# CoCo Job Monitor（Universal Agent Andon）要件定義・技術仕様書

- **対象システム**: `CoCo Job Monitor.app`（Rust + GPUI ネイティブ常駐モニター ＆ ローカルイベントバス）
- **アプリ配置先**: `~/Applications/CoCo Job Monitor.app`（プラグイン連携: `~/.snowflake/cortex/plugins/coco-job-monitor`）
- **対象OS**: macOS（Apple Silicon / Intel, AppKit / CoreGraphics / AudioToolbox ネイティブ統合）

---

## 1. 背景・解決する課題（要件定義）

### 1.1 課題背景
Coding Agent（Cortex Code / Claude Code / Codex CLI 等）を複数セッション並列で実行する運用において、以下の認知負荷・手待ちロスが発生する。

1. **サイレント停止（承認・回答待ちの見落とし）**: エージェントがローカル権限ゲート（`PermissionRequest` や `ask_user_question`）でブロックしていることに気づかず、数十分間ジョブが停止したままになる。
2. **稼働中とフリーズの判別困難**: 長時間推論やサブエージェント探索中なのか、対話プロンプト詰まり等でスタックしているのかが外から判別できない。
3. **完了確認の埋没とノイズ過多**: 過去に確認済みの完了セッションまで一覧に残り続けると、「今どの完了結果をまだ見ていないか」が埋もれる。またプロンプト送信のたびに音が鳴るとオオカミ少年化して無視される。

### 1.2 設計思想（製造現場の「アンドン・自働化」原則の適用）
1. **ゼロ・トークン／ゼロLLM監視**: 監視自体にはLLM推論を一切使用せず（0クレジット）、ローカルイベント駆動＋差分ファイルI/Oのみで CPU 負荷 0.2% 未満で常駐する。
2. **ボール保持者ベースの通知制御**: 人間がプロンプトを投げた時やエージェントが順調に稼働している間は無音とし、**「人間の承認・判断が必要になった瞬間」と「タスクが完了／異常停止した瞬間」だけ** 音声と視覚で知らせる。
3. **Inbox Zero（未確認完了のみ表示）**: 完了ジョブは「まだ人間が結果画面を開いていないもの（未確認完了）」だけを表示し、エージェント画面で4秒以上閲覧（または既読化）した時点でモニターから自動消去する。
4. **周辺視野モニタリング（RunCat方式）**: ウィンドウを閉じていても、macOSメニューバー上のドット絵キャラクターの「ポーズ・色・走る速度」だけで全体の並列負荷と異常有無が即座に分かるようにする。

---

## 2. システム全体アーキテクチャ

```text
┌────────────────────────────────────────────────────────────────────────────┐
│ ① エージェント / LLM 実行層（イベント発生源）                               │
│  ├─ Cortex Code Desktop / CLI  ──(Hooks: notify.sh)──────────────┐         │
│  ├─ Claude Code / Gemini CLI   ──(共通 Hook アダプタ ※拡張)──────┤         │
│  ├─ Codex / Aider / 一般CLI    ──(PTY Wrapper: andon -- <cmd>)───┤         │
│  └─ Python SDK / LangGraph     ──(HTTP Callback Client)──────────┤         │
└──────────────────────────────────────────────────────────────────┼─────────┘
                                                                   ▼
                        POST http://127.0.0.1:47823/event (JSON, 本文不含)
                                                                   │
┌──────────────────────────────────────────────────────────────────┼─────────┐
│ ② CoCo Job Monitor 本体 (Rust + gpui-kit + smol + objc2)         ▼         │
│  ├─ Loopback HTTP Server (127.0.0.1:47823) : Hook受信・REST API・Web UI配信 │
│  ├─ FSEvents Watcher + Scanner             : 会話 .json / .jsonl 末尾差分読取│
│  ├─ Read-State Sync (SQLite ?mode=ro)      : アクティブ会話・未読/既読の同期 │
│  ├─ JobStore (単一状態統合パイプライン)    : 優先度ソート・スタック判定・消込 │
│  └─ macOS Native Integration (objc2 / FFI)                                 │
│      ├─ NSStatusItem GIF Animator          : 可変FPSドット絵メニューバー表示 │
│      ├─ CGWindowList Tracker               : CoCoウィンドウ右側への自動追従  │
│      └─ AudioToolbox Alert Router          : 仮想デバイス回避チャイム鳴動    │
└────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. 機能仕様（現行実装済み機能）

### 3.1 4つの表示サーフェス
| サーフェス | 役割・仕様 |
| --- | --- |
| **① サイドパネル小窓 (`PanelView`)** | 幅 `368px` × 高さ `580px`。Cortex Code ウィンドウの右上／右下へ自動スナップ追従（または自由配置）。CoCo 最小化時は自動で隠れ、復帰時に再表示される。 |
| **② 1行ミニピルモード** | 幅 `368px` × 高さ `54px`。`稼働N · 待ちM` と最優先ジョブ1件のみを1行表示し、画面占有を最小化する。 |
| **③ 3カラム詳細ボード (`BoardView`)** | `1240×780px`。「要対応（承認・回答待ち）」「稼働中・スタック」「完了履歴（確認済み表示トグル付）」の3レーン俯瞰ビュー。 |
| **④ Chrome Webダッシュボード** | `http://127.0.0.1:47823/` で配信される自動更新Web画面。外部ブラウザでの常時表示用。 |

### 3.2 ジョブ状態（`JobStatus`）と遷移ルール
表示優先度は上から順（同順位内は最終アクティビティの新しい順）とする。

| 優先度 | ステータス | 判定条件 | パネル表示色 | サウンド |
| --- | --- | --- | --- | --- |
| **1** | `WaitingPermission`（許可待ち） | `PermissionRequest` Hook 受信時 | 琥珀色 (`#F59E0B`) | `Ping.aiff` |
| **2** | `WaitingInput`（回答・確認待ち） | `PreToolUse` で `ask_user_question` / `create_plan` 実行時、または `.history.jsonl` 末尾が未応答の同ツール呼び出し時 | 琥珀色 (`#F59E0B`) | `Ping.aiff` |
| **3** | `Running`（稼働中） | `lastTurn.outcome == "running"` かつ無応答時間が `180秒` 未満（またはプロンプト送信直後で `.history.jsonl` 末尾が45秒以内の `role: "user"`） | シアン (`#29B5E8`) | **無音** |
| **4** | `Stalled`（スタック疑い） | `outcome == "running"` のまま Hook 発火・JSONL追記・サブエージェント通信のいずれも **`180秒以上`** 途絶した時 | 赤 (`#EF4444`) | 無音（手動 `×` 非表示可） |
| **5** | `Error`（エラー終了） | `lastTurn.outcome == "error"` | 赤 (`#EF4444`) | `Sosumi.aiff` |
| **6** | `Completed`（新着完了・未確認） | `Stop` Hook または `outcome == "success"` 遷移時。`unseen_done == true` の間のみ小窓に表示 | エメラルド (`#10B981`) | `Glass.aiff` |

#### ブリンク防止・レースコンディション対策仕様
- **空チャット枠の除外**: `SessionStart` Hook 単体、および `isEmpty: true`・サブエージェント専用セッション（`subagentSessionId` 紐づき）はメインジョブ一覧から除外する。
- **サブエージェント生存伝播**: 子エージェントの Hook 発火時は一時オーバーレイを作らず、親ワークスペースのハートビートのみを更新する（ただし既に180秒超スタック済みの古いセッションは復活させない）。
- **プロンプト送信直後の誤完了防止**: `UserPromptSubmit` 受信から `4,000ms` 間はグレース期間として `Running` を維持し、`.json` 更新遅延による `Running → Completed` 誤チャイムを抑止する。

### 3.3 Inbox Zero（未確認完了の自動消込）仕様
完了ジョブ（`unseen_done = true`）は、以下のいずれかを満たした瞬間に「確認済み」として小窓リストから自動消去する。
1. Cortex Code が最前面（Frontmost）にあり、かつ `state.vscdb` の `agentManager.activeConversationV2` が当該 `session_id` と一致した状態が **連続 4.0秒以上** 継続したとき。
2. `state.vscdb` の `agentManager.isRead.<ws>\x00<session_id>` が `'0'`（未読）から `'1'`（既読）へ変化したとき。
3. ユーザーがジョブカード本体をクリックして CoCo を開いたとき、個別 `×` ボタンを押したとき、または「一括クリア」を押したとき。

### 3.4 ウィンドウフォーカス仕様（IDE誤起動の防止）
- ジョブカードクリック時はワークスペースのフォルダパスを直接 `open` せず（Editor/IDEモードが開くのを防ぐため）、Agent 専用ワークスペース定義 `~/Library/Application Support/Cortex Code/User/agent-manager.code-workspace` を指定して `Cortex Code.app` の CoWork（Agent）画面のみを前面化する。

### 3.5 サウンド出力仕様（仮想オーディオデバイス回避）
- Web会議ツール（`ZoomAudioDevice` 等）が macOS の `Default Output Device` を占有していても確実にスピーカーから通知音を鳴らすため、`/usr/bin/afplay` は使用しない。
- macOS `AudioToolbox.framework` の `AudioServicesCreateSystemSoundID` ＋ `AudioServicesPlayAlertSound` を FFI で直接呼び出し、OS の **システム効果音出力先（Alert Output Device）** へルーティングする。

---

## 4. メニューバー・アニメーション仕様（RunCat方式・烏羽の狩人）

### 4.1 アセット仕様とホットリロード
- **参照パス**: `~/.snowflake/cortex/job-monitor/icons/`（未配置・欠損時はバイナリ内蔵 `assets/icons/*.gif` に自動フォールバック）
- **対応形式**: マルチフレーム透過GIF（`<stem>.gif`）または連番透過PNGディレクトリ（`<stem>/00.png, 01.png...`）
- **推奨解像度**: `64×36px`（Retina 2x / メニューバー実表示 `32×18pt`、任意の縦横比を高さ `18pt` 基準で自動スケーリング）
- **自動ホットリロード**: 3秒周期でアイコンディレクトリのタイムスタンプ・サイズハッシュを検査し、変更があればアプリ再起動なしで即時再デコードする。
- **カラー自動判定**: `idle` / `run` は全ピクセルがモノクロの場合のみ `NSImage::setTemplate(true)` を有効化（OSのダーク／ライトモードに自動追従）。差し色を含むカスタムGIFが置かれた場合は自動で `setTemplate(false)` に切り替える。

### 4.2 状態優先度と可変フレームレート表

| 優先度 | 判定条件 | 再生ファイル | フレーム間隔 (FPS) | `狩人+数` モード時の横テキスト |
| --- | --- | --- | --- | --- |
| **1** | `waiting_count > 0` | `wait.gif`（琥珀色の鐘） | `220ms` (4.5 fps) | `待ち{W}·稼働{R}`（稼働0なら `待ち{W}`） |
| **2** | `(error > 0 \|\| stalled > 0) && running == 0` | `trouble.gif`（深紅警告） | `280ms` (3.6 fps) | `異常{E}` または `停滞{S}` |
| **3** | `running >= 3` または サブエージェント並列中 | `run.gif`（烏羽疾走） | **`55ms` (18.2 fps 猛ダッシュ)** | `稼働{R}` |
| **4** | `running == 2` | `run.gif`（烏羽疾走） | **`85ms` (11.8 fps 疾走)** | `稼働2` |
| **5** | `running == 1` | `run.gif`（烏羽疾走） | **`130ms` (7.7 fps 通常走行)** | `稼働1` |
| **6** | `unseen_completed_count > 0` | `done.gif`（青緑の灯り） | `320ms` (3.1 fps) | `完了{D}` |
| **7** | すべて `0`（完全待機） | `idle.gif`（静かに佇む） | `600ms` (1.7 fps 省電力) | テキストなし（アイコンのみ表示） |

### 4.3 表示モード切替
パネル上部のボタンで以下3モードを即時切替可能とする。
1. **`バー:狩人+数`**（デフォルト）: アニメーションアイコン＋コンパクト件数表示
2. **`バー:狩人のみ`**: アイコンのみ表示（速度とポーズのみで状態判別）
3. **`バー:文字`**: アイコンなしのテキストのみ表示

---

## 5. ローカルAPI・イベントスキーマ仕様

### 5.1 エンドポイント一覧（`127.0.0.1:47823` 限定バインド）
- `POST /event` : エージェント Hook ／ 外部ラッパーからの状態イベント受信
- `GET /api/jobs` : 現在の `MonitorSnapshot` JSON の取得
- `POST /api/focus` : 指定ワークスペース（Agent画面）の前面化トリガー
- `POST /api/show` : サイドパネルの表示／非表示トグル
- `POST /api/board` : 3カラム詳細ボードの起動・前面化
- `GET /` : Chrome 用ライブWebダッシュボードHTML配信

### 5.2 イベント受信ペイロード（`POST /event`）
セキュリティ要件として、プロンプト本文・SQLクエリ文字列・ファイル中身・認証情報は一切送信しない（メタデータのみ）。

```json
{
  "session_id": "01934a10-7c00-7000-8000-000000000001",
  "source": "cortex-code",
  "title": "売上テーブル欠損率集計",
  "cwd": "~/workspace/analytics-project",
  "hook_event_name": "PermissionRequest",
  "tool_name": "write",
  "tool_use_id": "toolu_012WriteHtml9981",
  "agent_id": "",
  "agent_type": "",
  "model": "claude-sonnet-4-6"
}
```
※ `source` / `title` / `model` は省略可能（次期マルチエージェント拡張で利用）。

---

## 6. 次期拡張仕様：汎用マルチエージェント／LLMラッパー（Universal Agent Andon）

Cortex Code 以外の Coding Agent CLI や独自LLMスクリプトも同じモニター・同じメニューバーアイコンで一元監視するための拡張仕様。

### 6.1 アダプタ階層設計

#### Tier 1: マルチエージェント共通 Hook スクリプト（設定追加のみ）
Claude Code（`~/.claude/settings.json`）、Gemini CLI、Cursor 等の Hook 機構は stdin JSON 構造が共通しているため、既存の `notify.sh` を拡張してイベント名を正規化し `127.0.0.1:47823/event` へ転送する。
- `JobStore` 側の変更点: ローカル会話 `.json` ファイルが存在しない外部ツールの `session_id` でも、Hook イベント単体から仮想 `JobItem`（`source: "claude-code"` 等）を生成・保持し、`Stop` イベントで `unseen_done` に遷移させる。

#### Tier 2: 汎用 PTY（仮想端末）ラッパー CLI（`andon` コマンド）
Hooks 機能を持たない CLI エージェント（`codex`, `aider`, `goose`, `ollama`）や長時間バッチ（`dbt build`, `pytest`, 学習スクリプト）向けの軽量ラッパーバイナリ。

- **コマンド構文**:
  ```bash
  andon [--title "タスク名"] -- <任意のコマンドと引数...>
  # 例:
  andon -- codex "APIの型エラーを修正して"
  andon -- dbt build --select +fct_orders
  ```
- **内部動作仕様**:
  1. 親端末のサイズ（`TIOCGWINSZ`）を引き継いで疑似端末（PTY）を生成し、子プロセスをアタッチして入出力を完全透過中継する（色・TUI描画・キー入力を一切損なわない）。
  2. 起動時およびユーザーの Enter 入力時に `UserPromptSubmit` 相当（`Running`）を `127.0.0.1:47823/event` へ非同期送信する。
  3. **対話・承認待ち自動検知**: 子プロセスの stdout/stderr 出力が `1,500ms` 停止し、かつ直近出力の末尾行（ANSIエスケープ除去後）が対話プロンプト正規表現（`(?i)(\[y/n\]|\(y/n\)|allow|approve|confirm\?|password:|press enter|\?\s*$)`）に一致した場合、`PermissionRequest` 相当（`WaitingPermission`）を送信してチャイムと琥珀色アイコンを発火させる。ユーザーがキーを入力した瞬間に `Running` へ戻す。
  4. プロセス終了時: 終了コード `0` なら `Stop`（完了チャイム）、非 `0` なら `Error`（異常チャイム）を送信する。

#### Tier 3: Python / TypeScript SDK Callback（自作エージェント連携）
LangGraph や OpenAI / Anthropic SDK を直接呼ぶ自作スクリプトから 5行でアンドン連携できる軽量ヘルパー。

```python
# 利用イメージ（標準ライブラリ urllib のみ・外部依存ゼロ）
from andon_client import AndonSession

with AndonSession(title="日次特徴量パイプライン", model="gpt-5") as andon:
    andon.tool_start("snowflake_query")
    run_pipeline()
    # 例外発生時は自動で Error 送信、正常終了時は自動で Completed 送信
```

### 6.2 外部セッションのフォーカス動作仕様
`source` ごとにカードクリック時の前面化アプリを切り替える。
- `source == "cortex-code"` : `Cortex Code.app`（`agent-manager.code-workspace`）を前面化
- `source == "pty"` / ターミナル系CLI : 起動元ターミナルアプリ（`TERM_PROGRAM` から取得した `iTerm2` / `Terminal` / `Ghostty` / `WezTerm` / `Warp`）を前面化
- `source == "cursor"` : `Cursor.app` を前面化

---

## 7. 非機能要件・セキュリティ仕様

1. **完全ローカル完結**: 通信は `127.0.0.1` ループバックのみとし、外部ネットワーク通信を一切行わない。
2. **機密情報の非保持**: Hook 連携スクリプトおよび PTY ラッパーは、プロンプト本文・標準出力全文・環境変数・APIキーを破棄し、状態遷移イベント種別と所要時間メタデータのみをメモリ上で扱う。
3. **リソース上限**:
   - アイドル時 CPU 使用率: `0.2%` 未満（メニューバー `600ms` スリープ、FSEvents イベント駆動）
   - メモリ使用量（RSS）: `50MB` 前後（GIFフレーム `NSImage` は起動時1回のみデコードしてキャッシュ再利用）
4. **耐障害性**: モニターアプリが停止していても、エージェント側の Hook（`curl --max-time 0.3 ... || true`）は最大 `0.3秒` でタイムアウト無視され、本来のエージェント実行を一切阻害しない。
