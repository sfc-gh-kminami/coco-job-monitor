use crate::scanner::ConversationScanner;
use crate::types::{BackendEvent, HookEvent, LOOPBACK_PORT, MonitorSnapshot};
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use smol::channel::Sender;
use smol::io::{AsyncReadExt, AsyncWriteExt};
use smol::net::TcpListener;
use std::sync::{Arc, RwLock};
use std::time::Duration;

pub fn start_fs_watcher(tx: Sender<BackendEvent>) {
    std::thread::spawn(move || {
        let watch_dir = ConversationScanner::default_base_dir();
        if !watch_dir.exists() {
            return;
        }
        let tx_cb = tx.clone();
        let mut watcher = match RecommendedWatcher::new(
            move |res: Result<notify::Event, notify::Error>| {
                if let Ok(ev) = res {
                    let relevant = ev.paths.iter().any(|p| {
                        p.file_name()
                            .and_then(|s| s.to_str())
                            .map(|n| n.ends_with(".json") || n.ends_with(".jsonl"))
                            .unwrap_or(false)
                    });
                    if relevant {
                        let _ = tx_cb.try_send(BackendEvent::FsChanged);
                    }
                }
            },
            Config::default(),
        ) {
            Ok(w) => w,
            Err(_) => return,
        };

        if watcher.watch(&watch_dir, RecursiveMode::Recursive).is_err() {
            return;
        }
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    });
}

pub async fn run_loopback_server(
    tx: Sender<BackendEvent>,
    shared_snapshot: Arc<RwLock<MonitorSnapshot>>,
) -> anyhow::Result<()> {
    let addr = format!("127.0.0.1:{LOOPBACK_PORT}");
    let listener = TcpListener::bind(&addr).await?;

    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            continue;
        };
        let tx_conn = tx.clone();
        let snap_conn = shared_snapshot.clone();

        smol::spawn(async move {
            let mut buf = vec![0u8; 65536];
            let Ok(mut n) = stream.read(&mut buf).await else {
                return;
            };
            if n == 0 {
                return;
            }

            // Read remaining body bytes if Content-Length exceeds first packet
            if let Some(header_end) = find_subslice(&buf[..n], b"\r\n\r\n") {
                let body_start = header_end + 4;
                let headers_str = String::from_utf8_lossy(&buf[..header_end]);
                let content_len = headers_str
                    .lines()
                    .find_map(|l| {
                        let lower = l.to_ascii_lowercase();
                        lower
                            .strip_prefix("content-length:")
                            .and_then(|v| v.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                while n < body_start + content_len && n < buf.len() {
                    match stream.read(&mut buf[n..]).await {
                        Ok(0) | Err(_) => break,
                        Ok(more) => n += more,
                    }
                }
            }

            let req = &buf[..n];
            let Some(header_end) = find_subslice(req, b"\r\n\r\n") else {
                return;
            };
            let head = String::from_utf8_lossy(&req[..header_end]);
            let body = &req[header_end + 4..];
            let first_line = head.lines().next().unwrap_or("");
            let mut parts = first_line.split_whitespace();
            let method = parts.next().unwrap_or("GET");
            let path = parts.next().unwrap_or("/");

            match (method, path) {
                ("POST", "/event") => {
                    if let Ok(ev) = serde_json::from_slice::<HookEvent>(body) {
                        let _ = tx_conn.try_send(BackendEvent::Hook(ev));
                    }
                    let _ = write_http(&mut stream, 200, "application/json", b"{\"ok\":true}").await;
                }
                ("POST", "/api/focus") => {
                    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(body)
                        && let Some(cwd) = v.get("cwd").and_then(|s| s.as_str())
                    {
                        let _ = tx_conn.try_send(BackendEvent::FocusWorkspace(cwd.to_string()));
                    }
                    let _ = write_http(&mut stream, 200, "application/json", b"{\"ok\":true}").await;
                }
                ("POST", "/api/show") => {
                    let _ = tx_conn.try_send(BackendEvent::ToggleMainPanel);
                    let _ = write_http(&mut stream, 200, "application/json", b"{\"ok\":true}").await;
                }
                ("POST", "/api/board") => {
                    let _ = tx_conn.try_send(BackendEvent::OpenBoardWindow);
                    let _ = write_http(&mut stream, 200, "application/json", b"{\"ok\":true}").await;
                }
                ("GET", p) if p.starts_with("/api/jobs") => {
                    let json_bytes = snap_conn
                        .read()
                        .ok()
                        .and_then(|g| serde_json::to_vec(&*g).ok())
                        .unwrap_or_else(|| b"{}".to_vec());
                    let _ = write_http(&mut stream, 200, "application/json; charset=utf-8", &json_bytes).await;
                }
                _ => {
                    let _ = write_http(
                        &mut stream,
                        200,
                        "text/html; charset=utf-8",
                        WEB_DASHBOARD_HTML.as_bytes(),
                    )
                    .await;
                }
            }
        })
        .detach();
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

async fn write_http(
    stream: &mut smol::net::TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let reason = if status == 200 { "OK" } else { "Bad Request" };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await
}

const WEB_DASHBOARD_HTML: &str = r#"<!DOCTYPE html>
<html lang="ja">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>CoCo Job Monitor — Wide Board</title>
<style>
  :root {
    --bg: #0b1120;
    --panel: #111827;
    --card: #172033;
    --border: #26334d;
    --text: #f1f5f9;
    --muted: #94a3b8;
    --cyan: #29b5e8;
    --amber: #f59e0b;
    --coral: #ef4444;
    --green: #10b981;
  }
  * { box-sizing: border-box; }
  body {
    margin: 0; padding: 20px 24px;
    font-family: -apple-system, BlinkMacSystemFont, "Hiragino Sans", sans-serif;
    background: var(--bg); color: var(--text);
  }
  header {
    display: flex; align-items: center; justify-content: space-between;
    padding-bottom: 16px; border-bottom: 1px solid var(--border); margin-bottom: 20px;
  }
  .brand { display: flex; align-items: center; gap: 12px; }
  .badge-pill {
    padding: 4px 10px; border-radius: 999px; font-size: 12px; font-weight: 700;
    background: rgba(41,181,232,0.16); color: var(--cyan); border: 1px solid rgba(41,181,232,0.4);
  }
  .kpis { display: flex; gap: 12px; flex-wrap: wrap; }
  .kpi {
    padding: 8px 14px; border-radius: 10px; background: var(--panel); border: 1px solid var(--border);
    font-size: 13px; display: flex; align-items: center; gap: 8px;
  }
  .kpi strong { font-size: 18px; }
  .grid {
    display: grid; grid-template-columns: repeat(3, minmax(300px, 1fr)); gap: 18px;
  }
  @media (max-width: 1024px) { .grid { grid-template-columns: 1fr; } }
  .lane {
    background: var(--panel); border: 1px solid var(--border); border-radius: 14px; padding: 14px;
    min-height: 72vh;
  }
  .lane h2 { margin: 0 0 12px; font-size: 14px; color: var(--muted); letter-spacing: 0.03em; }
  .card {
    background: var(--card); border: 1px solid var(--border); border-radius: 11px;
    padding: 12px 14px; margin-bottom: 10px; cursor: pointer; transition: transform 0.12s, border-color 0.12s;
  }
  .card:hover { transform: translateY(-1px); border-color: var(--cyan); }
  .card.waiting { border-left: 4px solid var(--amber); background: #221c14; }
  .card.running { border-left: 4px solid var(--cyan); }
  .card.stalled { border-left: 4px solid var(--coral); }
  .card.completed { border-left: 4px solid var(--green); opacity: 0.88; }
  .row { display: flex; justify-content: space-between; align-items: center; gap: 8px; }
  .ws { font-size: 11px; padding: 2px 7px; border-radius: 6px; background: #0f172a; color: #93c5fd; }
  .hb { font-size: 11px; color: #38bdf8; font-weight: 600; }
  .title { font-size: 14px; font-weight: 700; margin: 7px 0 5px; line-height: 1.35; }
  .phase { font-size: 12px; color: #cbd5e1; margin-bottom: 6px; }
  .bar { height: 6px; background: #0f172a; border-radius: 99px; overflow: hidden; margin: 6px 0 4px; }
  .bar > div { height: 100%; background: linear-gradient(90deg, #29b5e8, #10b981); }
  .meta { font-size: 11px; color: var(--muted); display: flex; gap: 10px; flex-wrap: wrap; margin-top: 6px; }
  .sub { margin-top: 7px; padding-top: 6px; border-top: 1px dashed #293548; font-size: 11px; color: #93c5fd; }
</style>
</head>
<body>
<header>
  <div class="brand">
    <span class="badge-pill">COCO JOB MONITOR</span>
    <strong style="font-size:18px">全ワークスペース横断 Job 管理ボード</strong>
  </div>
  <div class="kpis" id="kpis"></div>
</header>
<div class="grid">
  <section class="lane"><h2>要対応（承認・回答待ち / スタック疑い）</h2><div id="lane-wait"></div></section>
  <section class="lane"><h2>稼働中（思考・ツール・サブエージェント）</h2><div id="lane-run"></div></section>
  <section class="lane"><h2>直近完了・エラー履歴</h2><div id="lane-done"></div></section>
</div>
<script>
function dur(s){ if(s<60)return s+"秒"; if(s<3600)return Math.floor(s/60)+"分"+(s<600?(s%60)+"秒":""); return Math.floor(s/3600)+"時間"+Math.floor((s%3600)/60)+"分"; }
function esc(s){ const d=document.createElement("div"); d.textContent=s||""; return d.innerHTML; }
async function focusWs(cwd){ await fetch("/api/focus",{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({cwd})}); }
function cardHtml(j){
  const cls = (j.status==="waiting_permission"||j.status==="waiting_input")?"waiting":(j.status==="running"?"running":(j.status==="stalled"?"stalled":"completed"));
  const phase = j.phase?.kind==="thinking"?"思考・応答生成中":(j.phase?.kind==="tool_running"?"実行中: "+j.phase.detail:(j.phase?.kind==="subagents_running"?"サブエージェント稼働中 ("+j.phase.detail+"体)":(j.phase?.kind==="waiting_tool"?"ユーザー操作待ち: "+j.phase.detail:"待機 / 完了")));
  let todo="";
  if(j.todo && j.todo.total>0){
    const pct=Math.round((j.todo.completed/j.todo.total)*100);
    todo=`<div class="bar"><div style="width:${pct}%"></div></div><div style="font-size:11px;color:#94a3b8">Todo ${j.todo.completed}/${j.todo.total}${j.todo.current_task?" · "+esc(j.todo.current_task):""}</div>`;
  }
  let subs="";
  if(j.subagents && j.subagents.length>0){
    subs=`<div class="sub">`+j.subagents.slice(0,4).map(s=>`${s.active?"●":"✓"} ${esc(s.title)} (${esc(s.current_step)})`).join("<br>")+`</div>`;
  }
  const ctx = j.context_window>0 ? `<span>Ctx ${Math.round(j.context_tokens/1000)}k/${Math.round(j.context_window/1000)}k</span>` : "";
  const diff = j.diff_files>0 ? `<span>+${j.diff_added} -${j.diff_removed} (${j.diff_files}f)</span>` : "";
  return `<div class="card ${cls}" data-cwd="${esc(j.working_directory)}">
    <div class="row"><span class="ws">${esc(j.workspace_name)}</span><span class="hb">最終動作 ${dur(j.heartbeat_secs)}前 · 経過 ${dur(j.elapsed_secs)}</span></div>
    <div class="title">${esc(j.title)}</div>
    <div class="phase">${esc(phase)}</div>
    ${todo}${subs}
    <div class="meta"><span>${esc(j.model)}</span>${ctx}${diff}</div>
  </div>`;
}
async function poll(){
  try{
    const r = await fetch("/api/jobs"); const d = await r.json();
    const cr = d.daily_credits_used!=null ? `<div class="kpi">本日クレジット <strong>${d.daily_credits_used.toFixed(1)}</strong> / ${d.daily_credits_limit}</div>` : "";
    document.getElementById("kpis").innerHTML =
      `<div class="kpi" style="border-color:#f59e0b">要対応 <strong style="color:#f59e0b">${d.waiting_count}</strong></div>`+
      `<div class="kpi" style="border-color:#29b5e8">稼働中 <strong style="color:#29b5e8">${d.running_count}</strong></div>`+
      `<div class="kpi">スタック疑い <strong style="color:#ef4444">${d.stalled_count}</strong></div>`+
      `<div class="kpi">新着完了(未確認) <strong style="color:#10b981">${d.unseen_completed_count}</strong></div>`+cr;
    const wait=[], run=[], done=[];
    for(const j of (d.jobs||[])){
      if(j.status==="waiting_permission"||j.status==="waiting_input"||j.status==="stalled") wait.push(cardHtml(j));
      else if(j.status==="running") run.push(cardHtml(j));
      else if(j.unseen_done) done.push(cardHtml(j));
    }
    document.getElementById("lane-wait").innerHTML = wait.join("") || `<div style="color:#64748b;font-size:13px">現在対応待ちのジョブはありません</div>`;
    document.getElementById("lane-run").innerHTML = run.join("") || `<div style="color:#64748b;font-size:13px">稼働中のジョブはありません</div>`;
    document.getElementById("lane-done").innerHTML = done.join("") || `<div style="color:#64748b;font-size:13px">未確認の完了ジョブはありません</div>`;
    document.querySelectorAll(".card").forEach(el=>el.onclick=()=>focusWs(el.dataset.cwd));
  }catch(_){}
}
poll(); setInterval(poll, 1000);
</script>
</body>
</html>
"#;
