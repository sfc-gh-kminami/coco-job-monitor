use crate::types::{
    JobItem, JobStatus, RunPhase, STALL_THRESHOLD_SECS, SubagentSummary, TodoProgress,
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_SESSION_AGE_SECS: u64 = 36 * 3600;
const TAIL_BYTES: u64 = 196 * 1024;

#[derive(Debug, Clone)]
struct RawSession {
    session_id: String,
    workspace_id: String,
    title: String,
    working_directory: String,
    session_type: String,
    is_empty: bool,
    outcome: Option<String>,
    model: String,
    creation_date_ms: u64,
    last_user_msg_ms: u64,
    last_msg_ms: u64,
    file_activity_ms: u64,
    diff_files: u32,
    diff_added: u32,
    diff_removed: u32,
    context_tokens: u64,
    context_window: u64,
    daily_credits_used: Option<f64>,
    daily_credits_limit: Option<f64>,
    subagent_ids: Vec<String>,
    tail_info: HistoryTailInfo,
    todo: Option<TodoProgress>,
}

#[derive(Debug, Clone, Default)]
pub struct HistoryTailInfo {
    pub pending_tool: Option<String>,
    pub last_role: Option<String>,
}

#[derive(Debug, Clone)]
struct CacheEntry {
    json_mtime_ms: u64,
    hist_mtime_ms: u64,
    hist_len: u64,
    raw: RawSession,
}

pub struct ScanResult {
    pub jobs: Vec<JobItem>,
    pub credits: Option<(f64, f64)>,
    pub non_primary_ids: HashSet<String>,
    pub coco_active_session_id: Option<String>,
    pub coco_unread_ids: HashSet<String>,
}

pub struct ConversationScanner {
    base_dir: PathBuf,
    cache: HashMap<PathBuf, CacheEntry>,
    vscdb_mtime_key: (u64, u64),
    cached_active_sid: Option<String>,
    cached_unread_sids: HashSet<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SessionMetaJson {
    title: Option<String>,
    session_id: Option<String>,
    working_directory: Option<String>,
    #[serde(rename = "sessionType")]
    session_type: Option<String>,
    #[serde(rename = "isEmpty")]
    is_empty: bool,
    #[serde(rename = "creationDate")]
    creation_date: u64,
    #[serde(rename = "lastMessageDate")]
    last_message_date: u64,
    #[serde(rename = "lastUserMessageDate")]
    last_user_message_date: u64,
    #[serde(rename = "lastTurn")]
    last_turn: Option<LastTurnJson>,
    #[serde(rename = "inputState")]
    input_state: Option<InputStateJson>,
    #[serde(rename = "responseMetadata")]
    response_metadata: Option<ResponseMetaJson>,
    stats: Option<StatsJson>,
    #[serde(rename = "subagentRefs")]
    subagent_refs: Vec<SubagentRefJson>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LastTurnJson {
    outcome: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct InputStateJson {
    #[serde(rename = "selectedCortexModelId")]
    selected_model: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ResponseMetaJson {
    orchestration_model: Option<String>,
    created_on: u64,
    usage: Option<UsageJson>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct UsageJson {
    limits: Vec<LimitJson>,
    tokens_consumed: Vec<TokenConsumedJson>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LimitJson {
    limit: f64,
    name: String,
    usage: f64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TokenConsumedJson {
    context_window: u64,
    model_role: Option<String>,
    input_tokens: Option<TokenCountJson>,
    output_tokens: Option<TokenCountJson>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TokenCountJson {
    total: u64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct StatsJson {
    diff: Option<DiffStatsJson>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct DiffStatsJson {
    #[serde(rename = "fileCount")]
    file_count: u32,
    added: u32,
    removed: u32,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SubagentRefJson {
    #[serde(rename = "subagentSessionId")]
    subagent_session_id: String,
}

impl ConversationScanner {
    pub fn new(base_dir: PathBuf) -> Self {
        Self {
            base_dir,
            cache: HashMap::new(),
            vscdb_mtime_key: (0, 0),
            cached_active_sid: None,
            cached_unread_sids: HashSet::new(),
        }
    }

    pub fn default_base_dir() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".snowflake/cortex/conversations")
    }

    pub fn scan(&mut self, now_ms: u64) -> ScanResult {
        let cutoff_ms = now_ms.saturating_sub(MAX_SESSION_AGE_SECS * 1000);
        let json_files = self.collect_recent_json_files(cutoff_ms);

        let mut seen_paths = HashSet::with_capacity(json_files.len());
        let mut raw_sessions = Vec::with_capacity(json_files.len());

        for (workspace_id, json_path, json_mtime_ms) in json_files {
            seen_paths.insert(json_path.clone());
            let hist_path = history_path_for(&json_path);
            let (hist_mtime_ms, hist_len) = file_mtime_and_len(&hist_path);

            if let Some(cached) = self.cache.get(&json_path)
                && cached.json_mtime_ms == json_mtime_ms
                && cached.hist_mtime_ms == hist_mtime_ms
                && cached.hist_len == hist_len
            {
                raw_sessions.push(cached.raw.clone());
                continue;
            }

            if let Some(raw) =
                parse_session_files(&workspace_id, &json_path, json_mtime_ms, &hist_path, hist_mtime_ms)
            {
                self.cache.insert(
                    json_path,
                    CacheEntry {
                        json_mtime_ms,
                        hist_mtime_ms,
                        hist_len,
                        raw: raw.clone(),
                    },
                );
                raw_sessions.push(raw);
            }
        }

        self.cache.retain(|k, _| seen_paths.contains(k));
        self.refresh_coco_vscdb_state();
        let (jobs, credits, non_primary_ids) = build_job_items(raw_sessions, now_ms);
        ScanResult {
            jobs,
            credits,
            non_primary_ids,
            coco_active_session_id: self.cached_active_sid.clone(),
            coco_unread_ids: self.cached_unread_sids.clone(),
        }
    }

    fn refresh_coco_vscdb_state(&mut self) {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let db_path = PathBuf::from(&home)
            .join("Library/Application Support/Cortex Code/User/globalStorage/state.vscdb");
        if !db_path.exists() {
            return;
        }
        let wal_path = PathBuf::from(format!("{}-wal", db_path.to_string_lossy()));
        let (db_m, _) = file_mtime_and_len(&db_path);
        let (wal_m, _) = file_mtime_and_len(&wal_path);
        if self.vscdb_mtime_key == (db_m, wal_m) && db_m > 0 {
            return;
        }
        self.vscdb_mtime_key = (db_m, wal_m);

        let uri = format!("file:{}?mode=ro", db_path.to_string_lossy());
        let sql = "SELECT 'A', hex(value) FROM ItemTable WHERE key='agentManager.activeConversationV2' UNION ALL SELECT 'U', hex(key) FROM ItemTable WHERE key GLOB 'agentManager.isRead.*' AND value='0';";
        let Ok(out) = Command::new("/usr/bin/sqlite3")
            .arg("-batch")
            .arg("-noheader")
            .arg(&uri)
            .arg(sql)
            .output()
        else {
            return;
        };
        if !out.status.success() {
            return;
        }

        let mut active_sid = None;
        let mut unread = HashSet::new();
        let stdout = String::from_utf8_lossy(&out.stdout);
        for line in stdout.lines() {
            let Some((tag, hex_str)) = line.split_once('|') else {
                continue;
            };
            let Some(bytes) = decode_hex(hex_str.trim()) else {
                continue;
            };
            if tag == "A" {
                if let Some(sid) = extract_sid_from_v2_bytes(&bytes) {
                    active_sid = Some(sid);
                }
            } else if tag == "U" {
                if let Some(sid) = extract_sid_from_isread_key(&bytes) {
                    unread.insert(sid);
                }
            }
        }
        self.cached_active_sid = active_sid;
        self.cached_unread_sids = unread;
    }

    fn collect_recent_json_files(&self, cutoff_ms: u64) -> Vec<(String, PathBuf, u64)> {
        let mut out = Vec::new();
        let Ok(entries) = fs::read_dir(&self.base_dir) else {
            return out;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_file() {
                if is_session_json(&path) {
                    let (mtime_ms, _) = file_mtime_and_len(&path);
                    if mtime_ms >= cutoff_ms {
                        out.push(("root".to_string(), path, mtime_ms));
                    }
                }
            } else if ft.is_dir() {
                let ws_id = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("workspace")
                    .to_string();
                let Ok(sub_entries) = fs::read_dir(&path) else {
                    continue;
                };
                for sub in sub_entries.flatten() {
                    let sp = sub.path();
                    if is_session_json(&sp) {
                        let (mtime_ms, _) = file_mtime_and_len(&sp);
                        if mtime_ms >= cutoff_ms {
                            out.push((ws_id.clone(), sp, mtime_ms));
                        }
                    }
                }
            }
        }
        out
    }
}

fn is_session_json(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    name.ends_with(".json") && !name.ends_with(".history.jsonl")
}

fn history_path_for(json_path: &Path) -> PathBuf {
    let s = json_path.to_string_lossy();
    let base = s.strip_suffix(".json").unwrap_or(&s);
    PathBuf::from(format!("{base}.history.jsonl"))
}

fn file_mtime_and_len(path: &Path) -> (u64, u64) {
    let Ok(meta) = fs::metadata(path) else {
        return (0, 0);
    };
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    (mtime_ms, meta.len())
}

pub fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn parse_session_files(
    workspace_id: &str,
    json_path: &Path,
    json_mtime_ms: u64,
    hist_path: &Path,
    hist_mtime_ms: u64,
) -> Option<RawSession> {
    let bytes = fs::read(json_path).ok()?;
    let meta: SessionMetaJson = serde_json::from_slice(&bytes).ok()?;

    let fallback_id = json_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
    let session_id = meta.session_id.filter(|s| !s.is_empty()).unwrap_or(fallback_id);
    let title = meta
        .title
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| format!("Session {}", &session_id[..session_id.len().min(8)]));

    let working_directory = meta.working_directory.unwrap_or_default();
    let session_type = meta.session_type.unwrap_or_default();
    let outcome = meta.last_turn.and_then(|lt| lt.outcome);

    let resp_model = meta
        .response_metadata
        .as_ref()
        .and_then(|rm| rm.orchestration_model.clone());
    let input_model = meta
        .input_state
        .as_ref()
        .and_then(|ist| ist.selected_model.clone());
    let model = resp_model
        .or(input_model)
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| "auto".to_string());

    let resp_created_ms = meta
        .response_metadata
        .as_ref()
        .map(|rm| rm.created_on)
        .unwrap_or(0);

    let mut context_tokens = 0;
    let mut context_window = 0;
    let mut daily_credits_used = None;
    let mut daily_credits_limit = None;

    if let Some(usage) = meta.response_metadata.and_then(|rm| rm.usage) {
        for lim in usage.limits {
            if lim.name.contains("daily_estimated_credits") {
                daily_credits_used = Some(lim.usage);
                daily_credits_limit = Some(lim.limit);
            }
        }
        for tc in usage.tokens_consumed {
            let is_orch = tc
                .model_role
                .as_deref()
                .map(|r| r == "orchestration")
                .unwrap_or(tc.context_window >= 500_000);
            if is_orch || context_window == 0 {
                let inp = tc.input_tokens.map(|t| t.total).unwrap_or(0);
                let out = tc.output_tokens.map(|t| t.total).unwrap_or(0);
                context_tokens = inp + out;
                context_window = tc.context_window;
            }
        }
    }

    let (diff_files, diff_added, diff_removed) = meta
        .stats
        .and_then(|s| s.diff)
        .map(|d| (d.file_count, d.added, d.removed))
        .unwrap_or((0, 0, 0));

    let subagent_ids = meta
        .subagent_refs
        .into_iter()
        .map(|r| r.subagent_session_id)
        .filter(|s| !s.is_empty())
        .collect();

    let (tail_info, todo) = if hist_mtime_ms > 0 {
        parse_history_jsonl(hist_path)
    } else {
        (HistoryTailInfo::default(), None)
    };

    let file_activity_ms = json_mtime_ms.max(hist_mtime_ms).max(resp_created_ms);

    Some(RawSession {
        session_id,
        workspace_id: workspace_id.to_string(),
        title,
        working_directory,
        session_type,
        is_empty: meta.is_empty,
        outcome,
        model,
        creation_date_ms: meta.creation_date,
        last_user_msg_ms: meta.last_user_message_date,
        last_msg_ms: meta.last_message_date,
        file_activity_ms,
        diff_files,
        diff_added,
        diff_removed,
        context_tokens,
        context_window,
        daily_credits_used,
        daily_credits_limit,
        subagent_ids,
        tail_info,
        todo,
    })
}

pub fn parse_history_jsonl(path: &Path) -> (HistoryTailInfo, Option<TodoProgress>) {
    let Ok(mut file) = File::open(path) else {
        return (HistoryTailInfo::default(), None);
    };
    let Ok(meta) = file.metadata() else {
        return (HistoryTailInfo::default(), None);
    };
    let len = meta.len();
    if len == 0 {
        return (HistoryTailInfo::default(), None);
    }

    // Read tail chunk for recent tool_use / tool_result state
    let offset = len.saturating_sub(TAIL_BYTES);
    let _ = file.seek(SeekFrom::Start(offset));
    let mut tail_buf = Vec::with_capacity((len - offset) as usize);
    if file.read_to_end(&mut tail_buf).is_err() {
        return (HistoryTailInfo::default(), None);
    }
    let tail_str = String::from_utf8_lossy(&tail_buf);
    let tail_slice = if offset > 0 {
        match tail_str.find('\n') {
            Some(idx) => &tail_str[idx + 1..],
            None => "",
        }
    } else {
        &tail_str[..]
    };

    let tail_info = analyze_tail_lines(tail_slice);
    let mut todo = extract_latest_todo(tail_slice);

    // If todo wasn't in the last 196KB and file is larger, scan full file cheaply via substring filter
    if todo.is_none() && offset > 0 && len <= 16 * 1024 * 1024 {
        if let Ok(full) = fs::read_to_string(path) {
            todo = extract_latest_todo(&full);
        }
    }

    (tail_info, todo)
}

pub fn analyze_tail_lines(jsonl_slice: &str) -> HistoryTailInfo {
    let mut completed_tool_ids = HashSet::new();
    let mut parsed_events: Vec<(String, Vec<(String, String, bool)>)> = Vec::new();

    for line in jsonl_slice.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(val) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let role = val
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let mut items = Vec::new();
        if let Some(arr) = val.get("content").and_then(|v| v.as_array()) {
            for item in arr {
                let Some(t) = item.get("type").and_then(|v| v.as_str()) else {
                    continue;
                };
                if t == "tool_result" {
                    let inner = item.get("tool_result").unwrap_or(item);
                    if let Some(tid) = inner.get("tool_use_id").and_then(|v| v.as_str()) {
                        completed_tool_ids.insert(tid.to_string());
                        items.push(("tool_result".to_string(), tid.to_string(), true));
                    }
                } else if t == "tool_use" {
                    let inner = item.get("tool_use").unwrap_or(item);
                    let tid = inner
                        .get("tool_use_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = inner
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("tool")
                        .to_string();
                    items.push((name, tid, false));
                }
            }
        }
        parsed_events.push((role, items));
        if parsed_events.len() >= 14 {
            break;
        }
    }

    let last_role = parsed_events.first().map(|(r, _)| r.clone());
    let mut pending_tool = None;

    for (role, items) in &parsed_events {
        if role == "assistant" {
            for (name, tid, is_result) in items.iter().rev() {
                if !*is_result && !tid.is_empty() && !completed_tool_ids.contains(tid) {
                    pending_tool = Some(name.clone());
                    break;
                }
            }
            if pending_tool.is_some() {
                break;
            }
        }
    }

    HistoryTailInfo {
        pending_tool,
        last_role,
    }
}

pub fn extract_latest_todo(jsonl_text: &str) -> Option<TodoProgress> {
    for line in jsonl_text.lines().rev() {
        if !line.contains("system_todo_write") {
            continue;
        }
        let Ok(val) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(arr) = val.get("content").and_then(|v| v.as_array()) else {
            continue;
        };
        for item in arr.iter().rev() {
            if item.get("type").and_then(|v| v.as_str()) != Some("tool_use") {
                continue;
            }
            let inner = item.get("tool_use").unwrap_or(item);
            if inner.get("name").and_then(|v| v.as_str()) != Some("system_todo_write") {
                continue;
            }
            if let Some(todos) = inner
                .get("input")
                .and_then(|inp| inp.get("todos"))
                .and_then(|t| t.as_array())
            {
                let total = todos.len();
                if total == 0 {
                    continue;
                }
                let mut completed = 0;
                let mut current_task = None;
                for td in todos {
                    let st = td.get("status").and_then(|v| v.as_str()).unwrap_or("");
                    let content = td
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim();
                    if st == "completed" {
                        completed += 1;
                    } else if st == "in_progress" && current_task.is_none() && !content.is_empty() {
                        current_task = Some(content.to_string());
                    }
                }
                if current_task.is_none() {
                    for td in todos {
                        let st = td.get("status").and_then(|v| v.as_str()).unwrap_or("");
                        let content = td
                            .get("content")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .trim();
                        if st == "pending" && !content.is_empty() {
                            current_task = Some(content.to_string());
                            break;
                        }
                    }
                }
                return Some(TodoProgress {
                    completed,
                    total,
                    current_task,
                });
            }
        }
    }
    None
}

fn build_job_items(
    raw_sessions: Vec<RawSession>,
    now_ms: u64,
) -> (Vec<JobItem>, Option<(f64, f64)>, HashSet<String>) {
    let mut subagents_by_id: HashMap<String, RawSession> = HashMap::new();
    let mut non_primary_ids: HashSet<String> = HashSet::new();
    let mut primary_sessions: Vec<RawSession> = Vec::new();
    let mut latest_credits: Option<(u64, f64, f64)> = None;

    for raw in raw_sessions {
        if let (Some(used), Some(limit)) = (raw.daily_credits_used, raw.daily_credits_limit) {
            if latest_credits
                .map(|(ts, _, _)| raw.file_activity_ms >= ts)
                .unwrap_or(true)
            {
                latest_credits = Some((raw.file_activity_ms, used, limit));
            }
        }

        if raw.session_type == "subagent" {
            non_primary_ids.insert(raw.session_id.clone());
            subagents_by_id.insert(raw.session_id.clone(), raw);
        } else if raw.is_empty || raw.title.starts_with("Chat for session: ") {
            non_primary_ids.insert(raw.session_id.clone());
        } else {
            primary_sessions.push(raw);
        }
    }

    let mut assigned_subagents = HashSet::new();
    let mut jobs = Vec::with_capacity(primary_sessions.len());

    // First pass: collect explicit subagentRefs and match active workspace subagents
    for raw in &primary_sessions {
        for sid in &raw.subagent_ids {
            assigned_subagents.insert(sid.clone());
        }
    }

    for raw in primary_sessions {
        let is_running_turn = raw.outcome.as_deref() == Some("running");
        let started_at_ms = if raw.last_user_msg_ms > 0 {
            raw.last_user_msg_ms
        } else if raw.last_msg_ms > 0 {
            raw.last_msg_ms
        } else {
            raw.creation_date_ms.max(raw.file_activity_ms)
        };

        let mut subs = Vec::new();
        for sid in &raw.subagent_ids {
            if let Some(sub) = subagents_by_id.get(sid) {
                subs.push(to_subagent_summary(sub, now_ms, is_running_turn));
            }
        }

        // Also attach currently active unassigned subagents in the same workspace started after parent turn
        if is_running_turn {
            for (sid, sub) in &subagents_by_id {
                if assigned_subagents.contains(sid) {
                    continue;
                }
                let same_ws = sub.workspace_id == raw.workspace_id
                    && (!raw.working_directory.is_empty()
                        && sub.working_directory == raw.working_directory);
                let recent_sub = now_ms.saturating_sub(sub.file_activity_ms) < 600_000
                    && sub.creation_date_ms.saturating_add(5_000) >= started_at_ms;
                if same_ws && recent_sub {
                    subs.push(to_subagent_summary(sub, now_ms, true));
                }
            }
        }

        subs.sort_by(|a, b| b.last_activity_ms.cmp(&a.last_activity_ms));
        let active_sub_count = subs.iter().filter(|s| s.active).count();
        let max_sub_activity = subs.iter().map(|s| s.last_activity_ms).max().unwrap_or(0);
        let last_activity_ms = raw.file_activity_ms.max(max_sub_activity);

        let heartbeat_secs = now_ms.saturating_sub(last_activity_ms) / 1000;

        let (status, phase) = classify_from_logs(
            raw.outcome.as_deref(),
            &raw.tail_info,
            active_sub_count,
            heartbeat_secs,
        );

        let elapsed_secs = if matches!(status, JobStatus::Completed | JobStatus::Error) {
            last_activity_ms.saturating_sub(started_at_ms) / 1000
        } else {
            now_ms.saturating_sub(started_at_ms) / 1000
        };

        // Skip old sessions without a recognized turn outcome unless recently active
        if raw.outcome.is_none() && heartbeat_secs > 1800 {
            continue;
        }

        let workspace_name = Path::new(&raw.working_directory)
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("global")
            .to_string();

        jobs.push(JobItem {
            session_id: raw.session_id,
            title: raw.title,
            workspace_id: raw.workspace_id,
            workspace_name,
            working_directory: raw.working_directory,
            status,
            phase,
            model: raw.model,
            started_at_ms,
            last_activity_ms,
            elapsed_secs,
            heartbeat_secs,
            todo: raw.todo,
            subagents: subs,
            diff_files: raw.diff_files,
            diff_added: raw.diff_added,
            diff_removed: raw.diff_removed,
            context_tokens: raw.context_tokens,
            context_window: raw.context_window,
            via_hook: false,
            unseen_done: false,
        });
    }

    let credits = latest_credits.map(|(_, u, l)| (u, l));
    (jobs, credits, non_primary_ids)
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for i in (0..bytes.len()).step_by(2) {
        let hi = (bytes[i] as char).to_digit(16)?;
        let lo = (bytes[i + 1] as char).to_digit(16)?;
        out.push(((hi << 4) | lo) as u8);
    }
    Some(out)
}

fn extract_sid_from_v2_bytes(bytes: &[u8]) -> Option<String> {
    if let Some(pos) = bytes.iter().position(|&b| b == 0) {
        let s = std::str::from_utf8(&bytes[pos + 1..]).ok()?.trim();
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    None
}

fn extract_sid_from_isread_key(bytes: &[u8]) -> Option<String> {
    let prefix = b"agentManager.isRead.";
    if !bytes.starts_with(prefix) {
        return None;
    }
    let rest = &bytes[prefix.len()..];
    if let Some(pos) = rest.iter().position(|&b| b == 0) {
        let s = std::str::from_utf8(&rest[pos + 1..]).ok()?.trim();
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    None
}

fn to_subagent_summary(sub: &RawSession, now_ms: u64, parent_running: bool) -> SubagentSummary {
    let age_secs = now_ms.saturating_sub(sub.file_activity_ms) / 1000;
    let active = parent_running
        && sub.outcome.as_deref() != Some("success")
        && sub.outcome.as_deref() != Some("error")
        && (age_secs < 120 || sub.tail_info.pending_tool.is_some());
    let current_step = if let Some(ref tool) = sub.tail_info.pending_tool {
        format!("実行中: {tool}")
    } else if active {
        "調査・思考中".to_string()
    } else {
        "完了".to_string()
    };
    SubagentSummary {
        session_id: sub.session_id.clone(),
        title: sub.title.clone(),
        last_activity_ms: sub.file_activity_ms,
        active,
        current_step,
    }
}

fn classify_from_logs(
    outcome: Option<&str>,
    tail: &HistoryTailInfo,
    active_sub_count: usize,
    heartbeat_secs: u64,
) -> (JobStatus, RunPhase) {
    match outcome {
        Some("running") => {
            if let Some(ref tool) = tail.pending_tool {
                if tool == "ask_user_question" || tool == "create_plan" {
                    return (
                        JobStatus::WaitingInput,
                        RunPhase::WaitingTool(tool.clone()),
                    );
                }
                if tool == "task" || active_sub_count > 0 {
                    let count = active_sub_count.max(1);
                    let st = if heartbeat_secs >= STALL_THRESHOLD_SECS {
                        JobStatus::Stalled
                    } else {
                        JobStatus::Running
                    };
                    return (st, RunPhase::SubagentsRunning(count));
                }
                let st = if heartbeat_secs >= STALL_THRESHOLD_SECS {
                    JobStatus::Stalled
                } else {
                    JobStatus::Running
                };
                return (st, RunPhase::ToolRunning(tool.clone()));
            }

            if active_sub_count > 0 {
                return (
                    JobStatus::Running,
                    RunPhase::SubagentsRunning(active_sub_count),
                );
            }

            let st = if heartbeat_secs >= STALL_THRESHOLD_SECS {
                JobStatus::Stalled
            } else {
                JobStatus::Running
            };
            (st, RunPhase::Thinking)
        }
        Some("error") => (JobStatus::Error, RunPhase::Idle),
        _ => {
            if tail.last_role.as_deref() == Some("user") && heartbeat_secs < 45 {
                (JobStatus::Running, RunPhase::Thinking)
            } else {
                (JobStatus::Completed, RunPhase::Idle)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_pending_ask_user_question_and_completed_tools() {
        let sample = r#"{"role":"assistant","content":[{"type":"tool_use","tool_use":{"tool_use_id":"t1","name":"bash","input":{}}}]}
{"role":"user","content":[{"type":"tool_result","tool_result":{"tool_use_id":"t1"}}]}
{"role":"assistant","content":[{"type":"tool_use","tool_use":{"tool_use_id":"t2","name":"ask_user_question","input":{}}}]}"#;

        let info = analyze_tail_lines(sample);
        assert_eq!(info.pending_tool.as_deref(), Some("ask_user_question"));
        let (st, _) = classify_from_logs(Some("running"), &info, 0, 10);
        assert_eq!(st, JobStatus::WaitingInput);
    }

    #[test]
    fn extracts_latest_todo_progress() {
        let sample = r#"{"role":"assistant","content":[{"type":"tool_use","tool_use":{"tool_use_id":"td1","name":"system_todo_write","input":{"todos":[{"content":"Step 1","status":"completed"},{"content":"Step 2 active","status":"in_progress"},{"content":"Step 3","status":"pending"}]}}}]}"#;
        let todo = extract_latest_todo(sample).expect("todo extracted");
        assert_eq!(todo.completed, 1);
        assert_eq!(todo.total, 3);
        assert_eq!(todo.current_task.as_deref(), Some("Step 2 active"));
    }

    #[test]
    fn classifies_stalled_after_threshold() {
        let info = HistoryTailInfo {
            pending_tool: None,
            last_role: Some("user".to_string()),
        };
        let (st, phase) = classify_from_logs(Some("running"), &info, 0, STALL_THRESHOLD_SECS + 5);
        assert_eq!(st, JobStatus::Stalled);
        assert_eq!(phase, RunPhase::Thinking);
    }
}
