use super::{Tool, ToolContext, ToolOutput};
use crate::bus::{Bus, BusEvent, TodoEvent};
use crate::todo::{
    LOW_HILL_CLIMBABILITY, LOW_INTENT_UNDERSTANDING, TODO_HILL_CLIMBABILITY_CONTINUATION_MESSAGE,
    TODO_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE, TODO_OWNERSHIP_CONTINUATION_MESSAGE,
    TODO_WRITE_REJECTED_NOTICE, TodoGoal, TodoGoalChange, TodoGoalField, TodoItem, TodoPlan,
    TodoPlanChange, TodoPlanField, load_goals, load_plan, load_todos,
    newly_completed_groups_have_sufficient_ownership, save_goals, save_plan, save_todos,
};
use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

pub struct TodoTool;

impl TodoTool {
    pub fn new() -> Self {
        Self
    }
}

/// Fold each incoming todo's confidence into its tool-maintained history.
///
/// The model reports `confidence` while working and `completion_confidence` at
/// completion. Each todo-tool write contributes at most one observation so a
/// single completion update cannot manufacture an apparent intermediate step.
/// The append-only trail lets downstream consumers distinguish an
/// evidence-driven rise (75 -> 85 -> 95 -> 100) from a bulk end-of-task stamp
/// (75 -> 100). Model-supplied `confidence_history` is ignored: the tool owns
/// this field.
fn merge_confidence_history(previous: &[TodoItem], incoming: &mut [TodoItem]) {
    let prior: HashMap<&str, &TodoItem> = previous
        .iter()
        .map(|todo| (todo.id.as_str(), todo))
        .collect();
    for todo in incoming.iter_mut() {
        let previous_todo = prior.get(todo.id.as_str()).copied();
        let mut history = previous_todo
            .map(|prev| prev.confidence_history.clone())
            .unwrap_or_default();
        if history.is_empty()
            && let Some(value) = previous_todo.and_then(|prev| {
                if prev.status == "completed" {
                    prev.completion_confidence.or(prev.confidence)
                } else {
                    prev.confidence
                }
            })
        {
            history.push(value);
        }
        let observation = if todo.status == "completed" {
            todo.completion_confidence.or(todo.confidence)
        } else {
            todo.confidence
        };
        if let Some(value) = observation
            && history.last() != Some(&value)
        {
            history.push(value);
        }
        todo.confidence_history = history;
    }
}

#[derive(Deserialize)]
struct TodoInput {
    todos: Option<Vec<TodoItem>>,
    goals: Option<Vec<TodoGoal>>,
    plan: Option<TodoPlan>,
}

/// Normalize a goal's group label: trimmed, with empty/whitespace collapsed
/// to `None` (the implicit goal of an ungrouped list).
fn goal_group_key(group: Option<&str>) -> Option<String> {
    group
        .map(str::trim)
        .filter(|group| !group.is_empty())
        .map(str::to_string)
}

/// Merge incoming goal assessments with the stored ones.
///
/// Incoming goals win per group key; stored goals for groups the write does
/// not mention are retained (a todo update should not silently discard goal
/// assessments).
fn merge_goals(stored: &[TodoGoal], incoming: Option<Vec<TodoGoal>>) -> Vec<TodoGoal> {
    let Some(incoming) = incoming else {
        return stored.to_vec();
    };
    let mut merged: Vec<TodoGoal> = Vec::new();
    for mut goal in incoming {
        goal.group = goal_group_key(goal.group.as_deref());
        if let Some(slot) = merged
            .iter_mut()
            .find(|existing| existing.group == goal.group)
        {
            *slot = goal;
        } else {
            merged.push(goal);
        }
    }
    for prev in stored {
        let key = goal_group_key(prev.group.as_deref());
        if !merged.iter().any(|goal| goal.group == key) {
            merged.push(prev.clone());
        }
    }
    merged
}

fn changed_goal_fields(before: Option<&TodoGoal>, after: Option<&TodoGoal>) -> Vec<TodoGoalField> {
    let mut fields = Vec::new();
    if before.and_then(|goal| goal.hill_climbability)
        != after.and_then(|goal| goal.hill_climbability)
    {
        fields.push(TodoGoalField::HillClimbability);
    }
    if before.and_then(|goal| goal.feedback_loop.as_ref())
        != after.and_then(|goal| goal.feedback_loop.as_ref())
    {
        fields.push(TodoGoalField::FeedbackLoop);
    }
    if before.and_then(|goal| goal.end_to_end_ownership)
        != after.and_then(|goal| goal.end_to_end_ownership)
    {
        fields.push(TodoGoalField::EndToEndOwnership);
    }
    fields
}

/// Merge the incoming plan-level intent assessment with the stored one.
///
/// User intention describes why the user asked for the work and should remain
/// stable while the agent revises its steps or scores, so an omitted intention
/// inherits the stored value. Sending an empty string clears it.
fn merge_plan(stored: &TodoPlan, incoming: Option<TodoPlan>) -> TodoPlan {
    let Some(mut plan) = incoming else {
        return stored.clone();
    };
    if plan.user_intention.is_none() {
        plan.user_intention = stored.user_intention.clone();
    }
    if plan.understands_user_intent.is_none() {
        plan.understands_user_intent = stored.understands_user_intent;
    }
    plan
}

fn plan_change(before: &TodoPlan, after: &TodoPlan) -> Option<TodoPlanChange> {
    let mut fields = Vec::new();
    if before.user_intention != after.user_intention {
        fields.push(TodoPlanField::UserIntention);
    }
    if before.understands_user_intent != after.understands_user_intent {
        fields.push(TodoPlanField::UnderstandsUserIntent);
    }
    (!fields.is_empty()).then(|| TodoPlanChange {
        before: Some(before.clone()),
        after: Some(after.clone()),
        fields,
    })
}

fn goal_changes(before: &[TodoGoal], after: &[TodoGoal]) -> Vec<TodoGoalChange> {
    let mut changes = Vec::new();
    for current in after {
        let key = goal_group_key(current.group.as_deref());
        let previous = before
            .iter()
            .find(|goal| goal_group_key(goal.group.as_deref()) == key);
        let fields = changed_goal_fields(previous, Some(current));
        if !fields.is_empty() {
            changes.push(TodoGoalChange {
                before: previous.cloned(),
                after: Some(current.clone()),
                fields,
            });
        }
    }
    for previous in before {
        let key = goal_group_key(previous.group.as_deref());
        if after
            .iter()
            .any(|goal| goal_group_key(goal.group.as_deref()) == key)
        {
            continue;
        }
        let fields = changed_goal_fields(Some(previous), None);
        if !fields.is_empty() {
            changes.push(TodoGoalChange {
                before: Some(previous.clone()),
                after: None,
                fields,
            });
        }
    }
    changes
}

/// Reframe nudges for goals whose hill-climbability is too low to support a
/// trustworthy feedback loop, plus the plan-level intent check.
///
/// A low score means there is no credible metric to iterate against, so the
/// work must be reframed into something measurable. The nudge is intentionally
/// returned on every applicable todo write until it clears or the work closes.
fn take_reframe_nudges(plan: &TodoPlan, goals: &[TodoGoal], todos: &[TodoItem]) -> Vec<String> {
    let mut nudges = Vec::new();
    let any_open = todos
        .iter()
        .any(|todo| todo.status != "completed" && todo.status != "cancelled");
    if any_open
        && plan
            .understands_user_intent
            .is_none_or(|score| score < LOW_INTENT_UNDERSTANDING)
    {
        nudges.push(TODO_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE.to_string());
    }
    for goal in goals {
        let group_open = todos.iter().any(|todo| {
            goal_group_key(todo.group.as_deref()) == goal.group
                && todo.status != "completed"
                && todo.status != "cancelled"
        });
        if !group_open {
            continue;
        }
        if goal
            .hill_climbability
            .is_none_or(|score| score < LOW_HILL_CLIMBABILITY)
        {
            nudges.push(TODO_HILL_CLIMBABILITY_CONTINUATION_MESSAGE.to_string());
        }
    }
    nudges
}

fn build_todo_output(
    todos: Vec<TodoItem>,
    plan: TodoPlan,
    goals: Vec<TodoGoal>,
    plan_change: Option<TodoPlanChange>,
    goal_changes: Option<Vec<TodoGoalChange>>,
    continuations: impl IntoIterator<Item = String>,
) -> Result<ToolOutput> {
    let remaining = todos
        .iter()
        .filter(|todo| todo.status != "completed")
        .count();
    let mut text = serde_json::to_string_pretty(&todos)?;
    if plan != TodoPlan::default() {
        text.push_str("\n\nPlan:\n");
        text.push_str(&serde_json::to_string_pretty(&plan)?);
    }
    if !goals.is_empty() {
        text.push_str("\n\nGoals:\n");
        text.push_str(&serde_json::to_string_pretty(&goals)?);
    }
    if let Some(plan_change) = plan_change.as_ref() {
        text.push_str("\n\nPlan updates:\n");
        text.push_str(&serde_json::to_string_pretty(plan_change)?);
    }
    if let Some(goal_changes) = goal_changes.as_ref().filter(|changes| !changes.is_empty()) {
        text.push_str("\n\nGoal updates:\n");
        text.push_str(&serde_json::to_string_pretty(goal_changes)?);
    }
    for continuation in continuations {
        text.push_str("\n\n");
        text.push_str(&continuation);
    }
    let mut metadata = json!({"todos": todos, "plan": plan, "goals": goals});
    if let Some(plan_change) = plan_change {
        metadata["plan_update"] = serde_json::to_value(plan_change)?;
    }
    if let Some(goal_changes) = goal_changes.filter(|changes| !changes.is_empty()) {
        metadata["goal_updates"] = serde_json::to_value(goal_changes)?;
    }
    Ok(ToolOutput::new(text)
        .with_title(format!("{} todos", remaining))
        .with_metadata(metadata))
}

/// Leniently normalize raw todo-tool arguments before strict deserialization.
///
/// Some providers (notably Claude tool calling) intermittently emit tool
/// arguments as JSON *strings* instead of native types: the whole `todos`
/// array as one stringified JSON blob, individual items as stringified
/// objects, or numeric fields like `confidence` as `"90"`. Strict
/// `serde_json::from_value` rejects these with `invalid type: string ...`,
/// failing the entire call (issue #357; same provider quirk as #106).
fn normalize_todo_input(mut input: Value) -> Value {
    let Some(obj) = input.as_object_mut() else {
        return input;
    };
    if let Some(plan) = obj.get_mut("plan") {
        if let Value::String(raw) = plan {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                *plan = Value::Null;
            } else if let Ok(parsed @ (Value::Object(_) | Value::Null)) =
                serde_json::from_str::<Value>(trimmed)
            {
                *plan = parsed;
            }
        }
        if let Some(fields) = plan.as_object_mut() {
            for key in [
                "alignment_score",
                "user_intention_alignment",
                "understands_user_intent",
            ] {
                if let Some(value) = fields.get_mut(key) {
                    coerce_value_to_integer(value);
                }
            }
        }
    }
    for key in ["todos", "goals"] {
        let Some(entries) = obj.get_mut(key) else {
            continue;
        };

        // Whole array sent as a stringified JSON blob.
        if let Value::String(raw) = entries {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                *entries = Value::Null;
            } else if let Ok(parsed @ (Value::Array(_) | Value::Null)) =
                serde_json::from_str::<Value>(trimmed)
            {
                *entries = parsed;
            }
        }

        if let Value::Array(items) = entries {
            for item in items.iter_mut() {
                // Individual item sent as a stringified JSON object.
                if let Value::String(raw) = item
                    && let Ok(parsed @ Value::Object(_)) = serde_json::from_str::<Value>(raw.trim())
                {
                    *item = parsed;
                }
                let Some(fields) = item.as_object_mut() else {
                    continue;
                };
                for key in [
                    "confidence",
                    "completion_confidence",
                    "alignment_score",
                    "user_intention_alignment",
                    "hill_climbability",
                    "end_to_end_ownership",
                ] {
                    if let Some(value) = fields.get_mut(key) {
                        coerce_value_to_integer(value);
                    }
                }
            }
        }
    }
    input
}

/// Coerce a numeric string (`"90"`) or whole float (`90.0`) to a JSON integer,
/// and an empty string to `null`. Leaves anything else untouched so strict
/// deserialization can report a precise error.
fn coerce_value_to_integer(value: &mut Value) {
    match value {
        Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                *value = Value::Null;
            } else if let Ok(parsed) = trimmed.parse::<u64>() {
                *value = Value::from(parsed);
            }
        }
        Value::Number(num) => {
            if num.as_u64().is_none()
                && let Some(float) = num.as_f64()
                && float.fract() == 0.0
                && (0.0..=u64::MAX as f64).contains(&float)
            {
                *value = Value::from(float as u64);
            }
        }
        _ => {}
    }
}

#[async_trait]
impl Tool for TodoTool {
    fn name(&self) -> &str {
        "todo"
    }

    fn description(&self) -> &str {
        // SECURITY/EVAL: This is model-visible calibration text. Keep it
        // deliberately handwritten. Never generate it from gate constants or
        // interpolate private thresholds, because that would teach the model
        // how to target the evaluator instead of reporting an honest assessment.
        "Read or update structured todo items and optional goal-level assessments."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "intent": super::intent_schema_property(),
                "todos": {
                    "type": "array",
                    "description": "Todo list to save.",
                    "items": {
                        "type": "object",
                        "required": ["content", "status", "priority", "id", "confidence"],
                        "properties": {
                            "content": {
                                "type": "string",
                                "description": "Task."
                            },
                            "status": {
                                "type": "string",
                                "description": "Status."
                            },
                            "priority": {
                                "type": "string",
                                "description": "Priority."
                            },
                            "id": {
                                "type": "string",
                                "description": "ID."
                            },
                            "group": {
                                "type": "string",
                                "description": "Optional group label. Todos sharing a group render together under one header. Use one group per coherent goal (e.g. 'optimize rendering'). When the user steers into new work, start a new group instead of renaming the existing one. Omit for an ungrouped flat list."
                            },
                            "confidence": {
                                "type": "integer",
                                "minimum": 0,
                                "maximum": 100,
                                "description": "Self-assessed confidence, 0-100, that this todo can be completed correctly. Reassess it as evidence accumulates while working."
                            },
                            "completion_confidence": {
                                "type": "integer",
                                "minimum": 0,
                                "maximum": 100,
                                "description": "Self-assessed confidence, 0-100, that this todo was completed correctly. Use only for completed items."
                            }
                        }
                    }
                },
                "plan": {
                    "type": "object",
                    "description": "Plan-level understanding of the user's request, covering the whole todo list. Send it on the first write and whenever your understanding changes.",
                    "required": ["user_intention", "understands_user_intent"],
                    "properties": {
                        "user_intention": {
                            "type": "string",
                            "description": "Concise statement of what the user actually wants: their underlying reason and desired end state for this work. Omit on later updates to retain the stored intention."
                        },
                        "understands_user_intent": {
                            "type": "integer",
                            "minimum": 0,
                            "maximum": 100,
                            "description": "Self-assessment, 0-100, of how well you understand what the user actually wants and how faithfully this plan represents it: their underlying goal, what they left implicit, and what outcome would make them consider this done. Before scoring, form a requirement inventory covering outcomes, deliverables, constraints, prohibited actions, integration paths, edge cases, and necessary follow-through, and check that the plan and its feedback loops name an explicit observation or check for each item. A generic instruction to run tests, verify, or review does not establish coverage: tests count only for behaviors they actually enforce, while non-testable requirements such as edit scope, dependency limits, required reporting, branches or commits, and prohibited modifications need separate explicit checks. Score low when interpretations of the request still materially diverge, you are guessing at intent, or any material item is unrepresented. Prefer resolving low understanding by re-reading the request and investigating the conversation and codebase over asking the user, since asking blocks them."
                        }
                    }
                },
                "goals": {
                    "type": "array",
                    "description": "Optional goal-level assessments, one per todo group. Use group: null for an ungrouped list. Stored assessments for groups omitted from an update are retained.",
                    "items": {
                        "type": "object",
                        "required": ["hill_climbability", "feedback_loop"],
                        "properties": {
                            "group": {
                                "type": "string",
                                "description": "Group label this goal describes. Omit or null for the ungrouped list."
                            },
                            "hill_climbability": {
                                "type": "integer",
                                "minimum": 0,
                                "maximum": 100,
                                "description": "Self-assessment, 0-100, of how readily progress toward this goal can be measured and compared across iterations."
                            },
                            "feedback_loop": {
                                "type": "string",
                                "description": "Concrete requirement-to-check process used to compare progress across iterations and detect whether the user's intention is satisfied or violated. Name an explicit observation or check for every material behavior, deliverable, constraint, prohibited action, integration path, edge case, and necessary follow-through. Generic phrases such as run tests, verify, or review count only for requirements those named checks demonstrably enforce; add separate checks for non-testable prompt requirements."
                            },
                            "end_to_end_ownership": {
                                "type": "integer",
                                "minimum": 0,
                                "maximum": 100,
                                "description": "Completion-time self-assessment, 0-100, of whether the full intended user outcome and its necessary follow-through were delivered, rather than only the immediate implementation. Use only when completing the goal."
                            }
                        }
                    }
                }
            }
        })
    }

    async fn execute(&self, input: Value, ctx: ToolContext) -> Result<ToolOutput> {
        let params: TodoInput = serde_json::from_value(normalize_todo_input(input))?;
        let is_write = params.todos.is_some() || params.goals.is_some() || params.plan.is_some();
        let operation = if is_write { "write" } else { "read" };
        let result = if is_write {
            // Goals/plan-only writes keep the stored todo list.
            let previous = load_todos(&ctx.session_id).unwrap_or_default();
            let mut todos = params.todos.unwrap_or_else(|| previous.clone());
            merge_confidence_history(&previous, &mut todos);
            (|| {
                let stored_goals = load_goals(&ctx.session_id).unwrap_or_default();
                let stored_plan = load_plan(&ctx.session_id).unwrap_or_default();
                let goals = merge_goals(&stored_goals, params.goals);
                let plan = merge_plan(&stored_plan, params.plan);
                if !newly_completed_groups_have_sufficient_ownership(&previous, &todos, &goals) {
                    crate::telemetry::record_todo_gate(crate::telemetry::TodoGateKind::Ownership);
                    // The refusal must be visible AS a refusal: returning the
                    // stored (pre-write) state without saying so reads as a
                    // silent no-op and sends the model into a retry loop
                    // against an invisible wall (observed live, 2026-07-20:
                    // six identical rejected writes diagnosed as "the todo
                    // store is frozen").
                    return build_todo_output(
                        previous,
                        stored_plan,
                        stored_goals,
                        None,
                        None,
                        [
                            TODO_WRITE_REJECTED_NOTICE.to_string(),
                            TODO_OWNERSHIP_CONTINUATION_MESSAGE.to_string(),
                        ],
                    );
                }
                let nudges = take_reframe_nudges(&plan, &goals, &todos);
                for nudge in &nudges {
                    // `take_reframe_nudges` only emits these two kinds, so the
                    // hill-climbability nudge is the remaining case.
                    let kind = if nudge == TODO_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE {
                        crate::telemetry::TodoGateKind::IntentUnderstanding
                    } else {
                        crate::telemetry::TodoGateKind::HillClimbability
                    };
                    crate::telemetry::record_todo_gate(kind);
                }
                // Assessment-only writes, especially quality-gate retries,
                // should render the fields that changed instead of repeating an
                // otherwise identical todo plan.
                let assessment_only = todos == previous;
                let concise_goal_changes = (assessment_only && !stored_goals.is_empty())
                    .then(|| goal_changes(&stored_goals, &goals));
                let concise_plan_change = assessment_only
                    .then(|| plan_change(&stored_plan, &plan))
                    .flatten();
                save_todos(&ctx.session_id, &todos)?;
                save_goals(&ctx.session_id, &goals)?;
                save_plan(&ctx.session_id, &plan)?;

                Bus::global().publish(BusEvent::TodoUpdated(TodoEvent {
                    session_id: ctx.session_id.clone(),
                    todos: todos.clone(),
                }));

                build_todo_output(
                    todos,
                    plan,
                    goals,
                    concise_plan_change,
                    concise_goal_changes,
                    nudges,
                )
            })()
        } else {
            (|| {
                let todos = load_todos(&ctx.session_id)?;
                let goals = load_goals(&ctx.session_id).unwrap_or_default();
                let plan = load_plan(&ctx.session_id).unwrap_or_default();
                build_todo_output(todos, plan, goals, None, None, Vec::new())
            })()
        };
        result.map_err(|err| {
            crate::logging::warn(&format!(
                "[tool:todo] operation failed operation={} session_id={} error={}",
                operation, ctx.session_id, err
            ));
            err
        })
    }
}

#[cfg(test)]
#[path = "todo_tests.rs"]
mod tests;
