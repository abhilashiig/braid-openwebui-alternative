//! Outbound email for invitations and password resets. Without SMTP, admins copy links from the UI instead.

use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::header::ContentType,
    transport::smtp::authentication::Credentials,
};

use crate::{settings, state::AppState};

/// Sends an email if SMTP is configured. Returns Ok(false) when it is not.
pub async fn send(state: &AppState, to: &str, subject: &str, body: &str) -> anyhow::Result<bool> {
    let s = settings::get(state).await.map_err(|e| anyhow::anyhow!(e.message))?;
    let (Some(host), Some(from)) = (s.smtp_host.filter(|h| !h.is_empty()), s.smtp_from.filter(|f| !f.is_empty())) else {
        return Ok(false);
    };
    let mut builder = match s.smtp_tls.as_str() {
        "tls" => AsyncSmtpTransport::<Tokio1Executor>::relay(&host)?,
        "none" => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&host),
        _ => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&host)?,
    }
    .port(s.smtp_port);
    if let Some(user) = s.smtp_username.filter(|u| !u.is_empty()) {
        builder = builder.credentials(Credentials::new(user, settings::smtp_password(state).await.unwrap_or_default()));
    }
    let msg = Message::builder()
        .from(from.parse()?)
        .to(to.parse()?)
        .subject(subject)
        .header(ContentType::TEXT_PLAIN)
        .body(body.to_string())?;
    builder.build().send(msg).await?;
    Ok(true)
}

/// Fire-and-forget variant for request paths; failures are logged.
pub fn send_later(state: &AppState, to: String, subject: String, body: String) {
    let state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = send(&state, &to, &subject, &body).await {
            tracing::warn!(to, "email failed: {e:#}");
        }
    });
}

pub async fn enabled(state: &AppState) -> bool {
    settings::get(state).await.is_ok_and(|s| s.smtp_host.is_some_and(|h| !h.is_empty()) && s.smtp_from.is_some_and(|f| !f.is_empty()))
}
