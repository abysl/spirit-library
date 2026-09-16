use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::{Endpoint, EndpointAddr};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::sync::{Arc, Mutex};

pub const ALPN: &[u8] = b"spirit-gossip/0";

const MAX_MESSAGE_BYTES: usize = 1 << 20;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RefAdvert {
    pub name: String,
    pub manifest: String,
    pub total: u32,
    pub held: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}

impl RefAdvert {
    pub fn complete(&self) -> bool {
        self.total > 0 && self.held >= self.total
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableAdvert {
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeardTable {
    pub host: String,
    pub table: TableAdvert,
    #[serde(default)]
    pub heard_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct View {
    pub addr: EndpointAddr,
    pub peers: Vec<EndpointAddr>,
    pub refs: Vec<RefAdvert>,
    #[serde(default)]
    pub table: Option<TableAdvert>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub heard_tables: Vec<HeardTable>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dgid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vouch: Option<String>,
}

impl View {
    pub fn vouched_dgid(&self) -> Option<spirit_core::Dgid> {
        let dgid = spirit_core::Dgid::parse(self.dgid.as_deref()?)?;
        let signature = spirit_core::Signature::parse(self.vouch.as_deref()?)?;
        spirit_core::identity::vouched(dgid, self.addr.id.as_bytes(), signature).then_some(dgid)
    }
}

pub fn encode(view: &View) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
    let mut bytes = Vec::new();
    ciborium::into_writer(view, &mut bytes)?;
    Ok(bytes)
}

pub fn decode(bytes: &[u8]) -> Result<View, Box<dyn Error + Send + Sync>> {
    Ok(ciborium::from_reader(bytes)?)
}

pub trait ViewSource: Send + Sync + std::fmt::Debug + 'static {
    fn local_view(&self) -> View;
    fn merge(&self, from: &View);
}

#[derive(Clone)]
pub struct GossipHandler {
    source: Arc<dyn ViewSource>,
    exchanges: Arc<Mutex<u64>>,
}

impl std::fmt::Debug for GossipHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GossipHandler").finish()
    }
}

impl GossipHandler {
    pub fn new(source: Arc<dyn ViewSource>) -> Self {
        Self {
            source,
            exchanges: Arc::new(Mutex::new(0)),
        }
    }

    pub fn exchanges(&self) -> u64 {
        *self.exchanges.lock().unwrap()
    }
}

impl ProtocolHandler for GossipHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let (mut send, mut recv) = connection.accept_bi().await?;
        let bytes = recv
            .read_to_end(MAX_MESSAGE_BYTES)
            .await
            .map_err(AcceptError::from_err)?;
        let incoming = decode(&bytes).map_err(|error| {
            AcceptError::from_boxed(format!("bad gossip message: {error}").into())
        })?;
        self.source.merge(&incoming);

        let reply = encode(&self.source.local_view())
            .map_err(|error| AcceptError::from_boxed(format!("encode failed: {error}").into()))?;
        send.write_all(&reply)
            .await
            .map_err(AcceptError::from_err)?;
        send.finish()?;
        connection.closed().await;
        *self.exchanges.lock().unwrap() += 1;
        Ok(())
    }
}

pub async fn exchange(
    endpoint: &Endpoint,
    peer: EndpointAddr,
    ours: &View,
) -> Result<View, Box<dyn Error + Send + Sync>> {
    let connection = endpoint.connect(peer, ALPN).await?;
    let (mut send, mut recv) = connection.open_bi().await?;
    send.write_all(&encode(ours)?).await?;
    send.finish()?;
    let bytes = recv.read_to_end(MAX_MESSAGE_BYTES).await?;
    let theirs = decode(&bytes)?;
    connection.close(0u32.into(), b"done");
    Ok(theirs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;

    fn addr(seed: u8) -> EndpointAddr {
        let secret = SecretKey::from_bytes(&[seed; 32]);
        EndpointAddr::new(secret.public())
    }

    #[test]
    fn a_view_survives_a_cbor_round_trip() {
        let view = View {
            addr: addr(1),
            peers: vec![addr(2), addr(3)],
            refs: vec![RefAdvert {
                name: "hob".into(),
                manifest: "ab".repeat(32),
                total: 194,
                held: 194,
                owner: None,
            }],
            table: None,
            heard_tables: vec![],
            dgid: None,
            vouch: None,
        };
        let decoded = decode(&encode(&view).unwrap()).unwrap();
        assert_eq!(decoded.addr.id, view.addr.id);
        assert!(decoded.vouched_dgid().is_none());
        assert_eq!(decoded.peers.len(), 2);
        assert_eq!(decoded.refs[0].name, "hob");
        assert!(decoded.refs[0].complete());
        assert!(decoded.table.is_none());
    }

    #[test]
    fn a_table_advert_survives_a_cbor_round_trip() {
        let view = View {
            addr: addr(4),
            peers: vec![],
            refs: vec![],
            table: Some(TableAdvert {
                name: "rae's table".into(),
            }),
            heard_tables: vec![HeardTable {
                host: "aa".into(),
                table: TableAdvert {
                    name: "porch".into(),
                },
                heard_at: 1_700_000_000,
            }],
            dgid: None,
            vouch: None,
        };
        let decoded = decode(&encode(&view).unwrap()).unwrap();
        assert_eq!(
            decoded.table,
            Some(TableAdvert {
                name: "rae's table".into()
            })
        );
        assert_eq!(decoded.heard_tables, view.heard_tables);
    }

    #[test]
    fn a_view_from_before_relayed_tables_still_parses() {
        let mut old = ciborium::value::Value::Map(vec![]);
        if let ciborium::value::Value::Map(entries) = &mut old {
            let mut addr_bytes = Vec::new();
            ciborium::into_writer(&addr(8), &mut addr_bytes).unwrap();
            let addr_value: ciborium::value::Value =
                ciborium::from_reader(addr_bytes.as_slice()).unwrap();
            entries.push(("addr".into(), addr_value));
            entries.push(("peers".into(), ciborium::value::Value::Array(vec![])));
            entries.push(("refs".into(), ciborium::value::Value::Array(vec![])));
        }
        let mut bytes = Vec::new();
        ciborium::into_writer(&old, &mut bytes).unwrap();
        let decoded = decode(&bytes).unwrap();
        assert!(decoded.table.is_none());
        assert!(decoded.heard_tables.is_empty());
    }

    #[test]
    fn a_partial_ref_does_not_claim_to_be_complete() {
        let advert = RefAdvert {
            name: "hob".into(),
            manifest: "cd".repeat(32),
            total: 194,
            held: 100,
            owner: None,
        };
        assert!(!advert.complete());
    }

    #[test]
    fn an_empty_ref_is_not_complete() {
        let advert = RefAdvert {
            name: "hob".into(),
            manifest: "ef".repeat(32),
            total: 0,
            held: 0,
            owner: None,
        };
        assert!(!advert.complete());
    }

    #[test]
    fn a_vouched_dgid_verifies_only_for_the_sender() {
        let group = spirit_core::Identity::from_secret([11; 32]);
        let me = addr(6);
        let vouch = spirit_core::identity::vouch(&group, me.id.as_bytes()).to_string();
        let view = View {
            addr: me,
            peers: vec![],
            refs: vec![],
            table: None,
            heard_tables: vec![],
            dgid: Some(group.dgid().to_string()),
            vouch: Some(vouch.clone()),
        };
        assert_eq!(view.vouched_dgid(), Some(group.dgid()));
        let mut forged = decode(&encode(&view).unwrap()).unwrap();
        forged.addr = addr(7);
        assert!(forged.vouched_dgid().is_none());
    }
}
