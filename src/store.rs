use crate::macos::{AlertSound, is_coco_frontmost, play_alert_sound};
use crate::scanner::{ConversationScanner, ScanResult, now_epoch_ms};
use crate::types::{
    HookEvent, JobItem, JobStatus, MonitorSnapshot, RunPhase, STALL_THRESHOLD_SECS,
};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone)]
struct HookOverlay {
    status: JobStatus,
    phase: RunPhase,
    cwd: String,
    received_at_ms: u64,
    is_subagent: bool,
    is_prompt_submit: bool,
}

pub struct JobStore {
    scanner: ConversationScanner,
    last_scan: Option<ScanResult>,
    overlays: HashMap<String, HookOverlay>,
    cwd_subagent_ping_ms: HashMap<String, u64>,
    previous_statuses: HashMap<String, JobStatus>,
    unseen_done_at: HashMap<String, u64>,
    dismissed_stalled_activity: HashMap<String, u64>,
    viewing_since: HashMap<String, u64>,
    prev_coco_unread: HashSet<String>,
    bootstrapped: bool,
    pub snapshot: MonitorSnapshot,
    pub shared_snapshot: Arc<RwLock<MonitorSnapshot>>,
    pub notify_enabled: bool,
    pub sound_enabled: bool,
}

impl JobStore {
    pub fn new(shared_snapshot: Arc<RwLock<MonitorSnapshot>>) -> Self {
        let mut store = Self {
            scanner: ConversationScanner::new(ConversationScanner::default_base_dir()),
            last_scan: None,
            overlays: HashMap::new(),
            cwd_subagent_ping_ms: HashMap::new(),
            previous_statuses: HashMap::new(),
            unseen_done_at: HashMap::new(),
            dismissed_stalled_activity: HashMap::new(),
            viewing_since: HashMap::new(),
            prev_coco_unread: HashSet::new(),
            bootstrapped: false,
            snapshot: MonitorSnapshot::default(),
            shared_snapshot,
            notify_enabled: true,
            sound_enabled: true,
        };
        store.refresh(true);
        store
    }

    pub fn dismiss_done(&mut self, session_id: &str) {
        self.unseen_done_at.remove(session_id);
        self.viewing_since.remove(session_id);
        if let Some(job) = self.snapshot.jobs.iter().find(|j| j.session_id == session_id)
            && job.status == JobStatus::Stalled
        {
            self.dismissed_stalled_activity
                .insert(session_id.to_string(), job.last_activity_ms);
        }
        self.refresh(false);
    }

    pub fn dismiss_all_done(&mut self) {
        self.unseen_done_at.clear();
        self.viewing_since.clear();
        self.refresh(false);
    }

    pub fn apply_hook(&mut self, ev: HookEvent) {
        if ev.session_id.is_empty() {
            return;
        }
        let now = now_epoch_ms();
        let is_subagent = !ev.agent_id.is_empty() || !ev.agent_type.is_empty();

        match ev.hook_event_name.as_str() {
            // Opening an empty chat tab fires SessionStart before any prompt is sent; ignore it.
            "SessionStart" => return,
            "SubagentStop" => {
                self.overlays.remove(&ev.session_id);
                if !ev.cwd.is_empty() {
                    self.cwd_subagent_ping_ms.insert(ev.cwd, now);
                }
                self.refresh(false);
                return;
            }
            _ => {}
        }

        if is_subagent && !ev.cwd.is_empty() {
            self.cwd_subagent_ping_ms.insert(ev.cwd.clone(), now);
        }

        let overlay = match ev.hook_event_name.as_str() {
            "PermissionRequest" => Some(HookOverlay {
                status: JobStatus::WaitingPermission,
                phase: RunPhase::WaitingTool(if ev.tool_name.is_empty() {
                    "permission".to_string()
                } else {
                    ev.tool_name
                }),
                cwd: ev.cwd,
                received_at_ms: now,
                is_subagent,
                is_prompt_submit: false,
            }),
            "PreToolUse" => {
                if ev.tool_name == "ask_user_question" || ev.tool_name == "create_plan" {
                    Some(HookOverlay {
                        status: JobStatus::WaitingInput,
                        phase: RunPhase::WaitingTool(ev.tool_name),
                        cwd: ev.cwd,
                        received_at_ms: now,
                        is_subagent,
                        is_prompt_submit: false,
                    })
                } else if ev.tool_name == "task" {
                    Some(HookOverlay {
                        status: JobStatus::Running,
                        phase: RunPhase::SubagentsRunning(1),
                        cwd: ev.cwd,
                        received_at_ms: now,
                        is_subagent,
                        is_prompt_submit: false,
                    })
                } else {
                    Some(HookOverlay {
                        status: JobStatus::Running,
                        phase: RunPhase::ToolRunning(ev.tool_name),
                        cwd: ev.cwd,
                        received_at_ms: now,
                        is_subagent,
                        is_prompt_submit: false,
                    })
                }
            }
            "PostToolUse" | "UserPromptSubmit" => Some(HookOverlay {
                status: JobStatus::Running,
                phase: RunPhase::Thinking,
                cwd: ev.cwd,
                received_at_ms: now,
                is_subagent,
                is_prompt_submit: ev.hook_event_name == "UserPromptSubmit",
            }),
            "Stop" | "SessionEnd" => Some(HookOverlay {
                status: JobStatus::Completed,
                phase: RunPhase::Idle,
                cwd: ev.cwd,
                received_at_ms: now,
                is_subagent,
                is_prompt_submit: false,
            }),
            _ => None,
        };

        if let Some(ov) = overlay {
            self.overlays.insert(ev.session_id, ov);
            self.refresh(false);
        }
    }

    pub fn refresh(&mut self, rescan_files: bool) {
        let now = now_epoch_ms();

        if rescan_files || self.last_scan.is_none() {
            self.last_scan = Some(self.scanner.scan(now));
        }

        let Some(scan) = self.last_scan.as_ref() else {
            return;
        };

        let mut jobs = scan.jobs.clone();
        let credits = scan.credits;
        let non_primary_ids = &scan.non_primary_ids;
        let coco_active_sid = scan.coco_active_session_id.clone();
        let coco_unread_ids = scan.coco_unread_ids.clone();

        // Prune expired hook overlays
        self.overlays.retain(|sid, ov| {
            let age_ms = now.saturating_sub(ov.received_at_ms);
            if non_primary_ids.contains(sid) && ov.status == JobStatus::Completed {
                return false;
            }
            if ov.status == JobStatus::WaitingPermission {
                age_ms < 30 * 60 * 1000
            } else {
                age_ms < 5 * 60 * 1000
            }
        });

        // Any overlay from an unknown/subagent session in the same cwd acts as a live subagent heartbeat
        for (sid, ov) in &self.overlays {
            if (ov.is_subagent || non_primary_ids.contains(sid)) && !ov.cwd.is_empty() {
                let entry = self.cwd_subagent_ping_ms.entry(ov.cwd.clone()).or_insert(0);
                *entry = (*entry).max(ov.received_at_ms);
            }
        }

        let mut known_ids = HashSet::with_capacity(jobs.len());
        for job in &mut jobs {
            known_ids.insert(job.session_id.clone());

            // Update nested subagents from live hook overlays if matching
            for sub in &mut job.subagents {
                if let Some(sub_ov) = self.overlays.get(&sub.session_id) {
                    sub.last_activity_ms = sub.last_activity_ms.max(sub_ov.received_at_ms);
                    sub.active = sub_ov.status != JobStatus::Completed;
                    sub.current_step = sub_ov.phase.short_text();
                    job.last_activity_ms = job.last_activity_ms.max(sub_ov.received_at_ms);
                }
            }

            if let Some(ov) = self.overlays.get(&job.session_id) {
                let prompt_grace_ms = if ov.is_prompt_submit || ov.status == JobStatus::Running {
                    4_000
                } else {
                    0
                };
                if ov.received_at_ms.saturating_add(prompt_grace_ms) >= job.last_activity_ms
                    || ov.status == JobStatus::WaitingPermission
                {
                    if ov.status == JobStatus::WaitingPermission
                        && matches!(job.status, JobStatus::Completed | JobStatus::Error)
                        && job.last_activity_ms > ov.received_at_ms.saturating_add(1_500)
                    {
                        // Disk already recorded turn completion after permission prompt
                    } else {
                        job.status = ov.status;
                        job.phase = ov.phase.clone();
                        job.last_activity_ms = job.last_activity_ms.max(ov.received_at_ms);
                        job.via_hook = true;
                    }
                }
            }

            // Keep parent heartbeat alive when ephemeral subagents in same workspace are firing hooks
            if job.status == JobStatus::Running
                && now.saturating_sub(job.last_activity_ms) < STALL_THRESHOLD_SECS * 1000
                && !job.working_directory.is_empty()
                && let Some(&ping_ms) = self.cwd_subagent_ping_ms.get(&job.working_directory)
                && ping_ms >= job.started_at_ms
            {
                job.last_activity_ms = job.last_activity_ms.max(ping_ms);
            }
        }

        // Only allow temporary synthetic card for brand-new UserPromptSubmit within 6s before .json flushes
        for (sid, ov) in &self.overlays {
            let age_ms = now.saturating_sub(ov.received_at_ms);
            if !known_ids.contains(sid)
                && !non_primary_ids.contains(sid)
                && !ov.is_subagent
                && ov.is_prompt_submit
                && ov.status != JobStatus::Completed
                && age_ms <= 6_000
            {
                let ws_name = Path::new(&ov.cwd)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("coco")
                    .to_string();
                jobs.push(JobItem {
                    session_id: sid.clone(),
                    title: "プロンプト送信中...".to_string(),
                    workspace_id: "hook".to_string(),
                    workspace_name: ws_name,
                    working_directory: ov.cwd.clone(),
                    status: ov.status,
                    phase: ov.phase.clone(),
                    model: "auto".to_string(),
                    started_at_ms: ov.received_at_ms,
                    last_activity_ms: ov.received_at_ms,
                    elapsed_secs: age_ms / 1000,
                    heartbeat_secs: age_ms / 1000,
                    todo: None,
                    subagents: Vec::new(),
                    diff_files: 0,
                    diff_added: 0,
                    diff_removed: 0,
                    context_tokens: 0,
                    context_window: 0,
                    via_hook: true,
                    unseen_done: false,
                });
            }
        }

        // Unified timing & stall classification pass (identical whether rescan_files is true or false)
        for job in &mut jobs {
            job.heartbeat_secs = now.saturating_sub(job.last_activity_ms) / 1000;
            if matches!(job.status, JobStatus::Completed | JobStatus::Error) {
                job.elapsed_secs = job.last_activity_ms.saturating_sub(job.started_at_ms) / 1000;
            } else {
                job.elapsed_secs = now.saturating_sub(job.started_at_ms) / 1000;
                if job.status == JobStatus::Running && job.heartbeat_secs >= STALL_THRESHOLD_SECS {
                    job.status = JobStatus::Stalled;
                } else if job.status == JobStatus::Stalled
                    && job.heartbeat_secs < STALL_THRESHOLD_SECS
                {
                    job.status = JobStatus::Running;
                }

                if job.status == JobStatus::Stalled
                    && let Some(&dismissed_ms) = self.dismissed_stalled_activity.get(&job.session_id)
                {
                    if job.last_activity_ms <= dismissed_ms {
                        job.status = JobStatus::Completed;
                        job.phase = RunPhase::Idle;
                        job.elapsed_secs =
                            job.last_activity_ms.saturating_sub(job.started_at_ms) / 1000;
                    } else {
                        self.dismissed_stalled_activity.remove(&job.session_id);
                    }
                }
            }
        }

        self.update_unseen_done_and_alerts(
            &mut jobs,
            now,
            coco_active_sid.as_deref(),
            &coco_unread_ids,
        );
        self.prev_coco_unread = coco_unread_ids;
        self.bootstrapped = true;

        jobs.sort_by(|a, b| {
            a.status
                .cmp(&b.status)
                .then_with(|| b.unseen_done.cmp(&a.unseen_done))
                .then_with(|| b.last_activity_ms.cmp(&a.last_activity_ms))
        });

        let waiting_count = jobs.iter().filter(|j| j.status.is_attention()).count();
        let running_count = jobs.iter().filter(|j| j.status == JobStatus::Running).count();
        let stalled_count = jobs.iter().filter(|j| j.status == JobStatus::Stalled).count();
        let unseen_completed_count = jobs
            .iter()
            .filter(|j| matches!(j.status, JobStatus::Completed | JobStatus::Error) && j.unseen_done)
            .count();
        let completed_count = jobs
            .iter()
            .filter(|j| j.status == JobStatus::Completed)
            .count();

        let snap = MonitorSnapshot {
            updated_at_ms: now,
            waiting_count,
            running_count,
            stalled_count,
            unseen_completed_count,
            completed_count,
            daily_credits_used: credits.map(|(u, _)| u),
            daily_credits_limit: credits.map(|(_, l)| l),
            jobs,
        };
        self.snapshot = snap.clone();
        if let Ok(mut guard) = self.shared_snapshot.write() {
            *guard = snap;
        }
    }

    fn update_unseen_done_and_alerts(
        &mut self,
        jobs: &mut [JobItem],
        now: u64,
        coco_active_sid: Option<&str>,
        coco_unread_ids: &HashSet<String>,
    ) {
        let notify_on = self.notify_enabled;
        let sound_on = self.sound_enabled;
        let is_boot = !self.bootstrapped;
        let coco_front = is_coco_frontmost();

        for job in jobs.iter_mut() {
            let sid = &job.session_id;
            let prev = self.previous_statuses.insert(sid.clone(), job.status);
            let is_finished = matches!(job.status, JobStatus::Completed | JobStatus::Error);

            if is_boot {
                // On startup, only surface completed jobs that CoCo itself still marks unread (within 12h)
                if is_finished && coco_unread_ids.contains(sid) && job.heartbeat_secs < 12 * 3600 {
                    self.unseen_done_at.insert(sid.clone(), now);
                }
            } else if !is_finished {
                // Job started or received a follow-up prompt -> remove from unseen_done
                self.unseen_done_at.remove(sid);
                self.viewing_since.remove(sid);

                if let Some(prev_status) = prev {
                    if job.status.is_attention() && !prev_status.is_attention() {
                        fire_os_alert(
                            &format!("CoCo {} ({})", job.status.label(), job.workspace_name),
                            &job.title,
                            AlertSound::Attention,
                            notify_on,
                            sound_on,
                        );
                    }
                } else if job.status.is_attention() {
                    fire_os_alert(
                        &format!("CoCo 要対応 ({})", job.workspace_name),
                        &format!("「{}」が{}です", job.title, job.status.label()),
                        AlertSound::Attention,
                        notify_on,
                        sound_on,
                    );
                }
            } else {
                let newly_completed = match prev {
                    Some(prev_status) => {
                        prev_status != job.status
                            && (prev_status.is_active() || prev_status.is_attention())
                    }
                    // Brand-new short session that started and finished within one scan interval
                    None => job.heartbeat_secs <= 12,
                };

                if newly_completed {
                    self.unseen_done_at.insert(sid.clone(), now);
                    self.viewing_since.remove(sid);

                    if job.status == JobStatus::Completed {
                        fire_os_alert(
                            &format!("CoCo ジョブ完了 ({})", job.workspace_name),
                            &job.title,
                            AlertSound::Done,
                            notify_on,
                            sound_on,
                        );
                    } else if job.status == JobStatus::Error {
                        fire_os_alert(
                            &format!("CoCo エラー終了 ({})", job.workspace_name),
                            &job.title,
                            AlertSound::Error,
                            notify_on,
                            sound_on,
                        );
                    }
                }
            }

            // Check auto-clear rules for unseen finished jobs
            if let Some(&done_ts) = self.unseen_done_at.get(sid) {
                let mut clear_now = false;

                // 1) CoCo sidebar unread flag flipped from '0' -> '1'
                if self.prev_coco_unread.contains(sid) && !coco_unread_ids.contains(sid) {
                    clear_now = true;
                }

                // 2) User currently has this exact chat open in frontmost Cortex Code window for >= 4s
                if coco_front && coco_active_sid == Some(sid.as_str()) {
                    let since = *self.viewing_since.entry(sid.clone()).or_insert(now);
                    if now.saturating_sub(since) >= 4_000 {
                        clear_now = true;
                    }
                } else {
                    self.viewing_since.remove(sid);
                }

                // 3) Already marked read in CoCo and no follow-up for 20 minutes
                if !coco_unread_ids.contains(sid) && now.saturating_sub(done_ts) >= 20 * 60 * 1000 {
                    clear_now = true;
                }

                if clear_now {
                    self.unseen_done_at.remove(sid);
                    self.viewing_since.remove(sid);
                }
            }

            job.unseen_done = is_finished && self.unseen_done_at.contains_key(sid);
        }
    }
}

fn fire_os_alert(
    title: &str,
    body: &str,
    sound_kind: AlertSound,
    notify_on: bool,
    sound_on: bool,
) {
    if sound_on {
        play_alert_sound(sound_kind);
    }
    if notify_on {
        let title_owned = title.replace('"', "'").replace('\\', "");
        let body_owned = body.replace('"', "'").replace('\\', "");
        std::thread::spawn(move || {
            let script =
                format!("display notification \"{body_owned}\" with title \"{title_owned}\"");
            let _ = Command::new("/usr/bin/osascript")
                .arg("-e")
                .arg(script)
                .output();
        });
    }
}

pub fn focus_coco_workspace(_working_directory: &str) {
    std::thread::spawn(move || {
        let home = std::env::var("HOME").unwrap_or_default();
        let agent_ws = format!(
            "{home}/Library/Application Support/Cortex Code/User/agent-manager.code-workspace"
        );
        if !home.is_empty() && Path::new(&agent_ws).exists() {
            let _ = Command::new("/usr/bin/open")
                .arg("-a")
                .arg("Cortex Code")
                .arg(&agent_ws)
                .status();
        } else {
            let _ = Command::new("/usr/bin/open")
                .arg("-a")
                .arg("Cortex Code")
                .status();
        }
    });
}

pub fn open_chrome_dashboard(port: u16) {
    let url = format!("http://127.0.0.1:{port}/");
    std::thread::spawn(move || {
        let _ = Command::new("/usr/bin/open")
            .arg("-a")
            .arg("Google Chrome")
            .arg(&url)
            .status();
    });
}
