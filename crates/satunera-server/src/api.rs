//! The read API.
//!
//! Four endpoints over the immutable index. The list view deliberately omits
//! statement bodies: the list is the most requested page and the statements
//! are the largest field.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use satunera_contracts::{Language, Problem, ProblemId};

use crate::content::ProblemIndex;

pub fn router(index: Arc<ProblemIndex>) -> Router {
    Router::new()
        .route("/api/v1/catalog", get(catalog))
        .route("/api/v1/problems", get(list_problems))
        .route("/api/v1/problems/{id}", get(problem_detail))
        .route("/api/v1/problems/{id}/scaffold/{lang}", get(scaffold))
        .with_state(index)
}

#[derive(Serialize)]
struct ApiError {
    error: String,
}

fn not_found(message: impl Into<String>) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(ApiError {
            error: message.into(),
        }),
    )
        .into_response()
}

async fn catalog(State(index): State<Arc<ProblemIndex>>) -> Response {
    #[derive(Serialize)]
    struct Catalog<'a> {
        title: &'a str,
        commit_sha: &'a str,
        categories: &'a [satunera_contracts::Category],
        tracks: &'a [satunera_contracts::Track],
        features: &'a satunera_contracts::Features,
    }

    Json(Catalog {
        title: &index.config.title,
        commit_sha: &index.commit_sha,
        categories: &index.config.categories,
        tracks: &index.config.tracks,
        features: &index.config.features,
    })
    .into_response()
}

#[derive(Deserialize)]
struct ListFilter {
    category: Option<String>,
    difficulty: Option<String>,
}

/// The list view: manifest summary, no statement body.
#[derive(Serialize)]
struct ProblemSummary<'a> {
    id: &'a str,
    title: &'a str,
    category: &'a str,
    difficulty: &'a str,
    languages: &'a [Language],
}

fn summary(problem: &Problem) -> ProblemSummary<'_> {
    ProblemSummary {
        id: problem.id.as_str(),
        title: &problem.title,
        category: problem.category.as_str(),
        difficulty: problem.difficulty.as_str(),
        languages: &problem.languages,
    }
}

async fn list_problems(
    State(index): State<Arc<ProblemIndex>>,
    Query(filter): Query<ListFilter>,
) -> Response {
    let problems: Vec<ProblemSummary<'_>> = index
        .iter()
        .map(|loaded| &loaded.problem)
        .filter(|p| {
            filter
                .category
                .as_deref()
                .is_none_or(|c| p.category.as_str() == c)
        })
        .filter(|p| {
            filter
                .difficulty
                .as_deref()
                .is_none_or(|d| p.difficulty.as_str() == d)
        })
        .map(summary)
        .collect();

    Json(problems).into_response()
}

async fn problem_detail(
    State(index): State<Arc<ProblemIndex>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(loaded) = index.get(&ProblemId::new(id.as_str())) else {
        return not_found(format!("no problem {id:?}"));
    };

    // Strong ETag over manifest + rendered statement; the index is immutable
    // between restarts, so a match is authoritative.
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == loaded.etag)
    {
        return (
            StatusCode::NOT_MODIFIED,
            [(header::ETAG, loaded.etag.clone())],
        )
            .into_response();
    }

    #[derive(Serialize)]
    struct Detail<'a> {
        #[serde(flatten)]
        manifest: &'a Problem,
        statement_html: &'a str,
    }

    (
        [(header::ETAG, loaded.etag.clone())],
        Json(Detail {
            manifest: &loaded.problem,
            statement_html: &loaded.statement_html,
        }),
    )
        .into_response()
}

async fn scaffold(
    State(index): State<Arc<ProblemIndex>>,
    Path((id, lang)): Path<(String, String)>,
) -> Response {
    let Some(loaded) = index.get(&ProblemId::new(id.as_str())) else {
        return not_found(format!("no problem {id:?}"));
    };
    let Some(language) = Language::parse(&lang) else {
        return not_found(format!("no language {lang:?}"));
    };
    let Some(tarball) = loaded.scaffolds.get(&language) else {
        return not_found(format!("problem {id:?} does not accept {lang}"));
    };

    (
        [
            (header::CONTENT_TYPE, "application/gzip".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{id}-{lang}.tar.gz\""),
            ),
            (header::ETAG, loaded.etag.clone()),
        ],
        tarball.clone(),
    )
        .into_response()
}
