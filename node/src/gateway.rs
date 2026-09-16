use crate::mesh::{local_refs, Mesh};
use crate::ops::{self, Edit, Local};
use crate::peers;
use serde_json::json;
use spirit_core::{BlobHash, BlobStore, CiHash, TdHash, TrustLevel};
use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

const MAX_REQUEST_LINE: u64 = 8192;
const MAX_HEADERS: usize = 64;
const MAX_BODY: usize = 256 << 20;
pub const TOKEN_FILE: &str = "gateway-token";
pub const PORT_FILE: &str = "gateway-port";
pub const UI: &str = include_str!("../ui/index.html");

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResolveRequest {
    pub name: String,
    pub params: BTreeMap<String, String>,
}

impl ResolveRequest {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.params.get(key).map(String::as_str)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveReply {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

impl ResolveReply {
    pub fn json(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "application/json".to_string(),
            body: body.into().into_bytes(),
        }
    }

    pub fn error(status: u16, message: &str) -> Self {
        Self::json(status, json!({ "error": message }).to_string())
    }
}

pub type Resolver = Arc<dyn Fn(&ResolveRequest) -> ResolveReply + Send + Sync>;
pub type Resolvers = BTreeMap<String, Resolver>;

pub struct Gateway {
    pub dir: PathBuf,
    pub node_id: String,
    pub mesh: Arc<Mesh>,
    pub resolvers: Resolvers,
}

pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub immutable: bool,
    pub body: Vec<u8>,
    pub disposition: Option<String>,
}

impl Response {
    fn json(status: u16, value: serde_json::Value) -> Self {
        Self {
            status,
            content_type: "application/json",
            immutable: false,
            body: value.to_string().into_bytes(),
            disposition: None,
        }
    }

    fn error(status: u16, message: &str) -> Self {
        Self::json(status, json!({ "error": message }))
    }

    fn html(body: &str) -> Self {
        Self {
            status: 200,
            content_type: "text/html; charset=utf-8",
            immutable: false,
            body: body.as_bytes().to_vec(),
            disposition: None,
        }
    }

    fn empty(status: u16) -> Self {
        Self {
            status,
            content_type: "text/plain",
            immutable: false,
            body: Vec::new(),
            disposition: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: String,
    pub bearer: Option<String>,
    pub body: Vec<u8>,
}

pub struct Context<'a> {
    pub dir: &'a Path,
    pub node_id: &'a str,
    pub peer_count: usize,
    pub resolvers: &'a Resolvers,
    pub token: Option<&'a str>,
    pub started: Option<Instant>,
    pub mesh: Option<&'a Mesh>,
    pub local: bool,
}

pub fn token_path(dir: &Path) -> PathBuf {
    dir.join(TOKEN_FILE)
}

pub fn port_path(dir: &Path) -> PathBuf {
    dir.join(PORT_FILE)
}

pub fn read_port(dir: &Path) -> Option<u16> {
    std::fs::read_to_string(port_path(dir))
        .ok()?
        .trim()
        .parse()
        .ok()
}

pub fn load_or_create_token(dir: &Path) -> std::io::Result<String> {
    let path = token_path(dir);
    if let Ok(text) = std::fs::read_to_string(&path) {
        let text = text.trim().to_string();
        if text.len() >= 32 {
            return Ok(text);
        }
    }
    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret).map_err(std::io::Error::other)?;
    let token: String = secret.iter().map(|byte| format!("{byte:02x}")).collect();
    std::fs::create_dir_all(dir)?;
    std::fs::write(&path, format!("{token}\n"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(token)
}

pub struct Service {
    gateway: Gateway,
    token: Option<String>,
    started: Instant,
}

impl Service {
    pub fn new(gateway: Gateway) -> Arc<Self> {
        let token = match load_or_create_token(&gateway.dir) {
            Ok(token) => Some(token),
            Err(error) => {
                eprintln!("gateway: writes disabled, no token: {error}");
                None
            }
        };
        Arc::new(Self {
            gateway,
            token,
            started: Instant::now(),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.gateway.dir
    }

    pub async fn dispatch(self: &Arc<Self>, request: Request, local: bool) -> Response {
        if request.method == "POST" && request.path == "/gateway/join" {
            let context = self.context(local);
            return match authorized(&request, &context) {
                Ok(()) => join_route(&request.body, &self.gateway.mesh).await,
                Err(response) => response,
            };
        }
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            let context = service.context(local);
            route_request(&request, &context)
        })
        .await
        .unwrap_or_else(|_| Response::error(500, "request handler panicked"))
    }

    fn context(&self, local: bool) -> Context<'_> {
        Context {
            dir: &self.gateway.dir,
            node_id: &self.gateway.node_id,
            peer_count: self.gateway.mesh.known_peers().len(),
            resolvers: &self.gateway.resolvers,
            token: self.token.as_deref(),
            started: Some(self.started),
            mesh: Some(&self.gateway.mesh),
            local,
        }
    }
}

pub async fn spawn(port: u16, gateway: Gateway) -> Result<u16, Box<dyn Error>> {
    spawn_http(port, Service::new(gateway)).await
}

pub async fn spawn_http(port: u16, service: Arc<Service>) -> Result<u16, Box<dyn Error>> {
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    let bound = listener.local_addr()?.port();
    let _ = std::fs::create_dir_all(service.dir());
    if let Err(error) = std::fs::write(port_path(service.dir()), format!("{bound}\n")) {
        eprintln!("gateway: could not record the port: {error}");
    }
    let inner = service;
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let inner = inner.clone();
            tokio::spawn(async move {
                let _ = handle(stream, &inner).await;
            });
        }
    });
    Ok(bound)
}

async fn read_request(reader: &mut BufReader<TcpStream>) -> Result<Request, Response> {
    let mut request_line = String::new();
    reader
        .take(MAX_REQUEST_LINE)
        .read_line(&mut request_line)
        .await
        .map_err(|_| Response::error(400, "unreadable request line"))?;
    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    for _ in 0..MAX_HEADERS {
        let mut header = String::new();
        let read = reader
            .take(MAX_REQUEST_LINE)
            .read_line(&mut header)
            .await
            .map_err(|_| Response::error(400, "unreadable header"))?;
        if read == 0 || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default().to_string();
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path.to_string(), query.to_string()),
        None => (target, String::new()),
    };
    let bearer = headers
        .get("authorization")
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(|value| value.trim().to_string());
    let length: usize = headers
        .get("content-length")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    if length > MAX_BODY {
        return Err(Response::error(413, "body exceeds 256 MiB"));
    }
    let mut body = vec![0u8; length];
    if length > 0 {
        reader
            .read_exact(&mut body)
            .await
            .map_err(|_| Response::error(400, "body shorter than content-length"))?;
    }
    Ok(Request {
        method,
        path,
        query,
        bearer,
        body,
    })
}

async fn handle(stream: TcpStream, service: &Arc<Service>) -> Result<(), Box<dyn Error>> {
    let mut reader = BufReader::new(stream);
    let response = match read_request(&mut reader).await {
        Ok(request) => service.dispatch(request, false).await,
        Err(response) => response,
    };
    write_response(reader.into_inner(), response).await
}

async fn write_response(mut stream: TcpStream, response: Response) -> Result<(), Box<dyn Error>> {
    let reason = match response.status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        422 => "Unprocessable Entity",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    };
    let cache = if response.immutable {
        "public, max-age=31536000, immutable"
    } else {
        "no-store"
    };
    let disposition = response
        .disposition
        .as_deref()
        .map(|name| format!("Content-Disposition: attachment; filename=\"{name}\"\r\n"))
        .unwrap_or_default();
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: {}\r\n{}Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: *\r\nConnection: close\r\n\r\n",
        response.status,
        reason,
        response.content_type,
        response.body.len(),
        cache,
        disposition,
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&response.body).await?;
    stream.shutdown().await?;
    Ok(())
}

pub fn route(
    method: &str,
    path: &str,
    dir: &Path,
    node_id: &str,
    peer_count: usize,
    resolvers: &Resolvers,
) -> Response {
    let (path, query) = match path.split_once('?') {
        Some((path, query)) => (path.to_string(), query.to_string()),
        None => (path.to_string(), String::new()),
    };
    route_request(
        &Request {
            method: method.to_string(),
            path,
            query,
            bearer: None,
            body: Vec::new(),
        },
        &Context {
            dir,
            node_id,
            peer_count,
            resolvers,
            token: None,
            started: None,
            mesh: None,
            local: false,
        },
    )
}

pub fn route_request(request: &Request, context: &Context<'_>) -> Response {
    let path = request.path.as_str();
    match request.method.as_str() {
        "OPTIONS" => Response::empty(204),
        "GET" => route_get(path, &request.query, context),
        "POST" => match authorized(request, context) {
            Ok(()) => route_post(path, &request.body, context),
            Err(response) => response,
        },
        _ => Response::error(405, "only GET, POST and OPTIONS are served"),
    }
}

fn authorized(request: &Request, context: &Context<'_>) -> Result<(), Response> {
    if context.local {
        return Ok(());
    }
    let Some(token) = context.token else {
        return Err(Response::error(
            403,
            "writes are disabled on this gateway: no token",
        ));
    };
    let Some(bearer) = request.bearer.as_deref() else {
        return Err(Response::error(401, "writes need a bearer token"));
    };
    if !constant_time_eq(bearer.as_bytes(), token.as_bytes()) {
        return Err(Response::error(401, "wrong token"));
    }
    Ok(())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

async fn join_route(body: &[u8], mesh: &Arc<Mesh>) -> Response {
    let input: serde_json::Value = match serde_json::from_slice(body) {
        Ok(input) => input,
        Err(error) => return Response::error(400, &format!("body is not JSON: {error}")),
    };
    let url = match text(&input, "url") {
        Ok(url) => url,
        Err(error) => return Response::error(400, &error.to_string()),
    };
    let invite = match crate::pair::Invite::parse(&url) {
        Ok(invite) => invite,
        Err(error) => return Response::error(400, &error),
    };
    match crate::pair::join_with_mesh(mesh, &invite).await {
        Ok(joined) => Response::json(
            200,
            json!({
                "dgid": joined.dgid.to_string(),
                "head": joined.head.map(|hash| hash.to_string()),
                "members": joined.members.iter().map(|addr| addr.id.to_string()).collect::<Vec<_>>(),
            }),
        ),
        Err(error) => Response::error(422, &error.to_string()),
    }
}

fn pair_json(mesh: &Mesh, invite: Option<crate::pair::Invite>) -> serde_json::Value {
    let offer = mesh.current_offer();
    let url = invite.as_ref().map(|invite| invite.url());
    let qr = url.as_deref().and_then(|url| {
        qrcode::QrCode::new(url.as_bytes()).ok().map(|code| {
            code.render::<qrcode::render::unicode::Dense1x2>()
                .dark_color(qrcode::render::unicode::Dense1x2::Light)
                .light_color(qrcode::render::unicode::Dense1x2::Dark)
                .build()
        })
    });
    json!({
        "offered": offer.is_some(),
        "seconds_left": offer.as_ref().map(|offer| offer.seconds_left()),
        "url": url,
        "qr": qr,
        "dgid": mesh.self_dgid().map(|dgid| dgid.to_string()),
        "members": mesh.members(),
    })
}

fn route_get(path: &str, query: &str, context: &Context<'_>) -> Response {
    let dir = context.dir;
    match path {
        "/gateway/pair" => match context.mesh {
            Some(mesh) => Response::json(200, pair_json(mesh, None)),
            None => Response::error(503, "no mesh on this gateway"),
        },
        "/gateway/members" => with_local(dir, |local| Ok(json!(local.members()))),
        "/" | "/ui" | "/index.html" => Response::html(UI),
        "/gateway/status" => status(context),
        "/gateway/stats" => stats(context),
        "/gateway/refs" => Response::json(200, refs_json(dir)),
        "/gateway/modules" => modules_json(dir),
        "/gateway/blobs" => with_local(dir, |local| Ok(json!(local.blobs()))),
        "/gateway/index" => with_local(dir, |local| Ok(json!(local.index()))),
        "/gateway/collections" => with_local(dir, |local| Ok(json!(local.collections()))),
        "/gateway/trust" => with_local(dir, |local| Ok(json!(local.trust_entries()))),
        "/gateway/resolvers" => {
            Response::json(200, json!(context.resolvers.keys().collect::<Vec<_>>()))
        }
        _ => {
            if let Some(name) = path.strip_prefix("/gateway/resolve/") {
                return resolve(name, query, context.resolvers);
            }
            if let Some(name) = manifest_ref_name(path) {
                return manifest(dir, name);
            }
            if let Some(hash) = path.strip_prefix("/gateway/blob/") {
                return blob(dir, hash, parse_params(query).get("name").cloned());
            }
            if let Some(hash) = path.strip_prefix("/gateway/record/") {
                return match ops::parse_hash(hash) {
                    Ok(hash) => with_local(dir, |local| local.record(hash).map(|view| json!(view))),
                    Err(error) => Response::error(404, &error),
                };
            }
            if let Some(ci) = path.strip_prefix("/gateway/artifacts/") {
                let params = parse_params(query);
                let minimum = params
                    .get("min")
                    .and_then(|level| TrustLevel::parse(level))
                    .unwrap_or(TrustLevel::Cache);
                let prefer_td = params
                    .get("td")
                    .and_then(|td| ops::parse_hash(td).ok())
                    .map(TdHash::from_hash);
                return match ops::parse_hash(ci) {
                    Ok(hash) => with_local(dir, |local| {
                        Ok(json!(local.artifacts(
                            CiHash::from_hash(hash),
                            minimum,
                            prefer_td
                        )))
                    }),
                    Err(error) => Response::error(404, &error),
                };
            }
            if let Some(rest) = path.strip_prefix("/gateway/collection/") {
                return match rest.strip_suffix("/ops") {
                    Some(name) => match collection_name(name) {
                        Some(name) => with_local(dir, |local| {
                            local.collection_ops(&name).map(|ops| json!(ops))
                        }),
                        None => Response::error(404, "not a collection name"),
                    },
                    None => match collection_name(rest) {
                        Some(name) => {
                            with_local(dir, |local| local.collection(&name).map(|view| json!(view)))
                        }
                        None => Response::error(404, "not a collection name"),
                    },
                };
            }
            Response::error(404, "unknown gateway path")
        }
    }
}

fn route_post(path: &str, body: &[u8], context: &Context<'_>) -> Response {
    let dir = context.dir;
    match path {
        "/gateway/pair" => match context.mesh {
            Some(mesh) => match mesh.offer_pairing() {
                Some(invite) => Response::json(200, pair_json(mesh, Some(invite))),
                None => Response::error(503, "this node holds no group key to pair into"),
            },
            None => Response::error(503, "no mesh on this gateway"),
        },
        "/gateway/revoke" => with_json(body, |input| {
            with_local(dir, |local| {
                let node = text(input, "node_id")?;
                let head = local.revoke(&node)?;
                if let Some(mesh) = context.mesh {
                    mesh.refresh_membership();
                }
                Ok(json!({ "node_id": node, "head": head.map(|hash| hash.to_string()) }))
            })
        }),
        "/gateway/seed" => with_json(body, |input| {
            let Some(mesh) = context.mesh else {
                return Response::error(503, "no mesh on this gateway");
            };
            let value = match text(input, "value") {
                Ok(value) => value,
                Err(error) => return Response::error(400, &error.to_string()),
            };
            match mesh.seed(&value) {
                Ok(node_id) => Response::json(200, json!({ "node_id": node_id })),
                Err(error) => Response::error(422, &error.to_string()),
            }
        }),
        "/gateway/blob" => with_local(dir, |local| {
            let hash = local.store.put(body)?;
            Ok(json!({ "hash": hash.to_string(), "size": body.len(), "kind": ops::describe(body) }))
        }),
        "/gateway/cir" => with_json(body, |input| {
            with_local(dir, |local| {
                let kind = text(input, "kind")?;
                let ci = local.mint_cir(&kind, input.get("body").unwrap_or(&json!({})))?;
                Ok(json!({ "ci": ci.to_string() }))
            })
        }),
        "/gateway/tdr" => with_json(body, |input| {
            with_local(dir, |local| {
                let kind = text(input, "kind")?;
                let td = local.mint_tdr(&kind, input.get("body").unwrap_or(&json!({})))?;
                Ok(json!({ "td": td.to_string() }))
            })
        }),
        "/gateway/attest" => with_json(body, |input| {
            with_local(dir, |local| {
                let ci = CiHash::from_hash(hash_field(input, "ci")?);
                let att = match input.get("relation").and_then(|v| v.as_str()) {
                    Some(relation) => {
                        let other = CiHash::from_hash(hash_field(input, "other")?);
                        local.attest_relation(relation, ci, other)?
                    }
                    None => {
                        let td = TdHash::from_hash(hash_field(input, "td")?);
                        let blob = hash_field(input, "blob")?;
                        local.attest_content(ci, td, blob)?
                    }
                };
                Ok(json!({ "att": att.to_string() }))
            })
        }),
        "/gateway/trust" => with_json(body, |input| {
            with_local(dir, |local| {
                let who = text(input, "who")?;
                let level = TrustLevel::parse(&text(input, "level")?)
                    .ok_or("level must be one of: unknown, contact, cache, mesh")?;
                let dgid = local.set_trust(&who, level)?;
                Ok(json!({ "dgid": dgid.to_string(), "level": level.label() }))
            })
        }),
        _ => {
            if let Some(name) = path
                .strip_prefix("/gateway/collection/")
                .and_then(collection_name)
            {
                return with_json(body, |input| {
                    with_local(dir, |local| {
                        let edit = match text(input, "op")?.as_str() {
                            "add" => Edit::Add {
                                ci: CiHash::from_hash(hash_field(input, "ci")?),
                                label: input
                                    .get("label")
                                    .and_then(|v| v.as_str())
                                    .filter(|s| !s.is_empty())
                                    .map(String::from),
                                default_td: match input.get("td").and_then(|v| v.as_str()) {
                                    Some(td) if !td.is_empty() => {
                                        Some(TdHash::from_hash(ops::parse_hash(td)?))
                                    }
                                    _ => None,
                                },
                            },
                            "remove" => Edit::Remove {
                                ci: CiHash::from_hash(hash_field(input, "ci")?),
                            },
                            "attest" => Edit::Attest {
                                hash: hash_field(input, "hash")?,
                            },
                            "record" => Edit::Record {
                                hash: hash_field(input, "hash")?,
                            },
                            other => return Err(format!("unknown op {other:?}").into()),
                        };
                        let head = local.edit_collection(&name, edit)?;
                        Ok(json!({ "name": name, "head": head.to_string() }))
                    })
                });
            }
            Response::error(404, "unknown gateway path")
        }
    }
}

fn collection_name(raw: &str) -> Option<String> {
    let name = percent_decode(raw)?;
    spirit_core::refs::valid_name(&name).then_some(name)
}

fn text(input: &serde_json::Value, field: &str) -> Result<String, Box<dyn Error>> {
    input
        .get(field)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or_else(|| format!("missing {field:?}").into())
}

fn hash_field(input: &serde_json::Value, field: &str) -> Result<BlobHash, Box<dyn Error>> {
    Ok(ops::parse_hash(&text(input, field)?)?)
}

fn with_json(body: &[u8], f: impl FnOnce(&serde_json::Value) -> Response) -> Response {
    match serde_json::from_slice::<serde_json::Value>(body) {
        Ok(input) if input.is_object() => f(&input),
        Ok(_) => Response::error(400, "body must be a JSON object"),
        Err(error) => Response::error(400, &format!("body is not JSON: {error}")),
    }
}

fn with_local(
    dir: &Path,
    f: impl FnOnce(&Local) -> Result<serde_json::Value, Box<dyn Error>>,
) -> Response {
    let local = match Local::open(dir) {
        Ok(local) => local,
        Err(error) => return Response::error(500, &format!("store unavailable: {error}")),
    };
    match f(&local) {
        Ok(value) => Response::json(200, value),
        Err(error) => Response::error(422, &error.to_string()),
    }
}

fn stats(context: &Context<'_>) -> Response {
    let dir = context.dir;
    with_local(dir, |local| {
        let blobs = local.blobs();
        let snapshot: Vec<serde_json::Value> = peers::snapshot()
            .iter()
            .map(|peer| {
                json!({
                    "id": peer.id,
                    "state": peer.state.label(),
                    "path": peer.path_kind().label(),
                    "discovery": peer.discovery.label(),
                    "rtt_ms": peer.rtt.map(|rtt| rtt.as_secs_f64() * 1000.0),
                    "sent": peer.wire_bytes_sent,
                    "received": peer.wire_bytes_received,
                    "payload": peer.payload_bytes_received,
                    "connections": peer.live_connections,
                    "dial_failures": peer.dial_failures,
                    "refs": peer.refs_advertised,
                    "last_activity": peers::format_age(peer.last_activity),
                    "outcome": peer.outcome,
                })
            })
            .collect();
        let trusted: Vec<serde_json::Value> = context
            .mesh
            .map(|mesh| {
                mesh.trusted_peers()
                    .into_iter()
                    .map(|(id, level)| json!({ "id": id, "level": level.label() }))
                    .collect()
            })
            .unwrap_or_default();
        let tables: Vec<String> = context
            .mesh
            .map(|mesh| {
                mesh.open_tables()
                    .into_iter()
                    .map(|table| table.name)
                    .collect()
            })
            .unwrap_or_default();
        Ok(json!({
            "node_id": context.node_id,
            "dgid": local.identity.dgid().to_string(),
            "uptime_secs": context.started.map(|started| started.elapsed().as_secs()).unwrap_or(0),
            "writes_enabled": context.token.is_some() || context.local,
            "local_api": crate::api::path(dir).exists(),
            "peers": { "known": context.peer_count, "snapshot": snapshot, "trusted": trusted },
            "refs": refs_json(dir),
            "blobs": { "count": blobs.len(), "bytes": blobs.iter().map(|blob| blob.size).sum::<u64>() },
            "collections": local.collections(),
            "trust": local.trust_entries(),
            "tables": tables,
        }))
    })
}

pub fn percent_decode(text: &str) -> Option<String> {
    let mut bytes = Vec::new();
    let mut input = text.bytes();
    while let Some(byte) = input.next() {
        match byte {
            b'%' => {
                let high = input.next()?;
                let low = input.next()?;
                let hex = [high, low];
                let hex = std::str::from_utf8(&hex).ok()?;
                bytes.push(u8::from_str_radix(hex, 16).ok()?);
            }
            b'+' => bytes.push(b' '),
            other => bytes.push(other),
        }
    }
    String::from_utf8(bytes).ok()
}

fn parse_params(query: &str) -> BTreeMap<String, String> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            Some((percent_decode(key)?, percent_decode(value)?))
        })
        .collect()
}

fn resolve(name: &str, query: &str, resolvers: &Resolvers) -> Response {
    let Some(name) = percent_decode(name).filter(|name| !name.is_empty()) else {
        return Response::error(400, "resolver name is not valid percent-encoding");
    };
    let Some(resolver) = resolvers.get(&name) else {
        return Response::error(404, &format!("no resolver named {name:?} on this gateway"));
    };
    let reply = resolver(&ResolveRequest {
        name,
        params: parse_params(query),
    });
    Response {
        status: reply.status,
        content_type: match reply.content_type.as_str() {
            "application/json" => "application/json",
            "text/plain" => "text/plain",
            "application/octet-stream" => "application/octet-stream",
            _ => "application/octet-stream",
        },
        immutable: false,
        body: reply.body,
        disposition: None,
    }
}

fn manifest_ref_name(path: &str) -> Option<&str> {
    let name = path
        .strip_prefix("/gateway/ref/")?
        .strip_suffix("/manifest")?;
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        return None;
    }
    Some(name)
}

fn refs_json(dir: &Path) -> serde_json::Value {
    let refs: Vec<serde_json::Value> = local_refs(dir)
        .into_iter()
        .map(|advert| {
            json!({
                "name": advert.name,
                "manifest": advert.manifest,
                "total": advert.total,
                "held": advert.held,
                "complete": advert.complete(),
                "owner": advert.owner,
            })
        })
        .collect();
    json!(refs)
}

fn modules_json(dir: &Path) -> Response {
    let Ok(store) = spirit_core::BlobStore::open(dir) else {
        return Response::error(500, "no store on this node");
    };
    let trust = match spirit_core::identity::load(dir) {
        Some(identity) => spirit_core::Trust::load(dir).with_own(identity.dgid()),
        None => spirit_core::Trust::load(dir),
    };
    let listed: Vec<serde_json::Value> = spirit_schema::modules::list(&store)
        .into_iter()
        .flat_map(|(name, versions)| {
            let trust = &trust;
            versions.into_iter().map(move |version| {
                json!({
                    "name": name,
                    "role": version.module.role.label(),
                    "version": version.module.version,
                    "abi_version": version.module.abi_version,
                    "ci": version.ci.to_string(),
                    "blob": version.blob.map(|blob| blob.to_string()),
                    "signer": version.signer.map(|dgid| dgid.to_string()),
                    "trusted": version.trusted(trust, spirit_core::TrustLevel::Cache),
                    "held": version.held,
                    "legacy": version.legacy,
                })
            })
        })
        .collect();
    Response::json(200, json!(listed))
}

fn status(context: &Context<'_>) -> Response {
    let ticket = context.mesh.map(|mesh| {
        crate::iroh_tickets::endpoint::EndpointTicket::from(mesh.endpoint_addr()).to_string()
    });
    let dgid = context
        .mesh
        .and_then(|mesh| mesh.self_dgid())
        .map(|dgid| dgid.to_string());
    Response::json(
        200,
        json!({
            "node_id": context.node_id,
            "ticket": ticket,
            "dgid": dgid,
            "peers": context.peer_count,
            "refs": refs_json(context.dir),
        }),
    )
}

fn manifest(dir: &Path, name: &str) -> Response {
    let Ok(store) = BlobStore::open(dir) else {
        return Response::error(500, "store unavailable");
    };
    let Ok(text) = std::fs::read_to_string(store.root().join("refs").join(name)) else {
        return Response::error(404, "no such ref");
    };
    let Some(hash) = BlobHash::parse(&text) else {
        return Response::error(500, "ref is not a blob hash");
    };
    let Ok(bytes) = store.get(hash) else {
        return Response::error(404, "manifest blob not held");
    };
    let Ok(manifest) = ciborium::from_reader::<ciborium::Value, _>(bytes.as_slice()) else {
        return Response::error(500, "manifest does not decode");
    };
    if spirit_core::envelope::of(&bytes).is_none() {
        return Response::error(500, "manifest does not decode");
    }
    match serde_json::to_vec(&ops::cbor_to_json(&manifest)) {
        Ok(body) => Response {
            status: 200,
            content_type: "application/json",
            immutable: false,
            body,
            disposition: None,
        },
        Err(_) => Response::error(500, "manifest does not encode"),
    }
}

fn blob(dir: &Path, hash: &str, name: Option<String>) -> Response {
    let Some(hash) = BlobHash::parse(hash) else {
        return Response::error(404, "not a blob hash");
    };
    let Ok(store) = BlobStore::open(dir) else {
        return Response::error(500, "store unavailable");
    };
    match store.get(hash) {
        Ok(body) => Response {
            status: 200,
            content_type: "application/octet-stream",
            immutable: true,
            body,
            disposition: name.filter(|name| !name.is_empty()).map(|name| {
                name.chars()
                    .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
                    .collect()
            }),
        },
        Err(_) => Response::error(404, "blob not held"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize)]
    struct LegacyCard {
        name: String,
        image: String,
    }

    #[derive(serde::Serialize)]
    struct LegacyManifest {
        set: String,
        cards: Vec<LegacyCard>,
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("spirit-gateway-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn body_json(response: &Response) -> serde_json::Value {
        serde_json::from_slice(&response.body).unwrap()
    }

    fn get(path: &str, dir: &Path, resolvers: &Resolvers) -> Response {
        route("GET", path, dir, "n", 0, resolvers)
    }

    fn none() -> Resolvers {
        Resolvers::new()
    }

    fn one(name: &str, resolver: Resolver) -> Resolvers {
        let mut resolvers = Resolvers::new();
        resolvers.insert(name.to_string(), resolver);
        resolvers
    }

    #[test]
    fn seeding_a_peer_needs_a_mesh() {
        let dir = scratch("seed-no-mesh");
        let response = post(
            "/gateway/seed",
            br#"{"value":"endpointabc"}"#,
            Some("t"),
            &dir,
            Some("t"),
        );
        assert_eq!(response.status, 503);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_paths_and_methods_are_refused() {
        let dir = scratch("refuse");
        assert_eq!(get("/elsewhere", &dir, &none()).status, 404);
        assert_eq!(
            route("POST", "/gateway/refs", &dir, "n", 0, &none()).status,
            403
        );
        assert_eq!(
            route("PUT", "/gateway/refs", &dir, "n", 0, &none()).status,
            405
        );
        assert_eq!(
            route("OPTIONS", "/gateway/refs", &dir, "n", 0, &none()).status,
            204
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn post(
        path: &str,
        body: &[u8],
        bearer: Option<&str>,
        dir: &Path,
        token: Option<&str>,
    ) -> Response {
        let resolvers = none();
        route_request(
            &Request {
                method: "POST".into(),
                path: path.into(),
                query: String::new(),
                bearer: bearer.map(String::from),
                body: body.to_vec(),
            },
            &Context {
                dir,
                node_id: "n",
                peer_count: 0,
                resolvers: &resolvers,
                token,
                started: None,
                mesh: None,
                local: false,
            },
        )
    }

    #[test]
    fn writes_need_the_right_bearer_token() {
        let dir = scratch("token");
        assert_eq!(
            post("/gateway/blob", b"x", None, &dir, Some("secret")).status,
            401
        );
        assert_eq!(
            post("/gateway/blob", b"x", Some("wrong"), &dir, Some("secret")).status,
            401
        );
        let stored = post(
            "/gateway/blob",
            b"hello",
            Some("secret"),
            &dir,
            Some("secret"),
        );
        assert_eq!(stored.status, 200);
        let hash = BlobHash::of(b"hello").to_string();
        assert_eq!(body_json(&stored)["hash"], hash);
        let served = get(&format!("/gateway/blob/{hash}?name=hi.txt"), &dir, &none());
        assert_eq!(served.body, b"hello");
        assert_eq!(served.disposition.as_deref(), Some("hi.txt"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_write_lifecycle_runs_over_http() {
        let dir = scratch("lifecycle");
        let token = Some("t");
        let ci = body_json(&post(
            "/gateway/cir",
            br#"{"kind":"song","body":{"title":"A"}}"#,
            token,
            &dir,
            token,
        ))["ci"]
            .as_str()
            .unwrap()
            .to_string();
        let td = body_json(&post(
            "/gateway/tdr",
            br#"{"kind":"flac-encode","body":{"variant":"flac"}}"#,
            token,
            &dir,
            token,
        ))["td"]
            .as_str()
            .unwrap()
            .to_string();
        let blob = body_json(&post("/gateway/blob", b"flac", token, &dir, token))["hash"]
            .as_str()
            .unwrap()
            .to_string();
        let att = body_json(&post(
            "/gateway/attest",
            format!(r#"{{"ci":"{ci}","td":"{td}","blob":"{blob}"}}"#).as_bytes(),
            token,
            &dir,
            token,
        ))["att"]
            .as_str()
            .unwrap()
            .to_string();
        let added = post(
            "/gateway/collection/favorites",
            format!(r#"{{"op":"add","ci":"{ci}","label":"A"}}"#).as_bytes(),
            token,
            &dir,
            token,
        );
        assert_eq!(
            added.status,
            200,
            "{}",
            String::from_utf8_lossy(&added.body)
        );
        let attested = post(
            "/gateway/collection/favorites",
            format!(r#"{{"op":"attest","hash":"{att}"}}"#).as_bytes(),
            token,
            &dir,
            token,
        );
        assert_eq!(attested.status, 200);
        let view = body_json(&get(
            &format!("/gateway/artifacts/{}", ci.trim_start_matches("ci:")),
            &dir,
            &none(),
        ));
        assert_eq!(view["artifacts"][0]["variant"], "flac");
        assert_eq!(view["pick"]["blob"], blob);
        let collection = body_json(&get("/gateway/collection/favorites", &dir, &none()));
        assert_eq!(collection["items"][0]["label"], "A");
        let index = body_json(&get("/gateway/index", &dir, &none()));
        assert_eq!(index["identities"][0]["kind"], "song");
        assert!(get("/", &dir, &none()).body.starts_with(b"<!doctype html>"));
        let bad = post("/gateway/cir", b"[1]", token, &dir, token);
        assert_eq!(bad.status, 400);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn percent_decoding_handles_escapes_plus_and_garbage() {
        assert_eq!(
            percent_decode("https%3A%2F%2Fa.example%2Fx%3Fy%3D1"),
            Some("https://a.example/x?y=1".into())
        );
        assert_eq!(
            percent_decode("3+Emberwing+Scout"),
            Some("3 Emberwing Scout".into())
        );
        assert_eq!(percent_decode("plain"), Some("plain".into()));
        assert_eq!(percent_decode("%zz"), None);
        assert_eq!(percent_decode("%2"), None);
    }

    #[test]
    fn an_unregistered_resolver_name_is_not_found() {
        let dir = scratch("noresolver");
        let response = get("/gateway/resolve/deck?code=CEAAAAA", &dir, &none());
        assert_eq!(response.status, 404);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_resolver_receives_its_name_and_decoded_params() {
        let dir = scratch("resolver");
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let record = seen.clone();
        let resolvers = one(
            "deck",
            Arc::new(move |request: &ResolveRequest| {
                record.lock().unwrap().push(request.clone());
                ResolveReply::json(200, "{\"game\":\"riftbound\"}")
            }),
        );

        let url =
            "/gateway/resolve/deck?url=https%3A%2F%2Fpiltoverarchive.com%2Fdecks%2Fview%2Fabc";
        let response = get(url, &dir, &resolvers);
        assert_eq!(response.status, 200);
        assert_eq!(response.content_type, "application/json");
        assert_eq!(response.body, b"{\"game\":\"riftbound\"}");

        let response = get(
            "/gateway/resolve/deck?text=3+Emberwing+Scout%0A12+Ember+Rune",
            &dir,
            &resolvers,
        );
        assert_eq!(response.status, 200);

        let requests = seen.lock().unwrap().clone();
        assert_eq!(requests[0].name, "deck");
        assert_eq!(
            requests[0].get("url"),
            Some("https://piltoverarchive.com/decks/view/abc")
        );
        assert_eq!(
            requests[1].get("text"),
            Some("3 Emberwing Scout\n12 Ember Rune")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn url_policy_belongs_to_the_resolver_not_the_gateway() {
        // Spirit used to refuse off-allowlist deck sites itself, which meant it
        // shipped a list of Riftbound URLs. The gateway now forwards the
        // parameters untouched and the resolver decides.
        let dir = scratch("offlist");
        let resolvers = one(
            "deck",
            Arc::new(|request: &ResolveRequest| match request.get("url") {
                Some(url) if url.contains("example.com") => {
                    ResolveReply::error(403, "that site is not fetched")
                }
                Some(_) => ResolveReply::json(200, "{}"),
                None => ResolveReply::error(400, "needs a url"),
            }),
        );
        let response = get(
            "/gateway/resolve/deck?url=https%3A%2F%2Fexample.com%2Fdeck",
            &dir,
            &resolvers,
        );
        assert_eq!(response.status, 403);
        let response = get("/gateway/resolve/deck", &dir, &resolvers);
        assert_eq!(response.status, 400);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolver_error_replies_pass_through_as_json() {
        let dir = scratch("errors");
        let resolvers = one(
            "deck",
            Arc::new(|_: &ResolveRequest| {
                ResolveReply::json(422, "{\"error\":\"no cards resolved\"}")
            }),
        );
        let response = get("/gateway/resolve/deck?code=NOTADECK", &dir, &resolvers);
        assert_eq!(response.status, 422);
        let value = body_json(&response);
        assert_eq!(value["error"], "no cards resolved");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn registered_resolvers_are_listed() {
        let dir = scratch("list");
        let resolvers = one(
            "deck",
            Arc::new(|_: &ResolveRequest| ResolveReply::json(200, "{}")),
        );
        let response = get("/gateway/resolvers", &dir, &resolvers);
        assert_eq!(response.status, 200);
        assert_eq!(body_json(&response), serde_json::json!(["deck"]));
        assert_eq!(
            body_json(&get("/gateway/resolvers", &dir, &none())),
            serde_json::json!([])
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_manifest_keeps_fields_beyond_the_generic_shape() {
        #[derive(serde::Serialize)]
        struct WideCard {
            name: String,
            image: String,
            riftbound_id: String,
            energy: Option<i64>,
        }
        #[derive(serde::Serialize)]
        struct WideManifest {
            set: String,
            cards: Vec<WideCard>,
        }
        let dir = scratch("wide");
        let store = BlobStore::open(&dir).unwrap();
        let manifest = WideManifest {
            set: "riftbound".into(),
            cards: vec![WideCard {
                name: "Emberwing Scout".into(),
                image: BlobHash::of(b"art").to_string(),
                riftbound_id: "ogn-007-298".into(),
                energy: Some(2),
            }],
        };
        let mut encoded = Vec::new();
        ciborium::into_writer(&manifest, &mut encoded).unwrap();
        let manifest_hash = store.put(&encoded).unwrap();
        std::fs::create_dir_all(store.root().join("refs")).unwrap();
        std::fs::write(
            store.root().join("refs").join("riftbound"),
            format!("{manifest_hash}\n"),
        )
        .unwrap();
        let served = route(
            "GET",
            "/gateway/ref/riftbound/manifest",
            &dir,
            "n",
            0,
            &none(),
        );
        assert_eq!(served.status, 200);
        let decoded = body_json(&served);
        assert_eq!(decoded["set"], "riftbound");
        assert_eq!(decoded["cards"][0]["riftbound_id"], "ogn-007-298");
        assert_eq!(decoded["cards"][0]["energy"], 2);
        assert_eq!(decoded["cards"][0]["name"], "Emberwing Scout");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ref_names_cannot_escape_the_refs_dir() {
        assert_eq!(manifest_ref_name("/gateway/ref/hob/manifest"), Some("hob"));
        assert_eq!(manifest_ref_name("/gateway/ref/../secrets/manifest"), None);
        assert_eq!(manifest_ref_name("/gateway/ref//manifest"), None);
        assert_eq!(manifest_ref_name("/gateway/ref/a/b/manifest"), None);
    }

    #[test]
    fn the_module_endpoint_lists_versions_with_their_signers() {
        let dir = scratch("module-endpoint");
        let store = BlobStore::open(&dir).unwrap();
        let identity = spirit_core::identity::load_or_create(&dir).unwrap();
        for (version, bytes) in [
            ("0.1.0", b"first".as_slice()),
            ("0.2.0", b"second".as_slice()),
        ] {
            spirit_schema::modules::publish(
                &store,
                &identity,
                &spirit_schema::modules::Module::new(
                    "riftbound",
                    spirit_schema::modules::Role::Plugin,
                    version,
                    3,
                ),
                &spirit_core::record::Tdr::new("wasm-harden", &("test",)).unwrap(),
                bytes,
            )
            .unwrap();
        }

        let served = route("GET", "/gateway/modules", &dir, "n", 0, &none());
        assert_eq!(served.status, 200);
        let listed = body_json(&served);
        let rows = listed.as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["name"], "riftbound");
        assert_eq!(rows[0]["role"], "plugin");
        assert_eq!(rows[1]["version"], "0.2.0");
        assert_eq!(rows[1]["signer"], identity.dgid().to_string());
        assert_eq!(rows[1]["trusted"], true);
        assert_eq!(rows[1]["held"], true);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_module_ref_lists_and_serves_its_manifest_and_blob_by_hash() {
        let dir = scratch("modules");
        let store = BlobStore::open(&dir).unwrap();
        let module = b"hardened plugin bytes";
        let manifest = spirit_core::modules::ModuleManifest {
            name: "riftbound".into(),
            kind: spirit_core::modules::ModuleKind::Plugin,
            abi_version: 0,
            display: "Riftbound".into(),
            version: "0.1.0".into(),
            module: BlobHash::of(module).to_string(),
        };
        let manifest_hash =
            spirit_core::modules::publish_module(&store, &manifest, module).unwrap();

        let refs = route("GET", "/gateway/refs", &dir, "n", 0, &none());
        let listed = body_json(&refs);
        assert_eq!(listed[0]["name"], "modules/riftbound");
        assert_eq!(listed[0]["manifest"], manifest_hash.to_string());
        assert_eq!(listed[0]["complete"], true);

        let served = route(
            "GET",
            &format!("/gateway/blob/{manifest_hash}"),
            &dir,
            "n",
            0,
            &none(),
        );
        assert_eq!(served.status, 200);
        let decoded = spirit_core::modules::ModuleManifest::decode(&served.body).unwrap();
        assert_eq!(decoded, manifest);

        let blob = route(
            "GET",
            &format!("/gateway/blob/{}", BlobHash::of(module)),
            &dir,
            "n",
            0,
            &none(),
        );
        assert_eq!(blob.status, 200);
        assert!(blob.immutable);
        assert_eq!(blob.body, module);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn status_names_the_node_and_counts_peers() {
        let dir = scratch("status");
        let response = route("GET", "/gateway/status", &dir, "node-abc", 3, &none());
        assert_eq!(response.status, 200);
        let value = body_json(&response);
        assert_eq!(value["node_id"], "node-abc");
        assert_eq!(value["peers"], 3);
        assert!(value["ticket"].is_null());
        assert!(value["dgid"].is_null());
        assert!(value["refs"].as_array().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stored_ref_serves_its_manifest_as_json_and_its_blobs_immutably() {
        let dir = scratch("roundtrip");
        let store = BlobStore::open(&dir).unwrap();
        let art = store.put(b"jpeg bytes").unwrap();
        let manifest = LegacyManifest {
            set: "hob".into(),
            cards: vec![LegacyCard {
                name: "Attercop".into(),
                image: art.to_string(),
            }],
        };
        let mut encoded = Vec::new();
        ciborium::into_writer(&manifest, &mut encoded).unwrap();
        let manifest_hash = store.put(&encoded).unwrap();
        std::fs::create_dir_all(store.root().join("refs")).unwrap();
        std::fs::write(
            store.root().join("refs").join("hob"),
            format!("{manifest_hash}\n"),
        )
        .unwrap();

        let refs = route("GET", "/gateway/refs", &dir, "n", 0, &none());
        let listed = body_json(&refs);
        assert_eq!(listed[0]["name"], "hob");
        assert_eq!(listed[0]["complete"], true);

        let served = route("GET", "/gateway/ref/hob/manifest", &dir, "n", 0, &none());
        assert_eq!(served.status, 200);
        assert!(!served.immutable);
        let decoded = body_json(&served);
        assert_eq!(decoded["set"], "hob");
        assert_eq!(decoded["cards"][0]["image"], art.to_string());

        let blob = route(
            "GET",
            &format!("/gateway/blob/{art}"),
            &dir,
            "n",
            0,
            &none(),
        );
        assert_eq!(blob.status, 200);
        assert!(blob.immutable);
        assert_eq!(blob.body, b"jpeg bytes");

        let missing = route(
            "GET",
            &format!("/gateway/blob/{}", BlobHash::of(b"absent")),
            &dir,
            "n",
            0,
            &none(),
        );
        assert_eq!(missing.status, 404);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
