//! Usage rows go through a bounded channel to a batch writer so logging never blocks a request.

use std::time::Duration;

use sqlx::{PgPool, QueryBuilder};
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Debug, Default, Clone)]
pub struct UsageRow {
    pub source: &'static str,
    pub user_id: Option<Uuid>,
    pub api_key_id: Option<Uuid>,
    pub chat_id: Option<Uuid>,
    pub model_id: Option<Uuid>,
    pub provider_id: Option<Uuid>,
    pub model_name: Option<String>,
    pub provider_name: Option<String>,
    pub status: i32,
    pub error: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cached_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
    pub cost: Option<f64>,
    pub latency_ms: Option<i64>,
    pub ttft_ms: Option<i64>,
    pub output_tps: Option<f64>,
    pub prefill_tps: Option<f64>,
    pub metrics_source: Option<&'static str>,
}

pub fn spawn_writer(db: PgPool) -> mpsc::Sender<UsageRow> {
    let (tx, mut rx) = mpsc::channel::<UsageRow>(10_000);
    tokio::spawn(async move {
        let mut buf = Vec::with_capacity(500);
        loop {
            let n = rx.recv_many(&mut buf, 500).await;
            if n == 0 {
                break;
            }
            // Gather a little more so bursts become one insert.
            tokio::time::sleep(Duration::from_millis(200)).await;
            while buf.len() < 500 {
                match rx.try_recv() {
                    Ok(r) => buf.push(r),
                    Err(_) => break,
                }
            }
            if let Err(e) = insert(&db, &buf).await {
                tracing::error!("failed to write {} usage rows: {e}", buf.len());
            }
            buf.clear();
        }
    });
    tx
}

async fn insert(db: &PgPool, rows: &[UsageRow]) -> sqlx::Result<()> {
    let mut q = QueryBuilder::new(
        "insert into usage_log (source, user_id, api_key_id, chat_id, model_id, provider_id, model_name, provider_name, status,
         error, input_tokens, output_tokens, cached_tokens, cache_write_tokens, reasoning_tokens, cost, latency_ms, ttft_ms,
         output_tps, prefill_tps, metrics_source) ",
    );
    q.push_values(rows, |mut b, r| {
        b.push_bind(r.source)
            .push_bind(r.user_id)
            .push_bind(r.api_key_id)
            .push_bind(r.chat_id)
            .push_bind(r.model_id)
            .push_bind(r.provider_id)
            .push_bind(&r.model_name)
            .push_bind(&r.provider_name)
            .push_bind(r.status)
            .push_bind(&r.error)
            .push_bind(r.input_tokens.map(|v| v as i32))
            .push_bind(r.output_tokens.map(|v| v as i32))
            .push_bind(r.cached_tokens.map(|v| v as i32))
            .push_bind(r.cache_write_tokens.map(|v| v as i32))
            .push_bind(r.reasoning_tokens.map(|v| v as i32))
            .push_bind(r.cost)
            .push_bind(r.latency_ms.map(|v| v as i32))
            .push_bind(r.ttft_ms.map(|v| v as i32))
            .push_bind(r.output_tps.map(|v| v as f32))
            .push_bind(r.prefill_tps.map(|v| v as f32))
            .push_bind(r.metrics_source);
    });
    q.build().execute(db).await?;
    Ok(())
}

/// Per-reply speed and token figures (FR-CHT-15..20). Absent values stay `None` and are hidden in the UI.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct Metrics {
    pub ttft_ms: Option<i64>,
    pub duration_ms: Option<i64>,
    pub output_tps: Option<f64>,
    pub prefill_tps: Option<f64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cached_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
    pub cost: Option<f64>,
    /// "provider" when speeds come from provider timings, else "measured" (includes network latency).
    pub source: Option<String>,
}

pub struct Meter {
    start: std::time::Instant,
    first: Option<std::time::Instant>,
    last: Option<std::time::Instant>,
}

impl Meter {
    pub fn start() -> Self {
        Self { start: std::time::Instant::now(), first: None, last: None }
    }

    pub fn tick(&mut self) {
        let now = std::time::Instant::now();
        self.first.get_or_insert(now);
        self.last = Some(now);
    }

    pub fn finish(
        &self,
        usage: &crate::llm::Usage,
        timings: Option<(Option<f64>, Option<f64>)>,
        price_in: Option<f64>,
        price_out: Option<f64>,
    ) -> Metrics {
        let ttft = self.first.map(|f| f.duration_since(self.start));
        let span = self.first.zip(self.last).map(|(f, l)| l.duration_since(f).as_secs_f64());
        let measured_out = usage.output_tokens.zip(span).filter(|(n, s)| *n > 1 && *s > 0.0).map(|(n, s)| n as f64 / s);
        let measured_prefill =
            usage.input_tokens.zip(ttft).filter(|(_, t)| !t.is_zero()).map(|(n, t)| n as f64 / t.as_secs_f64());
        let (p_prefill, p_out) = timings.unwrap_or((None, None));
        let cost = match (price_in, price_out) {
            (None, None) => None,
            (pi, po) => Some(
                usage.input_tokens.unwrap_or(0) as f64 * pi.unwrap_or(0.0) / 1e6
                    + usage.output_tokens.unwrap_or(0) as f64 * po.unwrap_or(0.0) / 1e6,
            ),
        };
        Metrics {
            ttft_ms: ttft.map(|t| t.as_millis() as i64),
            duration_ms: Some(self.start.elapsed().as_millis() as i64),
            output_tps: p_out.or(measured_out),
            prefill_tps: p_prefill.or(measured_prefill),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cached_tokens: usage.cached_tokens,
            cache_write_tokens: usage.cache_write_tokens,
            reasoning_tokens: usage.reasoning_tokens,
            cost,
            source: Some(if p_out.is_some() || p_prefill.is_some() { "provider" } else { "measured" }.into()),
        }
    }
}

impl UsageRow {
    pub fn with_metrics(mut self, m: &Metrics) -> Self {
        self.input_tokens = m.input_tokens;
        self.output_tokens = m.output_tokens;
        self.cached_tokens = m.cached_tokens;
        self.cache_write_tokens = m.cache_write_tokens;
        self.reasoning_tokens = m.reasoning_tokens;
        self.cost = m.cost;
        self.latency_ms = m.duration_ms;
        self.ttft_ms = m.ttft_ms;
        self.output_tps = m.output_tps;
        self.prefill_tps = m.prefill_tps;
        self.metrics_source = m.source.as_deref().map(|s| if s == "provider" { "provider" } else { "measured" });
        self
    }
}
