use crate::peers::{registry, ConnectionKey, PeerRegistry};
use iroh::endpoint::{Connection, PathId, TransportAddrUsage};
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::{Endpoint, EndpointId, TransportAddr};
use std::time::Duration;

const SAMPLE_INTERVAL: Duration = Duration::from_millis(500);
const REACHABILITY_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct RecordingHandler<H> {
    inner: H,
    registry: PeerRegistry,
}

impl<H> RecordingHandler<H> {
    pub fn new(inner: H) -> Self {
        Self {
            inner,
            registry: registry().clone(),
        }
    }
}

impl<H: ProtocolHandler> ProtocolHandler for RecordingHandler<H> {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let id = connection.remote_id().to_string();
        let key = self.registry.connection_opened(&id);

        let sampler = n0_future::task::spawn({
            let registry = self.registry.clone();
            let connection = connection.clone();
            let key = key.clone();
            async move {
                loop {
                    n0_future::time::sleep(SAMPLE_INTERVAL).await;
                    sample(&registry, &key, &connection);
                    if connection.close_reason().is_some() {
                        break;
                    }
                }
            }
        });

        let result = self.inner.accept(connection.clone()).await;
        sampler.abort();
        sample(&self.registry, &key, &connection);
        self.registry.connection_closed(&key);
        result
    }

    async fn shutdown(&self) {
        self.inner.shutdown().await;
    }
}

fn sample(registry: &PeerRegistry, key: &ConnectionKey, connection: &Connection) {
    let stats = connection.stats();
    registry.observe(
        key,
        stats.udp_tx.bytes,
        stats.udp_rx.bytes,
        connection.rtt(PathId::ZERO),
    );
}

fn describe_addr(addr: &TransportAddr) -> String {
    match addr {
        TransportAddr::Relay(url) => format!("relay {url}"),
        TransportAddr::Ip(socket) => format!("direct {socket}"),
        other => format!("other {other:?}"),
    }
}

pub async fn refresh_reachability(endpoint: &Endpoint) {
    let registry = registry();
    for id in registry.ids() {
        let Ok(parsed) = id.parse::<EndpointId>() else {
            continue;
        };
        let mut active = Vec::new();
        let mut inactive = Vec::new();
        if let Some(info) = endpoint.remote_info(parsed).await {
            for addr in info.addrs() {
                let text = describe_addr(addr.addr());
                match addr.usage() {
                    TransportAddrUsage::Active => active.push(text),
                    _ => inactive.push(text),
                }
            }
        }
        registry.set_addrs(&id, active, inactive);
    }
}

pub fn spawn_reachability_watch(endpoint: Endpoint) {
    n0_future::task::spawn(async move {
        loop {
            n0_future::time::sleep(REACHABILITY_INTERVAL).await;
            if endpoint.is_closed() {
                break;
            }
            refresh_reachability(&endpoint).await;
        }
    });
}
