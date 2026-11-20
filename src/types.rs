use serde::{Deserialize, Serialize};

pub const LOOPBACK_PORT: u16 = 47823;
pub const STALL_THRESHOLD_SECS: u64 = 180;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    WaitingPermission,
    WaitingInput,
    Running,
    Stalled,
    Error,
    Completed,
}

impl JobStatus {
    pub fn is_attention(self) -> bool {
        matches!(self, Self::WaitingPermission | Self::WaitingInput)
    }

    pub fn is_active(self) -> bool {
        matches!(self, Self::Running | Self::Stalled)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::WaitingPermission => "許可待ち",
            Self::WaitingInput => "回答・確認待ち",
            Self::Running => "稼働中",
            Self::Stalled => "無反応・スタック疑い",
            Self::Error => "エラー終了",
            Self::Completed => "完了",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum RunPhase {
    Thinking,
    ToolRunning(String),
    SubagentsRunning(usize),
    WaitingTool(String),
    Idle,
}

impl RunPhase {
    pub fn short_text(&self) -> String {
        match self {
            Self::Thinking => "思考・応答生成中".to_string(),
            Self::ToolRunning(tool) => format!("ツール実行中: {}", friendly_tool_name(tool)),
            Self::SubagentsRunning(n) => format!("サブエージェント並列稼働中 ({n}体)"),
            Self::WaitingTool(tool) => format!("ユーザー操作待ち: {}", friendly_tool_name(tool)),
            Self::Idle => "待機中".to_string(),
        }
    }
}

pub fn friendly_tool_name(raw: &str) -> String {
    if raw.is_empty() {
        return "tool".to_string();
    }
    if let Some(rest) = raw.strip_prefix("mcp_") {
        return format!("MCP:{rest}");
    }
    match raw {
        "ask_user_question" => "質問への回答待ち".to_string(),
        "create_plan" => "プラン承認待ち".to_string(),
        "task" => "サブエージェント(task)".to_string(),
        "bash" => "ターミナル(bash)".to_string(),
        "sql_execute" | "snowflake_sql_execute" => "Snowflake SQL実行".to_string(),
        "web_fetch" | "web_search" => format!("Web取得({raw})"),
        other => other.to_string(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoProgress {
    pub completed: usize,
    pub total: usize,
    pub current_task: Option<String>,
}

impl TodoProgress {
    pub fn ratio(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.completed as f32 / self.total as f32).clamp(0.0, 1.0)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentSummary {
    pub session_id: String,
    pub title: String,
    pub last_activity_ms: u64,
    pub active: bool,
    pub current_step: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobItem {
    pub session_id: String,
    pub title: String,
    pub workspace_id: String,
    pub workspace_name: String,
    pub working_directory: String,
    pub status: JobStatus,
    pub phase: RunPhase,
    pub model: String,
    pub started_at_ms: u64,
    pub last_activity_ms: u64,
    pub elapsed_secs: u64,
    pub heartbeat_secs: u64,
    pub todo: Option<TodoProgress>,
    pub subagents: Vec<SubagentSummary>,
    pub diff_files: u32,
    pub diff_added: u32,
    pub diff_removed: u32,
    pub context_tokens: u64,
    pub context_window: u64,
    pub via_hook: bool,
    pub unseen_done: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MonitorSnapshot {
    pub updated_at_ms: u64,
    pub waiting_count: usize,
    pub running_count: usize,
    pub stalled_count: usize,
    pub unseen_completed_count: usize,
    pub completed_count: usize,
    pub daily_credits_used: Option<f64>,
    pub daily_credits_limit: Option<f64>,
    pub jobs: Vec<JobItem>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HookEvent {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub hook_event_name: String,
    #[serde(default)]
    pub tool_name: String,
    #[serde(default)]
    pub tool_use_id: String,
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub agent_type: String,
    #[serde(default)]
    pub notification_type: String,
}

#[derive(Debug, Clone)]
pub enum BackendEvent {
    Hook(HookEvent),
    FsChanged,
    FocusWorkspace(String),
    ToggleMainPanel,
    OpenBoardWindow,
}

pub fn format_duration_ja(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}秒")
    } else if secs < 3600 {
        let m = secs / 60;
        let s = secs % 60;
        if m < 10 && s > 0 {
            format!("{m}分{s:02}秒")
        } else {
            format!("{m}分")
        }
    } else {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        format!("{h}時間{m:02}分")
    }
}
