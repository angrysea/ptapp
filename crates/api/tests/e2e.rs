//! End-to-end API test against a local DynamoDB/S3 emulator.
//!
//!   docker compose up -d --wait    (moto on :5055, or DynamoDB Local + any S3 mock)
//!   E2E_ENDPOINT=http://localhost:5055 cargo test -p api --test e2e
//!
//! Skipped when E2E_ENDPOINT is unset. Uses the DEV_AUTH header instead of
//! Cognito, so it exercises routing, authorization and storage, not JWTs.

use aws_sdk_dynamodb::types::{AttributeDefinition, BillingMode, KeySchemaElement, KeyType, ScalarAttributeType};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

struct T {
    app: axum::Router,
}

impl T {
    async fn call(&self, method: &str, path: &str, user: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(path).header("content-type", "application/json");
        if let Some(u) = user {
            req = req.header("x-dev-user", u);
        }
        let req = req.body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty)).unwrap();
        let res = self.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 10 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }
}

#[tokio::test]
async fn full_client_journey() {
    let Ok(endpoint) = std::env::var("E2E_ENDPOINT") else {
        eprintln!("E2E_ENDPOINT not set; skipping");
        return;
    };
    // SAFETY: set before any other thread reads the environment.
    unsafe {
        std::env::set_var("AWS_ENDPOINT_URL", &endpoint);
        std::env::set_var("AWS_REGION", "us-east-1");
        std::env::set_var("AWS_ACCESS_KEY_ID", "test");
        std::env::set_var("AWS_SECRET_ACCESS_KEY", "test");
        std::env::set_var("DEV_AUTH", "1");
        std::env::set_var("TABLE_NAME", format!("e2e-{}", uuid::Uuid::new_v4().simple()));
        std::env::set_var("MEDIA_BUCKET", format!("e2e-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]));
    }
    let config = aws_config::from_env().load().await;
    let ddb = aws_sdk_dynamodb::Client::new(&config);
    let s3 = aws_sdk_s3::Client::from_conf(aws_sdk_s3::config::Builder::from(&config).force_path_style(true).build());
    let table = std::env::var("TABLE_NAME").unwrap();
    ddb.create_table()
        .table_name(&table)
        .billing_mode(BillingMode::PayPerRequest)
        .attribute_definitions(AttributeDefinition::builder().attribute_name("pk").attribute_type(ScalarAttributeType::S).build().unwrap())
        .attribute_definitions(AttributeDefinition::builder().attribute_name("sk").attribute_type(ScalarAttributeType::S).build().unwrap())
        .key_schema(KeySchemaElement::builder().attribute_name("pk").key_type(KeyType::Hash).build().unwrap())
        .key_schema(KeySchemaElement::builder().attribute_name("sk").key_type(KeyType::Range).build().unwrap())
        .send()
        .await
        .unwrap();
    s3.create_bucket().bucket(std::env::var("MEDIA_BUCKET").unwrap()).send().await.unwrap();

    let mut state = api::AppState::from_env().await;
    state.s3 = s3.clone();
    let t = T { app: api::router(state) };
    let today = chrono::Utc::now().date_naive();

    // --- health + auth
    assert_eq!(t.call("GET", "/health", None, None).await.0, StatusCode::OK);
    assert_eq!(t.call("GET", "/me", None, None).await.0, StatusCode::UNAUTHORIZED);
    let (s, me) = t.call("GET", "/me", Some("coach1"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(me["profile"].is_null());
    assert_eq!(t.call("GET", "/clients", Some("coach1"), None).await.0, StatusCode::FORBIDDEN);

    // --- coach onboarding + invite (ON-1)
    let (s, p) = t.call("POST", "/coach/register", Some("coach1"), Some(json!({ "name": "Coach One" }))).await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(p["role"], "coach");
    assert_eq!(t.call("POST", "/coach/register", Some("coach1"), Some(json!({ "name": "x" }))).await.0, StatusCode::CONFLICT);

    let (s, inv) = t.call("POST", "/invites", Some("coach1"), Some(json!({ "email": "c@x.com", "clientName": "Cli Ent" }))).await;
    assert_eq!(s, StatusCode::OK, "{inv}");
    let code = inv["code"].as_str().unwrap().to_string();
    let (_, pending) = t.call("GET", "/invites", Some("coach1"), None).await;
    assert_eq!(pending.as_array().unwrap().len(), 1);

    let (s, peek) = t.call("GET", &format!("/invites/{code}"), Some("client1"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(peek["coachName"], "Coach One");
    let (s, cp) = t.call("POST", &format!("/invites/{code}/accept"), Some("client1"), Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{cp}");
    assert_eq!(cp["role"], "client");
    assert_eq!(cp["coachId"], "coach1");
    // single use
    assert_eq!(t.call("POST", &format!("/invites/{code}/accept"), Some("client2"), Some(json!({}))).await.0, StatusCode::NOT_FOUND);
    let (_, pending) = t.call("GET", "/invites", Some("coach1"), None).await;
    assert_eq!(pending.as_array().unwrap().len(), 0);

    // revoke normalizes case: a lowercase code must actually revoke the invite
    let (_, inv2) = t.call("POST", "/invites", Some("coach1"), Some(json!({ "email": "d@x.com", "clientName": "Other" }))).await;
    let code2 = inv2["code"].as_str().unwrap().to_lowercase();
    assert_eq!(t.call("DELETE", &format!("/invites/{code2}"), Some("coach1"), None).await.0, StatusCode::OK);
    assert_eq!(t.call("GET", &format!("/invites/{code2}"), Some("client2"), None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(t.call("GET", "/invites", Some("coach1"), None).await.1.as_array().unwrap().len(), 0);

    // --- intake + waiver (ON-2, ON-3)
    let (s, _) = t
        .call("PUT", "/clients/client1/intake", Some("client1"), Some(json!({
            "goals": "Squat 140", "equipment": ["barbell"], "sex": "male", "birthDate": "1996-01-15", "heightCm": 180
        })))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(t.call("PUT", "/clients/client1/intake", Some("coach1"), Some(json!({}))).await.0, StatusCode::FORBIDDEN);
    let (s, w) = t
        .call("POST", "/clients/client1/waiver", Some("client1"), Some(json!({
            "waiverVersion": "2026-10", "signedName": "Cli Ent", "agreed": true,
            "parq": [{ "question": "Heart condition?", "answer": false }, { "question": "Chest pain?", "answer": true }]
        })))
        .await;
    assert_eq!(s, StatusCode::OK, "{w}");
    assert_eq!(w["parqFlagged"], true);

    // --- exercises (WB-1)
    let (s, ex) = t.call("POST", "/exercises", Some("coach1"), Some(json!({ "name": "Zercher Squat", "cues": ["Hug the bar"] }))).await;
    assert_eq!(s, StatusCode::OK);
    let custom_id = ex["id"].as_str().unwrap().to_string();
    let (_, lib) = t.call("GET", "/exercises", Some("client1"), None).await;
    let lib = lib.as_array().unwrap();
    assert!(lib.iter().any(|e| e["id"] == "back-squat"));
    assert!(lib.iter().any(|e| e["id"] == custom_id.as_str()));
    assert_eq!(t.call("PUT", "/exercises/back-squat", Some("coach1"), Some(json!({ "name": "x" }))).await.0, StatusCode::FORBIDDEN);
    let (s, up) = t.call("POST", &format!("/exercises/{custom_id}/demo-upload"), Some("coach1"), Some(json!({ "contentType": "video/mp4" }))).await;
    assert_eq!(s, StatusCode::OK, "{up}");
    assert!(up["uploadUrl"].as_str().unwrap().contains("X-Amz-Signature"));

    // --- program + assignment (WB-2..4)
    let item = |id: &str| json!({ "exerciseId": "back-squat", "exerciseName": "Back Squat", "sets": 3, "reps": "5", "loadKg": 100.0, "restSec": 120, "rpe": 8, "id": id });
    let program = json!({
        "name": "Base Strength", "weeks": 2, "isTemplate": true,
        "workouts": [
            { "week": 2, "day": 3, "workout": { "name": "B", "blocks": [{ "kind": "straight", "items": [item("")] }] } },
            { "week": 1, "day": 1, "workout": { "name": "A", "blocks": [
                { "kind": "straight", "items": [item("")] },
                { "kind": "emom", "durationSec": 600, "items": [{ "exerciseId": "burpee", "exerciseName": "Burpee", "sets": 10, "reps": "8" }] }
            ] } }
        ]
    });
    let (s, prog) = t.call("POST", "/programs", Some("coach1"), Some(program.clone())).await;
    assert_eq!(s, StatusCode::OK, "{prog}");
    assert_eq!(prog["workouts"][0]["workout"]["name"], "A", "sorted by week/day");
    assert!(!prog["workouts"][0]["workout"]["blocks"][0]["items"][0]["id"].as_str().unwrap().is_empty());
    let pid = prog["id"].as_str().unwrap().to_string();
    let mut bad = program.clone();
    bad["weeks"] = json!(1);
    assert_eq!(t.call("POST", "/programs", Some("coach1"), Some(bad)).await.0, StatusCode::BAD_REQUEST);
    let (s, copy) = t.call("POST", &format!("/programs/{pid}/copy"), Some("coach1"), Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(copy["name"], "Base Strength (copy)");
    assert_eq!(t.call("GET", "/programs", Some("client1"), None).await.0, StatusCode::FORBIDDEN);

    let start = today - chrono::Duration::days(1);
    let (s, asg) = t
        .call("POST", "/clients/client1/assignments", Some("coach1"), Some(json!({ "programId": pid, "startDate": start.to_string() })))
        .await;
    assert_eq!(s, StatusCode::OK, "{asg}");
    let (s, sched) = t.call("GET", "/clients/client1/schedule", Some("client1"), None).await;
    assert_eq!(s, StatusCode::OK);
    let sched = sched.as_array().unwrap();
    assert_eq!(sched.len(), 2);
    assert_eq!(sched[0]["date"], start.to_string());
    let first = &sched[0];
    let item_id = first["workout"]["blocks"][0]["items"][0]["id"].as_str().unwrap();
    // overlapping assignment is refused (would double-schedule the same dates)
    assert_eq!(t.call("POST", "/clients/client1/assignments", Some("coach1"), Some(json!({ "programId": pid, "startDate": start.to_string() }))).await.0, StatusCode::BAD_REQUEST);

    // --- logging with last-write-wins (LG-1, LG-3, LG-4, FB-2)
    let log = |ts: &str, reps: u32| json!({
        "id": "log-abc", "scheduledId": first["id"], "date": start.to_string(), "workoutName": "A",
        "status": "completed", "updatedAt": ts, "comment": "felt good",
        "sets": ([0, 1, 2].iter().map(|i| json!({ "itemId": item_id, "exerciseId": "back-squat", "exerciseName": "Back Squat",
            "setIndex": i, "reps": reps, "loadKg": 100.0, "rpe": 8, "done": true })).collect::<Vec<_>>()),
        "check": { "rpe": 8, "energy": 4, "sleep": 3, "soreness": 2, "pain": true, "painLocation": "left knee" }
    });
    let now = chrono::Utc::now();
    let newer = (now - chrono::Duration::minutes(1)).to_rfc3339();
    let older = (now - chrono::Duration::minutes(10)).to_rfc3339();
    let (s, l) = t.call("PUT", "/clients/client1/logs/log-abc", Some("client1"), Some(log(&newer, 5))).await;
    assert_eq!(s, StatusCode::OK, "{l}");
    let (s, conflict) = t.call("PUT", "/clients/client1/logs/log-abc", Some("client1"), Some(log(&older, 3))).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(conflict["current"]["sets"][0]["reps"], 5);
    let (_, logs) = t.call("GET", "/clients/client1/logs", Some("coach1"), None).await;
    assert_eq!(logs.as_array().unwrap().len(), 1);
    // same log id replayed with an edited date replaces the old-dated copy
    let mut moved = log(&now.to_rfc3339(), 5);
    moved["date"] = json!(today.to_string());
    assert_eq!(t.call("PUT", "/clients/client1/logs/log-abc", Some("client1"), Some(moved)).await.0, StatusCode::OK);
    let (_, logs) = t.call("GET", "/clients/client1/logs", Some("coach1"), None).await;
    assert_eq!(logs.as_array().unwrap().len(), 1);
    // ...but an older replay still carrying the original date must not undo the move
    let (s, conflict) = t.call("PUT", "/clients/client1/logs/log-abc", Some("client1"), Some(log(&older, 3))).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(conflict["current"]["date"], today.to_string());
    let (_, logs) = t.call("GET", "/clients/client1/logs", Some("coach1"), None).await;
    assert_eq!(logs.as_array().unwrap().len(), 1);
    assert_eq!(logs[0]["date"], today.to_string());

    // --- dashboard + roster (LG-6)
    let (s, dash) = t.call("GET", "/clients/client1/dashboard", Some("coach1"), None).await;
    assert_eq!(s, StatusCode::OK, "{dash}");
    assert_eq!(dash["adherence"]["due"], 1);
    assert_eq!(dash["adherence"]["pct"], 100.0);
    assert_eq!(dash["weeklyVolume"].as_array().unwrap().iter().map(|w| w["volumeKg"].as_f64().unwrap()).sum::<f64>(), 1500.0);
    assert_eq!(dash["strength"][0]["bestE1rmKg"], 116.7);
    let (_, roster) = t.call("GET", "/clients", Some("coach1"), None).await;
    assert_eq!(roster[0]["painFlag"], true);
    assert_eq!(roster[0]["waiverSigned"], true);
    assert_eq!(roster[0]["parqFlagged"], true);
    assert_eq!(roster[0]["hasUpcoming"], true, "workout B is 9 days out, none today");

    // --- body metrics (BM-1..4)
    let (s, m) = t
        .call("POST", "/clients/client1/metrics", Some("client1"), Some(json!({ "date": today.to_string(), "kind": "weight", "weightKg": 82.5 })))
        .await;
    assert_eq!(s, StatusCode::OK, "{m}");
    let (s, sf) = t
        .call("POST", "/clients/client1/metrics", Some("coach1"), Some(json!({
            "date": today.to_string(), "kind": "skinfold",
            "skinfold": { "method": "jacksonPollock3", "equation": "siri", "bodyWeightKg": 82.5,
                          "sitesMm": { "chest": [15, 15.4], "abdomen": [25], "thigh": [20] } }
        })))
        .await;
    assert_eq!(s, StatusCode::OK, "{sf}");
    assert_eq!(sf["takenBy"], "coach");
    assert!(sf["skinfold"]["result"]["bodyFatPct"].as_f64().unwrap() > 10.0);
    let (_, ms) = t.call("GET", "/clients/client1/metrics?kind=skinfold", Some("client1"), None).await;
    assert_eq!(ms.as_array().unwrap().len(), 1);
    assert_eq!(ms[0]["skinfold"]["sitesMm"]["chest"][1], 15.4);
    let (s, _) = t
        .call("POST", "/clients/client1/metrics", Some("client1"), Some(json!({ "date": today.to_string(), "kind": "tape", "tapeCm": { "elbow": 30 } })))
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // skinfold body weight is range-checked like a plain weight reading
    assert_eq!(t
        .call("POST", "/clients/client1/metrics", Some("client1"), Some(json!({
            "date": today.to_string(), "kind": "skinfold",
            "skinfold": { "method": "jacksonPollock3", "equation": "siri", "bodyWeightKg": 5000,
                          "sitesMm": { "chest": [15], "abdomen": [25], "thigh": [20] } }
        })))
        .await.0, StatusCode::BAD_REQUEST);

    // --- photos (BM-7)
    let (s, ph) = t
        .call("POST", "/clients/client1/photos", Some("client1"), Some(json!({ "date": today.to_string(), "pose": "front", "contentType": "image/jpeg" })))
        .await;
    assert_eq!(s, StatusCode::OK, "{ph}");
    assert!(ph["photo"]["key"].as_str().unwrap().starts_with("media/coach1/client1/photos/"));
    // stand in for the browser's presigned PUT; listing drops records with no object
    s3.put_object()
        .bucket(std::env::var("MEDIA_BUCKET").unwrap())
        .key(ph["photo"]["key"].as_str().unwrap())
        .body(b"jpeg".to_vec().into())
        .send()
        .await
        .unwrap();
    let (_, photos) = t.call("GET", "/clients/client1/photos", Some("coach1"), None).await;
    assert!(photos[0]["url"].as_str().unwrap().contains("X-Amz-Signature"));

    // --- comments (FB-1)
    let (s, _) = t
        .call("POST", "/clients/client1/comments", Some("coach1"), Some(json!({ "text": "Great depth on set 2", "logId": "log-abc", "setIndex": 1 })))
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, cs) = t.call("GET", "/clients/client1/comments", Some("client1"), None).await;
    assert_eq!(cs[0]["authorRole"], "coach");

    // --- tenant isolation
    t.call("POST", "/coach/register", Some("coach2"), Some(json!({ "name": "Coach Two" }))).await;
    for path in ["/clients/client1/metrics", "/clients/client1/photos", "/clients/client1/logs", "/clients/client1"] {
        assert_eq!(t.call("GET", path, Some("coach2"), None).await.0, StatusCode::FORBIDDEN, "{path}");
    }
    let (_, r2) = t.call("GET", "/clients", Some("coach2"), None).await;
    assert_eq!(r2.as_array().unwrap().len(), 0);
    let (_, lib2) = t.call("GET", "/exercises", Some("coach2"), None).await;
    assert!(!lib2.as_array().unwrap().iter().any(|e| e["id"] == custom_id.as_str()));
    assert_eq!(t.call("GET", "/clients/coach1/metrics", Some("client1"), None).await.0, StatusCode::FORBIDDEN);

    // --- unassign keeps logged + past workouts, removes the future one
    let (s, un) = t
        .call("DELETE", &format!("/clients/client1/assignments/{}", asg["id"].as_str().unwrap()), Some("coach1"), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(un["removed"], 1);

    // --- audit trail of coach access
    let audits: Vec<Value> = {
        let out = ddb
            .query()
            .table_name(&table)
            .key_condition_expression("pk = :pk AND begins_with(sk, :a)")
            .expression_attribute_values(":pk", aws_sdk_dynamodb::types::AttributeValue::S("CLIENT#client1".into()))
            .expression_attribute_values(":a", aws_sdk_dynamodb::types::AttributeValue::S("AUDIT#".into()))
            .send()
            .await
            .unwrap();
        out.items.unwrap_or_default().into_iter().map(|i| serde_dynamo::aws_sdk_dynamodb_1::from_item(i).unwrap()).collect()
    };
    assert!(audits.iter().any(|a| a["action"] == "view-photos" && a["actorId"] == "coach1"));
}
