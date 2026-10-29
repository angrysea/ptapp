//! Body metrics (BM-1..BM-4, BM-8) and private progress photos (BM-7).

use std::collections::BTreeMap;

use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::{Duration, NaiveDate, Utc};
use ptcore::bodycomp::{compute, navy_body_fat, BodyFatReading, BodyFatSource, DensityEquation, NavyTape, SkinfoldInput, SkinfoldMethod};
use ptcore::model::{Metric, MetricKind, Photo, PhotoPose, Role, SkinfoldRecord, TAPE_SITES};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::auth::{client_scope, Ctx};
use crate::error::{bad_request, ApiError, ApiResult};
use crate::util::{audit, check_size, dated_id, ext_for, presign_get, presign_put, s3_object_exists, untag, IMAGE_TYPES, MAX_PHOTO_BYTES, PENDING_TAG};
use crate::AppState;

#[derive(Deserialize)]
pub struct KindFilter {
    kind: Option<MetricKind>,
}

pub async fn list(
    State(st): State<AppState>,
    ctx: Ctx,
    Path(cid): Path<String>,
    Query(f): Query<KindFilter>,
) -> ApiResult<Json<Vec<Metric>>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    audit(&st, &ctx, &scope, "view-metrics").await;
    let all: Vec<Metric> = st.store.query_prefix(&scope.pk(), "METRIC#").await?;
    Ok(Json(all.into_iter().filter(|m| f.kind.is_none_or(|k| m.kind == k)).collect()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinfoldIn {
    method: SkinfoldMethod,
    equation: DensityEquation,
    sites_mm: BTreeMap<String, Vec<f64>>,
    body_weight_kg: Option<f64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricIn {
    date: NaiveDate,
    kind: MetricKind,
    weight_kg: Option<f64>,
    tape_cm: Option<BTreeMap<String, f64>>,
    skinfold: Option<SkinfoldIn>,
    body_fat: Option<BodyFatIn>,
    #[serde(default)]
    notes: String,
}

/// A body-fat reading: tape measurements for the Navy formula, or a
/// percentage from a scan or scale.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BodyFatIn {
    source: BodyFatSource,
    pct: Option<f64>,
    navy: Option<NavyTape>,
}

pub async fn create(
    State(st): State<AppState>,
    ctx: Ctx,
    Path(cid): Path<String>,
    Json(body): Json<MetricIn>,
) -> ApiResult<Json<Metric>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    let mut m = Metric {
        id: dated_id(body.date),
        date: body.date,
        kind: body.kind,
        taken_by: scope.actor_role(),
        weight_kg: None,
        tape_cm: None,
        skinfold: None,
        body_fat: None,
        notes: body.notes,
        created_at: Utc::now(),
    };
    match body.kind {
        MetricKind::Weight => {
            let w = body.weight_kg.filter(|w| (20.0..=400.0).contains(w)).ok_or_else(|| bad_request("weight must be 20-400 kg"))?;
            m.weight_kg = Some(w);
        }
        MetricKind::Tape => {
            let tape = body.tape_cm.unwrap_or_default();
            if tape.is_empty() {
                return Err(bad_request("enter at least one measurement"));
            }
            for (site, v) in &tape {
                if !TAPE_SITES.contains(&site.as_str()) && !site.starts_with("custom:") {
                    return Err(bad_request(format!("unknown tape site {site}")));
                }
                if !(5.0..=300.0).contains(v) {
                    return Err(bad_request(format!("{site} must be 5-300 cm")));
                }
            }
            m.tape_cm = Some(tape);
        }
        MetricKind::Skinfold => {
            let sf = body.skinfold.ok_or_else(|| bad_request("skinfold readings are required"))?;
            if sf.body_weight_kg.is_some_and(|w| !(20.0..=400.0).contains(&w)) {
                return Err(bad_request("body weight must be 20-400 kg"));
            }
            let sex = scope.client.sex.ok_or_else(|| bad_request("set sex in the intake form first"))?;
            let age = scope
                .client
                .age_on(body.date)
                .ok_or_else(|| bad_request("set birth date in the intake form first"))?;
            let result = compute(&SkinfoldInput {
                method: sf.method,
                equation: sf.equation,
                sex,
                age_years: age,
                body_weight_kg: sf.body_weight_kg,
                sites_mm: &sf.sites_mm,
            })
            .map_err(|e| bad_request(e.to_string()))?;
            m.weight_kg = sf.body_weight_kg;
            m.skinfold = Some(SkinfoldRecord {
                method: sf.method,
                equation: sf.equation,
                sites_mm: sf.sites_mm,
                sex,
                age_years: (age * 10.0).round() / 10.0,
                body_weight_kg: sf.body_weight_kg,
                result,
            });
        }
        MetricKind::BodyFat => {
            let bf = body.body_fat.ok_or_else(|| bad_request("a body-fat reading is required"))?;
            m.body_fat = Some(match bf.source {
                BodyFatSource::Navy => {
                    let tape = bf.navy.ok_or_else(|| bad_request("the Navy estimate needs height, neck and waist (and hips for women)"))?;
                    let sex = scope.client.sex.ok_or_else(|| bad_request("set sex in the intake form first"))?;
                    let pct = navy_body_fat(sex, &tape).map_err(bad_request)?;
                    BodyFatReading { source: bf.source, pct, navy: Some(tape) }
                }
                _ => {
                    let pct = bf.pct.filter(|p| (2.0..=70.0).contains(p)).ok_or_else(|| bad_request("body fat must be 2-70%"))?;
                    BodyFatReading { source: bf.source, pct: (pct * 10.0).round() / 10.0, navy: None }
                }
            });
        }
    }
    st.store.put(&scope.pk(), &format!("METRIC#{}", m.id), &m).await?;
    Ok(Json(m))
}

pub async fn remove(
    State(st): State<AppState>,
    ctx: Ctx,
    Path((cid, mid)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    let sk = format!("METRIC#{mid}");
    let m: Metric = st.store.get(&scope.pk(), &sk).await?.ok_or(ApiError::NotFound("metric"))?;
    if m.taken_by != scope.actor_role() {
        return Err(ApiError::Forbidden("only whoever recorded an entry can delete it"));
    }
    st.store.delete(&scope.pk(), &sk).await?;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- photos

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoOut {
    #[serde(flatten)]
    photo: Photo,
    url: String,
}

pub async fn list_photos(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>) -> ApiResult<Json<Vec<PhotoOut>>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    audit(&st, &ctx, &scope, "view-photos").await;
    let photos: Vec<Photo> = st.store.query_prefix(&scope.pk(), "PHOTO#").await?;
    // Only confirmed photos are shown, so no per-photo S3 check is needed. A
    // pending one may still be uploading; once its presigned URL has expired
    // it was abandoned (closed tab, lost upload): the record is dropped and
    // the bucket's lifecycle rule removes any object, still tagged pending.
    let abandoned_before = Utc::now() - Duration::from_std(st.presign_ttl).unwrap_or(Duration::hours(1)) - Duration::minutes(5);
    let mut out = Vec::with_capacity(photos.len());
    for p in photos {
        if p.pending {
            if p.created_at < abandoned_before {
                st.store.delete(&scope.pk(), &format!("PHOTO#{}", p.id)).await?;
            }
            continue;
        }
        let url = presign_get(&st, &p.key).await?;
        out.push(PhotoOut { photo: p, url });
    }
    Ok(Json(out))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoIn {
    date: NaiveDate,
    pose: PhotoPose,
    content_type: String,
    size: i64,
}

/// Step 1: creates a pending photo record and returns a short-lived
/// presigned PUT URL. The bucket is private; photos are only ever served via
/// presigned GETs.
pub async fn create_photo(
    State(st): State<AppState>,
    ctx: Ctx,
    Path(cid): Path<String>,
    Json(body): Json<PhotoIn>,
) -> ApiResult<Json<Value>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    let ext = ext_for(&body.content_type, IMAGE_TYPES)?;
    check_size(body.size, MAX_PHOTO_BYTES)?;
    let id = dated_id(body.date);
    let key = format!("media/{}/{}/photos/{id}.{ext}", scope.client.coach_id, scope.client.user_id);
    let photo = Photo {
        id,
        date: body.date,
        pose: body.pose,
        key,
        content_type: body.content_type,
        taken_by: Some(scope.actor_role()),
        pending: true,
        created_at: Utc::now(),
    };
    let upload = presign_put(&st, &photo.key, &photo.content_type, body.size, Some(PENDING_TAG)).await?;
    st.store.put(&scope.pk(), &format!("PHOTO#{}", photo.id), &photo).await?;
    Ok(Json(json!({ "photo": photo, "uploadUrl": upload.url, "uploadHeaders": upload.headers })))
}

/// Step 2: once the browser's PUT has succeeded, the photo is shown.
pub async fn confirm_photo(
    State(st): State<AppState>,
    ctx: Ctx,
    Path((cid, pid)): Path<(String, String)>,
) -> ApiResult<Json<Photo>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    let sk = format!("PHOTO#{pid}");
    let mut photo: Photo = st.store.get(&scope.pk(), &sk).await?.ok_or(ApiError::NotFound("photo"))?;
    if !photo.pending {
        return Ok(Json(photo));
    }
    if !s3_object_exists(&st, &photo.key).await? {
        return Err(bad_request("the photo upload has not arrived; try uploading again"));
    }
    // Untag first: a confirmed photo must not be expired by the lifecycle rule.
    untag(&st, &photo.key).await?;
    photo.pending = false;
    st.store.put(&scope.pk(), &sk, &photo).await?;
    Ok(Json(photo))
}

pub async fn remove_photo(
    State(st): State<AppState>,
    ctx: Ctx,
    Path((cid, pid)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    let photo: Photo = st
        .store
        .get(&scope.pk(), &format!("PHOTO#{pid}"))
        .await?
        .ok_or(ApiError::NotFound("photo"))?;
    // Clients control their own photos; a coach may only remove ones they took.
    if scope.by_coach && photo.taken_by != Some(Role::Coach) {
        return Err(ApiError::Forbidden("only the client can delete photos they uploaded"));
    }
    st.s3
        .delete_object()
        .bucket(&st.bucket)
        .key(&photo.key)
        .send()
        .await
        .map_err(|e| ApiError::Internal(format!("s3 delete: {e}")))?;
    st.store.delete(&scope.pk(), &format!("PHOTO#{pid}")).await?;
    Ok(Json(json!({ "ok": true })))
}
