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
        std::env::set_var("ADMIN_EMAILS", "Admin1@dev.local, other@example.com");
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

    // --- admin bootstrap: only emails on ADMIN_EMAILS may set up an admin account
    assert_eq!(me["adminEligible"], false);
    let (_, me_admin) = t.call("GET", "/me", Some("admin1"), None).await;
    assert_eq!(me_admin["adminEligible"], true);
    assert_eq!(t.call("POST", "/admin/register", Some("coach1"), Some(json!({ "name": "x" }))).await.0, StatusCode::FORBIDDEN);
    let (s, a) = t.call("POST", "/admin/register", Some("admin1"), Some(json!({ "name": "Ada Admin" }))).await;
    assert_eq!(s, StatusCode::OK, "{a}");
    assert_eq!(a["role"], "admin");
    assert_eq!(t.call("POST", "/admin/register", Some("admin1"), Some(json!({ "name": "x" }))).await.0, StatusCode::CONFLICT);

    // --- no open coach signup: coaches only come from an admin's emailed invitation
    assert_eq!(t.call("POST", "/coach/register", Some("coach1"), Some(json!({ "name": "x" }))).await.0, StatusCode::NOT_FOUND);
    assert_eq!(t.call("PUT", "/coach-request", Some("coach1"), Some(json!({ "name": "x" }))).await.0, StatusCode::NOT_FOUND);
    assert_eq!(t.call("GET", "/admin/requests", Some("admin1"), None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(t.call("POST", "/admin/invites", Some("coach1"), Some(json!({ "email": "coach1@dev.local", "name": "x" }))).await.0, StatusCode::FORBIDDEN);
    let (s, ci) = t.call("POST", "/admin/invites", Some("admin1"), Some(json!({ "email": "coach1@dev.local", "name": "Coach One" }))).await;
    assert_eq!(s, StatusCode::OK, "{ci}");
    assert_eq!(ci["kind"], "coach");
    // no INVITE_FROM here, so nothing is emailed and the admin page offers the link instead
    assert_eq!(ci["emailed"], false);
    let coach_code = ci["code"].as_str().unwrap().to_string();
    // the sign-up page reads the invitation without signing in, to fill in its email
    let (s, pubinv) = t.call("GET", &format!("/invites/{}/public", coach_code.to_lowercase()), None, None).await;
    assert_eq!(s, StatusCode::OK, "{pubinv}");
    assert_eq!(pubinv["email"], "coach1@dev.local");
    assert_eq!(pubinv["kind"], "coach");
    assert_eq!(t.call("GET", "/invites/NOSUCHCODE/public", None, None).await.0, StatusCode::NOT_FOUND);
    let (s, resent) = t.call("POST", &format!("/admin/invites/{coach_code}/resend"), Some("admin1"), None).await;
    assert_eq!(s, StatusCode::OK, "{resent}");
    assert_eq!(resent["code"], coach_code.as_str());
    assert_eq!(t.call("POST", &format!("/admin/invites/{coach_code}/resend"), Some("coach1"), None).await.0, StatusCode::FORBIDDEN);
    let (_, peek) = t.call("GET", &format!("/invites/{coach_code}"), Some("coach1"), None).await;
    assert_eq!(peek["kind"], "coach");
    assert_eq!(t.call("POST", &format!("/invites/{coach_code}/accept"), Some("coach9"), Some(json!({}))).await.0, StatusCode::FORBIDDEN);
    let (s, p) = t.call("POST", &format!("/invites/{coach_code}/accept"), Some("coach1"), Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(p["role"], "coach");
    assert_eq!(p["coachId"], "coach1");
    assert_eq!(p["name"], "Coach One");
    let (_, admin_invites) = t.call("GET", "/admin/invites", Some("admin1"), None).await;
    assert_eq!(admin_invites.as_array().unwrap().len(), 0);
    // once used, the link stops working: the sign-up page's lookup and resend both refuse it
    assert_eq!(t.call("GET", &format!("/invites/{coach_code}/public"), None, None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(t.call("POST", &format!("/admin/invites/{coach_code}/resend"), Some("admin1"), None).await.0, StatusCode::NOT_FOUND);
    // a second coach the same way
    let (_, ci2) = t.call("POST", "/admin/invites", Some("admin1"), Some(json!({ "email": "coach2@dev.local", "name": "Coach Two" }))).await;
    let (s, c2) = t.call("POST", &format!("/invites/{}/accept", ci2["code"].as_str().unwrap()), Some("coach2"), Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{c2}");
    assert_eq!(c2["role"], "coach");
    let (_, coaches) = t.call("GET", "/admin/coaches", Some("admin1"), None).await;
    let names: Vec<&str> = coaches.as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Coach One", "Coach Two"]);
    // admins see no coach or client data
    assert_eq!(t.call("GET", "/clients", Some("admin1"), None).await.0, StatusCode::FORBIDDEN);

    // --- coach profiles and the public directory (tasks/coaches)
    // password changes go through the API (Cognito isn't configured locally)
    let (s, e) = t.call("POST", "/me/password", Some("coach1"), Some(json!({ "current": "old-pass-1", "new": "new-pass-1" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{e}");
    assert_eq!(t.call("POST", "/me/password", None, Some(json!({ "current": "a", "new": "b" }))).await.0, StatusCode::UNAUTHORIZED);
    let (_, me) = t.call("GET", "/me", Some("coach1"), None).await;
    assert_eq!(me["coachProfileComplete"], false);
    let (_, mine) = t.call("GET", "/me/coach-profile", Some("coach1"), None).await;
    assert_eq!(mine["profile"], Value::Null);
    let mut cp = json!({
        "legalName": "Jordan Q. One", "phone": "(512) 555-0100",
        "address": { "line1": "1 Main St", "city": "Austin", "state": "TX", "postalCode": "78701", "country": "US" },
        "cprExpires": "2027-05-01", "insurance": null,
        "displayName": "Coach One", "headline": "Strength for busy parents",
        "bio": "I help busy parents get strong in three short sessions a week, with plans that fit around real life.",
        "photoKey": null, "specialties": ["Strength training"],
        "certifications": [{ "name": "CSCS", "issuer": "NSCA", "number": "SECRET-123", "expires": null }],
        "yearsExperience": 8, "formats": ["inPerson", "online"], "languages": ["English"], "rates": "From $60/session",
        "contactEmail": "Coach1@Example.com", "website": "", "instagram": "", "acceptingClients": true, "listed": true
    });
    // incomplete profiles are refused with what's missing
    let mut short = cp.clone();
    short["bio"] = json!("Too short");
    short["phone"] = json!("555");
    let (s, e) = t.call("PUT", "/me/coach-profile", Some("coach1"), Some(short)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let msg = e["error"].as_str().unwrap_or_default();
    assert!(msg.contains("Bio") && msg.contains("Phone"), "{e}");
    // a photo key must be the coach's own
    let mut theirs = cp.clone();
    theirs["photoKey"] = json!("media/coach2/profile/x.jpg");
    assert_eq!(t.call("PUT", "/me/coach-profile", Some("coach1"), Some(theirs)).await.0, StatusCode::BAD_REQUEST);
    let (s, up) = t.call("POST", "/me/coach-profile/photo", Some("coach1"), Some(json!({ "contentType": "image/jpeg", "size": 1000 }))).await;
    assert_eq!(s, StatusCode::OK, "{up}");
    cp["photoKey"] = up["photoKey"].clone();
    let (s, saved) = t.call("PUT", "/me/coach-profile", Some("coach1"), Some(cp.clone())).await;
    assert_eq!(s, StatusCode::OK, "{saved}");
    assert_eq!(saved["listed"], true);
    assert_eq!(saved["profile"]["contactEmail"], "coach1@example.com");
    assert_eq!(t.call("GET", "/me", Some("coach1"), None).await.1["coachProfileComplete"], true);
    // a new display name reaches the admin's list too
    let mut renamed = cp.clone();
    renamed["displayName"] = json!("Coach Uno");
    t.call("PUT", "/me/coach-profile", Some("coach1"), Some(renamed)).await;
    let (_, coaches) = t.call("GET", "/admin/coaches", Some("admin1"), None).await;
    assert!(coaches.as_array().unwrap().iter().any(|c| c["name"] == "Coach Uno"), "{coaches}");
    t.call("PUT", "/me/coach-profile", Some("coach1"), Some(cp.clone())).await;
    // only coaches have one
    assert_eq!(t.call("PUT", "/me/coach-profile", Some("admin1"), Some(cp.clone())).await.0, StatusCode::FORBIDDEN);

    // anyone, signed in or not, sees the public copy, and nothing private
    let (s, list) = t.call("GET", "/public/coaches", None, None).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    assert_eq!(list.as_array().unwrap().len(), 1);
    let (s, page) = t.call("GET", "/public/coaches/coach1", None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!((page["displayName"].as_str(), page["city"].as_str()), (Some("Coach One"), Some("Austin")));
    assert!(page["photoUrl"].as_str().is_some_and(|u| u.contains("profile")), "{page}");
    let public = format!("{list}{page}");
    for private in ["Jordan Q. One", "555-0100", "1 Main St", "78701", "SECRET-123", "cprExpires", "legalName"] {
        assert!(!public.contains(private), "{private} leaked into the directory: {public}");
    }
    assert_eq!(t.call("GET", "/public/coaches/coach2", None, None).await.0, StatusCode::NOT_FOUND);
    // an admin sees everything and can hide a coach; the coach's saves keep it hidden
    let (s, full) = t.call("GET", "/admin/coaches/coach1", Some("admin1"), None).await;
    assert_eq!(s, StatusCode::OK, "{full}");
    assert_eq!(full["profile"]["legalName"], "Jordan Q. One");
    assert_eq!(t.call("GET", "/admin/coaches/coach1", Some("coach2"), None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(t.call("PUT", "/admin/coaches/coach1/hidden", Some("admin1"), Some(json!({ "hidden": true }))).await.1["listed"], false);
    t.call("PUT", "/me/coach-profile", Some("coach1"), Some(cp.clone())).await;
    assert_eq!(t.call("GET", "/public/coaches", None, None).await.1.as_array().unwrap().len(), 0);
    assert_eq!(t.call("PUT", "/admin/coaches/coach1/hidden", Some("admin1"), Some(json!({ "hidden": false }))).await.1["listed"], true);
    // unlisting takes the coach out of the directory
    let mut unlisted = cp.clone();
    unlisted["listed"] = json!(false);
    t.call("PUT", "/me/coach-profile", Some("coach1"), Some(unlisted)).await;
    assert_eq!(t.call("GET", "/public/coaches/coach1", None, None).await.0, StatusCode::NOT_FOUND);
    t.call("PUT", "/me/coach-profile", Some("coach1"), Some(cp)).await;

    // --- client invite (ON-1)
    // the device's time zone is stored so "today" is the user's local date
    assert_eq!(t.call("PUT", "/me", Some("coach1"), Some(json!({ "timeZone": "Not/AZone" }))).await.0, StatusCode::BAD_REQUEST);
    let (s, p) = t.call("PUT", "/me", Some("coach1"), Some(json!({ "timeZone": "America/Chicago" }))).await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(p["timeZone"], "America/Chicago");

    let (s, inv) = t.call("POST", "/invites", Some("coach1"), Some(json!({ "email": "Client1@dev.local", "clientName": "Cli Ent" }))).await;
    assert_eq!(s, StatusCode::OK, "{inv}");
    let code = inv["code"].as_str().unwrap().to_string();
    let (_, pending) = t.call("GET", "/invites", Some("coach1"), None).await;
    assert_eq!(pending.as_array().unwrap().len(), 1);

    let (s, peek) = t.call("GET", &format!("/invites/{code}"), Some("client1"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(peek["coachName"], "Coach One");
    // only the invited address can accept (dev users are {id}@dev.local)
    assert_eq!(t.call("POST", &format!("/invites/{code}/accept"), Some("client2"), Some(json!({}))).await.0, StatusCode::FORBIDDEN);
    let (s, cp) = t.call("POST", &format!("/invites/{code}/accept"), Some("client1"), Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{cp}");
    assert_eq!(cp["role"], "client");
    assert_eq!(cp["coachId"], "coach1");
    // the client's header shows their coach
    let (_, me) = t.call("GET", "/me", Some("client1"), None).await;
    assert_eq!(me["coach"]["name"], "Coach One");
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
    let demo_upload = |size: i64| json!({ "contentType": "video/mp4", "size": size });
    let upload_path = format!("/exercises/{custom_id}/demo-upload");
    assert_eq!(t.call("POST", &upload_path, Some("coach1"), Some(demo_upload(1 << 40))).await.0, StatusCode::BAD_REQUEST);
    let (s, up) = t.call("POST", &upload_path, Some("coach1"), Some(demo_upload(3))).await;
    assert_eq!(s, StatusCode::OK, "{up}");
    assert!(up["uploadUrl"].as_str().unwrap().contains("X-Amz-SignedHeaders=content-length"));
    assert_eq!(up["uploadHeaders"]["x-amz-tagging"], "state=pending");
    // confirming before the upload lands is refused, and the library links no demo yet
    let confirm_path = format!("/exercises/{custom_id}/demo-confirm");
    assert_eq!(t.call("POST", &confirm_path, Some("coach1"), Some(json!({}))).await.0, StatusCode::BAD_REQUEST);
    let (_, lib) = t.call("GET", "/exercises", Some("client1"), None).await;
    assert!(lib.as_array().unwrap().iter().any(|e| e["id"] == custom_id.as_str() && e["demoVideoUrl"].is_null()));
    let (_, ex_now) = t.call("GET", "/exercises", Some("coach1"), None).await;
    let pending = ex_now.as_array().unwrap().iter().find(|e| e["id"] == custom_id.as_str()).unwrap()["pendingDemoKey"].as_str().unwrap().to_string();
    let bucket = std::env::var("MEDIA_BUCKET").unwrap();
    // stand in for the browser's PUT, which sends the signed pending tag
    s3.put_object().bucket(&bucket).key(&pending).tagging("state=pending").body(b"mp4".to_vec().into()).send().await.unwrap();
    let (s, confirmed) = t.call("POST", &confirm_path, Some("coach1"), Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{confirmed}");
    // confirmed demos lose the tag, so the lifecycle rule won't expire them
    let tags = s3.get_object_tagging().bucket(&bucket).key(&pending).send().await.unwrap();
    assert!(tags.tag_set().is_empty(), "{:?}", tags.tag_set());
    assert_eq!(confirmed["demoKey"], pending.as_str());
    let (_, lib) = t.call("GET", "/exercises", Some("client1"), None).await;
    assert!(lib.as_array().unwrap().iter().any(|e| e["id"] == custom_id.as_str() && e["demoVideoUrl"].is_string()));

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
        "check": { "rpe": 8, "energy": 4, "sleep": 3, "soreness": 2, "pain": true, "painLocation": "left knee" },
        "exerciseNotes": { item_id: "  Left hip a bit tight on the way down  ", "other": "   " },
        "rating": 4
    });
    let now = chrono::Utc::now();
    let newer = (now - chrono::Duration::minutes(1)).to_rfc3339();
    let older = (now - chrono::Duration::minutes(10)).to_rfc3339();
    assert_eq!(t.call("PUT", "/clients/client1/logs/log-abc", Some("coach1"), Some(log(&newer, 5))).await.0, StatusCode::FORBIDDEN);
    // the workout rating is 1-5
    let mut rated = log(&newer, 5);
    rated["rating"] = json!(6);
    assert_eq!(t.call("PUT", "/clients/client1/logs/log-abc", Some("client1"), Some(rated)).await.0, StatusCode::BAD_REQUEST);
    let (s, l) = t.call("PUT", "/clients/client1/logs/log-abc", Some("client1"), Some(log(&newer, 5))).await;
    assert_eq!(s, StatusCode::OK, "{l}");
    // notes are trimmed and empty ones dropped; the coach sees them and the rating
    let (_, logs) = t.call("GET", "/clients/client1/logs", Some("coach1"), None).await;
    assert_eq!(logs[0]["exerciseNotes"], json!({ item_id: "Left hip a bit tight on the way down" }));
    assert_eq!(logs[0]["rating"], 4);
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
    // entries can only be deleted by whoever recorded them
    let metric_path = |m: &Value| format!("/clients/client1/metrics/{}", m["id"].as_str().unwrap());
    assert_eq!(t.call("DELETE", &metric_path(&sf), Some("client1"), None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(t.call("DELETE", &metric_path(&m), Some("coach1"), None).await.0, StatusCode::FORBIDDEN);
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
        .call("POST", "/clients/client1/photos", Some("client1"), Some(json!({ "date": today.to_string(), "pose": "front", "contentType": "image/jpeg", "size": 4 })))
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
    assert_eq!(photos[0]["takenBy"], "client");
    // the client's own photo is theirs to delete, not the coach's
    let photo_path = format!("/clients/client1/photos/{}", photos[0]["id"].as_str().unwrap());
    assert_eq!(t.call("DELETE", &photo_path, Some("coach1"), None).await.0, StatusCode::FORBIDDEN);

    // --- set videos (VS-1..VS-3): multipart upload, complete, list
    let bucket = std::env::var("MEDIA_BUCKET").unwrap();
    let part = 8 * 1024 * 1024usize;
    let size = part + 5; // two parts: 8 MiB and 5 bytes
    let new_video = |extra: Value| {
        let mut v = json!({ "logId": "log-abc", "itemId": item_id, "setIndex": 0, "exerciseId": "back-squat",
            "exerciseName": "Back Squat", "contentType": "video/mp4", "size": size, "durationSec": 12.0, "posterSize": 4 });
        v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        v
    };
    assert_eq!(t.call("POST", "/clients/client1/videos", Some("coach1"), Some(new_video(json!({})))).await.0, StatusCode::FORBIDDEN);
    assert_eq!(t.call("POST", "/clients/client1/videos", Some("coach2"), Some(new_video(json!({})))).await.0, StatusCode::FORBIDDEN);
    for bad in [json!({ "size": 600 * 1024 * 1024 }), json!({ "contentType": "image/gif" }), json!({ "durationSec": 90.0 }),
        json!({ "trimStart": 10.0, "trimEnd": 80.0 }), json!({ "trimStart": 5.0 }), json!({ "logId": "../x" })] {
        assert_eq!(t.call("POST", "/clients/client1/videos", Some("client1"), Some(new_video(bad.clone()))).await.0, StatusCode::BAD_REQUEST, "{bad}");
    }
    let (s, cv) = t.call("POST", "/clients/client1/videos", Some("client1"), Some(new_video(json!({})))).await;
    assert_eq!(s, StatusCode::OK, "{cv}");
    let vid = cv["video"]["id"].as_str().unwrap().to_string();
    let key = cv["video"]["key"].as_str().unwrap().to_string();
    assert!(key.starts_with("media/coach1/client1/videos/") && key.ends_with(".mp4"), "{key}");
    assert_eq!(cv["video"]["status"], "uploading");
    let parts = cv["parts"].as_array().unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0]["size"], part);
    assert_eq!(parts[1]["size"], 5);
    let url0 = parts[0]["url"].as_str().unwrap();
    assert!(url0.contains("partNumber=1") && url0.contains("uploadId=") && url0.contains("X-Amz-SignedHeaders=content-length"), "{url0}");
    assert_eq!(cv["poster"]["headers"]["x-amz-tagging"], "state=pending");
    // an expired part URL can be re-issued
    let (s, re) = t.call("POST", &format!("/clients/client1/videos/{vid}/parts"), Some("client1"), Some(json!({ "parts": [2] }))).await;
    assert_eq!(s, StatusCode::OK, "{re}");
    assert_eq!(re["parts"][0]["n"], 2);
    // duplicates collapse; asking for more parts than exist is refused
    let (_, dup) = t.call("POST", &format!("/clients/client1/videos/{vid}/parts"), Some("client1"), Some(json!({ "parts": [2, 2, 2] }))).await;
    assert_eq!(dup["parts"].as_array().unwrap().len(), 1);
    assert_eq!(t.call("POST", &format!("/clients/client1/videos/{vid}/parts"), Some("client1"), Some(json!({ "parts": [1, 2, 3] }))).await.0, StatusCode::BAD_REQUEST);
    // completing before the parts are uploaded fails and leaves the video uploading
    let early = json!({ "parts": [{ "n": 1, "etag": "\"x\"" }, { "n": 2, "etag": "\"y\"" }] });
    assert_eq!(t.call("POST", &format!("/clients/client1/videos/{vid}/complete"), Some("client1"), Some(early)).await.0, StatusCode::BAD_REQUEST);
    // stand in for the browser's part PUTs and poster PUT
    let upload_id = s3
        .list_multipart_uploads()
        .bucket(&bucket)
        .prefix(&key)
        .send()
        .await
        .unwrap()
        .uploads()[0]
        .upload_id()
        .unwrap()
        .to_string();
    let mut etags = vec![];
    for (n, len) in [(1, part), (2, 5)] {
        let out = s3.upload_part().bucket(&bucket).key(&key).upload_id(&upload_id).part_number(n).body(vec![7u8; len].into()).send().await.unwrap();
        etags.push(json!({ "n": n, "etag": out.e_tag().unwrap() }));
    }
    let poster_key = key.replace(".mp4", ".jpg");
    s3.put_object().bucket(&bucket).key(&poster_key).tagging("state=pending").body(b"jpeg".to_vec().into()).send().await.unwrap();
    // wrong part numbering is refused
    let gap = json!({ "parts": [etags[0].clone()] });
    assert_eq!(t.call("POST", &format!("/clients/client1/videos/{vid}/complete"), Some("client1"), Some(gap)).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(t.call("POST", &format!("/clients/client1/videos/{vid}/complete"), Some("coach1"), Some(json!({ "parts": etags }))).await.0, StatusCode::FORBIDDEN);
    let (s, done) = t.call("POST", &format!("/clients/client1/videos/{vid}/complete"), Some("client1"), Some(json!({ "parts": etags }))).await;
    assert_eq!(s, StatusCode::OK, "{done}");
    assert_eq!(done["status"], "ready");
    assert_eq!(done["posterKey"], poster_key.as_str());
    assert!(done["uploadId"].is_null());
    // repeating complete (a lost response) returns the same ready video
    let (s, again) = t.call("POST", &format!("/clients/client1/videos/{vid}/complete"), Some("client1"), Some(json!({ "parts": etags }))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(again["status"], "ready");
    // confirmed objects lose the pending tag, so the lifecycle rule keeps them
    for k in [&key, &poster_key] {
        let tags = s3.get_object_tagging().bucket(&bucket).key(k).send().await.unwrap();
        assert!(tags.tag_set().is_empty(), "{k}: {:?}", tags.tag_set());
    }
    // the coach and the client see it with playback URLs; other coaches don't
    let (s, vids) = t.call("GET", "/clients/client1/videos", Some("coach1"), None).await;
    assert_eq!(s, StatusCode::OK, "{vids}");
    assert_eq!(vids.as_array().unwrap().len(), 1);
    assert_eq!(vids[0]["setIndex"], 0);
    assert!(vids[0]["url"].as_str().unwrap().contains("X-Amz-Signature"));
    assert!(vids[0]["posterUrl"].as_str().unwrap().contains("X-Amz-Signature"));
    let (_, mine) = t.call("GET", "/clients/client1/videos?logId=log-abc", Some("client1"), None).await;
    assert_eq!(mine.as_array().unwrap().len(), 1);
    let (_, other) = t.call("GET", "/clients/client1/videos?logId=log-other", Some("client1"), None).await;
    assert_eq!(other.as_array().unwrap().len(), 0);
    assert_eq!(t.call("GET", "/clients/client1/videos", Some("coach2"), None).await.0, StatusCode::FORBIDDEN);
    // a video whose upload never finished isn't listed
    let (_, pending) = t.call("POST", "/clients/client1/videos", Some("client1"), Some(new_video(json!({ "trimStart": 2.0, "trimEnd": 30.0, "durationSec": 75.0 })))).await;
    assert_eq!(pending["video"]["trimEnd"], 30.0);
    let (_, vids) = t.call("GET", "/clients/client1/videos", Some("client1"), None).await;
    assert_eq!(vids.as_array().unwrap().len(), 1);

    // --- coach review queue (VS-4): oldest first, mark reviewed
    let mut ids = vec![vid.clone()];
    for set_index in [1, 2] {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await; // distinct creation times
        let (_, c) = t
            .call("POST", "/clients/client1/videos", Some("client1"), Some(new_video(json!({ "size": 3, "setIndex": set_index, "posterSize": null }))))
            .await;
        let (id, k) = (c["video"]["id"].as_str().unwrap().to_string(), c["video"]["key"].as_str().unwrap().to_string());
        let uid = s3.list_multipart_uploads().bucket(&bucket).prefix(&k).send().await.unwrap().uploads()[0].upload_id().unwrap().to_string();
        let out = s3.upload_part().bucket(&bucket).key(&k).upload_id(&uid).part_number(1).body(b"mp4".to_vec().into()).send().await.unwrap();
        let (s, d) = t
            .call("POST", &format!("/clients/client1/videos/{id}/complete"), Some("client1"), Some(json!({ "parts": [{ "n": 1, "etag": out.e_tag().unwrap() }] })))
            .await;
        assert_eq!(s, StatusCode::OK, "{d}");
        ids.push(id);
    }
    assert_eq!(t.call("GET", "/videos/queue", Some("client1"), None).await.0, StatusCode::FORBIDDEN);
    let (_, q2) = t.call("GET", "/videos/queue", Some("coach2"), None).await;
    assert_eq!(q2.as_array().unwrap().len(), 0, "another coach's queue is separate");
    let (s, queue) = t.call("GET", "/videos/queue", Some("coach1"), None).await;
    assert_eq!(s, StatusCode::OK, "{queue}");
    let order: Vec<&str> = queue.as_array().unwrap().iter().map(|e| e["videoId"].as_str().unwrap()).collect();
    assert_eq!(order, ids.iter().map(String::as_str).collect::<Vec<_>>(), "oldest first");
    assert_eq!(queue[0]["clientName"], "Cli Ent");
    assert_eq!(queue[0]["exerciseName"], "Back Squat");
    assert!(queue[0]["url"].as_str().unwrap().contains("X-Amz-Signature"));
    assert!(queue[0]["posterUrl"].as_str().unwrap().contains("X-Amz-Signature"));
    // only the client's coach marks it reviewed; doing it twice is fine
    let reviewed = |id: &str| format!("/clients/client1/videos/{id}/reviewed");
    assert_eq!(t.call("POST", &reviewed(&ids[1]), Some("client1"), None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(t.call("POST", &reviewed(&ids[1]), Some("coach2"), None).await.0, StatusCode::FORBIDDEN);
    let (s, r) = t.call("POST", &reviewed(&ids[1]), Some("coach1"), None).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert!(r["reviewedAt"].is_string());
    assert_eq!(t.call("POST", &reviewed(&ids[1]), Some("coach1"), None).await.0, StatusCode::OK);
    let pending_id = pending["video"]["id"].as_str().unwrap();
    assert_eq!(t.call("POST", &reviewed(pending_id), Some("coach1"), None).await.0, StatusCode::BAD_REQUEST);
    let (_, queue) = t.call("GET", "/videos/queue", Some("coach1"), None).await;
    let order: Vec<&str> = queue.as_array().unwrap().iter().map(|e| e["videoId"].as_str().unwrap()).collect();
    assert_eq!(order, [ids[0].as_str(), ids[2].as_str()]);
    // the client's list keeps the reviewed video, marked reviewed
    let (_, vids) = t.call("GET", "/clients/client1/videos", Some("client1"), None).await;
    let rv = vids.as_array().unwrap().iter().find(|v| v["id"] == ids[1].as_str()).unwrap();
    assert!(rv["reviewedAt"].is_string());
    // reviewing starts the retention clock: the lifecycle rule deletes objects tagged reviewed=true
    let rkey = rv["key"].as_str().unwrap();
    let tags = s3.get_object_tagging().bucket(&bucket).key(rkey).send().await.unwrap();
    assert!(tags.tag_set().iter().any(|t| t.key() == "reviewed" && t.value() == "true"), "{:?}", tags.tag_set());

    // --- the coach replies to a set video (VR-1..VR-3)
    let new_reply = format!("/clients/client1/videos/{}/replies", ids[2]);
    assert_eq!(t.call("POST", &new_reply, Some("client1"), Some(json!({ "note": "x" }))).await.0, StatusCode::FORBIDDEN);
    assert_eq!(t.call("POST", &new_reply, Some("coach2"), Some(json!({ "note": "x" }))).await.0, StatusCode::FORBIDDEN);
    assert_eq!(t.call("POST", &new_reply, Some("coach1"), Some(json!({}))).await.0, StatusCode::BAD_REQUEST, "nothing to send");
    let too_long = json!({ "contentType": "video/mp4", "size": 3, "durationSec": 400.0 });
    assert_eq!(t.call("POST", &new_reply, Some("coach1"), Some(too_long)).await.0, StatusCode::BAD_REQUEST, "over 5 minutes");
    let (s, cr) = t
        .call("POST", &new_reply, Some("coach1"), Some(json!({
            "contentType": "video/mp4", "size": 3, "durationSec": 42.0, "note": "Watch your knees at 0:12", "reviewMs": 180000
        })))
        .await;
    assert_eq!(s, StatusCode::OK, "{cr}");
    let rid = cr["reply"]["id"].as_str().unwrap().to_string();
    let rkey = cr["reply"]["key"].as_str().unwrap().to_string();
    assert!(rkey.starts_with("media/coach1/client1/replies/") && rkey.ends_with(".mp4"), "{rkey}");
    assert_eq!(cr["parts"].as_array().unwrap().len(), 1);
    let (_, listed) = t.call("GET", "/clients/client1/replies", Some("client1"), None).await;
    assert_eq!(listed.as_array().unwrap().len(), 0, "not shown until sent");
    let ruid = s3.list_multipart_uploads().bucket(&bucket).prefix(&rkey).send().await.unwrap().uploads()[0].upload_id().unwrap().to_string();
    let rout = s3.upload_part().bucket(&bucket).key(&rkey).upload_id(&ruid).part_number(1).body(b"mp4".to_vec().into()).send().await.unwrap();
    let rdone = json!({ "parts": [{ "n": 1, "etag": rout.e_tag().unwrap() }] });
    let complete = format!("/clients/client1/replies/{rid}/complete");
    assert_eq!(t.call("POST", &complete, Some("client1"), Some(rdone.clone())).await.0, StatusCode::FORBIDDEN);
    let (s, sent) = t.call("POST", &complete, Some("coach1"), Some(rdone.clone())).await;
    assert_eq!(s, StatusCode::OK, "{sent}");
    assert_eq!(sent["status"], "ready");
    assert_eq!(t.call("POST", &complete, Some("coach1"), Some(rdone)).await.0, StatusCode::OK, "repeating is harmless");
    // sending marked the video reviewed: out of the queue, both objects on the retention clock
    let (_, queue_now) = t.call("GET", "/videos/queue", Some("coach1"), None).await;
    let order: Vec<&str> = queue_now.as_array().unwrap().iter().map(|e| e["videoId"].as_str().unwrap()).collect();
    assert_eq!(order, [ids[0].as_str()]);
    let tags = s3.get_object_tagging().bucket(&bucket).key(&rkey).send().await.unwrap();
    assert!(tags.tag_set().iter().any(|t| t.key() == "reviewed" && t.value() == "true"), "{:?}", tags.tag_set());
    // the client gets it: listed with a URL, and a message in the thread that carries it
    let (s, listed) = t.call("GET", "/clients/client1/replies", Some("client1"), None).await;
    assert_eq!(s, StatusCode::OK, "{listed}");
    assert_eq!(listed[0]["id"], rid.as_str());
    assert_eq!(listed[0]["reviewMs"], 180000);
    assert!(listed[0]["url"].as_str().unwrap().contains("X-Amz-Signature"));
    let (_, thread) = t.call("GET", "/clients/client1/comments", Some("client1"), None).await;
    let carried: Vec<&Value> = thread.as_array().unwrap().iter().filter(|c| c["replyId"] == rid.as_str()).collect();
    assert_eq!(carried.len(), 1, "one message, even after a repeated complete");
    assert_eq!(carried[0]["text"], "Watch your knees at 0:12");
    assert_eq!(t.call("GET", "/clients/client1/replies", Some("coach2"), None).await.0, StatusCode::FORBIDDEN);
    // only the client marks it seen
    let seen = format!("/clients/client1/replies/{rid}/seen");
    assert_eq!(t.call("POST", &seen, Some("coach1"), None).await.0, StatusCode::FORBIDDEN);
    let (_, r) = t.call("POST", &seen, Some("client1"), None).await;
    assert!(r["seenAt"].is_string());
    // a written reply needs no upload
    let (s, written) = t
        .call("POST", &format!("/clients/client1/videos/{}/replies", ids[1]), Some("coach1"), Some(json!({ "note": "Solid set" })))
        .await;
    assert_eq!(s, StatusCode::OK, "{written}");
    assert_eq!(written["reply"]["status"], "ready");
    let (_, listed) = t.call("GET", "/clients/client1/replies", Some("client1"), None).await;
    assert_eq!(listed.as_array().unwrap().len(), 2);
    assert!(listed.as_array().unwrap().iter().any(|r| r["note"] == "Solid set" && r["url"].is_null()));
    t.call("POST", "/clients/client1/messages/read", Some("client1"), None).await;

    // --- the client deletes their own videos (VS-3)
    let del = |id: &str| format!("/clients/client1/videos/{id}");
    assert_eq!(t.call("DELETE", &del(&ids[2]), Some("coach1"), None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(t.call("DELETE", &del(&ids[2]), Some("client2"), None).await.0, StatusCode::FORBIDDEN);
    let gone_key = queue[1]["key"].as_str().unwrap().to_string();
    let (s, d) = t.call("DELETE", &del(&ids[2]), Some("client1"), None).await;
    assert_eq!(s, StatusCode::OK, "{d}");
    assert_eq!(t.call("DELETE", &del(&ids[2]), Some("client1"), None).await.0, StatusCode::NOT_FOUND);
    let (_, vids) = t.call("GET", "/clients/client1/videos", Some("client1"), None).await;
    assert!(!vids.as_array().unwrap().iter().any(|v| v["id"] == ids[2].as_str()));
    let (_, queue) = t.call("GET", "/videos/queue", Some("coach1"), None).await;
    let order: Vec<&str> = queue.as_array().unwrap().iter().map(|e| e["videoId"].as_str().unwrap()).collect();
    assert_eq!(order, [ids[0].as_str()], "a deleted video leaves the coach's queue");
    assert!(s3.head_object().bucket(&bucket).key(&gone_key).send().await.is_err(), "object deleted");
    // its replies go with it
    let (_, listed) = t.call("GET", "/clients/client1/replies", Some("client1"), None).await;
    assert!(!listed.as_array().unwrap().iter().any(|r| r["id"] == rid.as_str()), "{listed}");
    assert!(s3.head_object().bucket(&bucket).key(&rkey).send().await.is_err(), "reply object deleted");
    // deleting a video that never finished uploading aborts its multipart upload
    let pending_key = pending["video"]["key"].as_str().unwrap().to_string();
    let open_uploads = |k: String| {
        let s3 = s3.clone();
        let bucket = bucket.clone();
        async move { s3.list_multipart_uploads().bucket(&bucket).prefix(&k).send().await.unwrap().uploads().len() }
    };
    assert_eq!(open_uploads(pending_key.clone()).await, 1);
    assert_eq!(t.call("DELETE", &del(pending_id), Some("client1"), None).await.0, StatusCode::OK);
    assert_eq!(open_uploads(pending_key).await, 0);

    // --- comments (FB-1)
    let (s, _) = t
        .call("POST", "/clients/client1/comments", Some("coach1"), Some(json!({ "text": "Great depth on set 2", "logId": "log-abc", "setIndex": 1 })))
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, cs) = t.call("GET", "/clients/client1/comments", Some("client1"), None).await;
    assert_eq!(cs[0]["authorRole"], "coach");

    // --- message threads and unread counts (MS-1)
    let unread = |who: &'static str| {
        let t = &t;
        async move { t.call("GET", "/messages", Some(who), None).await.1["unreadTotal"].as_u64() }
    };
    assert_eq!(unread("client1").await, Some(1)); // the coach's comment above
    t.call("POST", "/clients/client1/messages/read", Some("client1"), None).await;
    assert_eq!(unread("client1").await, Some(0));
    for text in ["Knee felt better today", "Can we swap Friday for Saturday?"] {
        t.call("POST", "/clients/client1/comments", Some("client1"), Some(json!({ "text": text }))).await;
    }
    let (s, inbox) = t.call("GET", "/messages", Some("coach1"), None).await;
    assert_eq!(s, StatusCode::OK, "{inbox}");
    assert_eq!(inbox["unreadTotal"], 2);
    let row = &inbox["threads"][0];
    assert_eq!((row["clientId"].as_str(), row["unread"].as_u64()), (Some("client1"), Some(2)));
    let new: Vec<&str> = row["newMessages"].as_array().unwrap().iter().map(|m| m["text"].as_str().unwrap()).collect();
    assert_eq!(new, ["Knee felt better today", "Can we swap Friday for Saturday?"]);
    assert_eq!(row["lastText"], "Can we swap Friday for Saturday?");
    // only the client's coach reads or clears it
    assert_eq!(t.call("POST", "/clients/client1/messages/read", Some("coach2"), None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(t.call("GET", "/messages", Some("admin1"), None).await.0, StatusCode::FORBIDDEN);
    t.call("POST", "/clients/client1/messages/read", Some("coach1"), None).await;
    assert_eq!(unread("coach1").await, Some(0));
    assert_eq!(t.call("GET", "/messages", Some("coach1"), None).await.1["threads"][0]["newMessages"], json!([]));

    // --- tenant isolation
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
    // the kept past workout still blocks re-scheduling the same day
    let (s, e) = t
        .call("POST", "/clients/client1/assignments", Some("coach1"), Some(json!({ "programId": pid, "startDate": start.to_string() })))
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(e["error"].as_str().unwrap().contains("already has"), "{e}");

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
    assert!(audits.iter().any(|a| a["action"] == "view-video-queue" && a["actorId"] == "coach1"));
}
