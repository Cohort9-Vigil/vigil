//! JetStream event publisher for forward events and reorg retractions (RS-01, RS-04).

use std::sync::{Arc, Mutex};
use std::time::Duration;
use anyhow::{anyhow, Result};
use async_nats::jetstream::{self, Context as JetStreamContext};
use async_nats::HeaderMap;
use common::events::{
    IndexedEvent, RetractEvent, SUBJECT_CHAIN_EVENTS, SUBJECT_RETRACT_EVENTS,
};
use tracing::{debug, info};

/// Trait abstracting event publication to NATS or test sinks.
#[allow(async_fn_in_trait)]
pub trait EventPublisher: Send + Sync {
    async fn publish_indexed_event(&self, event: &IndexedEvent) -> Result<()>;
    async fn publish_retract_event(&self, event: &RetractEvent) -> Result<()>;
}

/// Production JetStream publisher with deterministic idempotency keys and trace IDs.
pub struct NatsJetStreamPublisher {
    context: JetStreamContext,
}

impl NatsJetStreamPublisher {
    pub async fn connect(nats_url: &str) -> Result<Self> {
        let client = async_nats::connect(nats_url).await
            .map_err(|e| anyhow!("Failed to connect to NATS at {nats_url}: {e}"))?;
        let context = jetstream::new(client);

        // Ensure the VIGIL_EVENTS stream exists for both forward events and retractions
        // with a 5-minute deduplication window based on Nats-Msg-Id (RS-04)
        let stream_config = jetstream::stream::Config {
            name: "VIGIL_EVENTS".to_string(),
            subjects: vec![
                SUBJECT_CHAIN_EVENTS.to_string(),
                SUBJECT_RETRACT_EVENTS.to_string(),
            ],
            duplicate_window: Duration::from_secs(300),
            ..Default::default()
        };
        context.get_or_create_stream(stream_config).await
            .map_err(|e| anyhow!("Failed to ensure VIGIL_EVENTS JetStream stream: {e}"))?;

        info!(nats_url = %nats_url, "Connected to NATS JetStream and initialized VIGIL_EVENTS stream with deduplication window");

        Ok(Self { context })
    }

    pub fn from_context(context: JetStreamContext) -> Self {
        Self { context }
    }
}

impl EventPublisher for NatsJetStreamPublisher {
    async fn publish_indexed_event(&self, event: &IndexedEvent) -> Result<()> {
        let payload = serde_json::to_vec(event)
            .map_err(|e| anyhow!("Failed to serialize IndexedEvent: {e}"))?;

        let mut headers = HeaderMap::new();
        // Deterministic idempotency key as Nats-Msg-Id for broker-level deduplication (RS-04)
        let msg_id = event.idempotency_key.to_key_string();
        headers.insert(
            "Nats-Msg-Id",
            msg_id.as_str(),
        );
        // Trace propagation header
        headers.insert("trace_id", event.trace_id.as_str());

        let ack = self
            .context
            .publish_with_headers(SUBJECT_CHAIN_EVENTS, headers, payload.into())
            .await
            .map_err(|e| anyhow!("NATS publish failed: {e}"))?
            .await
            .map_err(|e| anyhow!("NATS publish ack failed: {e}"))?;

        debug!(
            seq = ack.sequence,
            idempotency_key = %msg_id,
            trace_id = %event.trace_id,
            "Published IndexedEvent to JetStream"
        );

        Ok(())
    }

    async fn publish_retract_event(&self, event: &RetractEvent) -> Result<()> {
        let payload = serde_json::to_vec(event)
            .map_err(|e| anyhow!("Failed to serialize RetractEvent: {e}"))?;

        let mut headers = HeaderMap::new();
        let msg_id = format!("retract:{}", event.idempotency_key.to_key_string());
        headers.insert(
            "Nats-Msg-Id",
            msg_id.as_str(),
        );
        headers.insert("trace_id", event.trace_id.as_str());

        let ack = self
            .context
            .publish_with_headers(SUBJECT_RETRACT_EVENTS, headers, payload.into())
            .await
            .map_err(|e| anyhow!("NATS publish retract failed: {e}"))?
            .await
            .map_err(|e| anyhow!("NATS publish retract ack failed: {e}"))?;

        debug!(
            seq = ack.sequence,
            idempotency_key = %msg_id,
            trace_id = %event.trace_id,
            "Published RetractEvent to JetStream"
        );

        Ok(())
    }
}

/// Record of a published event along with captured headers for testing.
#[derive(Debug, Clone)]
pub struct MockPublishedRecord<T> {
    pub payload: T,
    pub nats_msg_id: String,
    pub trace_id: String,
}

/// In-memory publisher recording all emitted events and headers for tests.
#[derive(Debug, Default, Clone)]
pub struct MockEventPublisher {
    pub indexed_events: Arc<Mutex<Vec<IndexedEvent>>>,
    pub retract_events: Arc<Mutex<Vec<RetractEvent>>>,
    pub indexed_records: Arc<Mutex<Vec<MockPublishedRecord<IndexedEvent>>>>,
    pub retract_records: Arc<Mutex<Vec<MockPublishedRecord<RetractEvent>>>>,
}

impl MockEventPublisher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_indexed_events(&self) -> Vec<IndexedEvent> {
        self.indexed_events.lock().unwrap().clone()
    }

    pub fn get_retract_events(&self) -> Vec<RetractEvent> {
        self.retract_events.lock().unwrap().clone()
    }

    pub fn get_indexed_records(&self) -> Vec<MockPublishedRecord<IndexedEvent>> {
        self.indexed_records.lock().unwrap().clone()
    }

    pub fn get_retract_records(&self) -> Vec<MockPublishedRecord<RetractEvent>> {
        self.retract_records.lock().unwrap().clone()
    }

    pub fn clear(&self) {
        self.indexed_events.lock().unwrap().clear();
        self.retract_events.lock().unwrap().clear();
        self.indexed_records.lock().unwrap().clear();
        self.retract_records.lock().unwrap().clear();
    }
}

impl EventPublisher for MockEventPublisher {
    async fn publish_indexed_event(&self, event: &IndexedEvent) -> Result<()> {
        let msg_id = event.idempotency_key.to_key_string();
        self.indexed_events.lock().unwrap().push(event.clone());
        self.indexed_records.lock().unwrap().push(MockPublishedRecord {
            payload: event.clone(),
            nats_msg_id: msg_id,
            trace_id: event.trace_id.clone(),
        });
        Ok(())
    }

    async fn publish_retract_event(&self, event: &RetractEvent) -> Result<()> {
        let msg_id = format!("retract:{}", event.idempotency_key.to_key_string());
        self.retract_events.lock().unwrap().push(event.clone());
        self.retract_records.lock().unwrap().push(MockPublishedRecord {
            payload: event.clone(),
            nats_msg_id: msg_id,
            trace_id: event.trace_id.clone(),
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::events::ChainEvent;
    use common::types::Address;

    #[tokio::test]
    async fn test_mock_publisher_records_events_and_headers() {
        let publisher = MockEventPublisher::new();

        let event = IndexedEvent::new(
            "trace-1",
            10,
            "0xhash",
            "0xtx",
            0,
            ChainEvent::Deposit {
                account: Address::new("0xuser"),
                asset: Address::new("0xasset"),
                amount: 1000,
                block_number: 10,
                tx_hash: "0xtx".into(),
            },
            1700000000,
        );

        let retract = RetractEvent::new(
            "trace-1-retract",
            10,
            "0xhash",
            "0xtx",
            0,
            Some(ChainEvent::Deposit {
                account: Address::new("0xuser"),
                asset: Address::new("0xasset"),
                amount: 1000,
                block_number: 10,
                tx_hash: "0xtx".into(),
            }),
            1700000001,
        );

        publisher.publish_indexed_event(&event).await.unwrap();
        publisher.publish_retract_event(&retract).await.unwrap();

        let indexed = publisher.get_indexed_events();
        let retractions = publisher.get_retract_events();

        assert_eq!(indexed.len(), 1);
        assert_eq!(retractions.len(), 1);
        assert_eq!(indexed[0].idempotency_key.to_key_string(), "0xhash:0");
        assert_eq!(retractions[0].idempotency_key.to_key_string(), "0xhash:0");

        let indexed_records = publisher.get_indexed_records();
        assert_eq!(indexed_records[0].nats_msg_id, "0xhash:0");
        assert_eq!(indexed_records[0].trace_id, "trace-1");

        let retract_records = publisher.get_retract_records();
        assert_eq!(retract_records[0].nats_msg_id, "retract:0xhash:0");
        assert_eq!(retract_records[0].trace_id, "trace-1-retract");
    }
}
