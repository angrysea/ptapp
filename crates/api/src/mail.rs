//! Outgoing email through Amazon SES (ON-1): invitations, sent from
//! INVITE_FROM (e.g. invites@gymtxcoach.com). The sending domain is an SES
//! identity in template.yaml; it sends once its DKIM records are in DNS, and
//! to any address once the account has SES production access (until then,
//! only to addresses verified in SES).

use aws_sdk_sesv2::types::{Body, Content, Destination, EmailContent, Message};
use ptcore::model::{Invite, InviteKind};

use crate::AppState;

/// The link an invitation's email carries; it opens the app's sign-up page.
pub fn invite_link(st: &AppState, invite: &Invite) -> String {
    format!("{}/#/join/{}", st.app_url.trim_end_matches('/'), invite.code)
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn text(s: String) -> Result<Content, String> {
    Content::builder().data(s).charset("UTF-8").build().map_err(|e| e.to_string())
}

/// Emails the invitation. `Ok(false)` when sending isn't configured (no
/// INVITE_FROM, e.g. local development); `Err` carries SES's reason.
pub async fn send_invite(st: &AppState, invite: &Invite) -> Result<bool, String> {
    let Some(from) = &st.invite_from else { return Ok(false) };
    let link = invite_link(st, invite);
    let (subject, what) = match invite.kind {
        InviteKind::Coach => ("You're invited to coach on PT Coach".to_string(), "set up a coach account on PT Coach".to_string()),
        InviteKind::Client => (format!("{} invited you to train on PT Coach", invite.coach_name), format!("train with {} on PT Coach", invite.coach_name)),
    };
    let hello = if invite.client_name.is_empty() { "Hi,".to_string() } else { format!("Hi {},", invite.client_name) };
    let plain = format!(
        "{hello}\n\nYou've been invited to {what}.\n\nCreate your account: {link}\n\nSign up with this email address ({email}). The link works once and expires in 14 days.\n",
        email = invite.email,
    );
    let html = format!(
        "<p>{hello}</p><p>You've been invited to {what}.</p><p><a href=\"{link}\">Create your account</a></p>\
         <p>Sign up with this email address ({email}). The link works once and expires in 14 days.</p>",
        hello = escape(&hello),
        what = escape(&what),
        link = escape(&link),
        email = escape(&invite.email),
    );
    let message = Message::builder()
        .subject(text(subject)?)
        .body(Body::builder().text(text(plain)?).html(text(html)?).build())
        .build();
    st.ses
        .send_email()
        .from_email_address(from)
        .destination(Destination::builder().to_addresses(&invite.email).build())
        .content(EmailContent::builder().simple(message).build())
        .send()
        .await
        .map_err(|e| format!("{}", aws_sdk_sesv2::error::DisplayErrorContext(e)))?;
    Ok(true)
}
