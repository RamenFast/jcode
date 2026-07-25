use super::*;

impl BackgroundTaskManager {
    /// Best-effort synchronous snapshot of currently running tasks.
    /// This avoids async calls in render paths.
    pub fn running_snapshot(&self) -> (usize, Vec<String>, Option<RunningBackgroundProgress>) {
        let Ok(tasks) = self.tasks.try_read() else {
            return (0, Vec::new(), None);
        };

        let mut rows: Vec<(String, RunningBackgroundProgress)> = Vec::new();
        for task in tasks.values() {
            let status = match std::fs::read_to_string(&task.status_path) {
                Ok(content) => match serde_json::from_str::<TaskStatusFile>(&content) {
                    Ok(status) => Some(status),
                    Err(_) => None,
                },
                Err(_) => None,
            };
            let progress = status.as_ref().and_then(|status| status.progress.clone());
            let label = status
                .as_ref()
                .and_then(|status| status.display_name.clone())
                .or_else(|| task.display_name.clone())
                .unwrap_or_else(|| task.tool_name.clone());

            rows.push((
                task.started_at_rfc3339.clone(),
                RunningBackgroundProgress {
                    task_id: task.task_id.clone(),
                    tool_name: task.tool_name.clone(),
                    label,
                    detail: progress.map(|progress| format_progress_display(&progress, 10)),
                },
            ));
        }

        // Newest first by start time (same wrap hazard as list(): the id's
        // timestamp prefix is truncated and wraps, so id order is not time
        // order). RFC3339 strings compare chronologically.
        rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.task_id.cmp(&a.1.task_id)));
        let rows: Vec<RunningBackgroundProgress> = rows.into_iter().map(|(_, row)| row).collect();
        let latest = rows.iter().find(|row| row.detail.is_some()).cloned();

        (
            tasks.len(),
            rows.iter().map(|row| row.label.clone()).collect(),
            latest,
        )
    }

    /// Best-effort synchronous lookup of detached tasks that are still running
    /// for a specific session.
    ///
    /// This is primarily used during self-dev reload recovery, where the new
    /// process needs to remind the agent that a previous `bash` command was
    /// persisted into the background instead of being interrupted.
    pub fn persisted_detached_running_tasks_for_session(
        &self,
        session_id: &str,
    ) -> Vec<TaskStatusFile> {
        let mut matches = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.output_dir) else {
            return matches;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }

            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(status) = serde_json::from_str::<TaskStatusFile>(&content) else {
                continue;
            };

            if status.session_id != session_id
                || status.status != BackgroundTaskStatus::Running
                || !status.detached
            {
                continue;
            }

            let Some(pid) = status.pid else {
                continue;
            };

            if crate::platform::is_process_running(pid) {
                matches.push(status);
            }
        }

        matches.sort_by(|a, b| a.task_id.cmp(&b.task_id));
        matches
    }
}
