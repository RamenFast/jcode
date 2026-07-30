use super::*;

#[test]
fn tool_is_named_todo() {
    assert_eq!(TodoTool::new().name(), "todo");
}

#[test]
fn schema_advertises_intent_and_todos() {
    let schema = TodoTool::new().parameters_schema();
    let props = schema
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("todo schema should have properties");
    assert_eq!(props.len(), 4);
    assert!(props.contains_key("intent"));
    assert!(props.contains_key("todos"));
    assert!(props.contains_key("plan"));
    assert!(props.contains_key("goals"));

    let item = props["todos"]
        .get("items")
        .and_then(|v| v.as_object())
        .expect("todos should describe item objects");
    let required = item
        .get("required")
        .and_then(|v| v.as_array())
        .expect("todo item should advertise required fields");
    assert!(required.iter().any(|v| v == "confidence"));
    let item_props = item
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("todo item should advertise properties");
    assert!(item_props.contains_key("confidence"));
    assert!(item_props.contains_key("completion_confidence"));
    assert!(!item_props.contains_key("hill_climbability"));
    assert_eq!(
        item_props["confidence"]["description"],
        "Self-assessed confidence, 0-100, that this todo can be completed correctly. Reassess it as evidence accumulates while working."
    );

    let plan_props = props["plan"]
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("plan should describe properties");
    assert!(plan_props.contains_key("user_intention"));
    assert!(plan_props.contains_key("understands_user_intent"));
    assert!(!plan_props.contains_key("alignment_score"));
    assert!(!plan_props.contains_key("user_intention_alignment"));
    assert_eq!(plan_props.len(), 2);
    let plan_required = props["plan"]["required"]
        .as_array()
        .expect("plan should advertise required fields");
    assert!(plan_required.iter().any(|value| value == "user_intention"));
    assert!(
        plan_required
            .iter()
            .any(|value| value == "understands_user_intent")
    );

    let goal_props = props["goals"]
        .get("items")
        .and_then(|v| v.get("properties"))
        .and_then(|v| v.as_object())
        .expect("goals should describe item objects");
    assert!(goal_props.contains_key("group"));
    assert!(goal_props.contains_key("hill_climbability"));
    assert!(goal_props.contains_key("feedback_loop"));
    assert!(goal_props.contains_key("end_to_end_ownership"));
    // Intent lives on the plan, not per goal.
    assert!(!goal_props.contains_key("user_intention"));
    assert!(!goal_props.contains_key("alignment_score"));
    assert!(!goal_props.contains_key("objective"));
    assert_eq!(goal_props.len(), 4);

    let goal_required = props["goals"]["items"]["required"]
        .as_array()
        .expect("goals should advertise required fields");
    assert!(
        goal_required
            .iter()
            .any(|value| value == "hill_climbability")
    );
    assert!(goal_required.iter().any(|value| value == "feedback_loop"));

    let alignment_description = plan_props["understands_user_intent"]
        .get("description")
        .and_then(Value::as_str)
        .expect("alignment score should describe representation coverage");
    for required_concept in [
        "what the user actually wants",
        "requirement inventory",
        "explicit observation or check",
        "generic instruction to run tests",
        "tests count only for behaviors they actually enforce",
        "non-testable requirements",
        "prohibited modifications",
        "integration path",
        "edge case",
        "necessary follow-through",
        "over asking the user",
    ] {
        assert!(
            alignment_description.contains(required_concept),
            "alignment description omitted {required_concept}: {alignment_description}"
        );
    }
    let feedback_description = goal_props["feedback_loop"]
        .get("description")
        .and_then(Value::as_str)
        .expect("feedback loop should describe requirement-to-check coverage");
    for required_concept in [
        "requirement-to-check",
        "explicit observation or check",
        "prohibited action",
        "non-testable prompt requirements",
    ] {
        assert!(
            feedback_description.contains(required_concept),
            "feedback description omitted {required_concept}: {feedback_description}"
        );
    }
    assert!(
        !alignment_description
            .to_ascii_lowercase()
            .contains("threshold")
    );

    let ownership_description = goal_props["end_to_end_ownership"]
        .get("description")
        .and_then(Value::as_str)
        .expect("ownership should have a neutral description");
    assert!(ownership_description.contains("Use only when completing the goal."));
    assert!(ownership_description.contains("full intended user outcome"));
    assert!(ownership_description.contains("necessary follow-through"));
    assert!(!ownership_description.contains("90"));
    assert!(
        !ownership_description
            .to_ascii_lowercase()
            .contains("threshold")
    );

    let hill_description = goal_props["hill_climbability"]
        .get("description")
        .and_then(Value::as_str)
        .expect("hill-climbability should describe the assessment neutrally");
    assert!(!hill_description.contains(&LOW_HILL_CLIMBABILITY.to_string()));
    assert!(!hill_description.to_ascii_lowercase().contains("threshold"));

    let model_visible_schema = serde_json::to_string(&schema)
        .expect("todo schema should serialize")
        .to_ascii_lowercase();
    for disclosure in [
        "threshold",
        "quality gate",
        "internal quality check",
        "not jump",
        "test that passes",
        "isn't high enough",
    ] {
        assert!(
            !model_visible_schema.contains(disclosure),
            "model-visible todo schema disclosed calibration wording: {disclosure}"
        );
    }
    for domain_hint in [
        "visual quality",
        "screenshot",
        "browser",
        "viewport",
        "console error",
    ] {
        assert!(
            !model_visible_schema.contains(domain_hint),
            "model-visible todo schema biased visual-work feedback: {domain_hint}"
        );
    }
}

fn parse(input: Value) -> Result<TodoInput, serde_json::Error> {
    serde_json::from_value(normalize_todo_input(input))
}

#[test]
fn accepts_stringified_todos_array() {
    let input = json!({
        "todos": "[{\"content\":\"a\",\"status\":\"pending\",\"priority\":\"high\",\"id\":\"1\",\"confidence\":90}]"
    });
    let parsed = parse(input).expect("stringified todos array should parse");
    let todos = parsed.todos.expect("todos present");
    assert_eq!(todos.len(), 1);
    assert_eq!(todos[0].content, "a");
    assert_eq!(todos[0].confidence, Some(90));
}

#[test]
fn accepts_stringified_todo_items_and_string_confidence() {
    let input = json!({
        "todos": [
            "{\"content\":\"b\",\"status\":\"completed\",\"priority\":\"low\",\"id\":\"2\",\"confidence\":\"85\",\"completion_confidence\":\"95\"}",
            {"content": "c", "status": "pending", "priority": "high", "id": "3", "confidence": "70"}
        ]
    });
    let parsed = parse(input).expect("string-coerced items should parse");
    let todos = parsed.todos.expect("todos present");
    assert_eq!(todos.len(), 2);
    assert_eq!(todos[0].confidence, Some(85));
    assert_eq!(todos[0].completion_confidence, Some(95));
    assert_eq!(todos[1].confidence, Some(70));
}

#[test]
fn accepts_float_confidence_and_empty_string_as_none() {
    let input = json!({
        "todos": [
            {"content": "d", "status": "pending", "priority": "high", "id": "4", "confidence": 90.0, "completion_confidence": ""}
        ]
    });
    let parsed = parse(input).expect("float confidence should parse");
    let todos = parsed.todos.expect("todos present");
    assert_eq!(todos[0].confidence, Some(90));
    assert_eq!(todos[0].completion_confidence, None);
}

#[test]
fn empty_string_todos_means_read() {
    let parsed = parse(json!({"todos": ""})).expect("empty string should parse");
    assert!(parsed.todos.is_none());
}

#[test]
fn native_input_still_parses() {
    let input = json!({
        "todos": [
            {"content": "e", "status": "pending", "priority": "high", "id": "5", "confidence": 80}
        ]
    });
    let parsed = parse(input).expect("native input should parse");
    assert_eq!(parsed.todos.expect("todos present")[0].confidence, Some(80));
}

#[test]
fn accepts_goals_and_plan_including_string_coercion() {
    let input = json!({
        "plan": {"user_intention": "make repository search feel instant", "understands_user_intent": "97"},
        "goals": [
            {"group": "optimize grep", "hill_climbability": "95", "feedback_loop": "run the grep benchmark and compare p50"},
            {"hill_climbability": 20}
        ]
    });
    let parsed = parse(input).expect("goals and plan should parse");
    let plan = parsed.plan.expect("plan present");
    assert_eq!(plan.understands_user_intent, Some(97));
    assert_eq!(
        plan.user_intention.as_deref(),
        Some("make repository search feel instant")
    );
    let goals = parsed.goals.expect("goals present");
    assert_eq!(goals[0].hill_climbability, Some(95));
    assert_eq!(
        goals[0].feedback_loop.as_deref(),
        Some("run the grep benchmark and compare p50")
    );
    // Runtime parsing remains backward-compatible with stored or older
    // provider payloads even though the advertised schema requires the field.
    assert_eq!(goals[1].feedback_loop, None);
    assert_eq!(goals[1].group, None);
}

#[test]
fn stringified_plan_object_is_accepted() {
    let parsed = parse(json!({
        "plan": "{\"user_intention\":\"ship it\",\"understands_user_intent\":\"96\"}"
    }))
    .expect("stringified plan should parse");
    let plan = parsed.plan.expect("plan present");
    assert_eq!(plan.user_intention.as_deref(), Some("ship it"));
    assert_eq!(plan.understands_user_intent, Some(96));
}

#[test]
fn accepts_legacy_plan_alignment_key_but_serializes_the_new_name() {
    let parsed = parse(json!({
        "plan": {"user_intention_alignment": "97"}
    }))
    .expect("legacy alignment key should remain readable");
    let plan = parsed.plan.expect("plan present");
    assert_eq!(plan.understands_user_intent, Some(97));

    let serialized = serde_json::to_value(plan).expect("plan should serialize");
    assert_eq!(serialized["understands_user_intent"], 97);
    assert!(serialized.get("user_intention_alignment").is_none());

    let legacy_field: TodoPlanField = serde_json::from_str("\"user_intention_alignment\"")
        .expect("legacy plan-change field should deserialize");
    assert_eq!(legacy_field, TodoPlanField::UnderstandsUserIntent);
    assert_eq!(
        serde_json::to_string(&legacy_field).expect("plan field should serialize"),
        "\"understands_user_intent\""
    );
}

fn goal(group: Option<&str>, score: u8) -> TodoGoal {
    TodoGoal {
        group: group.map(str::to_string),
        hill_climbability: Some(score),
        ..Default::default()
    }
}

/// A plan whose intent assessment clears the private gate, so goal-level
/// tests observe only hill-climbability behavior.
fn aligned_plan() -> TodoPlan {
    TodoPlan {
        user_intention: Some("understood".to_string()),
        understands_user_intent: Some(100),
    }
}

#[test]
fn merge_goals_retains_unmentioned_goals() {
    let stored = vec![goal(Some("a"), 20), goal(Some("b"), 90)];
    // Rewrite goal 'a', leave 'b' alone.
    let merged = merge_goals(&stored, Some(vec![goal(Some(" a "), 30)]));
    assert_eq!(merged.len(), 2);
    assert_eq!(merged[0].group.as_deref(), Some("a"));
    assert_eq!(merged[0].hill_climbability, Some(30));
    assert_eq!(merged[1].group.as_deref(), Some("b"));
    // No incoming goals: stored goals unchanged.
    assert_eq!(merge_goals(&stored, None).len(), 2);
}

#[test]
fn merge_plan_retains_stored_intent_when_update_omits_fields() {
    let stored = TodoPlan {
        user_intention: Some("make search feel instant".to_string()),
        understands_user_intent: Some(60),
    };

    let merged = merge_plan(
        &stored,
        Some(TodoPlan {
            user_intention: None,
            understands_user_intent: Some(95),
        }),
    );
    assert_eq!(
        merged.user_intention.as_deref(),
        Some("make search feel instant")
    );
    assert_eq!(merged.understands_user_intent, Some(95));

    // An omitted plan leaves the stored assessment untouched.
    assert_eq!(merge_plan(&stored, None), stored);
}

#[test]
fn plan_change_reports_only_updated_intent_fields() {
    let before = aligned_plan();
    let after = TodoPlan {
        user_intention: Some("understood better".to_string()),
        ..before.clone()
    };

    let change = plan_change(&before, &after).expect("intent change should be reported");
    assert_eq!(change.fields, vec![TodoPlanField::UserIntention]);
    assert_eq!(change.before.as_ref(), Some(&before));
    assert_eq!(change.after.as_ref(), Some(&after));
    assert!(plan_change(&before, &before).is_none());
}

fn open_todo(group: Option<&str>) -> TodoItem {
    TodoItem {
        id: "t1".to_string(),
        content: "work".to_string(),
        status: "in_progress".to_string(),
        priority: "high".to_string(),
        group: group.map(str::to_string),
        ..Default::default()
    }
}

#[test]
fn ownership_gate_output_preserves_the_saved_todo_card() {
    let todos = vec![open_todo(Some("ship"))];
    let plan = aligned_plan();
    let goals = vec![goal(Some("ship"), 96)];
    let output = build_todo_output(
        todos.clone(),
        plan.clone(),
        goals.clone(),
        None,
        None,
        [
            TODO_WRITE_REJECTED_NOTICE.to_string(),
            TODO_OWNERSHIP_CONTINUATION_MESSAGE.to_string(),
        ],
    )
    .expect("ownership gate should produce a structured todo result");

    assert_eq!(output.title.as_deref(), Some("1 todos"));
    assert!(output.output.starts_with('['));
    assert!(output.output.contains("\"status\": \"in_progress\""));
    assert!(output.output.contains(TODO_OWNERSHIP_CONTINUATION_MESSAGE));
    assert_eq!(
        output.metadata,
        Some(json!({"todos": todos, "plan": plan, "goals": goals}))
    );
}

#[test]
fn gated_rejection_declares_itself_not_saved() {
    // Regression (2026-07-20): a gated write silently returned the stored
    // state, which the model read as a frozen store and retried against
    // six times. The rejection must say, in the output, that nothing was
    // saved — while still disclosing no scores or thresholds.
    let output = build_todo_output(
        vec![open_todo(Some("ship"))],
        aligned_plan(),
        vec![goal(Some("ship"), 96)],
        None,
        None,
        [
            TODO_WRITE_REJECTED_NOTICE.to_string(),
            TODO_OWNERSHIP_CONTINUATION_MESSAGE.to_string(),
        ],
    )
    .expect("rejection output should build");

    assert!(output.output.contains("NOT saved"));
    assert!(output.output.contains("previously stored state"));
    assert!(output.output.contains(TODO_OWNERSHIP_CONTINUATION_MESSAGE));
    let lower = TODO_WRITE_REJECTED_NOTICE.to_ascii_lowercase();
    for disclosure in ["threshold", "score", "96", "quality gate"] {
        assert!(
            !lower.contains(disclosure),
            "rejection notice disclosed calibration: {disclosure}"
        );
    }
}

#[test]
fn goal_changes_include_only_updated_quality_fields() {
    let before = TodoGoal {
        group: Some("search".to_string()),
        hill_climbability: Some(90),
        feedback_loop: Some("Run one benchmark".to_string()),
        end_to_end_ownership: None,
    };
    let after = TodoGoal {
        hill_climbability: Some(98),
        feedback_loop: Some("Run five benchmarks and compare p50".to_string()),
        ..before.clone()
    };

    let changes = goal_changes(&[before.clone()], &[after.clone()]);

    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].before.as_ref(), Some(&before));
    assert_eq!(changes[0].after.as_ref(), Some(&after));
    assert_eq!(
        changes[0].fields,
        vec![TodoGoalField::HillClimbability, TodoGoalField::FeedbackLoop,]
    );
}

#[test]
fn reframe_nudge_recurs_for_every_low_open_goal_write() {
    let todos = vec![open_todo(Some("design"))];
    let plan = aligned_plan();
    let goals = vec![goal(Some("design"), 95), goal(Some("perf"), 96)];
    let nudges = take_reframe_nudges(&plan, &goals, &todos);
    assert_eq!(nudges.len(), 1);
    assert_eq!(nudges[0], TODO_HILL_CLIMBABILITY_CONTINUATION_MESSAGE);
    assert!(!nudges[0].contains("95"));
    assert!(nudges[0].contains("hill-climbability"));
    assert!(!nudges[0].to_ascii_lowercase().contains("threshold"));
    // The nudge must not disclose that a calibrated *quality gate* exists, but it
    // must still mark itself as an automated message rather than a user turn, and
    // the house marker for that names the gate. Forbid the calibration wording,
    // which is what the model-visible schema check forbids too, not the bare word.
    assert!(!nudges[0].to_ascii_lowercase().contains("quality gate"));
    assert!(nudges[0].contains("not a user message]"));
    // A subsequent write receives the same generic guidance while the
    // private condition remains applicable.
    assert_eq!(take_reframe_nudges(&plan, &goals, &todos).len(), 1);
}

#[test]
fn alignment_nudge_is_plan_level_and_independent_of_goals() {
    let todos = vec![open_todo(Some("coverage"))];
    let plan = TodoPlan {
        user_intention: Some("partially understood".to_string()),
        understands_user_intent: Some(95),
    };
    let nudges = take_reframe_nudges(&plan, &[goal(Some("coverage"), 96)], &todos);

    assert_eq!(nudges, vec![TODO_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE]);
    assert!(nudges[0].contains("user's intent"));
    assert!(nudges[0].contains("think harder"));
    assert!(nudges[0].contains("Do not ask the user"));
    assert!(!nudges[0].contains("95"));
    assert!(!nudges[0].to_ascii_lowercase().contains("threshold"));
    assert!(!nudges[0].to_ascii_lowercase().contains("quality gate"));
    assert!(nudges[0].contains("not a user message]"));
}

#[test]
fn plan_alignment_gate_applies_without_any_goals() {
    let todos = vec![open_todo(None)];
    assert_eq!(
        take_reframe_nudges(&TodoPlan::default(), &[], &todos),
        vec![TODO_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE]
    );
    // Closed work is not nudged.
    let mut done = open_todo(None);
    done.status = "completed".to_string();
    assert!(take_reframe_nudges(&TodoPlan::default(), &[], &[done]).is_empty());
}

#[test]
fn alignment_and_hill_nudges_report_both_independent_weak_links() {
    let todos = vec![open_todo(Some("coverage"))];
    let plan = TodoPlan {
        user_intention: Some("partially understood".to_string()),
        understands_user_intent: Some(95),
    };
    let nudges = take_reframe_nudges(&plan, &[goal(Some("coverage"), 95)], &todos);

    assert_eq!(
        nudges,
        vec![
            TODO_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE,
            TODO_HILL_CLIMBABILITY_CONTINUATION_MESSAGE,
        ]
    );
}

#[test]
fn missing_quality_scores_do_not_bypass_open_gates() {
    let todos = vec![open_todo(Some("coverage"))];
    let mut goal = goal(Some("coverage"), 96);
    goal.hill_climbability = None;

    assert_eq!(
        take_reframe_nudges(&TodoPlan::default(), &[goal], &todos),
        vec![
            TODO_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE,
            TODO_HILL_CLIMBABILITY_CONTINUATION_MESSAGE,
        ]
    );
}

#[test]
fn reframe_nudge_skips_closed_goals() {
    // Low goal whose todos are all completed: nothing to reframe.
    let mut done = open_todo(Some("legacy"));
    done.status = "completed".to_string();
    let goals = vec![goal(Some("legacy"), 10)];
    assert!(take_reframe_nudges(&aligned_plan(), &goals, &[done]).is_empty());
}

#[test]
fn reframe_nudge_covers_ungrouped_implicit_goal() {
    let todos = vec![open_todo(None)];
    let goals = vec![goal(None, 15)];
    let nudges = take_reframe_nudges(&aligned_plan(), &goals, &todos);
    assert_eq!(nudges.len(), 1);
    assert_eq!(nudges[0], TODO_HILL_CLIMBABILITY_CONTINUATION_MESSAGE);
}

#[test]
fn garbage_string_still_errors() {
    assert!(parse(json!({"todos": "not json at all"})).is_err());
}

fn history_todo(id: &str, confidence: Option<u8>, history: Vec<u8>) -> TodoItem {
    TodoItem {
        id: id.to_string(),
        content: format!("todo {id}"),
        status: "in_progress".to_string(),
        priority: "high".to_string(),
        confidence,
        confidence_history: history,
        ..Default::default()
    }
}

#[test]
fn confidence_history_appends_changes_and_skips_repeats() {
    let previous = vec![history_todo("1", Some(75), vec![75])];
    // Same confidence again: no new entry.
    let mut incoming = vec![history_todo("1", Some(75), Vec::new())];
    merge_confidence_history(&previous, &mut incoming);
    assert_eq!(incoming[0].confidence_history, vec![75]);
    // Raised confidence: appended.
    let mut incoming = vec![history_todo("1", Some(90), Vec::new())];
    merge_confidence_history(&previous, &mut incoming);
    assert_eq!(incoming[0].confidence_history, vec![75, 90]);
}

#[test]
fn confidence_history_records_completion_confidence() {
    let previous = vec![history_todo("1", Some(75), vec![75])];
    let mut done = history_todo("1", Some(100), Vec::new());
    done.status = "completed".to_string();
    done.completion_confidence = Some(100);
    let mut incoming = vec![done];
    merge_confidence_history(&previous, &mut incoming);
    // 75 (planning) -> 100 (final bulk stamp): the spike stays visible.
    assert_eq!(incoming[0].confidence_history, vec![75, 100]);
}

#[test]
fn completion_write_contributes_only_one_final_confidence_observation() {
    let previous = vec![history_todo("1", Some(70), vec![70])];
    let mut done = history_todo("1", Some(90), Vec::new());
    done.status = "completed".to_string();
    done.completion_confidence = Some(100);

    let mut incoming = vec![done];
    merge_confidence_history(&previous, &mut incoming);

    assert_eq!(incoming[0].confidence_history, vec![70, 100]);
}

#[test]
fn confidence_history_seeds_legacy_todos_before_completion() {
    let previous = vec![history_todo("1", Some(70), Vec::new())];
    let mut done = history_todo("1", Some(90), Vec::new());
    done.status = "completed".to_string();
    done.completion_confidence = Some(100);

    let mut incoming = vec![done];
    merge_confidence_history(&previous, &mut incoming);

    assert_eq!(incoming[0].confidence_history, vec![70, 100]);
}

#[test]
fn confidence_history_ignores_model_supplied_history_for_new_todos() {
    let mut incoming = vec![history_todo("9", Some(80), vec![1, 2, 3])];
    merge_confidence_history(&[], &mut incoming);
    assert_eq!(incoming[0].confidence_history, vec![80]);
}
