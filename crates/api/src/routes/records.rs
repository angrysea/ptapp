//! Personal records (LG-5; ptcore::records). Saving a log records any it
//! sets; the client's app then asks for this log's new records to
//! celebrate them (logs are saved through the offline outbox, so the save's
//! own response isn't seen by the screen).

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::Json;
use ptcore::model::WorkoutLog;
use ptcore::records::{new_records, PersonalRecord};
use serde::Deserialize;
use serde_json::{json, Value};

use ptcore::team::Perm;
use crate::auth::{client_scope, client_scope_as, Ctx};
use crate::error::{ApiError, ApiResult};
use crate::store::keyed;
use crate::AppState;

fn sk(key: &str) -> String {
    format!("PR#{key}")
}

/// Records whatever `log` sets. Called after the log is saved.
pub async fn record(st: &AppState, client_pk: &str, log: &WorkoutLog) -> ApiResult<Vec<PersonalRecord>> {
    if log.sets.iter().all(|s| !s.done) {
        return Ok(Vec::new());
    }
    let current: Vec<PersonalRecord> = st.store.query_prefix(client_pk, "PR#").await?;
    let current: HashMap<String, PersonalRecord> = current.into_iter().map(|r| (r.key.clone(), r)).collect();
    let new = new_records(log, &current);
    let items = new.iter().map(|r| keyed(client_pk, &sk(&r.key), r)).collect::<ApiResult<Vec<_>>>()?;
    st.store.batch_put(items).await?;
    Ok(new)
}

#[derive(Deserialize)]
pub struct RecordsQuery {
    /// Only the new bests (not baselines) this log set.
    log: Option<String>,
}

/// `GET /clients/{cid}/records[?log={id}]`: the client's current records,
/// newest first.
pub async fn list(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>, Query(q): Query<RecordsQuery>) -> ApiResult<Json<Vec<PersonalRecord>>> {
    let scope = client_scope_as(&st, &ctx, &cid, &[Perm::Progress, Perm::Programming]).await?;
    let mut all: Vec<PersonalRecord> = st.store.query_prefix(&scope.pk(), "PR#").await?;
    if let Some(log) = &q.log {
        all.retain(|r| &r.log_id == log && !r.first);
    }
    all.sort_by(|a, b| b.date.cmp(&a.date).then(a.key.cmp(&b.key)));
    Ok(Json(all))
}

/// `DELETE /clients/{cid}/records/{key}`: removes a bogus record (a typo'd
/// weight). The one before it, if any, becomes the record again.
pub async fn remove(State(st): State<AppState>, ctx: Ctx, Path((cid, key)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    let pk = scope.pk();
    let rec: PersonalRecord = st.store.get(&pk, &sk(&key)).await?.ok_or(ApiError::NotFound("record"))?;
    match rec.previous {
        Some(p) => {
            let restored = PersonalRecord { kg: p.kg, date: p.date, set: None, log_id: String::new(), first: false, previous: None, ..rec };
            st.store.put(&pk, &sk(&key), &restored).await?;
            Ok(Json(json!({ "restored": restored })))
        }
        None => {
            st.store.delete(&pk, &sk(&key)).await?;
            Ok(Json(json!({ "restored": null })))
        }
    }
}
