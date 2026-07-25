#[tokio::test]
async fn await_members_keeps_unknown_worker_pending_until_timeout() {
    let (_env, _runtime_dir) = RuntimeEnvGuard::new();
    let swarm_id = "swarm-unknown";
    let requester = "req";
    let missing_peer = "never-observed";
    let target_status = vec!["completed".to_string()];
    let requested_ids = vec![missing_peer.to_string()];
    let await_runtime = AwaitMembersRuntime::default();

    let (client_tx, mut client_rx) = mpsc::unbounded_channel();
    let swarm_members = Arc::new(RwLock::new(HashMap::from([(
        requester.to_string(),
        member(requester, swarm_id, "ready"),
    )])));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::from([(
        swarm_id.to_string(),
        HashSet::from([requester.to_string()]),
    )])));
    let (swarm_event_tx, _swarm_event_rx) = broadcast::channel(32);

    handle_comm_await_members(
        1,
        requester.to_string(),
        target_status.clone(),
        requested_ids.clone(),
        None,
        Some(0),
        false,
        false,
        false,
        CommAwaitMembersContext {
            client_event_tx: &client_tx,
            swarm_members: &swarm_members,
            swarms_by_id: &swarms_by_id,
            swarm_event_tx: &swarm_event_tx,
            await_members_runtime: &await_runtime,
        },
    )
    .await;

    let response = client_rx.recv().await.expect("timeout response should arrive");
    match response {
        ServerEvent::CommAwaitMembersResponse {
            completed,
            members,
            summary,
            ..
        } => {
            assert!(!completed, "unknown must never count as completed");
            assert_eq!(members.len(), 1);
            assert_eq!(members[0].status, "unknown");
            assert!(!members[0].done);
            assert!(summary.contains("unknown"), "summary: {summary}");
        }
        other => panic!("expected CommAwaitMembersResponse, got {other:?}"),
    }

    let key = crate::server::await_members_state::request_key(
        requester,
        swarm_id,
        &requested_ids,
        &target_status,
        None,
    );
    let state = crate::server::await_members_state::load_state(&key)
        .expect("unknown wait should persist its timeout result");
    assert!(
        state.observed_ids.is_empty(),
        "an ID that never appeared in live membership must not be recorded as observed"
    );
}

#[tokio::test]
async fn await_members_reports_observed_worker_that_vanishes_as_failed() {
    let (_env, _runtime_dir) = RuntimeEnvGuard::new();
    let swarm_id = "swarm-vanished";
    let requester = "req";
    let peer = "peer-1";
    let target_status = vec!["completed".to_string(), "failed".to_string()];
    let await_runtime = AwaitMembersRuntime::default();

    let (client_tx, mut client_rx) = mpsc::unbounded_channel();
    let swarm_members = Arc::new(RwLock::new(HashMap::from([
        (requester.to_string(), member(requester, swarm_id, "ready")),
        (peer.to_string(), member(peer, swarm_id, "running")),
    ])));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::from([(
        swarm_id.to_string(),
        HashSet::from([requester.to_string(), peer.to_string()]),
    )])));
    let (swarm_event_tx, _swarm_event_rx) = broadcast::channel(32);

    handle_comm_await_members(
        2,
        requester.to_string(),
        target_status.clone(),
        vec![],
        None,
        Some(5),
        false,
        false,
        false,
        CommAwaitMembersContext {
            client_event_tx: &client_tx,
            swarm_members: &swarm_members,
            swarms_by_id: &swarms_by_id,
            swarm_event_tx: &swarm_event_tx,
            await_members_runtime: &await_runtime,
        },
    )
    .await;

    let key = crate::server::await_members_state::request_key(
        requester,
        swarm_id,
        &[],
        &target_status,
        None,
    );
    let pending = crate::server::await_members_state::load_state(&key)
        .expect("active wait should be durable");
    assert_eq!(pending.observed_ids, vec![peer.to_string()]);
    assert!(pending.final_response.is_none());

    swarm_members.write().await.remove(peer);
    swarms_by_id
        .write()
        .await
        .get_mut(swarm_id)
        .expect("swarm exists")
        .remove(peer);
    let _ = swarm_event_tx.send(swarm_event(
        peer,
        swarm_id,
        SwarmEventType::MemberChange {
            action: "left".to_string(),
        },
    ));

    let response = tokio::time::timeout(Duration::from_secs(1), client_rx.recv())
        .await
        .expect("vanished worker should resolve the wait promptly")
        .expect("response channel should stay open");
    match response {
        ServerEvent::CommAwaitMembersResponse {
            completed,
            members,
            summary,
            ..
        } => {
            assert!(completed, "failed is an explicit target terminal status");
            assert_eq!(members.len(), 1);
            let vanished = &members[0];
            assert_eq!(vanished.session_id, peer);
            assert_eq!(vanished.status, "failed");
            assert!(vanished.done);
            assert!(
                vanished
                    .completion_report
                    .as_deref()
                    .is_some_and(|report| report.contains("Worker vanished"))
            );
            assert!(summary.contains("peer-1 (failed)"), "summary: {summary}");
            assert!(
                summary.contains("Failures require attention"),
                "summary: {summary}"
            );
            assert!(!summary.contains("unknown"), "summary: {summary}");
            assert!(!summary.contains("No other members"), "summary: {summary}");
        }
        other => panic!("expected CommAwaitMembersResponse, got {other:?}"),
    }

    let final_state = crate::server::await_members_state::load_state(&key)
        .expect("vanished result should remain durable");
    assert_eq!(final_state.observed_ids, vec![peer.to_string()]);
    let final_response = final_state
        .final_response
        .expect("vanished worker should persist a final result");
    assert!(final_response.completed);
    assert_eq!(final_response.members[0].status, "failed");
}
