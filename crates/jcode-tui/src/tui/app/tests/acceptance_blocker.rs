fn save_acceptance_blocker(session: &str) -> crate::todo::TodoGoal {
    use crate::todo::{
        Autonomy, ConfidenceState, DeliveryState, Difficulty, FeedbackLoopCoverage,
        FeedbackLoopRelevance, FeedbackLoopState, FeedbackLoopTraceability, IterationMaturity,
        TodoGoal, TodoItem, save_goals, save_todos,
    };
    save_todos(
        session,
        &[TodoItem {
            id: "acceptance".into(),
            content: "Configure and test native model support".into(),
            status: "completed".into(),
            confidence: Some(ConfidenceState::Verified),
            completion_confidence: Some(ConfidenceState::Verified),
            confidence_history: vec![ConfidenceState::Verified],
            ..Default::default()
        }],
    )
    .expect("save completed work");
    let goal = TodoGoal {
        difficulty: Some(Difficulty::Involved),
        delivery_state: Some(DeliveryState::Integrated),
        autonomy: Some(Autonomy::NecessaryFollowthrough),
        closed_feedback_loop: Some(FeedbackLoopState::Usable),
        feedback_loop: Some("Native request returned the provider's context-limit error.".into()),
        feedback_loop_relevance: Some(FeedbackLoopRelevance::AcceptanceBlocked),
        feedback_loop_coverage: Some(FeedbackLoopCoverage::EdgeAndIntegrationPaths),
        feedback_loop_traceability: Some(FeedbackLoopTraceability::Complete),
        iteration_maturity: Some(IterationMaturity::ConstraintsExhausted),
        stopping_evidence: Some(
            "Requested input was rejected. Revalidate when the provider raises its limit.".into(),
        ),
        ..Default::default()
    };
    save_goals(session, std::slice::from_ref(&goal)).expect("save blocked assessment");
    goal
}

#[test]
fn acceptance_blocker_stops_repeated_runtime_nudges_without_claiming_success() {
    with_temp_jcode_home(|| {
        for remote in [false, true] {
            let mut app = create_test_app();
            app.auto_poke_incomplete_todos = true;
            app.auto_poke_default_on = true;
            app.todo_final_response_requested = false;
            app.is_remote = remote;
            let session = if remote {
                format!("{}-remote", app.session.id)
            } else {
                app.session.id.clone()
            };
            if remote {
                app.remote_session_id = Some(session.clone());
            }
            let goal = save_acceptance_blocker(&session);
            crate::todo::append_gate_observations(
                &session,
                &[crate::todo::GateObservation {
                    kind: crate::todo::GateObservationKind::FeedbackLoopRelevance,
                    group: None,
                    state: Some("acceptance_blocked".into()),
                }],
            )
            .expect("record real acceptance gap");

            assert!(app.schedule_auto_poke_followup_if_needed());
            assert_eq!(app.queued_messages.len(), 1);
            assert!(
                app.queued_messages[0].contains("acceptance is blocked"),
                "{:?}",
                app.queued_messages
            );
            assert!(!app.queued_messages[0].contains("Quality checks passed"));
            assert!(crate::todo::is_auto_poke_message(&app.queued_messages[0]));
            app.queued_messages.clear();
            app.pending_queued_dispatch = false;
            for _ in 0..8 {
                assert!(!app.schedule_auto_poke_followup_if_needed());
                assert!(app.queued_messages.is_empty());
            }
            assert!(app.auto_poke_incomplete_todos, "future work stays armed");
            assert_eq!(
                crate::todo::load_goals(&session).unwrap(),
                vec![goal.clone()]
            );
            assert!(
                !crate::todo::delivery_state_passes(&goal),
                "blocked is not passed"
            );
            assert!(
                app.display_messages()
                    .iter()
                    .any(|m| m.content.contains("Acceptance is blocked"))
            );

            let mut todos = crate::todo::load_todos(&session).unwrap();
            todos[0].status = "in_progress".into();
            crate::todo::save_todos(&session, &todos).unwrap();
            assert!(
                app.schedule_auto_poke_followup_if_needed(),
                "reopened work must resume"
            );
            assert!(app.queued_messages[0].contains("incomplete todo"));
        }
    });
}

#[test]
fn acceptance_blocker_without_evidence_still_gets_runtime_followup() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.auto_poke_incomplete_todos = true;
        let mut goal = save_acceptance_blocker(&app.session.id);
        goal.stopping_evidence = Some("  ".into());
        crate::todo::save_goals(&app.session.id, &[goal]).unwrap();
        assert!(app.schedule_auto_poke_followup_if_needed());
        assert!(app.queued_messages[0].contains("public interfaces"));
    });
}
