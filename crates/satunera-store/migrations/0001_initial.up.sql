-- Durable state for the judge. ANSI-plain by policy (ADR 0002): explicit
-- types, TEXT ids and RFC 3339 TEXT timestamps, no SQLite-only syntax, so the
-- phase-2 Postgres port is a mechanical translation.

CREATE TABLE users (
    id            TEXT    NOT NULL PRIMARY KEY,
    handle        TEXT    NOT NULL,
    email         TEXT    NOT NULL,
    password_hash TEXT    NOT NULL,
    role          TEXT    NOT NULL DEFAULT 'learner'
                          CHECK (role IN ('learner', 'author', 'admin')),
    created_at    TEXT    NOT NULL,

    CONSTRAINT users_handle_unique UNIQUE (handle),
    CONSTRAINT users_email_unique UNIQUE (email)
);

-- Materialized from the content repo at boot (T07). The git tree stays the
-- source of truth; this table exists so submissions can foreign-key to
-- something stable and queries never walk the filesystem.
CREATE TABLE problems (
    id         TEXT NOT NULL PRIMARY KEY,
    commit_sha TEXT NOT NULL,
    title      TEXT NOT NULL,
    category   TEXT NOT NULL,
    difficulty TEXT NOT NULL CHECK (difficulty IN ('easy', 'medium', 'hard')),
    manifest   TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- What the learner sees: one row per submitted source, carrying the final
-- verdict. Attempt-level detail lives in runs.
CREATE TABLE submissions (
    id          TEXT NOT NULL PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users (id),
    problem_id  TEXT NOT NULL REFERENCES problems (id),
    language    TEXT NOT NULL CHECK (language IN ('rust', 'py', 'cpp')),
    source_hash TEXT NOT NULL,
    source_uri  TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'QUEUED'
                     CHECK (status IN (
                         'QUEUED', 'PREPARING', 'BUILDING',
                         'BOOTING', 'RUNNING', 'DONE'
                     )),
    verdict     TEXT CHECK (verdict IN (
                    'AC', 'WA', 'TLE', 'MLE', 'RE', 'CE', 'SE', 'IE'
                )),
    created_at  TEXT NOT NULL,
    started_at  TEXT,
    finished_at TEXT
);

-- Dedupe: an identical resubmission returns the prior verdict without running.
CREATE INDEX submissions_by_user_problem
    ON submissions (user_id, problem_id);
CREATE INDEX submissions_by_status
    ON submissions (status);
CREATE INDEX submissions_by_source_hash
    ON submissions (source_hash);

-- One row per attempt. A runner that dies mid-job and retries appears here
-- as a second attempt, never as a second row in the learner's history.
CREATE TABLE runs (
    id            TEXT    NOT NULL PRIMARY KEY,
    submission_id TEXT    NOT NULL REFERENCES submissions (id),
    attempt       INTEGER NOT NULL,
    runner_id     TEXT    NOT NULL,
    fixture       TEXT    NOT NULL,
    exit_code     INTEGER,
    verdict       TEXT CHECK (verdict IN (
                      'AC', 'WA', 'TLE', 'MLE', 'RE', 'CE', 'SE', 'IE'
                  )),
    output        TEXT,
    log_uri       TEXT,
    boot_ms       INTEGER,
    build_ms      INTEGER,
    run_ms        INTEGER,
    created_at    TEXT    NOT NULL,

    CONSTRAINT runs_attempt_unique UNIQUE (submission_id, attempt)
);

CREATE INDEX runs_by_submission
    ON runs (submission_id);

CREATE TABLE user_progress (
    user_id    TEXT    NOT NULL REFERENCES users (id),
    problem_id TEXT    NOT NULL REFERENCES problems (id),
    solved_at  TEXT,
    attempts   INTEGER NOT NULL DEFAULT 0,

    CONSTRAINT user_progress_pk PRIMARY KEY (user_id, problem_id)
);
