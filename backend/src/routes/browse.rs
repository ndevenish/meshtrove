//! Unified browse: models and bundles in one ranked, paginated result set so
//! they appear inline in the same grid. A single server-side UNION ALL + ORDER
//! BY keeps pagination and totals correct across the two types (a client-side
//! merge of two paginated lists cannot).

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Query, State},
    routing::get,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{QueryBuilder, Row};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::User;
use crate::routes::bundles::push_bundle_filters;
use crate::routes::models::{SearchQuery, parse_opts, push_filters};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/browse", get(browse))
}

#[derive(Serialize, ToSchema)]
pub struct BrowseItem {
    /// "model" | "bundle"
    #[serde(rename = "type")]
    pub item_type: String,
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub creator_id: Option<Uuid>,
    pub creator_name: Option<String>,
    pub primary_image_id: Option<Uuid>,
    pub tags: Vec<String>,
    /// null for bundles (likes are model-only)
    pub like_count: Option<i64>,
    /// variant_count for models, model_count for bundles
    pub count: i64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Serialize, ToSchema)]
pub struct BrowseResults {
    pub items: Vec<BrowseItem>,
    pub total: i64,
    pub page: u32,
    pub per_page: u32,
}

/// The bundle side of the union: excluded entirely when an `opts` (variant axis)
/// filter is present, since bundles have no variants to satisfy it.
fn push_bundle_where(
    qb: &mut QueryBuilder<sqlx::Postgres>,
    q: &str,
    tags: &[String],
    opts_present: bool,
) {
    if opts_present {
        qb.push(" AND FALSE");
    } else {
        push_bundle_filters(qb, q, tags);
    }
}

async fn browse(
    State(state): State<AppState>,
    _user: User,
    Query(query): Query<SearchQuery>,
) -> Result<Json<BrowseResults>, ApiError> {
    let q = query.q.unwrap_or_default().trim().to_string();
    let tags: Vec<String> = query
        .tags
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    let opts = parse_opts(&query.opts.unwrap_or_default())?;
    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(30).clamp(1, 100);

    // Count over a lean union of just the matching ids.
    let mut cq = QueryBuilder::new("SELECT count(*) FROM (SELECT m.id FROM models m WHERE TRUE");
    push_filters(&mut cq, &q, &tags, &opts);
    cq.push(" UNION ALL SELECT b.id FROM bundles b WHERE TRUE");
    push_bundle_where(&mut cq, &q, &tags, !opts.is_empty());
    cq.push(") x");
    let total: i64 = cq.build_query_scalar().fetch_one(&state.db).await?;

    // Page: full projection for both types, ranked together.
    let rank_model = if q.is_empty() {
        "0::float4".to_string()
    } else {
        "ts_rank(m.search, websearch_to_tsquery('english', ".to_string()
    };
    let mut qb = QueryBuilder::new(
        r#"SELECT * FROM (
            SELECT 'model' AS item_type, m.id, m.name, m.slug, m.creator_id, c.name AS creator_name,
                   (SELECT i.id FROM images i WHERE i.model_id = m.id AND i.is_primary) AS primary_image_id,
                   (SELECT count(*) FROM user_model_marks k WHERE k.model_id = m.id AND k.mark = 'liked') AS like_count,
                   (SELECT count(*) FROM model_variants v WHERE v.model_id = m.id) AS count,
                   coalesce((SELECT array_agg(t.name::text ORDER BY t.name) FROM model_tags mt
                             JOIN tags t ON t.id = mt.tag_id WHERE mt.model_id = m.id), '{}') AS tags,
                   m.updated_at, "#,
    );
    qb.push(&rank_model);
    if !q.is_empty() {
        qb.push_bind(q.clone()).push(")) AS rank");
    } else {
        qb.push(" AS rank");
    }
    qb.push(" FROM models m LEFT JOIN creators c ON c.id = m.creator_id WHERE TRUE");
    push_filters(&mut qb, &q, &tags, &opts);

    qb.push(
        r#" UNION ALL
            SELECT 'bundle' AS item_type, b.id, b.name, b.slug, b.creator_id, c.name AS creator_name,
                   (SELECT i.id FROM images i WHERE i.bundle_id = b.id AND i.is_primary) AS primary_image_id,
                   NULL::bigint AS like_count,
                   (SELECT count(*) FROM bundle_models bm WHERE bm.bundle_id = b.id) AS count,
                   coalesce((SELECT array_agg(t.name::text ORDER BY t.name) FROM bundle_tags bt
                             JOIN tags t ON t.id = bt.tag_id WHERE bt.bundle_id = b.id), '{}') AS tags,
                   b.updated_at, "#,
    );
    if q.is_empty() {
        qb.push("0::float4 AS rank");
    } else {
        qb.push("ts_rank(b.search, websearch_to_tsquery('english', ")
            .push_bind(q.clone())
            .push(")) AS rank");
    }
    qb.push(" FROM bundles b LEFT JOIN creators c ON c.id = b.creator_id WHERE TRUE");
    push_bundle_where(&mut qb, &q, &tags, !opts.is_empty());

    qb.push(") x ORDER BY rank DESC, updated_at DESC LIMIT ")
        .push_bind(per_page as i64)
        .push(" OFFSET ")
        .push_bind(((page - 1) * per_page) as i64);

    let rows: Vec<sqlx::postgres::PgRow> = qb.build().fetch_all(&state.db).await?;
    let items = rows
        .iter()
        .map(|row| -> Result<BrowseItem, sqlx::Error> {
            Ok(BrowseItem {
                item_type: row.try_get("item_type")?,
                id: row.try_get("id")?,
                name: row.try_get("name")?,
                slug: row.try_get("slug")?,
                creator_id: row.try_get("creator_id")?,
                creator_name: row.try_get("creator_name")?,
                primary_image_id: row.try_get("primary_image_id")?,
                tags: row.try_get("tags")?,
                like_count: row.try_get("like_count")?,
                count: row.try_get("count")?,
                updated_at: row.try_get("updated_at")?,
            })
        })
        .collect::<Result<_, _>>()
        .context("decoding browse row")?;

    Ok(Json(BrowseResults {
        items,
        total,
        page,
        per_page,
    }))
}
