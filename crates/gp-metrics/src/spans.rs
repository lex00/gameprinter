//! A `tracing` layer that times every span, so run manifests show where a
//! command spent its time without any extra instrumentation code.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tracing::Subscriber;
use tracing::span::{Attributes, Id};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpanTiming {
    pub name: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Wall time from creation to close.
    pub duration_us: u64,
}

#[derive(Clone, Default)]
pub struct SpanCollector {
    sink: Arc<Mutex<Vec<SpanTiming>>>,
}

impl SpanCollector {
    pub fn layer(&self) -> SpanLayer {
        SpanLayer {
            sink: Arc::clone(&self.sink),
        }
    }

    /// Takes every span closed so far, in the order they closed.
    pub fn drain(&self) -> Vec<SpanTiming> {
        std::mem::take(&mut *self.sink.lock().expect("span sink poisoned"))
    }
}

pub struct SpanLayer {
    sink: Arc<Mutex<Vec<SpanTiming>>>,
}

struct Started(Instant);

impl<S> Layer<S> for SpanLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, _attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(Started(Instant::now()));
        }
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(&id) else { return };
        let ext = span.extensions();
        let Some(Started(at)) = ext.get::<Started>() else {
            return;
        };
        let timing = SpanTiming {
            name: span.name().to_owned(),
            target: span.metadata().target().to_owned(),
            parent: span.parent().map(|p| p.name().to_owned()),
            duration_us: at.elapsed().as_micros() as u64,
        };
        self.sink.lock().expect("span sink poisoned").push(timing);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt;

    #[test]
    fn records_nested_spans_with_parents() {
        let collector = SpanCollector::default();
        let subscriber = tracing_subscriber::registry().with(collector.layer());
        tracing::subscriber::with_default(subscriber, || {
            let outer = tracing::info_span!("outer");
            let _o = outer.enter();
            let inner = tracing::info_span!("inner");
            let _i = inner.enter();
        });
        let spans = collector.drain();
        let names: Vec<_> = spans.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["inner", "outer"]);
        assert_eq!(spans[0].parent.as_deref(), Some("outer"));
        assert!(collector.drain().is_empty());
    }
}
