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

    /// An anonymous POST from a given sender address (DEV_AUTH's x-dev-ip).
    async fn post_from(&self, ip: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let req = Request::builder().method("POST").uri(path).header("content-type", "application/json").header("x-dev-ip", ip);
        let res = self.app.clone().oneshot(req.body(Body::from(body.to_string())).unwrap()).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 10 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    /// A GET whose body isn't JSON (the calendar feed).
    async fn text(&self, path: &str) -> (StatusCode, String, String) {
        let req = Request::builder().method("GET").uri(path).body(Body::empty()).unwrap();
        let res = self.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let kind = res.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let bytes = axum::body::to_bytes(res.into_body(), 10 << 20).await.unwrap();
        (status, kind, String::from_utf8_lossy(&bytes).into_owned())
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
    let t = T { app: api::router(state.clone()) };
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
    assert_eq!(t.call("POST", &format!("/invites/{coach_code}/accept"), Some("coach9"), Some(json!({ "consent": true }))).await.0, StatusCode::FORBIDDEN);
    // agreeing to how their data is used is part of accepting
    assert_eq!(t.call("POST", &format!("/invites/{coach_code}/accept"), Some("coach1"), Some(json!({}))).await.0, StatusCode::BAD_REQUEST);
    let (s, p) = t.call("POST", &format!("/invites/{coach_code}/accept"), Some("coach1"), Some(json!({ "consent": true }))).await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert!(p["consentedAt"].is_string(), "{p}");
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
    let (s, c2) = t.call("POST", &format!("/invites/{}/accept", ci2["code"].as_str().unwrap()), Some("coach2"), Some(json!({ "consent": true }))).await;
    assert_eq!(s, StatusCode::OK, "{c2}");
    assert_eq!(c2["role"], "coach");
    let (_, coaches) = t.call("GET", "/admin/coaches", Some("admin1"), None).await;
    let names: Vec<&str> = coaches.as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Coach One", "Coach Two"]);
    // --- coach request queue: anonymous ask-to-join, admin approves or denies
    // (a person takes a few seconds on the form; the page sends how long)
    let ask = |email: &str, name: &str| json!({ "email": email, "name": name, "elapsedMs": 8000 });
    async fn queue(t: &T) -> Vec<Value> {
        let (_, rq) = t.call("GET", "/admin/coach-requests", Some("admin1"), None).await;
        rq.as_array().unwrap().clone()
    }
    let status_of = |rq: &[Value], email: &str| rq.iter().find(|r| r["email"] == email).map(|r| r["status"].clone());
    for bad in ["nope", "a@b", "two words@x.co"] {
        assert_eq!(t.call("POST", "/public/coach-requests", None, Some(ask(bad, ""))).await.0, StatusCode::BAD_REQUEST, "{bad}");
    }
    let (s, w) = t.call("POST", "/public/coach-requests", None, Some(ask(" Wannabe@Dev.Local ", " Coach Wanna "))).await;
    assert_eq!(s, StatusCode::OK, "{w}");
    assert_eq!(w, json!({ "status": "received" }), "the same answer whatever happens");
    assert_eq!(t.call("GET", "/admin/coach-requests", Some("coach1"), None).await.0, StatusCode::FORBIDDEN, "only the admin reads the queue");
    let (_, again) = t.call("POST", "/public/coach-requests", None, Some(ask("wannabe@dev.local", "other name"))).await;
    assert_eq!(again, w, "re-asking looks the same");
    let rq = queue(&t).await;
    assert_eq!(rq.len(), 1, "one record per email");
    assert_eq!(rq[0]["email"], "wannabe@dev.local", "cleaned");
    assert_eq!(rq[0]["name"], "Coach Wanna", "the first ask won");
    assert_eq!(rq[0]["status"], "pending", "with auto-approve off the request just waits");
    // scripts get the same answer and leave nothing behind
    let (s, bot) = t.call("POST", "/public/coach-requests", None, Some(json!({ "email": "bot1@dev.local", "website": "http://spam.example", "elapsedMs": 8000 }))).await;
    assert_eq!((s, bot), (StatusCode::OK, w.clone()), "the hidden field was filled");
    assert_eq!(t.call("POST", "/public/coach-requests", None, Some(json!({ "email": "bot2@dev.local", "elapsedMs": 400 }))).await.1, w, "too fast");
    assert_eq!(t.call("POST", "/public/coach-requests", None, Some(json!({ "email": "bot3@dev.local" }))).await.1, w, "no timing at all");
    assert_eq!(queue(&t).await.len(), 1, "no bot made it into the queue");
    assert_eq!(t.call("POST", "/admin/coach-requests/wannabe@dev.local/deny", Some("coach1"), None).await.0, StatusCode::FORBIDDEN);
    // approving sends the same invitation an admin makes by hand
    let (s, ap) = t.call("POST", "/admin/coach-requests/wannabe@dev.local/invite", Some("admin1"), None).await;
    assert_eq!(s, StatusCode::OK, "{ap}");
    assert_eq!(ap["invite"]["kind"], "coach");
    assert_eq!(ap["invite"]["clientName"], "", "the visitor's typed name stays out of the email");
    assert_eq!(ap["invite"]["emailed"], false, "no SES locally; the admin page shares the link instead");
    let ap_code = ap["invite"]["code"].as_str().unwrap().to_string();
    assert_eq!(t.call("POST", "/admin/coach-requests/wannabe@dev.local/invite", Some("admin1"), None).await.0, StatusCode::BAD_REQUEST, "one invitation per email");
    assert_eq!(t.call("POST", "/admin/coach-requests/wannabe@dev.local/deny", Some("admin1"), None).await.0, StatusCode::BAD_REQUEST, "invited emails revoke the invite, not the queue row");
    assert_eq!(t.call("POST", "/public/coach-requests", None, Some(ask("wannabe@dev.local", ""))).await.1, w, "an invited email can't tell it was invited");
    // a request the admin says no to stays a no for now
    assert_eq!(t.call("POST", "/public/coach-requests", None, Some(ask("nosuch@email.example", "Nope"))).await.0, StatusCode::OK, "first ask");
    assert_eq!(t.call("POST", "/admin/coach-requests/nosuch@email.example/deny", Some("admin1"), None).await.0, StatusCode::OK);
    assert_eq!(t.call("POST", "/public/coach-requests", None, Some(ask("nosuch@email.example", "Nope again"))).await.1, w);
    assert_eq!(status_of(&queue(&t).await, "nosuch@email.example"), Some(json!("denied")), "asking again right away changes nothing");
    // the approved invitation sits on the admin's invites list like any other
    let (_, invs) = t.call("GET", "/admin/invites", Some("admin1"), None).await;
    assert!(invs.as_array().unwrap().iter().any(|i| i["code"] == ap_code.as_str()));
    // one sender gets a few an hour; the rest are answered the same and dropped
    for i in 0..7 {
        let (s, r) = t.post_from("203.0.113.9", "/public/coach-requests", ask(&format!("burst{i}@dev.local"), "")).await;
        assert_eq!((s, r), (StatusCode::OK, w.clone()));
    }
    let burst = queue(&t).await.iter().filter(|r| r["email"].as_str().unwrap().starts_with("burst")).count();
    assert_eq!(burst, 5, "MAX_PER_SENDER_HOUR from one address");
    assert_eq!(t.post_from("198.51.100.4", "/public/coach-requests", ask("other-sender@dev.local", "")).await.1, w);
    assert!(status_of(&queue(&t).await, "other-sender@dev.local").is_some(), "another address isn't held up");
    // the auto-approve switch needs the human check (Cloudflare Turnstile)
    let (s, off) = t.call("GET", "/admin/coach-requests/settings", Some("admin1"), None).await;
    assert_eq!(s, StatusCode::OK, "{off}");
    assert_eq!((off["autoApprove"].clone(), off["humanCheck"].clone()), (json!(false), json!(false)), "review by hand; no Turnstile here");
    assert_eq!(t.call("PUT", "/admin/coach-requests/settings", Some("coach1"), Some(json!({ "autoApprove": true }))).await.0, StatusCode::FORBIDDEN);
    assert_eq!(t.call("PUT", "/admin/coach-requests/settings", Some("admin1"), Some(json!({ "autoApprove": true }))).await.0, StatusCode::BAD_REQUEST, "not without Turnstile");
    // with Turnstile (Cloudflare's published test secrets: 1x… always passes, 2x… always fails)
    let with_secret = |secret: &str| T { app: api::router(api::AppState { turnstile_secret: Some(secret.into()), ..state.clone() }) };
    let failing = with_secret("2x0000000000000000000000000000000AA");
    let token = |email: &str| json!({ "email": email, "turnstile": "XXXX.DUMMY.TOKEN.XXXX" });
    assert_eq!(failing.call("POST", "/public/coach-requests", None, Some(token("robot@dev.local"))).await.1, w, "failed the check: same answer");
    assert!(status_of(&queue(&t).await, "robot@dev.local").is_none(), "and nothing kept");
    let tt = with_secret("1x0000000000000000000000000000000AA");
    assert_eq!(tt.call("POST", "/public/coach-requests", None, Some(json!({ "email": "notoken@dev.local", "elapsedMs": 8000 }))).await.1, w);
    assert!(status_of(&queue(&t).await, "notoken@dev.local").is_none(), "with Turnstile on, the page's timing alone isn't enough");
    let (_, on) = tt.call("PUT", "/admin/coach-requests/settings", Some("admin1"), Some(json!({ "autoApprove": true }))).await;
    assert_eq!((on["autoApprove"].clone(), on["humanCheck"].clone()), (json!(true), json!(true)));
    let (s, quick) = tt.call("POST", "/public/coach-requests", None, Some(json!({ "email": "quick@dev.local", "name": " Quick Coach ", "turnstile": "XXXX.DUMMY.TOKEN.XXXX" }))).await;
    assert_eq!((s, quick), (StatusCode::OK, w.clone()), "no hint that it was approved");
    let rq2 = queue(&t).await;
    assert_eq!(rq2[0]["email"], "quick@dev.local", "newest first");
    assert_eq!(rq2[0]["status"], "invited", "auto-approved on arrival");
    // auto-approve left on but Turnstile gone (a deploy without the secret): requests wait
    assert_eq!(t.call("POST", "/public/coach-requests", None, Some(ask("waits@dev.local", ""))).await.1, w);
    assert_eq!(status_of(&queue(&t).await, "waits@dev.local"), Some(json!("pending")));
    let (_, off) = t.call("PUT", "/admin/coach-requests/settings", Some("admin1"), Some(json!({ "autoApprove": false }))).await;
    assert_eq!(off["autoApprove"], false);
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
    // client invitations are emailed too (not configured locally, so the coach gets the link)
    assert_eq!(inv["emailed"], false);
    let (s, resent) = t.call("POST", &format!("/invites/{code}/resend"), Some("coach1"), None).await;
    assert_eq!((s, resent["code"].as_str()), (StatusCode::OK, Some(code.as_str())));
    assert_eq!(t.call("POST", &format!("/invites/{code}/resend"), Some("coach2"), None).await.0, StatusCode::NOT_FOUND);
    let (_, pending) = t.call("GET", "/invites", Some("coach1"), None).await;
    assert_eq!(pending.as_array().unwrap().len(), 1);

    let (s, peek) = t.call("GET", &format!("/invites/{code}"), Some("client1"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(peek["coachName"], "Coach One");
    // only the invited address can accept (dev users are {id}@dev.local)
    assert_eq!(t.call("POST", &format!("/invites/{code}/accept"), Some("client2"), Some(json!({ "consent": true }))).await.0, StatusCode::FORBIDDEN);
    let (s, cp) = t.call("POST", &format!("/invites/{code}/accept"), Some("client1"), Some(json!({ "consent": true }))).await;
    assert_eq!(s, StatusCode::OK, "{cp}");
    assert_eq!(cp["role"], "client");
    assert_eq!(cp["coachId"], "coach1");
    // the client's header shows their coach
    let (_, me) = t.call("GET", "/me", Some("client1"), None).await;
    assert_eq!(me["coach"]["name"], "Coach One");
    // single use
    assert_eq!(t.call("POST", &format!("/invites/{code}/accept"), Some("client2"), Some(json!({ "consent": true }))).await.0, StatusCode::NOT_FOUND);
    let (_, pending) = t.call("GET", "/invites", Some("coach1"), None).await;
    assert_eq!(pending.as_array().unwrap().len(), 0);

    // revoke normalizes case: a lowercase code must actually revoke the invite
    let (_, inv2) = t.call("POST", "/invites", Some("coach1"), Some(json!({ "email": "d@x.com", "clientName": "Other" }))).await;
    let code2 = inv2["code"].as_str().unwrap().to_lowercase();
    assert_eq!(t.call("DELETE", &format!("/invites/{code2}"), Some("coach1"), None).await.0, StatusCode::OK);
    assert_eq!(t.call("GET", &format!("/invites/{code2}"), Some("client2"), None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(t.call("GET", "/invites", Some("coach1"), None).await.1.as_array().unwrap().len(), 0);

    // --- client requests: a coach's own sign-up link (#/request/{coachId})
    let (s, page) = t.call("GET", "/public/join/coach1", None, None).await;
    assert_eq!(s, StatusCode::OK, "{page}");
    assert_eq!(page["coachName"], "Coach One");
    assert_eq!(t.call("GET", "/public/join/nosuchcoach", None, None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(t.call("GET", "/public/join/client1", None, None).await.0, StatusCode::NOT_FOUND, "clients have no sign-up link");
    assert_eq!(t.call("POST", "/public/join/client1", None, Some(ask("x@dev.local", ""))).await.0, StatusCode::NOT_FOUND);
    let (s, r) = t.call("POST", "/public/join/coach1", None, Some(ask(" Newbie@Dev.Local ", " New Bie "))).await;
    assert_eq!((s, r), (StatusCode::OK, w.clone()), "the same answer as every request");
    async fn coach_queue(t: &T, coach: &str) -> Vec<Value> {
        let (s, rq) = t.call("GET", "/requests", Some(coach), None).await;
        assert_eq!(s, StatusCode::OK, "{rq}");
        rq.as_array().unwrap().clone()
    }
    let q1 = coach_queue(&t, "coach1").await;
    assert_eq!(q1.len(), 1);
    assert_eq!((q1[0]["email"].clone(), q1[0]["name"].clone(), q1[0]["status"].clone()), (json!("newbie@dev.local"), json!("New Bie"), json!("pending")));
    assert!(coach_queue(&t, "coach2").await.is_empty(), "each coach sees only their own");
    assert_eq!(t.call("GET", "/requests", Some("client1"), None).await.0, StatusCode::FORBIDDEN);
    assert!(status_of(&queue(&t).await, "newbie@dev.local").is_none(), "not in the admin's coach queue");
    // already a client, or already invited: quietly nothing
    assert_eq!(t.call("POST", "/public/join/coach1", None, Some(ask("client1@dev.local", ""))).await.1, w);
    assert_eq!(coach_queue(&t, "coach1").await.len(), 1, "client1 is already coach1's client");
    // coach2 can't act on coach1's request
    assert_eq!(t.call("POST", "/requests/newbie@dev.local/invite", Some("coach2"), None).await.0, StatusCode::NOT_FOUND);
    // inviting sends the same client invitation as "+ Invite"
    let (s, ok) = t.call("POST", "/requests/newbie@dev.local/invite", Some("coach1"), None).await;
    assert_eq!(s, StatusCode::OK, "{ok}");
    assert_eq!((ok["invite"]["kind"].clone(), ok["invite"]["coachId"].clone()), (json!("client"), json!("coach1")));
    assert_eq!(ok["invite"]["clientName"], "", "the visitor's typed name stays out of the email");
    let ncode = ok["invite"]["code"].as_str().unwrap().to_string();
    let (_, open) = t.call("GET", "/invites", Some("coach1"), None).await;
    assert!(open.as_array().unwrap().iter().any(|i| i["code"] == ncode.as_str()), "listed with the coach's pending invites");
    assert_eq!(coach_queue(&t, "coach1").await[0]["status"], "invited");
    assert_eq!(t.call("POST", "/requests/newbie@dev.local/invite", Some("coach1"), None).await.0, StatusCode::BAD_REQUEST, "once");
    let (s, joined) = t.call("POST", &format!("/invites/{ncode}/accept"), Some("newbie"), Some(json!({ "consent": true, "name": "New Bie" }))).await;
    assert_eq!(s, StatusCode::OK, "{joined}");
    assert_eq!((joined["role"].clone(), joined["coachId"].clone()), (json!("client"), json!("coach1")));
    // they leave again, so coach1's roster is as the rest of this test expects
    assert_eq!(t.call("DELETE", "/me/account", Some("newbie"), Some(json!({ "confirm": "newbie@dev.local" }))).await.0, StatusCode::OK);
    // a request the coach turns down stays down for now
    t.call("POST", "/public/join/coach1", None, Some(ask("maybe@dev.local", ""))).await;
    assert_eq!(t.call("POST", "/requests/maybe@dev.local/deny", Some("coach1"), None).await.0, StatusCode::OK);
    t.call("POST", "/public/join/coach1", None, Some(ask("maybe@dev.local", "again"))).await;
    assert_eq!(status_of(&coach_queue(&t, "coach1").await, "maybe@dev.local"), Some(json!("denied")));
    // the coach's auto-approve: only with Turnstile, and only their own queue
    let (_, cs) = t.call("GET", "/requests/settings", Some("coach1"), None).await;
    assert_eq!((cs["autoApprove"].clone(), cs["humanCheck"].clone()), (json!(false), json!(false)));
    assert_eq!(t.call("PUT", "/requests/settings", Some("coach1"), Some(json!({ "autoApprove": true }))).await.0, StatusCode::BAD_REQUEST, "not without Turnstile");
    let tt = with_secret("1x0000000000000000000000000000000AA");
    let (_, on) = tt.call("PUT", "/requests/settings", Some("coach1"), Some(json!({ "autoApprove": true }))).await;
    assert_eq!(on["autoApprove"], true);
    assert_eq!(tt.call("GET", "/requests/settings", Some("coach2"), None).await.1["autoApprove"], false, "coach2's queue is untouched");
    let (s, r) = tt.call("POST", "/public/join/coach1", None, Some(json!({ "email": "quickclient@dev.local", "turnstile": "XXXX.DUMMY.TOKEN.XXXX" }))).await;
    assert_eq!((s, r), (StatusCode::OK, w.clone()));
    assert_eq!(status_of(&coach_queue(&t, "coach1").await, "quickclient@dev.local"), Some(json!("invited")), "auto-approved on arrival");
    let (_, open) = t.call("GET", "/invites", Some("coach1"), None).await;
    assert!(open.as_array().unwrap().iter().any(|i| i["email"] == "quickclient@dev.local"));
    t.call("PUT", "/requests/settings", Some("coach1"), Some(json!({ "autoApprove": false }))).await;
    // clean up the open invite so later counts of coach1's invites hold
    let qcode = open.as_array().unwrap().iter().find(|i| i["email"] == "quickclient@dev.local").unwrap()["code"].as_str().unwrap().to_string();
    assert_eq!(t.call("DELETE", &format!("/invites/{qcode}"), Some("coach1"), None).await.0, StatusCode::OK);

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
    // a steady-state block is one effort for time or distance
    let mut steady = program.clone();
    steady["workouts"][0]["workout"]["blocks"][0]["kind"] = json!("steady");
    assert_eq!(t.call("POST", "/programs", Some("coach1"), Some(steady)).await.0, StatusCode::BAD_REQUEST);
    // starter programs: listed for coaches, copied into the coach's own programs
    let (s, starters) = t.call("GET", "/programs/starters", Some("coach1"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(starters.as_array().unwrap().len(), 8);
    assert_eq!(t.call("GET", "/programs/starters", Some("client1"), None).await.0, StatusCode::FORBIDDEN);
    let (s, bw) = t.call("POST", "/programs/starters/starter-bodyweight/copy", Some("coach1"), Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{bw}");
    assert_eq!((bw["weeks"].as_u64(), bw["workouts"].as_array().unwrap().len()), (Some(12), 36));
    assert!(!bw["workouts"][0]["workout"]["blocks"][0]["items"][0]["id"].as_str().unwrap().is_empty(), "the copy gets ids");
    assert_eq!(t.call("POST", "/programs/starters/nope/copy", Some("coach1"), Some(json!({}))).await.0, StatusCode::NOT_FOUND);
    t.call("DELETE", &format!("/programs/{}", bw["id"].as_str().unwrap()), Some("coach1"), None).await;
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
        "rating": 4,
        "blockTimes": { "finisher": 754 }
    });
    let now = chrono::Utc::now();
    let newer = (now - chrono::Duration::minutes(1)).to_rfc3339();
    let older = (now - chrono::Duration::minutes(10)).to_rfc3339();
    assert_eq!(t.call("PUT", "/clients/client1/logs/log-abc", Some("coach1"), Some(log(&newer, 5))).await.0, StatusCode::FORBIDDEN);
    // the workout rating is 1-5
    let mut rated = log(&newer, 5);
    rated["rating"] = json!(6);
    assert_eq!(t.call("PUT", "/clients/client1/logs/log-abc", Some("client1"), Some(rated)).await.0, StatusCode::BAD_REQUEST);
    // a "for time" block's time is up to 6 hours
    let mut timed = log(&newer, 5);
    timed["blockTimes"] = json!({ "finisher": 0 });
    assert_eq!(t.call("PUT", "/clients/client1/logs/log-abc", Some("client1"), Some(timed)).await.0, StatusCode::BAD_REQUEST);
    let (s, l) = t.call("PUT", "/clients/client1/logs/log-abc", Some("client1"), Some(log(&newer, 5))).await;
    assert_eq!(s, StatusCode::OK, "{l}");
    // notes are trimmed and empty ones dropped; the coach sees them and the rating
    let (_, logs) = t.call("GET", "/clients/client1/logs", Some("coach1"), None).await;
    assert_eq!(logs[0]["exerciseNotes"], json!({ item_id: "Left hip a bit tight on the way down" }));
    assert_eq!(logs[0]["rating"], 4);
    assert_eq!(logs[0]["blockTimes"]["finisher"], 754, "the for-time result is kept");
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
    let confirm_path = format!("/clients/client1/photos/{}/confirm", ph["photo"]["id"].as_str().unwrap());
    // nothing uploaded yet: not confirmable, and not listed
    assert_eq!(t.call("POST", &confirm_path, Some("client1"), Some(json!({}))).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(t.call("GET", "/clients/client1/photos", Some("client1"), None).await.1, json!([]));
    // stand in for the browser's presigned PUT, then confirm it
    s3.put_object()
        .bucket(std::env::var("MEDIA_BUCKET").unwrap())
        .key(ph["photo"]["key"].as_str().unwrap())
        .body(b"jpeg".to_vec().into())
        .send()
        .await
        .unwrap();
    let (s, confirmed) = t.call("POST", &confirm_path, Some("client1"), Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{confirmed}");
    assert!(confirmed.get("pending").is_none());
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

    // --- photos and voice notes in messages
    let (s, up) = t
        .call("POST", "/clients/client1/attachments", Some("client1"), Some(json!({ "kind": "photo", "contentType": "image/jpeg", "size": 4 })))
        .await;
    assert_eq!(s, StatusCode::OK, "{up}");
    let akey = up["key"].as_str().unwrap().to_string();
    assert!(akey.starts_with("media/coach1/client1/messages/") && akey.ends_with(".jpg"), "{akey}");
    let bad_type = json!({ "kind": "voice", "contentType": "image/jpeg", "size": 4 });
    assert_eq!(t.call("POST", "/clients/client1/attachments", Some("client1"), Some(bad_type)).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(t.call("POST", "/clients/client1/attachments", Some("coach2"), Some(json!({ "kind": "photo", "contentType": "image/jpeg", "size": 4 }))).await.0, StatusCode::FORBIDDEN);
    let photo = |key: &str| json!({ "text": "", "attachment": { "kind": "photo", "key": key, "contentType": "image/jpeg" } });
    assert_eq!(t.call("POST", "/clients/client1/comments", Some("client1"), Some(photo(&akey))).await.0, StatusCode::BAD_REQUEST, "not uploaded yet");
    s3.put_object().bucket(&bucket).key(&akey).body(b"jpeg".to_vec().into()).tagging("state=pending").send().await.unwrap();
    assert_eq!(t.call("POST", "/clients/client1/comments", Some("client1"), Some(photo("media/coach2/x/messages/a.jpg"))).await.0, StatusCode::BAD_REQUEST, "another thread's file");
    let (s, sent) = t.call("POST", "/clients/client1/comments", Some("client1"), Some(photo(&akey))).await;
    assert_eq!(s, StatusCode::OK, "{sent}");
    let tags = s3.get_object_tagging().bucket(&bucket).key(&akey).send().await.unwrap();
    assert!(tags.tag_set().is_empty(), "confirmed, so it no longer expires");
    let (_, thread) = t.call("GET", "/clients/client1/comments", Some("coach1"), None).await;
    let last = thread.as_array().unwrap().last().unwrap();
    assert_eq!(last["attachment"]["kind"], "photo");
    assert!(last["attachmentUrl"].as_str().unwrap().contains("X-Amz-Signature"));
    let (_, inbox) = t.call("GET", "/messages", Some("coach1"), None).await;
    assert_eq!(inbox["threads"][0]["lastText"], "📷 Photo");
    t.call("POST", "/clients/client1/messages/read", Some("coach1"), None).await;

    // --- push subscriptions (MS-2): only real browser push services; idempotent; removable
    let sub = |endpoint: &str| json!({ "endpoint": endpoint, "keys": { "p256dh": "BPkey", "auth": "authsecret" }, "preview": false });
    for _ in 0..2 {
        assert_eq!(t.call("POST", "/me/push", Some("client1"), Some(sub("https://fcm.googleapis.com/fcm/send/abc"))).await.0, StatusCode::OK);
    }
    assert_eq!(t.call("POST", "/me/push", Some("client1"), Some(sub("https://169.254.169.254/latest"))).await.0, StatusCode::BAD_REQUEST);
    let mut keyless = sub("https://web.push.apple.com/x");
    keyless["keys"]["auth"] = json!("");
    assert_eq!(t.call("POST", "/me/push", Some("client1"), Some(keyless)).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(t.call("POST", "/me/push", None, Some(sub("https://fcm.googleapis.com/fcm/send/abc"))).await.0, StatusCode::UNAUTHORIZED);
    // messages still send when push isn't configured (locally there's no VAPID key)
    assert_eq!(t.call("POST", "/clients/client1/comments", Some("coach1"), Some(json!({ "text": "Push check" }))).await.0, StatusCode::OK);
    assert_eq!(t.call("GET", "/push/key", Some("client1"), None).await.1["publicKey"], Value::Null);
    // subscriptions per user, counted straight from the table
    let devices = |user: &'static str| {
        let (ddb, table) = (ddb.clone(), table.clone());
        async move {
            ddb.query().table_name(&table)
                .key_condition_expression("pk = :p AND begins_with(sk, :s)")
                .expression_attribute_values(":p", aws_sdk_dynamodb::types::AttributeValue::S(format!("USER#{user}")))
                .expression_attribute_values(":s", aws_sdk_dynamodb::types::AttributeValue::S("PUSH#".into()))
                .send().await.unwrap().count()
        }
    };
    assert_eq!(devices("client1").await, 1);
    // the same browser, now signed in as someone else: the endpoint moves to them
    assert_eq!(t.call("POST", "/me/push", Some("coach1"), Some(sub("https://fcm.googleapis.com/fcm/send/abc"))).await.0, StatusCode::OK);
    assert_eq!((devices("client1").await, devices("coach1").await), (0, 1), "a shared device notifies only its latest user");
    // but only that browser can hand it over: different keys are refused, the owner keeps it
    let mut forged = sub("https://fcm.googleapis.com/fcm/send/abc");
    forged["keys"]["auth"] = json!("someone-elses");
    assert_eq!(t.call("POST", "/me/push", Some("client1"), Some(forged)).await.0, StatusCode::BAD_REQUEST);
    assert_eq!((devices("client1").await, devices("coach1").await), (0, 1));
    // at most ten devices each, oldest dropped
    for n in 0..11 {
        t.call("POST", "/me/push", Some("coach1"), Some(sub(&format!("https://fcm.googleapis.com/fcm/send/dev{n}")))).await;
    }
    assert_eq!(devices("coach1").await, 10);
    let gone = json!({ "endpoint": "https://fcm.googleapis.com/fcm/send/dev10" });
    assert_eq!(t.call("DELETE", "/me/push", Some("coach1"), Some(gone)).await.0, StatusCode::OK);
    assert_eq!(devices("coach1").await, 9);
    t.call("POST", "/clients/client1/messages/read", Some("client1"), None).await;

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

    // --- a client exports their data, then deletes their account (privacy NFR)
    let (_, inv3) = t.call("POST", "/invites", Some("coach1"), Some(json!({ "email": "client3@dev.local", "clientName": "Three" }))).await;
    let code3 = inv3["code"].as_str().unwrap();
    assert_eq!(t.call("POST", &format!("/invites/{code3}/accept"), Some("client3"), Some(json!({ "consent": true }))).await.0, StatusCode::OK);
    t.call("POST", "/clients/client3/comments", Some("client3"), Some(json!({ "text": "Hello coach" }))).await;
    let (_, ph) = t
        .call("POST", "/clients/client3/photos", Some("client3"), Some(json!({ "date": "2026-10-01", "pose": "front", "contentType": "image/jpeg", "size": 4 })))
        .await;
    let media_bucket = std::env::var("MEDIA_BUCKET").unwrap();
    let pkey = ph["photo"]["key"].as_str().unwrap().to_string();
    s3.put_object().bucket(&media_bucket).key(&pkey).body(b"jpeg".to_vec().into()).send().await.unwrap();
    assert_eq!(t.call("GET", "/me/export", Some("coach1"), None).await.0, StatusCode::FORBIDDEN, "coach accounts aren't self-service");
    let (s, ex) = t.call("GET", "/me/export", Some("client3"), None).await;
    assert_eq!(s, StatusCode::OK, "{ex}");
    assert_eq!(ex["account"]["profile"]["name"], "Three");
    let records = ex["records"].as_array().unwrap();
    assert!(records.iter().any(|r| r["text"] == "Hello coach" && r["record"].as_str().unwrap().starts_with("COMMENT#")));
    assert!(records.iter().all(|r| r.get("pk").is_none()));
    assert!(ex["files"].as_array().unwrap().iter().any(|f| f["key"] == pkey.as_str() && f["url"].as_str().unwrap().contains("X-Amz-Signature")));
    // deleting needs the email typed, and only the client themselves
    assert_eq!(t.call("DELETE", "/me/account", Some("client3"), Some(json!({ "confirm": "nope" }))).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(t.call("DELETE", "/me/account", Some("coach1"), Some(json!({ "confirm": "coach1@dev.local" }))).await.0, StatusCode::FORBIDDEN);
    let (s, d) = t.call("DELETE", "/me/account", Some("client3"), Some(json!({ "confirm": "Client3@dev.local" }))).await;
    assert_eq!(s, StatusCode::OK, "{d}");
    assert_eq!(t.call("GET", "/me", Some("client3"), None).await.1["profile"], Value::Null);
    assert!(s3.head_object().bucket(&media_bucket).key(&pkey).send().await.is_err(), "files deleted");
    let left = ddb
        .query()
        .table_name(&table)
        .key_condition_expression("pk = :pk")
        .expression_attribute_values(":pk", aws_sdk_dynamodb::types::AttributeValue::S("CLIENT#client3".into()))
        .send()
        .await
        .unwrap();
    assert_eq!(left.count(), 0, "records deleted");
    let (_, roster) = t.call("GET", "/clients", Some("coach1"), None).await;
    assert!(!roster.as_array().unwrap().iter().any(|r| r["client"]["clientId"] == "client3"), "off the roster");
    let (_, inbox) = t.call("GET", "/messages", Some("coach1"), None).await;
    assert!(!inbox["threads"].as_array().unwrap().iter().any(|r| r["clientId"] == "client3"), "out of the inbox");

    // --- consent for accounts made before the agreement: idempotent
    let (s, c) = t.call("POST", "/me/consent", Some("coach1"), Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{c}");
    assert_eq!(c["consentedAt"], p["consentedAt"], "the first agreement's time is kept");

    // --- admin: suspend a coach, then lift it
    let suspend = |on: bool| json!({ "suspended": on });
    assert_eq!(t.call("PUT", "/admin/coaches/coach2/suspended", Some("coach1"), Some(suspend(true))).await.0, StatusCode::FORBIDDEN);
    assert_eq!(t.call("PUT", "/admin/coaches/nobody/suspended", Some("admin1"), Some(suspend(true))).await.0, StatusCode::NOT_FOUND);
    let (s, r) = t.call("PUT", "/admin/coaches/coach2/suspended", Some("admin1"), Some(suspend(true))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(t.call("GET", "/clients", Some("coach2"), None).await.0, StatusCode::FORBIDDEN, "a suspended coach can't use the API");
    assert!(t.call("GET", "/me", Some("coach2"), None).await.1["profile"]["suspendedAt"].is_string(), "but the app can tell them why");
    let (_, coaches) = t.call("GET", "/admin/coaches", Some("admin1"), None).await;
    assert!(coaches.as_array().unwrap().iter().any(|c| c["coachId"] == "coach2" && c["suspendedAt"].is_string()));
    t.call("PUT", "/admin/coaches/coach2/suspended", Some("admin1"), Some(suspend(false))).await;
    assert_eq!(t.call("GET", "/clients", Some("coach2"), None).await.0, StatusCode::OK);
    assert!(t.call("GET", "/me", Some("coach2"), None).await.1["profile"].get("suspendedAt").is_none());

    // --- admin: delete a coach; one with clients is refused
    let confirm = |email: &str| json!({ "confirm": email });
    let (s, busy) = t.call("DELETE", "/admin/coaches/coach1", Some("admin1"), Some(confirm("coach1@dev.local"))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{busy}");
    assert!(busy["current"]["clients"].as_u64().unwrap() > 0);
    assert_eq!(t.call("DELETE", "/admin/coaches/coach2", Some("coach1"), Some(confirm("coach2@dev.local"))).await.0, StatusCode::FORBIDDEN);
    assert_eq!(t.call("DELETE", "/admin/coaches/coach2", Some("admin1"), Some(confirm("coach1@dev.local"))).await.0, StatusCode::BAD_REQUEST);
    let (s, d) = t.call("DELETE", "/admin/coaches/coach2", Some("admin1"), Some(confirm("Coach2@dev.local"))).await;
    assert_eq!(s, StatusCode::OK, "{d}");
    assert_eq!(t.call("GET", "/me", Some("coach2"), None).await.1["profile"], Value::Null);
    let (_, coaches) = t.call("GET", "/admin/coaches", Some("admin1"), None).await;
    assert!(!coaches.as_array().unwrap().iter().any(|c| c["coachId"] == "coach2"), "off the admin's list");
    let left = ddb
        .query()
        .table_name(&table)
        .key_condition_expression("pk = :pk")
        .expression_attribute_values(":pk", aws_sdk_dynamodb::types::AttributeValue::S("COACH#coach2".into()))
        .send()
        .await
        .unwrap();
    assert_eq!(left.count(), 0, "the coach's records are gone");

    // --- loads as %1RM (WB-5): resolved from the client's maxes, in pounds
    const LB: f64 = 0.453_592_37;
    let pct_item = |id: &str, pct: f64| json!({ "exerciseId": id, "exerciseName": id, "sets": 3, "reps": "5", "loadPct": pct });
    let (s, pct_prog) = t
        .call("POST", "/programs", Some("coach1"), Some(json!({
            "name": "Pct block", "weeks": 1,
            "workouts": [{ "week": 1, "day": 1, "workout": { "name": "Heavy", "blocks": [{ "kind": "straight", "items": [pct_item("squat", 75.0), pct_item("bench", 70.0)] }] } }]
        })))
        .await;
    assert_eq!(s, StatusCode::OK, "{pct_prog}");
    let max = |name: &str, lb: f64| json!({ "exerciseName": name, "kg": lb * LB, "source": "tested" });
    assert_eq!(t.call("PUT", "/clients/client1/maxes/squat", Some("client1"), Some(json!({ "exerciseName": "Squat", "kg": 0, "source": "tested" }))).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(t.call("PUT", "/clients/client1/maxes/squat", Some("client2"), Some(max("Squat", 275.0))).await.0, StatusCode::FORBIDDEN);
    let (s, m) = t.call("PUT", "/clients/client1/maxes/squat", Some("client1"), Some(max("Squat", 275.0))).await;
    assert_eq!(s, StatusCode::OK, "{m}");
    assert_eq!(m["max"]["setBy"], "client");
    let day = today + chrono::Duration::days(200);
    let (s, a) = t
        .call("POST", "/clients/client1/assignments", Some("coach1"), Some(json!({ "programId": pct_prog["id"], "startDate": day.to_string() })))
        .await;
    assert_eq!(s, StatusCode::OK, "{a}");
    let heavy = |t2: &Value| t2.as_array().unwrap().iter().find(|w| w["workout"]["name"] == "Heavy").unwrap()["workout"]["blocks"][0]["items"].clone();
    let sched_path = format!("/clients/client1/schedule?from={day}&to={day}");
    let items = heavy(&t.call("GET", &sched_path, Some("client1"), None).await.1);
    let lbs = |v: &Value| v.as_f64().map(|kg| (kg / LB * 100.0).round() / 100.0);
    assert_eq!(lbs(&items[0]["loadKg"]), Some(205.0), "75% of 275 lb, to the nearest 5 lb");
    assert_eq!(items[1]["loadKg"], Value::Null, "no bench max yet: the client picks");
    assert_eq!(items[1]["loadPct"], 70.0);
    // a new max re-weighs the upcoming workout
    let (s, m) = t.call("PUT", "/clients/client1/maxes/bench", Some("coach1"), Some(max("Bench", 225.0))).await;
    assert_eq!(s, StatusCode::OK, "{m}");
    assert_eq!(m["updatedWorkouts"], 1);
    let items = heavy(&t.call("GET", &sched_path, Some("client1"), None).await.1);
    assert_eq!(lbs(&items[1]["loadKg"]), Some(160.0), "70% of 225 = 157.5 lb, rounds to 160");
    let (_, list) = t.call("GET", "/clients/client1/maxes", Some("coach1"), None).await;
    let names: Vec<&str> = list["maxes"].as_array().unwrap().iter().map(|m| m["exerciseName"].as_str().unwrap()).collect();
    assert_eq!(names, ["Bench", "Squat"]);
    assert!(list["estimates"].is_array());
    let (s, r) = t.call("DELETE", "/clients/client1/maxes/bench", Some("coach1"), None).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["updatedWorkouts"], 1);
    assert_eq!(heavy(&t.call("GET", &sched_path, Some("client1"), None).await.1)[1]["loadKg"], Value::Null);
    assert_eq!(t.call("DELETE", "/clients/client1/maxes/bench", Some("coach1"), None).await.0, StatusCode::NOT_FOUND);

    // --- personal records (LG-5): a baseline first, then a heavier set is a record
    let pr_log = |id: &str, kg: f64| json!({
        "id": id, "date": today.to_string(), "workoutName": "PR day", "status": "completed",
        "updatedAt": chrono::Utc::now().to_rfc3339(),
        "sets": [{ "itemId": "x", "exerciseId": "pr-lift", "exerciseName": "PR Lift", "setIndex": 0, "reps": 5, "loadKg": kg, "done": true }]
    });
    assert_eq!(t.call("PUT", "/clients/client1/logs/pr1", Some("client1"), Some(pr_log("pr1", 100.0))).await.0, StatusCode::OK);
    assert_eq!(t.call("GET", "/clients/client1/records?log=pr1", Some("client1"), None).await.1, json!([]), "a baseline isn't celebrated");
    assert_eq!(t.call("PUT", "/clients/client1/logs/pr2", Some("client1"), Some(pr_log("pr2", 105.0))).await.0, StatusCode::OK);
    let (_, new) = t.call("GET", "/clients/client1/records?log=pr2", Some("client1"), None).await;
    let keys: Vec<&str> = new.as_array().unwrap().iter().map(|r| r["key"].as_str().unwrap()).collect();
    assert_eq!(keys, ["pr-lift#E1RM", "pr-lift#R05"]);
    assert_eq!(new[1]["previous"]["kg"], 100.0);
    // re-saving the same log doesn't count it again
    assert_eq!(t.call("PUT", "/clients/client1/logs/pr2", Some("client1"), Some(pr_log("pr2", 105.0))).await.0, StatusCode::OK);
    assert_eq!(t.call("GET", "/clients/client1/records?log=pr2", Some("client1"), None).await.1.as_array().unwrap().len(), 2);
    assert_eq!(t.call("GET", "/clients/client1/records", Some("client2"), None).await.0, StatusCode::FORBIDDEN);
    // the coach removes a bogus record; the one before it comes back
    let (s, r) = t.call("DELETE", "/clients/client1/records/pr-lift%23R05", Some("coach1"), None).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["restored"]["kg"], 100.0);
    let (_, all) = t.call("GET", "/clients/client1/records", Some("coach1"), None).await;
    assert!(all.as_array().unwrap().iter().any(|r| r["key"] == "pr-lift#R05" && r["kg"] == 100.0));

    // --- progression suggestions (WB-6): a met rule suggests the next workout's load
    let rule_item = |kg: f64| json!({ "exerciseId": "press", "exerciseName": "Press", "sets": 2, "reps": "5", "loadKg": kg,
        "progression": { "maxRpe": 8, "addKg": 2.5, "capKg": 70 } });
    let mut bad_rule = rule_item(60.0);
    bad_rule["progression"] = json!({ "maxRpe": 8 });
    let prog_body = |items: [Value; 2]| json!({ "name": "Progressive", "weeks": 1, "workouts": [
        { "week": 1, "day": 1, "workout": { "name": "P1", "blocks": [{ "kind": "straight", "items": [items[0].clone()] }] } },
        { "week": 1, "day": 3, "workout": { "name": "P2", "blocks": [{ "kind": "straight", "items": [items[1].clone()] }] } }
    ] });
    assert_eq!(t.call("POST", "/programs", Some("coach1"), Some(prog_body([bad_rule, rule_item(60.0)]))).await.0, StatusCode::BAD_REQUEST, "a rule must add something");
    let (s, pp) = t.call("POST", "/programs", Some("coach1"), Some(prog_body([rule_item(60.0), rule_item(60.0)]))).await;
    assert_eq!(s, StatusCode::OK, "{pp}");
    let pday = today + chrono::Duration::days(300);
    let (s, a) = t.call("POST", "/clients/client1/assignments", Some("coach1"), Some(json!({ "programId": pp["id"], "startDate": pday.to_string() }))).await;
    assert_eq!(s, StatusCode::OK, "{a}");
    let range = format!("/clients/client1/schedule?from={pday}&to={}", pday + chrono::Duration::days(6));
    let (_, ws) = t.call("GET", &range, Some("client1"), None).await;
    let find = |ws: &Value, name: &str| ws.as_array().unwrap().iter().find(|w| w["workout"]["name"] == name).unwrap().clone();
    let p1 = find(&ws, "P1");
    let item_id = p1["workout"]["blocks"][0]["items"][0]["id"].as_str().unwrap().to_string();
    let press_log = |rpe: f64| json!({
        "id": "press1", "scheduledId": p1["id"], "date": pday.to_string(), "workoutName": "P1", "status": "completed",
        "updatedAt": chrono::Utc::now().to_rfc3339(),
        "sets": ([0, 1].iter().map(|i| json!({ "itemId": item_id, "exerciseId": "press", "exerciseName": "Press", "setIndex": i, "reps": 5, "loadKg": 60.0, "rpe": rpe, "done": true })).collect::<Vec<_>>())
    });
    // too hard: nothing suggested
    assert_eq!(t.call("PUT", "/clients/client1/logs/press1", Some("client1"), Some(press_log(9.0))).await.0, StatusCode::OK);
    assert_eq!(t.call("GET", "/suggestions", Some("coach1"), None).await.1, json!([]));
    // within the RPE: a suggestion for P2 (re-saving files it only once)
    assert_eq!(t.call("PUT", "/clients/client1/logs/press1", Some("client1"), Some(press_log(8.0))).await.0, StatusCode::OK);
    assert_eq!(t.call("PUT", "/clients/client1/logs/press1", Some("client1"), Some(press_log(7.5))).await.0, StatusCode::OK);
    assert_eq!(t.call("GET", "/suggestions", Some("client1"), None).await.0, StatusCode::FORBIDDEN);
    let (_, sugg) = t.call("GET", "/suggestions", Some("coach1"), None).await;
    let sugg = sugg.as_array().unwrap();
    assert_eq!(sugg.len(), 1, "{sugg:?}");
    assert_eq!(sugg[0]["proposed"]["loadKg"], 62.5);
    assert_eq!(sugg[0]["current"]["loadKg"], 60.0);
    assert!(!sugg[0]["clientName"].as_str().unwrap().is_empty());
    let sid = sugg[0]["id"].as_str().unwrap().to_string();
    let long = json!({ "reps": "x".repeat(21) });
    assert_eq!(t.call("POST", &format!("/suggestions/{sid}/accept"), Some("coach1"), Some(long)).await.0, StatusCode::BAD_REQUEST);
    let (s, acc) = t.call("POST", &format!("/suggestions/{sid}/accept"), Some("coach1"), Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{acc}");
    let (_, ws) = t.call("GET", &range, Some("client1"), None).await;
    assert_eq!(find(&ws, "P2")["workout"]["blocks"][0]["items"][0]["loadKg"], 62.5, "the next workout now has the new load");
    assert_eq!(t.call("POST", &format!("/suggestions/{sid}/accept"), Some("coach1"), Some(json!({}))).await.0, StatusCode::BAD_REQUEST, "already handled");
    assert_eq!(t.call("GET", "/suggestions", Some("coach1"), None).await.1, json!([]));

    // --- saved reply snippets (FB-4)
    assert_eq!(t.call("GET", "/snippets", Some("client1"), None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(t.call("POST", "/snippets", Some("coach1"), Some(json!({ "title": " ", "text": "x" }))).await.0, StatusCode::BAD_REQUEST);
    let (s, sn) = t.call("POST", "/snippets", Some("coach1"), Some(json!({ "title": "Depth", "text": "  Great depth, {name}!  " }))).await;
    assert_eq!(s, StatusCode::OK, "{sn}");
    assert_eq!(sn["text"], "Great depth, {name}!");
    t.call("POST", "/snippets", Some("coach1"), Some(json!({ "title": "brace", "text": "Big breath and brace before each rep." }))).await;
    let sn_path = format!("/snippets/{}", sn["id"].as_str().unwrap());
    let (s, up) = t.call("PUT", &sn_path, Some("coach1"), Some(json!({ "title": "Depth", "text": "Hit depth on every rep, {name}." }))).await;
    assert_eq!(s, StatusCode::OK, "{up}");
    let (_, all) = t.call("GET", "/snippets", Some("coach1"), None).await;
    let titles: Vec<&str> = all.as_array().unwrap().iter().map(|s| s["title"].as_str().unwrap()).collect();
    assert_eq!(titles, ["brace", "Depth"], "by name, ignoring case");
    assert_eq!(t.call("GET", "/snippets", Some("coach2"), None).await.0, StatusCode::FORBIDDEN, "coach2 was deleted above");
    assert_eq!(t.call("DELETE", &sn_path, Some("coach1"), None).await.0, StatusCode::OK);
    assert_eq!(t.call("DELETE", &sn_path, Some("coach1"), None).await.0, StatusCode::NOT_FOUND);

    // --- messages about a workout, set or video (MS-3)
    let (s, c) = t.call("POST", "/clients/client1/comments", Some("client1"), Some(json!({ "text": "Was this too heavy?", "logId": "pr2" }))).await;
    assert_eq!(s, StatusCode::OK, "{c}");
    assert_eq!(c["about"]["workoutName"], "PR day");
    assert_eq!(c["about"]["date"], today.to_string());
    let (_, c) = t.call("POST", "/clients/client1/comments", Some("coach1"), Some(json!({ "text": "Nice set", "logId": "pr2", "itemId": "x", "setIndex": 0 }))).await;
    assert_eq!(c["about"]["exerciseName"], "PR Lift");
    let (_, c) = t.call("POST", "/clients/client1/comments", Some("coach1"), Some(json!({ "text": "?", "logId": "not-synced-yet" }))).await;
    assert!(c.get("about").is_none(), "a log the server doesn't have yet is still a valid anchor");
    let (_, vids) = t.call("GET", "/clients/client1/videos", Some("coach1"), None).await;
    let v0 = &vids.as_array().unwrap()[0];
    let (s, c) = t.call("POST", "/clients/client1/comments", Some("coach1"), Some(json!({ "text": "Watch the knees", "videoId": v0["id"] }))).await;
    assert_eq!(s, StatusCode::OK, "{c}");
    assert_eq!(c["videoId"], v0["id"]);
    assert_eq!(c["logId"], v0["logId"]);
    assert_eq!(c["setIndex"], v0["setIndex"]);
    assert_eq!(t.call("POST", "/clients/client1/comments", Some("coach1"), Some(json!({ "text": "x", "videoId": "nope" }))).await.0, StatusCode::BAD_REQUEST);

    // --- filming guidance (VS-5): built-ins have defaults; coaches set it on their own exercises
    let (_, lib) = t.call("GET", "/exercises", Some("client1"), None).await;
    let squat = lib.as_array().unwrap().iter().find(|e| e["id"] == "back-squat").unwrap();
    assert_eq!(squat["filming"]["angle"], "diagonal");
    let film = |d: f64| json!({ "name": "Sled Push", "filming": { "angle": "side", "distanceM": d, "notes": " Whole sled in frame " } });
    assert_eq!(t.call("POST", "/exercises", Some("coach1"), Some(film(20.0))).await.0, StatusCode::BAD_REQUEST);
    let (s, sled) = t.call("POST", "/exercises", Some("coach1"), Some(film(4.0))).await;
    assert_eq!(s, StatusCode::OK, "{sled}");
    assert_eq!(sled["filming"]["notes"], "Whole sled in frame");
    let (_, sled) = t.call("PUT", &format!("/exercises/{}", sled["id"].as_str().unwrap()), Some("coach1"), Some(json!({ "name": "Sled Push" }))).await;
    assert!(sled.get("filming").is_none(), "leaving it out clears it");

    // --- alert inbox (LG-7): the pain reported on log-abc is waiting for the coach
    assert_eq!(t.call("GET", "/alerts", Some("client1"), None).await.0, StatusCode::FORBIDDEN);
    let (_, inbox) = t.call("GET", "/alerts", Some("coach1"), None).await;
    assert_eq!(inbox["settings"]["missedInARow"], 2);
    let pain = inbox["alerts"].as_array().unwrap().iter().find(|a| a["id"] == "pain.log-abc").expect("a pain alert").clone();
    assert_eq!(pain["kind"], "pain");
    assert!(pain["text"].as_str().unwrap().contains("left knee"), "{pain}");
    // re-saving that log doesn't file it again
    t.call("PUT", "/clients/client1/logs/log-abc", Some("client1"), Some(log(&chrono::Utc::now().to_rfc3339(), 5))).await;
    let (_, inbox) = t.call("GET", "/alerts", Some("coach1"), None).await;
    assert_eq!(inbox["alerts"].as_array().unwrap().iter().filter(|a| a["id"] == "pain.log-abc").count(), 1);
    assert_eq!(t.call("POST", "/alerts/pain.log-abc/done", Some("coach1"), None).await.0, StatusCode::OK);
    let (_, inbox) = t.call("GET", "/alerts", Some("coach1"), None).await;
    assert!(!inbox["alerts"].as_array().unwrap().iter().any(|a| a["id"] == "pain.log-abc"), "done alerts leave the inbox");
    assert_eq!(t.call("PUT", "/alerts/settings", Some("coach1"), Some(json!({ "missedInARow": 0 }))).await.0, StatusCode::BAD_REQUEST);
    let (s, set) = t.call("PUT", "/alerts/settings", Some("coach1"), Some(json!({ "missedInARow": 3 }))).await;
    assert_eq!(s, StatusCode::OK, "{set}");
    assert_eq!(t.call("GET", "/clients", Some("coach1"), None).await.0, StatusCode::OK, "the roster checks for missed runs");

    // --- weekly check-ins (FB-3)
    let (_, tpl) = t.call("GET", "/checkin-template", Some("client1"), None).await;
    assert_eq!(tpl["questions"][0]["id"], "overall", "the default questions");
    let mut mine = tpl.clone();
    mine["questions"] = json!([
        { "id": "overall", "kind": "scale", "label": " How was the week? ", "required": true },
        { "id": "steps", "kind": "number", "label": "Average daily steps", "unit": "steps" },
        { "id": "travel", "kind": "yesno", "label": "Travelling?" }
    ]);
    assert_eq!(t.call("PUT", "/checkin-template", Some("client1"), Some(mine.clone())).await.0, StatusCode::FORBIDDEN);
    let (s, saved) = t.call("PUT", "/checkin-template", Some("coach1"), Some(mine)).await;
    assert_eq!(s, StatusCode::OK, "{saved}");
    assert_eq!(saved["questions"][0]["label"], "How was the week?");
    let (_, ci) = t.call("GET", "/clients/client1/checkins", Some("client1"), None).await;
    let week = ci["thisWeek"].as_str().unwrap().to_string();
    assert_eq!(ci["template"]["questions"].as_array().unwrap().len(), 3);
    let path = format!("/clients/client1/checkins/{week}");
    assert_eq!(t.call("PUT", &path, Some("client1"), Some(json!({ "answers": { "steps": 9000 } }))).await.0, StatusCode::BAD_REQUEST, "a required answer is missing");
    assert_eq!(t.call("PUT", &path, Some("coach1"), Some(json!({ "answers": { "overall": 8 } }))).await.0, StatusCode::FORBIDDEN);
    let next = (chrono::NaiveDate::parse_from_str(&week, "%Y-%m-%d").unwrap() + chrono::Duration::days(7)).to_string();
    assert_eq!(t.call("PUT", &format!("/clients/client1/checkins/{next}"), Some("client1"), Some(json!({ "answers": { "overall": 8 } }))).await.0, StatusCode::BAD_REQUEST, "not next week");
    let (s, done) = t.call("PUT", &path, Some("client1"), Some(json!({ "answers": { "overall": 8, "steps": 9000, "travel": true } }))).await;
    assert_eq!(s, StatusCode::OK, "{done}");
    let (_, ci) = t.call("GET", "/clients/client1/checkins", Some("coach1"), None).await;
    assert_eq!(ci["checkins"][0]["answers"]["steps"], 9000);
    assert_eq!(ci["checkins"][0]["questions"][1]["label"], "Average daily steps", "it keeps the questions it answered");

    // --- automated messages and reminders (MS-4): the hourly sweep
    let (_, auto) = t.call("GET", "/automation", Some("coach1"), None).await;
    assert_eq!(auto["checkinReminder"], json!({ "days": [6], "hour": 17 }), "check-in reminders default to Sunday 17:00");
    let now = chrono::Utc::now();
    let (_, c1) = t.call("GET", "/clients/client1", Some("coach1"), None).await;
    let hour = ptcore::automation::local_time(now, c1["profile"]["timeZone"].as_str()).format("%H").to_string().parse::<u32>().unwrap();
    let rules = json!({
        "checkinReminder": { "days": [], "hour": hour },
        "workoutReminder": null,
        "messages": [{ "id": "hello", "text": " Automated hello, {name} ", "when": { "days": [], "hour": hour } }]
    });
    assert_eq!(t.call("PUT", "/automation", Some("client1"), Some(rules.clone())).await.0, StatusCode::FORBIDDEN);
    let mut bad = rules.clone();
    bad["messages"][0]["when"]["hour"] = json!(24);
    assert_eq!(t.call("PUT", "/automation", Some("coach1"), Some(bad)).await.0, StatusCode::BAD_REQUEST);
    let (s, saved) = t.call("PUT", "/automation", Some("coach1"), Some(rules)).await;
    assert_eq!(s, StatusCode::OK, "{saved}");
    assert_eq!(saved["messages"][0]["text"], "Automated hello, {name}");
    let report = api::sweep::run(&state, now).await;
    assert_eq!(report.errors, 0, "{report:?}");
    assert!(report.messages >= 1, "{report:?}");
    assert_eq!(report.checkin_reminders, 0, "client1 already sent this week's check-in");
    let (_, thread) = t.call("GET", "/clients/client1/comments", Some("client1"), None).await;
    let hello: Vec<&Value> = thread.as_array().unwrap().iter().filter(|c| c["text"].as_str().is_some_and(|t| t.starts_with("Automated hello, "))).collect();
    assert_eq!(hello.len(), 1);
    assert_eq!(hello[0]["authorRole"], "coach");
    assert!(!hello[0]["text"].as_str().unwrap().contains("{name}"));
    // a second sweep in the same hour sends nothing again (once per day per item)
    let again = api::sweep::run(&state, now).await;
    assert_eq!((again.messages, again.errors), (0, 0), "{again:?}");

    // --- habits (BZ-3)
    let (s, water) = t.call("POST", "/clients/client1/habits", Some("coach1"), Some(json!({ "name": " Water ", "kind": "count", "target": 8, "unit": "glasses" }))).await;
    assert_eq!(s, StatusCode::OK, "{water}");
    assert_eq!(water["name"], "Water");
    let (_, creatine) = t.call("POST", "/clients/client1/habits", Some("client1"), Some(json!({ "name": "Creatine", "kind": "check", "target": 5 }))).await;
    assert_eq!(creatine["target"], 1.0, "a check's target is 1");
    assert_eq!(t.call("POST", "/clients/client1/habits", Some("coach1"), Some(json!({ "name": "x", "kind": "count", "target": 0 }))).await.0, StatusCode::BAD_REQUEST);
    let wid = water["id"].as_str().unwrap().to_string();
    let (_, me1) = t.call("GET", "/clients/client1/habits", Some("client1"), None).await;
    let local_today = chrono::NaiveDate::parse_from_str(me1["today"].as_str().unwrap(), "%Y-%m-%d").unwrap();
    let day = |n: i64| (local_today - chrono::Duration::days(n)).to_string();
    for (n, v) in [(2, 8.0), (1, 9.0), (0, 5.0)] {
        let (s, r) = t.call("PUT", &format!("/clients/client1/habits/{wid}/days/{}", day(n)), Some("client1"), Some(json!({ "value": v }))).await;
        assert_eq!(s, StatusCode::OK, "{r}");
    }
    assert_eq!(t.call("PUT", &format!("/clients/client1/habits/{wid}/days/{}", day(0)), Some("coach1"), Some(json!({ "value": 1 }))).await.0, StatusCode::FORBIDDEN);
    assert_eq!(t.call("PUT", &format!("/clients/client1/habits/{wid}/days/{}", day(8)), Some("client1"), Some(json!({ "value": 1 }))).await.0, StatusCode::BAD_REQUEST, "too long ago");
    assert_eq!(t.call("PUT", &format!("/clients/client1/habits/{wid}/days/{}", day(-1)), Some("client1"), Some(json!({ "value": 1 }))).await.0, StatusCode::BAD_REQUEST, "not tomorrow");
    let (_, hs) = t.call("GET", "/clients/client1/habits", Some("coach1"), None).await;
    assert_eq!(hs["streaks"][&wid], 2, "today (5 of 8) isn't met yet, so yesterday's run of 2 stands");
    assert_eq!(hs["logs"].as_array().unwrap().iter().filter(|l| l["habitId"] == wid.as_str()).count(), 3);
    // reaching the target today extends it; 0 clears a day
    t.call("PUT", &format!("/clients/client1/habits/{wid}/days/{}", day(0)), Some("client1"), Some(json!({ "value": 8 }))).await;
    assert_eq!(t.call("GET", "/clients/client1/habits", Some("client1"), None).await.1["streaks"][&wid], 3);
    t.call("PUT", &format!("/clients/client1/habits/{wid}/days/{}", day(1)), Some("client1"), Some(json!({ "value": 0 }))).await;
    assert_eq!(t.call("GET", "/clients/client1/habits", Some("client1"), None).await.1["streaks"][&wid], 1);
    // archived habits leave Today but keep their history
    let (s, arch) = t.call("PUT", &format!("/clients/client1/habits/{wid}"), Some("coach1"), Some(json!({ "name": "Water", "kind": "count", "target": 8, "unit": "glasses", "active": false }))).await;
    assert_eq!(s, StatusCode::OK, "{arch}");
    let (_, hs) = t.call("GET", "/clients/client1/habits", Some("client1"), None).await;
    assert!(hs["streaks"].get(&wid).is_none());
    assert!(hs["habits"].as_array().unwrap().iter().any(|h| h["id"] == wid.as_str() && h["active"] == false));

    // --- food journal: client logs meals, coach searches and estimates
    let (s, lunch) = t.call("POST", "/clients/client1/meals", Some("client1"), Some(json!({
        "date": day(0), "label": " Lunch ",
        "items": [{ "text": "chicken 12oz" }, { "text": "rice" }]
    }))).await;
    assert_eq!(s, StatusCode::OK, "{lunch}");
    assert_eq!(lunch["label"].as_str().unwrap(), "Lunch");
    assert_eq!(lunch["items"][0]["text"].as_str().unwrap(), "chicken", "the typed amount parses into grams");
    let chicken_g: f64 = lunch["items"][0]["grams"].as_f64().unwrap_or(0.0);
    assert!(chicken_g > 340.0 && chicken_g < 341.0, "12 oz ≈ {chicken_g} g");
    assert_eq!(lunch["items"][1]["text"].as_str().unwrap(), "rice", "no amount stays as typed");
    assert_eq!(t.call("POST", "/clients/client1/meals", Some("client1"), Some(json!({ "date": day(0), "items": [] }))).await.0, StatusCode::BAD_REQUEST, "an empty meal is refused");
    assert_eq!(t.call("POST", "/clients/client1/meals", Some("client1"), Some(json!({ "date": day(8), "items": [{ "text": "x" }] }))).await.0, StatusCode::BAD_REQUEST, "not before the editable window");
    let lid = lunch["id"].as_str().unwrap();
    // the coach attaches an estimate (search is off without an FDC key, so the
    // snapshot comes from a lookup run elsewhere); a coach's number wins
    let (_, est) = t.call("PUT", &format!("/clients/client1/meals/{lid}"), Some("coach1"), Some(json!({
        "date": day(0), "label": "Lunch",
        "items": [
            { "text": "chicken", "grams": 340.2, "food": { "fdcId": "1702589", "description": "Chicken breast", "perGrams": 100.0, "kcalPer": 165.0 }, "basis": "searched" },
            { "text": "rice", "kcal": 200.0, "basis": "manual" }
        ]
    }))).await;
    assert_eq!(est["items"].as_array().unwrap().len(), 2);
    let (_, ms) = t.call("GET", "/clients/client1/meals?days=7", Some("coach1"), None).await;
    let total: f64 = ms["totals"][day(0)]["kcal"].as_f64().unwrap_or(0.0);
    assert!(total > 760.0 && total < 762.0, "165 kcal/100 g × 340.2 g + 200 = {total}");
    assert_eq!(ms["week"]["logged"], 1, "the week's summary counts logged days");
    assert!(ms["target"].is_null(), "no targets set yet");
    // targets and CSV import, in their own future: inline, the journey's
    // future outgrows the test thread's stack
    Box::pin(nutrition_targets_and_import(&t, &day)).await;
    assert_eq!(t.call("GET", "/clients/client1/meals", Some("coach2"), None).await.0, StatusCode::FORBIDDEN, "another coach's client");
    // food search: coach-only, and it says so when the key is off
    assert_eq!(t.call("GET", "/foods/search?q=chicken+breast", Some("client1"), None).await.0, StatusCode::FORBIDDEN);
    let (s, off) = t.call("GET", "/foods/search?q=chicken+breast", Some("coach1"), None).await;
    assert_eq!(s, StatusCode::OK, "no FDC key: search is off, which is not an error");
    assert!(off["off"].as_str().is_some_and(|m| m.contains("off")) && off["candidates"].as_array().unwrap().is_empty());
    assert_eq!(t.call("GET", "/foods/search?q=", Some("coach1"), None).await.0, StatusCode::BAD_REQUEST, "needs words");
    // a coach's saved staples live in the coach's partition
    let (s, staple) = t.call("POST", "/foods", Some("coach1"), Some(json!({ "name": " Usual lunch ", "snap": { "fdcId": "1702589", "perGrams": 100.0, "kcalPer": 165.0 } }))).await;
    assert_eq!(s, StatusCode::OK, "{staple}");
    assert_eq!(staple["name"].as_str().unwrap(), "Usual lunch");
    assert_eq!(t.call("POST", "/foods", Some("client1"), Some(json!({ "name": "x" }))).await.0, StatusCode::FORBIDDEN);
    // the client can delete their own meal, the coach cannot
    assert_eq!(t.call("DELETE", &format!("/clients/client1/meals/{lid}"), Some("coach1"), None).await.0, StatusCode::FORBIDDEN);
    let (s, gone) = t.call("DELETE", &format!("/clients/client1/meals/{lid}"), Some("client1"), None).await;
    assert_eq!(s, StatusCode::OK, "{gone}");
    let (_, ms) = t.call("GET", "/clients/client1/meals", Some("client1"), None).await;
    assert_eq!(ms["meals"].as_array().unwrap().len(), 0, "the estimate went with the meal");
    assert_eq!(t.call("POST", "/foods", Some("coach1"), Some(json!({ "name": "  " }))).await.0, StatusCode::BAD_REQUEST, "a blank name is refused");

    // --- scheduling (BZ-2): availability, open slots, booking, cancelling, the calendar feed
    let every_day = (0..7).map(|d| json!({ "day": d, "start": "08:00:00", "end": "10:00:00" })).collect::<Vec<_>>();
    assert_eq!(t.call("PUT", "/availability", Some("coach1"), Some(json!({ "windows": [{ "day": 0, "start": "10:00:00", "end": "09:00:00" }], "minutes": 60 }))).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(t.call("PUT", "/availability", Some("client1"), Some(json!({ "windows": every_day, "minutes": 60 }))).await.0, StatusCode::FORBIDDEN);
    let (s, av) = t.call("PUT", "/availability", Some("coach1"), Some(json!({ "windows": every_day, "minutes": 60, "noticeHours": 0, "location": " Gym floor " }))).await;
    assert_eq!(s, StatusCode::OK, "{av}");
    let (_, open) = t.call("GET", "/clients/client1/slots?days=3", Some("client1"), None).await;
    assert_eq!(open["timeZone"], "America/Chicago", "slots are in the coach's zone");
    let slots = open["slots"].as_array().unwrap().clone();
    assert!(slots.len() >= 4, "{open}");
    let first = slots[0].clone();
    let (s, booked) = t.call("POST", "/clients/client1/sessions", Some("client1"), Some(json!({ "start": first, "notes": "Deadlift technique" }))).await;
    assert_eq!(s, StatusCode::OK, "{booked}");
    assert_eq!((booked["minutes"].as_u64(), booked["location"].as_str()), (Some(60), Some("Gym floor")));
    assert_eq!(t.call("POST", "/clients/client1/sessions", Some("client1"), Some(json!({ "start": first }))).await.0, StatusCode::CONFLICT, "taken");
    let (_, open2) = t.call("GET", "/clients/client1/slots?days=3", Some("client1"), None).await;
    assert!(!open2["slots"].as_array().unwrap().contains(&first));
    assert_eq!(t.call("POST", "/clients/client1/sessions", Some("client1"), Some(json!({ "start": "2030-01-01T03:17:00Z" }))).await.0, StatusCode::BAD_REQUEST, "not a slot, and too far ahead");
    // the coach books outside their hours, but not over another session
    assert_eq!(t.call("POST", "/clients/client1/sessions", Some("coach1"), Some(json!({ "start": first, "minutes": 30 }))).await.0, StatusCode::BAD_REQUEST);
    let late = (chrono::Utc::now() + chrono::Duration::days(2)).format("%Y-%m-%dT22:00:00Z").to_string();
    let (s, extra) = t.call("POST", "/clients/client1/sessions", Some("coach1"), Some(json!({ "start": late, "minutes": 45 }))).await;
    assert_eq!(s, StatusCode::OK, "{extra}");
    let (_, mine) = t.call("GET", "/sessions", Some("client1"), None).await;
    assert_eq!(mine.as_array().unwrap().len(), 2);
    let (_, theirs) = t.call("GET", "/sessions", Some("coach1"), None).await;
    assert!(theirs.as_array().unwrap().iter().any(|x| x["id"] == booked["id"]));
    // the client cancels; the slot opens again
    let cancel = format!("/clients/client1/sessions/{}/cancel", booked["id"].as_str().unwrap());
    let (s, c) = t.call("POST", &cancel, Some("client1"), None).await;
    assert_eq!((s, c["status"].as_str()), (StatusCode::OK, Some("cancelled")));
    let (_, open3) = t.call("GET", "/clients/client1/slots?days=3", Some("client1"), None).await;
    assert!(open3["slots"].as_array().unwrap().contains(&first));
    // the private calendar feed: no sign-in; a reset turns the old link off
    let (_, cal) = t.call("GET", "/me/calendar", Some("client1"), None).await;
    let path = cal["path"].as_str().unwrap().to_string();
    assert!(path.starts_with("/cal/") && path.ends_with(".ics"));
    assert_eq!(t.call("GET", "/me/calendar", Some("client1"), None).await.1["path"], path.as_str(), "the same link each time");
    let (s, kind, body) = t.text(&path).await;
    assert_eq!(s, StatusCode::OK);
    assert!(kind.starts_with("text/calendar"), "{kind}");
    assert!(body.starts_with("BEGIN:VCALENDAR") && body.contains("Training session with") && body.contains("STATUS:CANCELLED"), "{body}");
    assert!(body.contains("SUMMARY:Workout: "), "a client's feed has their workouts");
    let (_, fresh) = t.call("POST", "/me/calendar/reset", Some("client1"), None).await;
    assert_ne!(fresh["path"], path.as_str());
    assert_eq!(t.text(&path).await.0, StatusCode::NOT_FOUND, "the old link is off");
    assert_eq!(t.text("/cal/not-a-token.ics").await.0, StatusCode::NOT_FOUND);

    // --- other body-fat estimates (BM-6): the US Navy tape formula, and readings from elsewhere
    let navy = |waist: f64| json!({ "date": today.to_string(), "kind": "bodyFat", "bodyFat": { "source": "navy", "navy": { "heightCm": 180, "neckCm": 38, "waistCm": waist } } });
    let (s, m) = t.call("POST", "/clients/client1/metrics", Some("client1"), Some(navy(85.0))).await;
    assert_eq!(s, StatusCode::OK, "{m}");
    assert_eq!(m["bodyFat"]["pct"], 16.1, "client1 is male: neck 38, waist 85, 180 cm");
    assert_eq!(t.call("POST", "/clients/client1/metrics", Some("client1"), Some(navy(30.0))).await.0, StatusCode::BAD_REQUEST, "waist under neck");
    let scan = |pct: f64| json!({ "date": today.to_string(), "kind": "bodyFat", "bodyFat": { "source": "dexa", "pct": pct } });
    assert_eq!(t.call("POST", "/clients/client1/metrics", Some("coach1"), Some(scan(90.0))).await.0, StatusCode::BAD_REQUEST);
    let (s, d) = t.call("POST", "/clients/client1/metrics", Some("coach1"), Some(scan(17.25))).await;
    assert_eq!((s, d["bodyFat"]["pct"].as_f64()), (StatusCode::OK, Some(17.3)));
    let (_, bf) = t.call("GET", "/clients/client1/metrics?kind=bodyFat", Some("coach1"), None).await;
    assert_eq!(bf.as_array().unwrap().len(), 2);

    // --- the coach's own intake questions (ON-5)
    assert_eq!(t.call("GET", "/intake-template", Some("client1"), None).await.1["questions"], json!([]), "none until the coach adds some");
    let extra = json!({ "questions": [
        { "id": "sport", "kind": "text", "label": " Your main sport ", "required": true },
        { "id": "days", "kind": "number", "label": "Days a week you can train", "unit": "days" }
    ] });
    assert_eq!(t.call("PUT", "/intake-template", Some("client1"), Some(extra.clone())).await.0, StatusCode::FORBIDDEN);
    let (s, saved) = t.call("PUT", "/intake-template", Some("coach1"), Some(extra)).await;
    assert_eq!((s, saved["questions"][0]["label"].as_str()), (StatusCode::OK, Some("Your main sport")));
    let intake = |answers: Value| json!({ "goals": "Squat 140", "equipment": ["barbell"], "sex": "male", "birthDate": "1996-01-15", "heightCm": 180, "extraAnswers": answers });
    assert_eq!(t.call("PUT", "/clients/client1/intake", Some("client1"), Some(intake(json!({ "days": 4 })))).await.0, StatusCode::BAD_REQUEST, "the required one is missing");
    let (s, saved) = t.call("PUT", "/clients/client1/intake", Some("client1"), Some(intake(json!({ "sport": "Powerlifting", "days": 4 })))).await;
    assert_eq!(s, StatusCode::OK, "{saved}");
    let (_, got) = t.call("GET", "/clients/client1/intake", Some("coach1"), None).await;
    assert_eq!(got["extraAnswers"]["sport"], "Powerlifting");
    assert_eq!(got["extraQuestions"][1]["unit"], "days", "kept with the answers");
    assert_eq!(t.call("PUT", "/intake-template", Some("coach1"), Some(json!({ "questions": [] }))).await.0, StatusCode::OK, "removable");

    // --- the starting point (ON-4): first entries until someone picks a date
    assert_eq!(t.call("GET", "/clients/client1/baseline", Some("coach1"), None).await.1["date"], Value::Null);
    let tomorrow = (today + chrono::Duration::days(1)).to_string();
    assert_eq!(t.call("PUT", "/clients/client1/baseline", Some("client1"), Some(json!({ "date": tomorrow }))).await.0, StatusCode::BAD_REQUEST);
    let start = (today - chrono::Duration::days(14)).to_string();
    let (s, b) = t.call("PUT", "/clients/client1/baseline", Some("coach1"), Some(json!({ "date": start }))).await;
    assert_eq!((s, b["setBy"].as_str()), (StatusCode::OK, Some("coach")));
    assert_eq!(t.call("GET", "/clients/client1/baseline", Some("client1"), None).await.1["date"], start.as_str());
    assert_eq!(t.call("GET", "/clients/client1/baseline", Some("client2"), None).await.0, StatusCode::FORBIDDEN);

    // --- coach time per client (the Phase 2 exit gate)
    let usage = |secs: u32| json!({ "entries": [{ "clientId": "client1", "seconds": secs }, { "clientId": "not-my-client", "seconds": 60 }] });
    assert_eq!(t.call("POST", "/usage", Some("client1"), Some(usage(60))).await.0, StatusCode::FORBIDDEN);
    assert_eq!(t.call("POST", "/usage", Some("coach1"), Some(usage(5000))).await.0, StatusCode::BAD_REQUEST);
    for _ in 0..3 {
        let (s, r) = t.call("POST", "/usage", Some("coach1"), Some(usage(120))).await;
        assert_eq!((s, r["added"].as_u64()), (StatusCode::OK, Some(120)), "only the coach's own client counts");
    }
    let (_, mine) = t.call("GET", "/usage?weeks=2", Some("coach1"), None).await;
    assert_eq!(mine["weeks"][1]["minutes"], 6);
    assert_eq!(mine["weeks"][1]["clientsSeen"], 1);
    assert!(mine["avgMinutesPerClientPerWeek"].as_f64().unwrap() > 0.0);
    let (s, theirs) = t.call("GET", "/admin/coaches/coach1/usage?weeks=2", Some("admin1"), None).await;
    assert_eq!((s, theirs["weeks"][1]["minutes"].as_u64()), (StatusCode::OK, Some(6)));
    assert_eq!(t.call("GET", "/admin/coaches/coach1/usage", Some("coach1"), None).await.0, StatusCode::FORBIDDEN);

    // --- security review: a suspended coach's calendar link stops working
    let (_, cal) = t.call("GET", "/me/calendar", Some("coach1"), None).await;
    let coach_feed = cal["path"].as_str().unwrap().to_string();
    assert_eq!(t.text(&coach_feed).await.0, StatusCode::OK);
    t.call("PUT", "/admin/coaches/coach1/suspended", Some("admin1"), Some(json!({ "suspended": true }))).await;
    assert_eq!(t.text(&coach_feed).await.0, StatusCode::NOT_FOUND);
    t.call("PUT", "/admin/coaches/coach1/suspended", Some("admin1"), Some(json!({ "suspended": false }))).await;
    assert_eq!(t.text(&coach_feed).await.0, StatusCode::NOT_FOUND, "suspending removed the link; the coach makes a new one");

    // --- security review: deleting a client's account removes the coach's copies of their sessions
    let (_, before) = t.call("GET", "/sessions", Some("coach1"), None).await;
    assert!(before.as_array().unwrap().iter().any(|s| s["clientId"] == "client1"));
    let (s, d) = t.call("DELETE", "/me/account", Some("client1"), Some(json!({ "confirm": "client1@dev.local" }))).await;
    assert_eq!(s, StatusCode::OK, "{d}");
    let (_, after) = t.call("GET", "/sessions", Some("coach1"), None).await;
    assert!(!after.as_array().unwrap().iter().any(|s| s["clientId"] == "client1"), "{after}");

    // --- the admin moves a client to another coach
    let (_, ci3) = t.call("POST", "/admin/invites", Some("admin1"), Some(json!({ "email": "coach3@dev.local", "name": "Coach Three" }))).await;
    assert_eq!(t.call("POST", &format!("/invites/{}/accept", ci3["code"].as_str().unwrap()), Some("coach3"), Some(json!({ "consent": true }))).await.0, StatusCode::OK);
    let (_, inv4) = t.call("POST", "/invites", Some("coach1"), Some(json!({ "email": "client4@dev.local", "clientName": "Four" }))).await;
    assert_eq!(t.call("POST", &format!("/invites/{}/accept", inv4["code"].as_str().unwrap()), Some("client4"), Some(json!({ "consent": true }))).await.0, StatusCode::OK);
    t.call("POST", "/clients/client4/comments", Some("client4"), Some(json!({ "text": "Hi coach one" }))).await;
    let soon = (chrono::Utc::now() + chrono::Duration::days(3)).format("%Y-%m-%dT21:00:00Z").to_string();
    let (s, sess) = t.call("POST", "/clients/client4/sessions", Some("coach1"), Some(json!({ "start": soon, "minutes": 30 }))).await;
    assert_eq!(s, StatusCode::OK, "{sess}");
    // a program with one of coach1's own exercises, starting today
    let (_, sled) = t.call("POST", "/exercises", Some("coach1"), Some(json!({ "name": "Sled Push", "cues": ["Low hips"] }))).await;
    let (_, prog) = t
        .call("POST", "/programs", Some("coach1"), Some(json!({ "name": "Move test", "weeks": 1, "workouts": [{ "week": 1, "day": 1, "workout": {
            "name": "A", "blocks": [{ "kind": "straight", "items": [{ "exerciseId": sled["id"], "exerciseName": "Sled Push", "sets": 1, "reps": "1" }] }] } }] })))
        .await;
    let start = chrono::Utc::now().date_naive().to_string();
    let (s, a) = t.call("POST", "/clients/client4/assignments", Some("coach1"), Some(json!({ "programId": prog["id"], "startDate": start }))).await;
    assert_eq!(s, StatusCode::OK, "{a}");
    let (_, c1) = t.call("GET", "/admin/coaches/coach1/clients", Some("admin1"), None).await;
    assert!(c1.as_array().unwrap().iter().any(|r| r["clientId"] == "client4"));
    assert_eq!(t.call("POST", "/admin/clients/client4/move", Some("coach1"), Some(json!({ "coachId": "coach3" }))).await.0, StatusCode::FORBIDDEN);
    assert_eq!(t.call("POST", "/admin/clients/client4/move", Some("admin1"), Some(json!({ "coachId": "coach1" }))).await.0, StatusCode::BAD_REQUEST, "already theirs");
    let (s, mv) = t.call("POST", "/admin/clients/client4/move", Some("admin1"), Some(json!({ "coachId": "coach3" }))).await;
    assert_eq!(s, StatusCode::OK, "{mv}");
    assert_eq!(mv["cancelledSessions"], 1);
    assert_eq!(mv["copiedExercises"], 1);
    let (_, lib) = t.call("GET", "/exercises", Some("client4"), None).await;
    assert!(lib.as_array().unwrap().iter().any(|e| e["id"] == sled["id"] && e["cues"][0] == "Low hips"), "the client keeps the old coach's exercise");
    assert_eq!(t.call("POST", "/admin/clients/client4/move", Some("admin1"), Some(json!({ "coachId": "coach3" }))).await.0, StatusCode::BAD_REQUEST, "nothing left to finish");
    assert_eq!(t.call("GET", "/me", Some("client4"), None).await.1["profile"]["coachId"], "coach3");
    assert_eq!(t.call("GET", "/clients/client4", Some("coach1"), None).await.0, StatusCode::FORBIDDEN, "the old coach has no access");
    let (_, thread) = t.call("GET", "/clients/client4/comments", Some("coach3"), None).await;
    assert!(thread.as_array().unwrap().iter().any(|c| c["text"] == "Hi coach one"), "the client's history moves with them");
    let (_, r1) = t.call("GET", "/clients", Some("coach1"), None).await;
    assert!(!r1.as_array().unwrap().iter().any(|r| r["client"]["clientId"] == "client4"));
    let (_, r3) = t.call("GET", "/clients", Some("coach3"), None).await;
    assert!(r3.as_array().unwrap().iter().any(|r| r["client"]["clientId"] == "client4"));
    let (_, mine) = t.call("GET", "/sessions", Some("client4"), None).await;
    assert_eq!(mine[0]["status"], "cancelled", "the session with the old coach is off");
    assert!(!t.call("GET", "/sessions", Some("coach1"), None).await.1.as_array().unwrap().iter().any(|x| x["clientId"] == "client4"));

    // back to coach1, who is now both the coach and a former coach: moving
    // there again finds nothing to finish and leaves coach1's roster alone
    let (s, back) = t.call("POST", "/admin/clients/client4/move", Some("admin1"), Some(json!({ "coachId": "coach1" }))).await;
    assert_eq!(s, StatusCode::OK, "{back}");
    assert_eq!(t.call("POST", "/admin/clients/client4/move", Some("admin1"), Some(json!({ "coachId": "coach1" }))).await.0, StatusCode::BAD_REQUEST);
    let (_, r1) = t.call("GET", "/clients", Some("coach1"), None).await;
    assert!(r1.as_array().unwrap().iter().any(|r| r["client"]["clientId"] == "client4"), "coach1 keeps the client");
}

/// BZ-4: the coach's nutrition targets and importing a Cronometer export.
/// Expects client1's lunch from the journey on `day(0)` (761 kcal).
async fn nutrition_targets_and_import(t: &T, day: &dyn Fn(i64) -> String) {
    // nutrition targets: the coach sets them, the client reads them with the journal
    assert_eq!(t.call("PUT", "/clients/client1/nutrition/target", Some("client1"), Some(json!({ "kcal": 2000 }))).await.0, StatusCode::FORBIDDEN, "only the coach sets targets");
    assert_eq!(t.call("PUT", "/clients/client1/nutrition/target", Some("coach1"), Some(json!({ "kcal": 50 }))).await.0, StatusCode::BAD_REQUEST, "an implausible target");
    let (s, tg) = t.call("PUT", "/clients/client1/nutrition/target", Some("coach1"), Some(json!({ "kcal": 800, "protein": 150, "note": " protein first " }))).await;
    assert_eq!(s, StatusCode::OK, "{tg}");
    assert_eq!(tg["target"]["note"], "protein first");
    let (_, ms) = t.call("GET", "/clients/client1/meals?days=1", Some("client1"), None).await;
    assert_eq!(ms["target"]["protein"], 150.0);
    assert_eq!(ms["week"]["onTarget"], 1, "761 kcal is within 10% of 800");
    assert_eq!(t.call("PUT", "/clients/client1/nutrition/target", Some("coach2"), Some(json!({ "kcal": 2000 }))).await.0, StatusCode::FORBIDDEN, "another coach");
    let (_, cleared) = t.call("PUT", "/clients/client1/nutrition/target", Some("coach1"), Some(json!({}))).await;
    assert!(cleared["target"].is_null(), "all empty clears the targets");
    // import a Cronometer export: rows group into meals, a re-import replaces them
    let csv = format!("Day,Time,Group,Food Name,Amount,Energy (kcal),Carbs (g),Fat (g),Protein (g)
{d1},8:00 AM,Breakfast,\"Oats, rolled\",40.00 g,150,27,2.5,5
{d1},8:01 AM,Breakfast,Milk,1.00 cup,103,12,2.4,8
{d2},12:00 PM,Lunch,Apple,182 g,95,25,0.3,0.5
2020-01-01,1:00 PM,Lunch,Too old,1 g,1,0,0,0
", d1 = day(1), d2 = day(2));
    let (s, imp) = t.call("POST", "/clients/client1/meals/import", Some("client1"), Some(json!({ "csv": csv }))).await;
    assert_eq!(s, StatusCode::OK, "{imp}");
    assert_eq!((imp["source"].as_str(), imp["meals"].as_i64(), imp["days"].as_i64(), imp["skipped"].as_i64()), (Some("cronometer"), Some(2), Some(2), Some(1)));
    t.call("POST", "/clients/client1/meals/import", Some("client1"), Some(json!({ "csv": csv }))).await;
    let (_, ms) = t.call("GET", "/clients/client1/meals?days=7", Some("coach1"), None).await;
    let imported: Vec<&Value> = ms["meals"].as_array().unwrap().iter().filter(|m| m["items"][0]["basis"] == "imported").collect();
    assert_eq!(imported.len(), 2, "importing again replaced, not doubled");
    assert_eq!(ms["totals"][day(1)]["protein"].as_f64(), Some(13.0), "imported macros count");
    assert_eq!(t.call("POST", "/clients/client1/meals/import", Some("client1"), Some(json!({ "csv": "Food,Calories
x,1
" }))).await.0, StatusCode::BAD_REQUEST, "no date column");
    assert_eq!(t.call("POST", "/clients/client1/meals/import", Some("coach2"), Some(json!({ "csv": csv }))).await.0, StatusCode::FORBIDDEN);
    for m in &imported {
        t.call("DELETE", &format!("/clients/client1/meals/{}", m["id"].as_str().unwrap()), Some("client1"), None).await;
    }
}
