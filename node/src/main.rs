use qrcode::render::unicode;
use qrcode::QrCode;
use spirit_node::peers::{format_age, format_bytes, PeerSnapshot};
use spirit_node::{fetch, gateway, mesh, peers, serve, serve_mesh, Serving};
use std::error::Error;
use std::path::{Path, PathBuf};

fn store_dir(arg: Option<String>) -> PathBuf {
    arg.or_else(|| std::env::var("SPIRIT_STORE").ok())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").expect("HOME is not set");
            PathBuf::from(home).join(".spirit/store")
        })
}

#[derive(Default)]
struct MeshArgs {
    dir: Option<String>,
    seeds: Vec<String>,
    wants: Vec<String>,
    no_pull: bool,
    gateway: Option<u16>,
}

fn parse_mesh_args(args: impl Iterator<Item = String>) -> Result<MeshArgs, Box<dyn Error>> {
    let mut parsed = MeshArgs::default();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seed" => parsed
                .seeds
                .push(args.next().ok_or("--seed needs a ticket")?),
            "--seed-file" => {
                let path = args.next().ok_or("--seed-file needs a path")?;
                parsed.seeds.extend(read_seed_file(Path::new(&path)));
            }
            "--want" => parsed.wants.push(args.next().ok_or("--want needs a set")?),
            "--no-pull" => parsed.no_pull = true,
            "--gateway" => {
                parsed.gateway = Some(
                    args.next()
                        .ok_or("--gateway needs a port")?
                        .parse()
                        .map_err(|_| "--gateway port must be a number")?,
                )
            }
            other if other.starts_with("--") => return Err(format!("unknown flag {other}").into()),
            other => parsed.dir = Some(other.to_string()),
        }
    }
    Ok(parsed)
}

fn read_seed_file(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| match mesh::is_seed(line) {
            Ok(_) => true,
            Err(error) => {
                eprintln!("skipping seed line {line:?}: {error}");
                false
            }
        })
        .map(String::from)
        .collect()
}

fn gateway_port(flag: Option<u16>) -> Option<u16> {
    flag.or_else(|| std::env::var("SPIRIT_GATEWAY").ok()?.parse().ok())
}

async fn start_gateway(
    port: Option<u16>,
    dir: &Path,
    serving: &Serving,
) -> Result<(), Box<dyn Error>> {
    let Some(port) = port else {
        return Ok(());
    };
    let bound = gateway::spawn(
        port,
        gateway::Gateway {
            dir: dir.to_path_buf(),
            node_id: serving.node_id.clone(),
            mesh: serving.mesh.clone(),
            resolvers: gateway::Resolvers::new(),
        },
    )
    .await?;
    println!(
        "gateway: http://127.0.0.1:{bound}/  (writes need the token in {})",
        gateway::token_path(dir).display()
    );
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("serve") => {
            let dir = store_dir(args.next());
            let serving = serve(&dir).await?;
            start_gateway(gateway_port(None), &dir, &serving).await?;
            println!("imported {} blobs", serving.imported);
            println!("node: {}", serving.node_id);
            println!("identity: {}", serving.ticket);
            let code = QrCode::new(serving.ticket.as_bytes())?;
            println!(
                "{}",
                code.render::<unicode::Dense1x2>()
                    .dark_color(unicode::Dense1x2::Light)
                    .light_color(unicode::Dense1x2::Dark)
                    .build()
            );
            for served in &serving.refs {
                println!("ref {}: {}", served.name, served.ticket);
            }
            println!("serving — ctrl-c to stop");
            let reporter = tokio::spawn(async move {
                let mut previous = String::new();
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    let snapshot = peers::snapshot();
                    let fingerprint = fingerprint(&snapshot);
                    if snapshot.is_empty() || fingerprint == previous {
                        continue;
                    }
                    previous = fingerprint;
                    println!("-- peers --");
                    for peer in &snapshot {
                        print_peer(peer);
                    }
                }
            });
            tokio::signal::ctrl_c().await?;
            reporter.abort();
            println!("-- peers (final) --");
            for peer in &peers::snapshot() {
                print_peer(peer);
            }
            serving.shutdown().await?;
            Ok(())
        }
        Some("fetch") => {
            let ticket = args
                .next()
                .ok_or("usage: spirit-node fetch <ticket> <ref-name> [store-dir]")?;
            let name = args
                .next()
                .ok_or("usage: spirit-node fetch <ticket> <ref-name> [store-dir]")?;
            let fetched = fetch(&ticket, &store_dir(args.next()), &name, |done, total| {
                if done % 25 == 0 {
                    println!("  {done}/{total}");
                }
            })
            .await?;
            println!(
                "fetched {} blobs ({}) -> refs/{}",
                fetched.blobs, fetched.kind, fetched.name
            );
            println!("-- peers --");
            for peer in &peers::snapshot() {
                print_peer(peer);
            }
            Ok(())
        }
        Some("mesh") => {
            let parsed = parse_mesh_args(args)?;
            let dir = store_dir(parsed.dir);
            let serving = serve_mesh(&dir, &parsed.seeds, &parsed.wants).await?;
            start_gateway(gateway_port(parsed.gateway), &dir, &serving).await?;
            if parsed.no_pull {
                serving.mesh.set_replicate(false);
            }
            println!("imported {} blobs", serving.imported);
            println!("node: {}", serving.node_id);
            println!("identity: {}", serving.ticket);
            let code = QrCode::new(serving.ticket.as_bytes())?;
            println!(
                "{}",
                code.render::<unicode::Dense1x2>()
                    .dark_color(unicode::Dense1x2::Light)
                    .light_color(unicode::Dense1x2::Dark)
                    .build()
            );
            for served in &serving.refs {
                println!("ref {}: {}", served.name, served.ticket);
            }
            println!(
                "mesh up — {} seed(s), {} want(s); ctrl-c to stop",
                parsed.seeds.len(),
                parsed.wants.len()
            );
            let reporter = tokio::spawn(async move {
                let mut previous = String::new();
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    let snapshot = peers::snapshot();
                    let refs = peers::registry().ref_statuses();
                    let current = format!("{}|{}", fingerprint(&snapshot), ref_fingerprint(&refs));
                    if current == previous {
                        continue;
                    }
                    previous = current;
                    report(&snapshot, &refs);
                }
            });
            wait_for_stop().await?;
            reporter.abort();
            println!("-- final --");
            report(&peers::snapshot(), &peers::registry().ref_statuses());
            serving.shutdown().await?;
            Ok(())
        }
        Some("pair") => {
            let (_, store) = spirit_node::cli::split_store_flag(args.collect())?;
            let dir = store_dir(store);
            let (status, body) =
                spirit_node::ops::daemon_call(&dir, "POST", "/gateway/pair", b"{}")
                    .ok_or("no running daemon for this store; start `spirit-node mesh` first")??;
            let reply: serde_json::Value = serde_json::from_slice(&body)?;
            if status != 200 {
                return Err(reply["error"]
                    .as_str()
                    .unwrap_or("pairing offer failed")
                    .into());
            }
            println!(
                "scan this on the joining device, or run:  spirit-node join '{}'",
                reply["url"].as_str().unwrap_or_default()
            );
            println!("{}", reply["qr"].as_str().unwrap_or_default());
            println!(
                "valid for {} seconds, one use",
                reply["seconds_left"].as_u64().unwrap_or(0)
            );
            Ok(())
        }
        Some("join") => {
            let (rest, store) = spirit_node::cli::split_store_flag(args.collect())?;
            let url = rest
                .first()
                .ok_or("usage: spirit-node join <spirit://pair?...> [--store <dir>]")?;
            let invite = spirit_node::pair::Invite::parse(url)?;
            let dir = store_dir(store);
            if let Some(port) =
                gateway::read_port(&dir).filter(|port| spirit_node::ops::gateway_alive(*port))
            {
                let token = std::fs::read_to_string(gateway::token_path(&dir))?;
                let (status, body) = spirit_node::ops::gateway_call(
                    port,
                    "POST",
                    "/gateway/join",
                    Some(token.trim()),
                    Some(serde_json::json!({ "url": url }).to_string().as_bytes()),
                )?;
                let reply: serde_json::Value = serde_json::from_slice(&body)?;
                if status != 200 {
                    return Err(reply["error"].as_str().unwrap_or("join failed").into());
                }
                println!(
                    "joined {} through the running daemon",
                    reply["dgid"].as_str().unwrap_or_default()
                );
                return Ok(());
            }
            let secret = spirit_node::node_secret(&dir)?;
            let endpoint =
                spirit_node::iroh::Endpoint::builder(spirit_node::iroh::endpoint::presets::N0)
                    .secret_key(secret)
                    .bind()
                    .await?;
            spirit_node::wait_online(&endpoint).await;
            let joined = spirit_node::pair::join(&endpoint, &dir, &invite).await?;
            println!("joined {}", joined.dgid);
            println!(
                "group head: {}",
                joined.head.map(|h| h.to_string()).unwrap_or_default()
            );
            println!(
                "{} member(s) recorded in {}",
                joined.members.len(),
                spirit_node::pair::seeds_path(&dir).display()
            );
            println!(
                "start the daemon to replicate: spirit-node mesh {}",
                dir.display()
            );
            endpoint.close().await;
            Ok(())
        }
        Some("help") | Some("--help") | Some("-h") | None => {
            println!("{}", spirit_node::cli::USAGE);
            Ok(())
        }
        Some(command) => {
            let (rest, store) = spirit_node::cli::split_store_flag(args.collect())?;
            let store = spirit_node::cli::Store::open(&store_dir(store))?;
            print!("{}", spirit_node::cli::run(&store, command, rest)?);
            Ok(())
        }
    }
}

async fn wait_for_stop() -> Result<(), Box<dyn Error>> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result?,
            _ = terminate.recv() => {}
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await?;
        Ok(())
    }
}

fn report(snapshot: &[PeerSnapshot], refs: &[peers::RefStatus]) {
    println!("-- peers --");
    for peer in snapshot {
        print_peer(peer);
    }
    println!("-- refs --");
    for status in refs {
        println!("  {}", status.label());
        if !status.providers.is_empty() {
            let names: Vec<String> = status
                .providers
                .iter()
                .map(|id| id.chars().take(12).collect())
                .collect();
            println!("    complete on: {}", names.join(", "));
        }
    }
}

fn ref_fingerprint(refs: &[peers::RefStatus]) -> String {
    refs.iter()
        .map(|status| {
            format!(
                "{}:{}:{}:{}",
                status.name,
                status.held,
                status.total,
                status.providers.join(",")
            )
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn fingerprint(snapshot: &[PeerSnapshot]) -> String {
    snapshot
        .iter()
        .map(|peer| {
            format!(
                "{}:{}:{}:{}:{}:{}",
                peer.id,
                peer.wire_bytes_sent,
                peer.wire_bytes_received,
                peer.payload_bytes_received,
                peer.state.label(),
                peer.active_addrs.join(",")
            )
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn print_peer(peer: &PeerSnapshot) {
    let mut roles = Vec::new();
    if peer.we_served_them {
        roles.push("served");
    }
    if peer.we_fetched_from_them {
        roles.push("fetched from");
    }
    println!(
        "  {} [{}] {} via {} — {}",
        peer.short_id(),
        roles.join("+"),
        peer.state.label(),
        peer.path_kind().label(),
        peer.discovery.label()
    );
    if peer.dial_failures > 0 {
        println!(
            "    dial failures: {} ({})",
            peer.dial_failures,
            peer.last_dial_error.as_deref().unwrap_or("unknown")
        );
    }
    if !peer.refs_advertised.is_empty() {
        println!("    advertises: {}", peer.refs_advertised.join(", "));
    }
    if peer.wire_bytes_known {
        println!(
            "    wire: sent {} / recv {} over {} connection(s)",
            format_bytes(peer.wire_bytes_sent),
            format_bytes(peer.wire_bytes_received),
            peer.total_connections
        );
    }
    if peer.payload_bytes_received > 0 {
        println!(
            "    payload received: {}",
            format_bytes(peer.payload_bytes_received)
        );
    }
    if let Some(rtt) = peer.rtt {
        println!("    rtt: {:.1}ms", rtt.as_secs_f64() * 1000.0);
    }
    for addr in &peer.active_addrs {
        println!("    active: {addr}");
    }
    for addr in &peer.inactive_addrs {
        println!("    inactive: {addr}");
    }
    if let Some(name) = &peer.introduced_by_ref {
        println!("    ref: {name}");
    }
    if let Some(outcome) = &peer.outcome {
        println!("    outcome: {outcome}");
    }
    println!("    last activity: {}", format_age(peer.last_activity));
}
