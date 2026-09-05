impl App {
    pub(super) fn schedule_auto_poke_followup_if_needed(&mut self) -> bool {
        if !self.auto_poke_incomplete_todos
            || self.pending_queued_dispatch
            || self.pending_turn
            || self.has_queued_followups()
        {
            return false;
        }

        let todos = super::commands::poke_todos(self);
        let todo_session_id = self
            .remote_session_id
            .as_deref()
            .unwrap_or(&self.session.id)
            .to_string();
        if !todos.is_empty()
            && crate::todo::take_long_session_review_if_due(&todo_session_id).unwrap_or(false)
        {
            self.push_display_message(DisplayMessage::system(
                "🔍 Rechecking the plan and assessments after extended work...",
            ));
            self.queued_messages
                .push(crate::todo::TODO_LONG_SESSION_REVIEW_MESSAGE.to_string());
            self.pending_queued_dispatch = true;
            return true;
        }
        let incomplete: Vec<_> = todos
            .iter()
            .filter(|todo| super::commands::is_incomplete_poke_todo(todo))
            .cloned()
            .collect();
        if incomplete.is_empty() {
            // Completing or removing a todo list ends the prior poke cycle. If
            // equivalent work appears later, it is a new cycle and deserves
            // one fresh nudge rather than being mistaken for the old stall.
            self.last_auto_poke_fingerprint = None;
            if todos.is_empty() {
                // No todo list exists yet for this session. Auto-poke is armed
                // by default (`features.auto_poke`), so disarming here would
                // silently kill the feature for the whole session after the
                // very first todo-free turn: every later turn that *does*
                // leave incomplete todos would never be poked. Stay armed and
                // simply do nothing this turn.
                crate::logging::info("AUTO_POKE_DECISION action=idle reason=no_todos incomplete=0");
                self.todo_final_response_requested = false;
                return false;
            }
            // Deferred quality checks land here, once, instead of interrupting
            // every todo write during the turn. Every point recorded during the
            // turn is raised, including ones whose score later climbed: work
            // done while the score was low never benefited from the assessment
            // that arrived after it.
            if self.deliver_deferred_gate_digest_if_needed() {
                return true;
            }
            let goals = match crate::todo::load_goals(&todo_session_id) {
                Ok(goals) => goals,
                Err(error) => {
                    crate::logging::warn(&format!("Could not load goal assessments: {error}"));
                    self.push_display_message(DisplayMessage::system(format!(
                        "Could not read goal assessments: {error}. Automatic follow-up was not queued."
                    )));
                    return false;
                }
            };
            let ownership_needs_followup = !crate::todo::completed_groups_can_stop(&todos, &goals);
            let gate_budget_left =
                self.todo_completion_gate_attempts < Self::TODO_COMPLETION_GATE_MAX_ATTEMPTS;
            if ownership_needs_followup && gate_budget_left {
                self.todo_completion_gate_attempts =
                    self.todo_completion_gate_attempts.saturating_add(1);
                crate::telemetry::record_todo_gate(crate::telemetry::TodoGateKind::Ownership);
                self.push_display_message(DisplayMessage::system(
                    "🔍 Checking end-to-end ownership before finishing...",
                ));
                self.queued_messages
                    .push(crate::todo::build_todo_ownership_continuation_message(
                        &todos, &goals,
                    ));
                self.pending_queued_dispatch = true;
                return true;
            }
            let confidence_summary = super::commands::todo_confidence_summary(&todos);
            let confidence_label =
                super::commands::format_todo_completion_confidence(confidence_summary);
            let needs_spike_challenge = confidence_summary.confidence_spike_detected
                && !self.todo_confidence_spike_challenged;
            if (confidence_summary.completion_confidence_needs_validation || needs_spike_challenge)
                && gate_budget_left
            {
                self.todo_completion_gate_attempts =
                    self.todo_completion_gate_attempts.saturating_add(1);
                let notice = if confidence_summary.completion_confidence_needs_validation {
                    crate::telemetry::record_todo_gate(crate::telemetry::TodoGateKind::Completion);
                    "🔍 Double-checking confidence for you..."
                } else {
                    self.todo_confidence_spike_challenged = true;
                    crate::telemetry::record_todo_gate(
                        crate::telemetry::TodoGateKind::ConfidenceSpike,
                    );
                    "🔍 Double-checking confidence jumps..."
                };
                self.push_display_message(DisplayMessage::system(notice));
                // User-role content: reminder-only turns read as empty user
                // messages and models answer instead of re-validating.
                let summary = super::commands::build_todo_confidence_summary_message(&todos);
                self.queued_messages.push(summary);
                self.pending_queued_dispatch = true;
                return true;
            }
            if (ownership_needs_followup
                || confidence_summary.completion_confidence_needs_validation
                || needs_spike_challenge)
                && !gate_budget_left
            {
                // The gate keeps failing but the model is no longer making
                // progress on it. Nudging again would loop forever, burning an
                // API call per turn (observed live: an unattended session
                // resent the same continuation every ~5s). Stop the cycle and
                // surface the stall instead.
                crate::logging::warn(&format!(
                    "Todo completion gate exhausted after {} attempts; stopping auto-poke to avoid an infinite continuation loop",
                    self.todo_completion_gate_attempts
                ));
                self.push_display_message(DisplayMessage::system(
                    "⚠️ We nudged the agent several times but its validation still isn't holding up. We stopped poking; review the remaining todos yourself.",
                ));
                self.auto_poke_incomplete_todos = false;
                self.todo_confidence_spike_challenged = false;
                self.todo_completion_gate_attempts = 0;
                self.todo_gate_digest_delivered = false;
                self.pending_queued_dispatch = false;
                return false;
            }
            // Cycle finished cleanly. When auto-poke is the configured default
            // it stays armed so the next batch of work is covered too; only an
            // explicit /poke off (or a circuit breaker above) disarms it.
            self.auto_poke_incomplete_todos = self.auto_poke_default_on;
            // A finished cycle re-arms the review for whatever work comes next;
            // without this a session could only ever deliver one digest.
            self.todo_gate_digest_delivered = false;
            self.todo_completion_gate_attempts = 0;
            if !self.todo_final_response_requested {
                self.todo_final_response_requested = true;
                let acceptance_is_blocked =
                    !crate::todo::completed_groups_have_sufficient_delivery(&todos, &goals);
                let (notice, response) = if acceptance_is_blocked {
                    (
                        "Acceptance is blocked by a documented constraint. Stopping repeated checks."
                            .to_string(),
                        crate::todo::TODO_BLOCKED_RESPONSE_CONTINUATION_MESSAGE,
                    )
                } else {
                    (
                        format!(
                            "✅ All todos done. Completion confidence: {}.",
                            confidence_label
                        ),
                        crate::todo::TODO_FINAL_RESPONSE_CONTINUATION_MESSAGE,
                    )
                };
                self.push_display_message(DisplayMessage::system(notice));
                self.queued_messages.push(response.to_string());
                self.pending_queued_dispatch = true;
                return true;
            }
            self.pending_queued_dispatch = false;
            return false;
        }

        let poke_message = super::commands::build_poke_message(&incomplete);
        self.todo_final_response_requested = false;
        // Open work begins a new completion cycle. Keep the prior spike check
        // latched until this point so the synthetic final-response turn cannot
        // retrigger the same evidence gate against unchanged completed todos.
        self.todo_confidence_spike_challenged = false;
        let fingerprint =
            serde_json::to_string(&incomplete).unwrap_or_else(|_| poke_message.clone());
        if self.last_auto_poke_fingerprint.as_ref() == Some(&fingerprint) {
            crate::logging::info(&format!(
                "AUTO_POKE_DECISION action=idle reason=unchanged_todos incomplete={}",
                incomplete.len()
            ));
            return false;
        }

        self.push_display_message(DisplayMessage::system(format!(
            "👉 {} incomplete todo{}. We poked it for you. /poke off to stop.",
            incomplete.len(),
            if incomplete.len() == 1 { "" } else { "s" },
        )));
        // Auto-poke previously had no log trail, so a continuation that was
        // queued but never dispatched looked identical in the logs to a silent
        // model. Emit a decision line on every arm so the queue -> send handoff
        // can be correlated with "Sending queued continuation message".
        crate::logging::info(&format!(
            "AUTO_POKE_DECISION action=queue_continuation incomplete={} queued_before={} is_processing={} pending_turn={}",
            incomplete.len(),
            self.queued_messages.len(),
            self.is_processing,
            self.pending_turn,
        ));
        // Open todos mean the model is still iterating; completion-gate
        // exhaustion should only trip when the gate itself stops moving.
        self.todo_completion_gate_attempts = 0;
        self.last_auto_poke_fingerprint = Some(fingerprint);
        self.queued_messages.push(poke_message);
        self.pending_queued_dispatch = true;
        true
    }
}
