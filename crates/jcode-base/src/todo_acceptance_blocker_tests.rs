fn acceptance_blocked_goal() -> TodoGoal {
    TodoGoal {
        difficulty: Some(Difficulty::Involved),
        feedback_loop: Some("The public request returned a provider limit error.".into()),
        feedback_loop_relevance: Some(FeedbackLoopRelevance::AcceptanceBlocked),
        iteration_maturity: Some(IterationMaturity::ConstraintsExhausted),
        stopping_evidence: Some("The provider must raise its limit before this can pass.".into()),
        ..delivery_goal(Some("blocked"), Some(DeliveryState::Integrated))
    }
}

#[test]
fn acceptance_blocker_requires_explicit_terminal_state_and_both_evidence_fields() {
    let goal = acceptance_blocked_goal();
    assert!(goal_has_evidenced_acceptance_blocker(&goal));
    assert!(!feedback_loop_relevance_passes(&goal));
    assert!(!delivery_state_passes(&goal));
    for evidence in [None, Some("".to_string()), Some(" \n\t".to_string())] {
        let mut missing = goal.clone();
        missing.feedback_loop = evidence.clone();
        assert!(!goal_has_evidenced_acceptance_blocker(&missing));
        missing = goal.clone();
        missing.stopping_evidence = evidence;
        assert!(!goal_has_evidenced_acceptance_blocker(&missing));
    }
    for maturity in [
        None,
        Some(IterationMaturity::Improving),
        Some(IterationMaturity::OutcomeReached),
        Some(IterationMaturity::BudgetExhausted),
    ] {
        let mut nonterminal = goal.clone();
        nonterminal.iteration_maturity = maturity;
        assert!(!goal_has_evidenced_acceptance_blocker(&nonterminal));
    }
    for relevance in [
        None,
        Some(FeedbackLoopRelevance::Synthetic),
        Some(FeedbackLoopRelevance::Representative),
    ] {
        let mut substitute = goal.clone();
        substitute.feedback_loop_relevance = relevance;
        assert!(!goal_has_evidenced_acceptance_blocker(&substitute));
    }
}

#[test]
fn acceptance_blocker_does_not_pass_delivery_or_hide_unrelated_groups() {
    let blocked = acceptance_blocked_goal();
    let mut todos = vec![todo("attempt acceptance", "completed", Some("blocked"))];
    assert!(completed_groups_can_stop(
        &todos,
        std::slice::from_ref(&blocked)
    ));
    assert!(!completed_groups_have_sufficient_delivery(
        &todos,
        std::slice::from_ref(&blocked)
    ));
    assert!(!newly_completed_groups_have_sufficient_delivery(
        &[],
        &todos,
        std::slice::from_ref(&blocked)
    ));
    todos.push(todo("finish delivery", "completed", Some("active")));
    assert!(!completed_groups_can_stop(
        &todos,
        std::slice::from_ref(&blocked)
    ));
    let mut active = delivery_goal(Some("active"), Some(DeliveryState::Integrated));
    let message =
        build_todo_ownership_continuation_message(&todos, &[blocked.clone(), active.clone()]);
    assert!(message.contains("Goal \"active\""));
    assert!(!message.contains("Goal \"blocked\""));
    assert!(!completed_groups_can_stop(
        &todos,
        &[blocked.clone(), active.clone()]
    ));
    active.delivery_state = Some(DeliveryState::WorkflowValidated);
    assert!(completed_groups_can_stop(&todos, &[blocked, active]));
}

#[test]
fn acceptance_blocker_digest_preserves_plan_review_and_unrelated_observations() {
    let goal = acceptance_blocked_goal();
    let observations: Vec<_> = [
        GateObservationKind::ClosedFeedbackLoop,
        GateObservationKind::FeedbackLoopRelevance,
        GateObservationKind::FeedbackLoopCoverage,
        GateObservationKind::FeedbackLoopTraceability,
    ]
    .into_iter()
    .map(|kind| GateObservation {
        kind,
        group: Some(" blocked ".into()),
        state: None,
    })
    .collect();
    assert!(
        build_gate_digest(
            &observations,
            &TodoPlan::default(),
            std::slice::from_ref(&goal)
        )
        .is_none()
    );
    let mut mixed = observations;
    mixed.push(intent_observation(Some(IntentUnderstanding::Partial)));
    mixed.push(loop_observation(
        Some("active"),
        Some(FeedbackLoopState::Weak),
    ));
    let digest = build_gate_digest(&mixed, &TodoPlan::default(), &[goal]).unwrap();
    assert!(digest.contains("understanding of what the user actually wants"));
    assert!(digest.contains("for \"active\""));
    assert!(!digest.contains("for \"blocked\""));
    assert_eq!(digest.matches("\n- ").count(), 2);
}

#[test]
fn acceptance_blocker_closeout_is_synthetic_and_never_claims_success() {
    let message = TODO_BLOCKED_RESPONSE_CONTINUATION_MESSAGE;
    assert!(is_auto_poke_message(message));
    assert!(message.contains("acceptance is blocked"));
    assert!(!message.contains("Quality checks passed"));
    assert!(!message.contains("Do not reply"));
    assert!(
        auto_poke_display_summary(message)
            .unwrap()
            .starts_with("Acceptance is blocked")
    );
    assert!(is_auto_poke_message(
        TODO_FINAL_RESPONSE_CONTINUATION_MESSAGE
    ));
    assert!(is_auto_poke_message(
        PRE_COMPACT_TODO_FINAL_RESPONSE_CONTINUATION_MESSAGE
    ));
}
