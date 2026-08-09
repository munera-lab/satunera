//! The suite the Postgres implementation must also pass in phase 2. Nothing
//! in here may reach past the `Store` trait.

use satunera_contracts::{Language, ProblemId, Verdict};
use satunera_store::{
    NewRun, NewSubmission, NewUser, ProblemRow, Role, SqliteStore, Store, StoreError,
    SubmissionStatus,
};
use time::OffsetDateTime;

async fn store() -> SqliteStore {
    SqliteStore::in_memory().await.unwrap()
}

fn new_user(handle: &str) -> NewUser {
    NewUser {
        handle: handle.to_string(),
        email: format!("{handle}@example.com"),
        password_hash: "argon2-opaque".to_string(),
        role: Role::Learner,
    }
}

fn problem_row(id: &str) -> ProblemRow {
    ProblemRow {
        id: ProblemId::new(id),
        commit_sha: "c1b6788".to_string(),
        title: format!("Problem {id}"),
        category: "transactions".to_string(),
        difficulty: "easy".to_string(),
        manifest: "{}".to_string(),
        updated_at: OffsetDateTime::now_utc(),
    }
}

async fn seeded(store: &SqliteStore) -> (satunera_store::User, ProblemId) {
    let user = store.create_user(new_user("alice")).await.unwrap();
    store
        .replace_problems(vec![problem_row("week-1")])
        .await
        .unwrap();
    (user, ProblemId::new("week-1"))
}

fn new_submission(
    user: &satunera_store::User,
    problem_id: &ProblemId,
    hash: &str,
) -> NewSubmission {
    NewSubmission {
        user_id: user.id,
        problem_id: problem_id.clone(),
        language: Language::Rust,
        source_hash: hash.to_string(),
        source_uri: format!("file:///var/judge/blobs/{hash}"),
    }
}

#[tokio::test]
async fn duplicate_handle_is_a_conflict_not_a_500() {
    let store = store().await;
    store.create_user(new_user("alice")).await.unwrap();

    let mut dup = new_user("alice");
    dup.email = "other@example.com".to_string();
    assert!(matches!(
        store.create_user(dup).await,
        Err(StoreError::Conflict {
            entity: "users",
            ..
        })
    ));
}

#[tokio::test]
async fn users_round_trip_by_id_and_handle() {
    let store = store().await;
    let created = store.create_user(new_user("alice")).await.unwrap();

    let by_id = store.user(created.id).await.unwrap();
    let by_handle = store.user_by_handle("alice").await.unwrap();
    assert_eq!(by_id, created);
    assert_eq!(by_handle, created);

    assert!(matches!(
        store.user_by_handle("nobody").await,
        Err(StoreError::NotFound { .. })
    ));
}

#[tokio::test]
async fn replace_problems_is_a_full_swap() {
    let store = store().await;
    store
        .replace_problems(vec![problem_row("week-1"), problem_row("week-2")])
        .await
        .unwrap();
    assert_eq!(store.problems().await.unwrap().len(), 2);

    store
        .replace_problems(vec![problem_row("week-3")])
        .await
        .unwrap();
    let after = store.problems().await.unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].id.as_str(), "week-3");

    assert!(matches!(
        store.problem(&ProblemId::new("week-1")).await,
        Err(StoreError::NotFound { .. })
    ));
}

#[tokio::test]
async fn submission_walks_the_happy_path() {
    let store = store().await;
    let (user, problem_id) = seeded(&store).await;

    let submission = store
        .create_submission(new_submission(&user, &problem_id, "abc123"))
        .await
        .unwrap();
    assert_eq!(submission.status, SubmissionStatus::Queued);
    assert!(submission.verdict.is_none());

    for status in [
        SubmissionStatus::Preparing,
        SubmissionStatus::Building,
        SubmissionStatus::Booting,
        SubmissionStatus::Running,
    ] {
        store
            .set_submission_status(submission.id, status)
            .await
            .unwrap();
    }
    store
        .finish_submission(submission.id, Verdict::Ac)
        .await
        .unwrap();

    let done = store.submission(submission.id).await.unwrap();
    assert_eq!(done.status, SubmissionStatus::Done);
    assert_eq!(done.verdict, Some(Verdict::Ac));
    assert!(done.started_at.is_some());
    assert!(done.finished_at.is_some());
}

#[tokio::test]
async fn illegal_transitions_are_refused() {
    let store = store().await;
    let (user, problem_id) = seeded(&store).await;
    let submission = store
        .create_submission(new_submission(&user, &problem_id, "abc123"))
        .await
        .unwrap();

    // QUEUED cannot jump straight to RUNNING.
    assert!(matches!(
        store
            .set_submission_status(submission.id, SubmissionStatus::Running)
            .await,
        Err(StoreError::IllegalTransition { .. })
    ));

    // And a finished submission is immutable.
    store
        .set_submission_status(submission.id, SubmissionStatus::Preparing)
        .await
        .unwrap();
    store
        .finish_submission(submission.id, Verdict::Wa)
        .await
        .unwrap();
    assert!(matches!(
        store
            .set_submission_status(submission.id, SubmissionStatus::Preparing)
            .await,
        Err(StoreError::IllegalTransition { .. })
    ));
    assert!(matches!(
        store.finish_submission(submission.id, Verdict::Ac).await,
        Err(StoreError::IllegalTransition { .. })
    ));
}

#[tokio::test]
async fn dedupe_finds_the_prior_verdict_and_only_with_a_verdict() {
    let store = store().await;
    let (user, problem_id) = seeded(&store).await;

    let first = store
        .create_submission(new_submission(&user, &problem_id, "samehash"))
        .await
        .unwrap();

    // In flight: no verdict yet, so no dedupe hit.
    assert!(store
        .latest_verdict_for_hash(user.id, "samehash")
        .await
        .unwrap()
        .is_none());

    store
        .set_submission_status(first.id, SubmissionStatus::Preparing)
        .await
        .unwrap();
    store
        .finish_submission(first.id, Verdict::Ac)
        .await
        .unwrap();

    let hit = store
        .latest_verdict_for_hash(user.id, "samehash")
        .await
        .unwrap()
        .expect("verdict exists now");
    assert_eq!(hit.id, first.id);
    assert_eq!(hit.verdict, Some(Verdict::Ac));

    // Another user's identical source is not this user's verdict.
    let bob = store.create_user(new_user("bob")).await.unwrap();
    assert!(store
        .latest_verdict_for_hash(bob.id, "samehash")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn history_is_newest_first_and_in_flight_scan_sees_only_open_work() {
    let store = store().await;
    let (user, problem_id) = seeded(&store).await;

    let first = store
        .create_submission(new_submission(&user, &problem_id, "h1"))
        .await
        .unwrap();
    let second = store
        .create_submission(new_submission(&user, &problem_id, "h2"))
        .await
        .unwrap();

    let history = store.submissions_for_user(user.id, 10).await.unwrap();
    assert_eq!(
        history.iter().map(|s| s.id).collect::<Vec<_>>(),
        vec![second.id, first.id]
    );

    store
        .set_submission_status(first.id, SubmissionStatus::Preparing)
        .await
        .unwrap();
    store
        .finish_submission(first.id, Verdict::Wa)
        .await
        .unwrap();

    let open = store.in_flight_submissions().await.unwrap();
    assert_eq!(
        open.iter().map(|s| s.id).collect::<Vec<_>>(),
        vec![second.id]
    );
}

#[tokio::test]
async fn runs_keep_attempt_history_without_touching_the_submission() {
    let store = store().await;
    let (user, problem_id) = seeded(&store).await;
    let submission = store
        .create_submission(new_submission(&user, &problem_id, "h1"))
        .await
        .unwrap();

    // Attempt 1 died as SE; attempt 2 succeeded.
    store
        .record_run(NewRun {
            submission_id: submission.id,
            attempt: 1,
            runner_id: "runner-a".to_string(),
            fixture: "funded-p2wpkh".to_string(),
            exit_code: None,
            verdict: Some(Verdict::Se),
            output: None,
            log_uri: None,
            boot_ms: Some(9000),
            build_ms: None,
            run_ms: None,
        })
        .await
        .unwrap();
    store
        .record_run(NewRun {
            submission_id: submission.id,
            attempt: 2,
            runner_id: "runner-a".to_string(),
            fixture: "funded-p2wpkh".to_string(),
            exit_code: Some(0),
            verdict: Some(Verdict::Ac),
            output: Some("{\"ok\":true}".to_string()),
            log_uri: Some("file:///var/judge/logs/x".to_string()),
            boot_ms: Some(800),
            build_ms: Some(30_000),
            run_ms: Some(1200),
        })
        .await
        .unwrap();

    let runs = store.runs_for_submission(submission.id).await.unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].attempt, 1);
    assert_eq!(runs[0].verdict, Some(Verdict::Se));
    assert_eq!(runs[1].attempt, 2);
    assert_eq!(runs[1].verdict, Some(Verdict::Ac));

    // The learner's history still shows one submission.
    assert_eq!(
        store.submissions_for_user(user.id, 10).await.unwrap().len(),
        1
    );

    // A duplicate attempt number is a conflict.
    assert!(matches!(
        store
            .record_run(NewRun {
                submission_id: submission.id,
                attempt: 2,
                runner_id: "runner-b".to_string(),
                fixture: "funded-p2wpkh".to_string(),
                exit_code: None,
                verdict: None,
                output: None,
                log_uri: None,
                boot_ms: None,
                build_ms: None,
                run_ms: None,
            })
            .await,
        Err(StoreError::Conflict { entity: "runs", .. })
    ));
}

#[tokio::test]
async fn progress_counts_attempts_and_stamps_solved_once() {
    let store = store().await;
    let (user, problem_id) = seeded(&store).await;

    async fn submit_and_finish(
        store: &SqliteStore,
        user: &satunera_store::User,
        problem_id: &ProblemId,
        hash: &str,
        verdict: Verdict,
    ) {
        let s = store
            .create_submission(new_submission(user, problem_id, hash))
            .await
            .unwrap();
        store
            .set_submission_status(s.id, SubmissionStatus::Preparing)
            .await
            .unwrap();
        store.finish_submission(s.id, verdict).await.unwrap();
    }

    assert!(store
        .progress(user.id, &problem_id)
        .await
        .unwrap()
        .is_none());

    submit_and_finish(&store, &user, &problem_id, "h1", Verdict::Wa).await;
    let p = store.progress(user.id, &problem_id).await.unwrap().unwrap();
    assert_eq!(p.attempts, 1);
    assert!(p.solved_at.is_none());

    submit_and_finish(&store, &user, &problem_id, "h2", Verdict::Ac).await;
    let p = store.progress(user.id, &problem_id).await.unwrap().unwrap();
    assert_eq!(p.attempts, 2);
    let solved_at = p.solved_at.expect("solved now");

    // A later AC does not move the solve time; an SE counts nothing.
    submit_and_finish(&store, &user, &problem_id, "h3", Verdict::Ac).await;
    submit_and_finish(&store, &user, &problem_id, "h4", Verdict::Se).await;
    let p = store.progress(user.id, &problem_id).await.unwrap().unwrap();
    assert_eq!(p.attempts, 3);
    assert_eq!(p.solved_at, Some(solved_at));
}

#[tokio::test]
async fn ten_thousand_submissions_keep_history_fast() {
    let store = store().await;
    let (user, problem_id) = seeded(&store).await;

    for i in 0..10_000 {
        store
            .create_submission(new_submission(&user, &problem_id, &format!("h{i}")))
            .await
            .unwrap();
    }

    let start = std::time::Instant::now();
    let history = store.submissions_for_user(user.id, 50).await.unwrap();
    let elapsed = start.elapsed();

    assert_eq!(history.len(), 50);
    assert!(
        elapsed < std::time::Duration::from_millis(50),
        "history query took {elapsed:?}"
    );
}

#[tokio::test]
async fn migrations_roll_forward_and_back() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite:{}/judge.db", dir.path().display());

    let store = SqliteStore::open(&url).await.unwrap();
    seeded(&store).await;

    // Roll all the way back, then forward again on the same file.
    satunera_store::MIGRATOR
        .undo(store.pool(), 0)
        .await
        .unwrap();
    let after: Option<(String,)> =
        sqlx::query_as("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'users'")
            .fetch_optional(store.pool())
            .await
            .unwrap();
    assert!(after.is_none(), "down migration left tables behind");

    satunera_store::MIGRATOR.run(store.pool()).await.unwrap();
    store.create_user(new_user("carol")).await.unwrap();
}
