use crate::ops::{self, Edit, Local};
use spirit_core::{BlobHash, CiHash, TdHash, TrustLevel};
use std::error::Error;
use std::fmt::Write as _;

pub use crate::ops::{parse_hash, Local as Store};

pub const USAGE: &str = "\
spirit-node <command> [args] [--store <dir>]

store
  identity                                   the store's dgid (group key) and node id (device key)
  refs                                       every ref name and the record it points at
  blob put <file|->                          store bytes, print the blob hash
  blob get <hash> [--out <file>]             read bytes back, verified
  blob has <hash>
  blob list                                  every blob with its size and record type
  record show <hash|ci:|td:|att:|col:>       decode any record and print it as JSON

records
  cir mint <kind> <json|@file|->             mint a Content Identity Record, print ci:
  tdr mint <kind> <json|@file|->             mint a Transform Definition Record, print td:
  lock <td|@file>                            pin every input of a transform; prints the locked td:
  run <td> --ci <ci>                         run a locked transform and attest its output to <ci>
  attest content <ci> <td> <blob>            sign (ci, td) -> blob with the store key, print att:
  attest relation <same-as|superseded-by|previous-version> <ci> <other>

collections
  collection list
  collection show <name>                     head, folded items, attestations
  collection ops <name>                      the signed op records in fold order
  collection add <name> <ci> [--label <l>] [--td <td>]
  collection remove <name> <ci>
  collection attest <name> <att>             carry an attestation in the head
  collection record <name> <hash>            carry a record or blob in the head

group
  members                                    the folded device-group membership
  revoke <node-id>                           drop a device from the group
  pair                                       offer a one-use pairing code (needs a running mesh --gateway)
  join <spirit://pair?...>                   join the group that offered the code

resolution
  index                                      the local fold: kinds, links, externals, attestations
  resolve <ci> [--min <level>] [--td <td>]   every candidate artifact and the resolver's pick
  trust [<node-id|dgid> <level>]             list or set trust levels

network
  serve [store-dir]
  fetch <ticket> <ref-name> [store-dir]
  mesh [store-dir] [--seed <ticket>]... [--seed-file <path>] [--want <set>]... [--gateway <port>]";

pub fn split_store_flag(args: Vec<String>) -> Result<(Vec<String>, Option<String>), String> {
    let mut rest = Vec::new();
    let mut store = None;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "--store" {
            store = Some(iter.next().ok_or("--store needs a directory")?);
        } else {
            rest.push(arg);
        }
    }
    Ok((rest, store))
}

fn take_flag(args: &mut Vec<String>, flag: &str) -> Result<Option<String>, String> {
    let Some(position) = args.iter().position(|arg| arg == flag) else {
        return Ok(None);
    };
    args.remove(position);
    if position >= args.len() {
        return Err(format!("{flag} needs a value"));
    }
    Ok(Some(args.remove(position)))
}

fn arg(args: &[String], index: usize, what: &str) -> Result<String, String> {
    args.get(index)
        .cloned()
        .ok_or_else(|| format!("missing {what}; see spirit-node help"))
}

fn published(name: &str, head: BlobHash) -> String {
    format!("refs/{name} -> col:{head}\n")
}

pub fn run(store: &Local, command: &str, args: Vec<String>) -> Result<String, Box<dyn Error>> {
    let mut args = args;
    match (command, args.first().map(String::as_str)) {
        ("identity", _) => {
            let info = store.identity_info();
            Ok(format!(
                "store: {}\ndgid: {}  (group key: {})\nnode id: {}  (device key: {})\n",
                info.store, info.dgid, info.key_path, info.node_id, info.node_key_path
            ))
        }
        ("refs", _) => {
            let mut out = String::new();
            for entry in store.refs() {
                writeln!(out, "{} -> {}  [{}]", entry.name, entry.hash, entry.kind)?;
            }
            Ok(or_empty(out, "no refs"))
        }
        ("blob", Some("put")) => {
            let bytes = read_input(&arg(&args, 1, "file")?)?;
            Ok(format!("{}\n", store.store.put(&bytes)?))
        }
        ("blob", Some("get")) => {
            let out = take_flag(&mut args, "--out")?;
            let bytes = store.store.get(parse_hash(&arg(&args, 1, "hash")?)?)?;
            match out {
                Some(path) => {
                    std::fs::write(&path, &bytes)?;
                    Ok(format!("wrote {} bytes to {path}\n", bytes.len()))
                }
                None => Ok(String::from_utf8_lossy(&bytes).into_owned()),
            }
        }
        ("blob", Some("has")) => Ok(format!(
            "{}\n",
            store.store.has(parse_hash(&arg(&args, 1, "hash")?)?)
        )),
        ("blob", Some("list")) => {
            let mut out = String::new();
            for blob in store.blobs() {
                writeln!(out, "{}  {:>8}  {}", blob.hash, blob.size, blob.kind)?;
            }
            Ok(or_empty(out, "no blobs"))
        }
        ("record", Some("show")) | ("cir", Some("show")) | ("tdr", Some("show")) => {
            let view = store.record(parse_hash(&arg(&args, 1, "hash")?)?)?;
            let mut out = format!("{}  {}\n", view.kind, view.address);
            if let Some(signer) = &view.signer {
                writeln!(
                    out,
                    "signer: {signer} ({})",
                    if view.verified {
                        "signature verifies"
                    } else {
                        "signature INVALID"
                    }
                )?;
            }
            writeln!(out, "{}", serde_json::to_string_pretty(&view.body)?)?;
            Ok(out)
        }
        ("cir", Some("mint")) => {
            let body = parse_body(&arg(&args, 2, "body")?)?;
            Ok(format!(
                "{}\n",
                store.mint_cir(&arg(&args, 1, "kind")?, &body)?
            ))
        }
        ("tdr", Some("mint")) => {
            let body = parse_body(&arg(&args, 2, "body")?)?;
            Ok(format!(
                "{}\n",
                store.mint_tdr(&arg(&args, 1, "kind")?, &body)?
            ))
        }
        ("lock", Some(source)) => {
            let tdr = if let Some(path) = source.strip_prefix('@') {
                let value: serde_json::Value = serde_json::from_slice(&read_input(path)?)?;
                let kind = value
                    .get("kind")
                    .and_then(|k| k.as_str())
                    .ok_or("a transform file is {\"kind\": ..., \"body\": {...}}")?;
                let body = value.get("body").cloned().unwrap_or(serde_json::json!({}));
                spirit_core::record::Tdr::new(kind, &body)?
            } else {
                let hash = parse_hash(source)?;
                spirit_core::record::Tdr::decode(&store.store.get(hash)?)
                    .map_err(|e| format!("{source} is not a TDR: {e}"))?
            };
            let locked = store.lock(&tdr)?;
            Ok(format!(
                "{}\n{} input(s) pinned, {} fetched\n",
                locked.td,
                spirit_routing::transform::inputs(&locked.tdr).len(),
                locked.fetched
            ))
        }
        ("run", Some(td)) => {
            let td = TdHash::from_hash(parse_hash(td)?);
            let ci = take_flag(&mut args, "--ci")?
                .ok_or("run needs --ci <ci>: the identity the output is attested to")?;
            let ci = CiHash::from_hash(parse_hash(&ci)?);
            let ran = store.run(td, ci)?;
            Ok(format!(
                "blob:{}\n{}\n{} round(s) via {}\n",
                ran.blob, ran.att, ran.rounds, ran.td
            ))
        }
        ("attest", Some("content")) => {
            let ci = CiHash::from_hash(parse_hash(&arg(&args, 1, "ci")?)?);
            let td = TdHash::from_hash(parse_hash(&arg(&args, 2, "td")?)?);
            let blob = parse_hash(&arg(&args, 3, "blob")?)?;
            Ok(format!("{}\n", store.attest_content(ci, td, blob)?))
        }
        ("attest", Some("relation")) => {
            let relation = arg(&args, 1, "relation kind")?;
            let from = CiHash::from_hash(parse_hash(&arg(&args, 2, "ci")?)?);
            let to = CiHash::from_hash(parse_hash(&arg(&args, 3, "other ci")?)?);
            Ok(format!("{}\n", store.attest_relation(&relation, from, to)?))
        }
        ("collection", Some("list")) => {
            let mut out = String::new();
            for summary in store.collections() {
                writeln!(
                    out,
                    "{}  col:{}  owner {}  {} item(s), {} attestation(s), {} blob(s) in closure",
                    summary.name,
                    summary.head,
                    short(&summary.owner),
                    summary.items,
                    summary.attestations,
                    summary.closure
                )?;
            }
            Ok(or_empty(out, "no collections"))
        }
        ("collection", Some("show")) => {
            let view = store.collection(&arg(&args, 1, "name")?)?;
            let mut out = format!("collection {}  col:{}\n", view.name, view.head);
            writeln!(out, "owner: {}", view.owner)?;
            if let Some(origin) = &view.forked_from {
                writeln!(out, "forked from: {origin}")?;
            }
            writeln!(out, "items ({}):", view.items.len())?;
            for (index, item) in view.items.iter().enumerate() {
                write!(out, "  {index:>3}  {}  [{}]", item.ci, item.kind)?;
                if let Some(label) = &item.label {
                    write!(out, "  {label:?}")?;
                }
                if let Some(td) = &item.default_td {
                    write!(out, "  default {td}")?;
                }
                writeln!(out)?;
            }
            writeln!(out, "attestations ({}):", view.attestations.len())?;
            for attestation in &view.attestations {
                writeln!(
                    out,
                    "  att:{}  {}",
                    attestation.hash,
                    claim_line(attestation)
                )?;
            }
            writeln!(
                out,
                "closure: {} op(s), {} record(s), {} blob(s) total",
                view.ops, view.records, view.closure
            )?;
            Ok(out)
        }
        ("collection", Some("ops")) => {
            let mut out = String::new();
            for op in store.collection_ops(&arg(&args, 1, "name")?)? {
                writeln!(
                    out,
                    "seq {:>3}  {}  {} {}  [{}]",
                    op.seq,
                    op.hash,
                    op.kind,
                    op.target,
                    if op.counted {
                        "owner-signed"
                    } else {
                        "ignored by fold"
                    }
                )?;
            }
            Ok(or_empty(out, "no ops"))
        }
        ("collection", Some("add")) => {
            let label = take_flag(&mut args, "--label")?;
            let default_td = take_flag(&mut args, "--td")?
                .map(|text| parse_hash(&text).map(TdHash::from_hash))
                .transpose()?;
            let name = arg(&args, 1, "name")?;
            let ci = CiHash::from_hash(parse_hash(&arg(&args, 2, "ci")?)?);
            let head = store.edit_collection(
                &name,
                Edit::Add {
                    ci,
                    label,
                    default_td,
                },
            )?;
            Ok(published(&name, head))
        }
        ("collection", Some("remove")) => {
            let name = arg(&args, 1, "name")?;
            let ci = CiHash::from_hash(parse_hash(&arg(&args, 2, "ci")?)?);
            let head = store.edit_collection(&name, Edit::Remove { ci })?;
            Ok(published(&name, head))
        }
        ("collection", Some("attest")) => {
            let name = arg(&args, 1, "name")?;
            let hash = parse_hash(&arg(&args, 2, "att")?)?;
            let head = store.edit_collection(&name, Edit::Attest { hash })?;
            Ok(published(&name, head))
        }
        ("collection", Some("record")) => {
            let name = arg(&args, 1, "name")?;
            let hash = parse_hash(&arg(&args, 2, "hash")?)?;
            let head = store.edit_collection(&name, Edit::Record { hash })?;
            Ok(published(&name, head))
        }
        ("index", _) => {
            let view = store.index();
            let mut out = String::from("collections:\n");
            for collection in &view.collections {
                writeln!(out, "  {}  owner {}", collection.name, collection.owner)?;
            }
            writeln!(out, "content identities:")?;
            for entry in &view.identities {
                writeln!(out, "  {}  [{}]", entry.ci, entry.kind)?;
                for other in &entry.linked_from {
                    writeln!(out, "    linked from {other}")?;
                }
                for attestation in &entry.attestations {
                    writeln!(
                        out,
                        "    {}  trust {}",
                        claim_line(attestation),
                        attestation.level
                    )?;
                }
            }
            writeln!(out, "external ids:")?;
            for external in &view.externals {
                writeln!(
                    out,
                    "  {}={}  {}",
                    external.key, external.value, external.ci
                )?;
            }
            Ok(out)
        }
        ("resolve", _) => {
            let minimum = match take_flag(&mut args, "--min")? {
                Some(level) => TrustLevel::parse(&level)
                    .ok_or("--min must be one of: unknown, contact, cache, mesh")?,
                None => TrustLevel::Cache,
            };
            let prefer_td = take_flag(&mut args, "--td")?
                .map(|text| parse_hash(&text).map(TdHash::from_hash))
                .transpose()?;
            let view = store.artifacts(
                CiHash::from_hash(parse_hash(&arg(&args, 0, "ci")?)?),
                minimum,
                prefer_td,
            );
            let mut out = format!("{}  [{}]\ncandidates:\n", view.ci, view.kind);
            for artifact in &view.artifacts {
                writeln!(
                    out,
                    "  blob:{}  {}{}  signer {}  trust {}  {}",
                    artifact.blob,
                    artifact.td_kind,
                    artifact
                        .variant
                        .as_ref()
                        .map(|variant| format!(" / {variant}"))
                        .unwrap_or_default(),
                    artifact
                        .signer
                        .as_deref()
                        .map(short)
                        .unwrap_or_else(|| "invalid".into()),
                    artifact.level,
                    match (artifact.held, artifact.expired) {
                        (_, true) => "expired",
                        (true, false) => "held",
                        (false, false) => "not held",
                    }
                )?;
            }
            for relation in &view.relations {
                writeln!(out, "  {}", claim_line(relation))?;
            }
            match &view.pick {
                Some(pick) => writeln!(
                    out,
                    "pick (minimum {}): blob:{}  signer {}  {}",
                    view.minimum,
                    pick.blob,
                    short(&pick.signer),
                    if pick.held { "held" } else { "not held" }
                )?,
                None => writeln!(out, "pick (minimum {}): none", view.minimum)?,
            }
            Ok(out)
        }
        ("members", _) => {
            let mut out = String::new();
            for member in store.members() {
                writeln!(
                    out,
                    "{}  {}{}{}",
                    member.node_id,
                    if member.verified {
                        "verified"
                    } else {
                        "UNVERIFIED"
                    },
                    if member.expired { ", expired" } else { "" },
                    if member.this_device {
                        "  (this device)"
                    } else {
                        ""
                    }
                )?;
            }
            Ok(or_empty(out, "no members; the group collection is empty"))
        }
        ("revoke", Some(node)) => match store.revoke(node)? {
            Some(head) => Ok(format!("revoked {node}; refs/device-group -> col:{head}\n")),
            None => Ok(format!("{node} is not a member\n")),
        },
        ("trust", None) => {
            let mut out = String::new();
            for (index, entry) in store.trust_entries().iter().enumerate() {
                if index == 0 {
                    writeln!(out, "own: {} (mesh)", entry.dgid)?;
                } else {
                    writeln!(out, "{} {}", entry.dgid, entry.level)?;
                }
            }
            Ok(out)
        }
        ("trust", Some(who)) => {
            let level = TrustLevel::parse(&arg(&args, 1, "level")?)
                .ok_or("level must be one of: unknown, contact, cache, mesh")?;
            let dgid = store.set_trust(who, level)?;
            Ok(format!("{dgid} {level}\n"))
        }
        _ => Err(format!("unknown command; see spirit-node help\n\n{USAGE}").into()),
    }
}

fn or_empty(out: String, empty: &str) -> String {
    if out.is_empty() {
        format!("{empty}\n")
    } else {
        out
    }
}

fn short(dgid: &str) -> String {
    dgid.trim_start_matches("dgid:").chars().take(8).collect()
}

fn claim_line(attestation: &ops::AttestationView) -> String {
    let signer = attestation
        .signer
        .as_deref()
        .map(short)
        .unwrap_or_else(|| "unsigned".into());
    match attestation.kind.as_str() {
        "content" => format!(
            "content {} via {} -> {}  by {signer}",
            attestation.ci,
            attestation.td.as_deref().unwrap_or_default(),
            attestation.blob.as_deref().unwrap_or_default()
        ),
        kind => format!(
            "{kind} {} -> {}  by {signer}",
            attestation.ci,
            attestation.other.as_deref().unwrap_or_default()
        ),
    }
}

fn read_input(arg: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    if arg == "-" {
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut std::io::stdin(), &mut bytes)?;
        return Ok(bytes);
    }
    Ok(std::fs::read(arg)?)
}

fn parse_body(arg: &str) -> Result<serde_json::Value, Box<dyn Error>> {
    let text = match arg.strip_prefix('@') {
        Some(path) => String::from_utf8(read_input(path)?)?,
        None if arg == "-" => String::from_utf8(read_input("-")?)?,
        None => arg.to_string(),
    };
    Ok(serde_json::from_str(&text)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> Local {
        let dir = std::env::temp_dir().join(format!("spirit-cli-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Local::open(&dir).unwrap()
    }

    fn run_ok(store: &Local, command: &str, args: &[&str]) -> String {
        run(store, command, args.iter().map(|s| s.to_string()).collect()).unwrap()
    }

    fn last_word(text: &str) -> String {
        text.split_whitespace().last().unwrap().to_string()
    }

    #[test]
    fn the_whole_lifecycle_runs_through_the_cli() {
        let store = scratch("lifecycle");
        let ci = run_ok(
            &store,
            "cir",
            &[
                "mint",
                "song",
                r#"{"artist":"Uncle Iroh","title":"Leaves from the Vine"}"#,
            ],
        );
        let ci = ci.trim().to_string();
        assert!(ci.starts_with("ci:"));
        let td = run_ok(
            &store,
            "tdr",
            &["mint", "flac-encode", r#"{"variant":"flac"}"#],
        );
        let td = td.trim().to_string();
        let flac = store.store.put(b"pretend flac bytes").unwrap();
        let att = run_ok(&store, "attest", &["content", &ci, &td, &flac.to_string()]);
        let att = att.trim().to_string();
        assert!(att.starts_with("att:"));

        run_ok(
            &store,
            "collection",
            &["add", "favorites", &ci, "--label", "Leaves"],
        );
        let published = run_ok(&store, "collection", &["attest", "favorites", &att]);
        assert!(published.starts_with("refs/favorites -> col:"));

        let shown = run_ok(&store, "collection", &["show", "favorites"]);
        assert!(shown.contains("[song]"));
        assert!(shown.contains("\"Leaves\""));
        assert!(shown.contains(&format!("blob:{flac}")));

        let resolved = run_ok(&store, "resolve", &[&ci]);
        assert!(resolved.contains("flac-encode / flac"));
        assert!(resolved.contains(&format!("pick (minimum cache): blob:{flac}")));

        let record = run_ok(&store, "record", &["show", &att]);
        assert!(record.contains("signature verifies"));
        assert!(record.contains("\"record\": \"attestation\""));

        let index = run_ok(&store, "index", &[]);
        assert!(index.contains("favorites"));
        assert!(index.contains("trust mesh"));
    }

    #[test]
    fn a_relation_and_a_removal_round_trip() {
        let store = scratch("relation");
        let a = last_word(&run_ok(
            &store,
            "cir",
            &["mint", "song", r#"{"title":"A"}"#],
        ));
        let b = last_word(&run_ok(
            &store,
            "cir",
            &["mint", "song", r#"{"title":"B"}"#],
        ));
        let att = last_word(&run_ok(&store, "attest", &["relation", "same-as", &a, &b]));
        let shown = run_ok(&store, "record", &["show", &att]);
        assert!(shown.contains("same-as"));
        run_ok(&store, "collection", &["add", "list", &a]);
        run_ok(&store, "collection", &["add", "list", &b]);
        run_ok(&store, "collection", &["remove", "list", &a]);
        let shown = run_ok(&store, "collection", &["show", "list"]);
        assert!(shown.contains("items (1):"));
        assert!(!shown.contains(&a));
        let ops = run_ok(&store, "collection", &["ops", "list"]);
        assert!(ops.contains("owner-signed"));
        let unindexed = run_ok(&store, "resolve", &[&a]);
        assert!(!unindexed.contains("same-as"));
        run_ok(&store, "collection", &["attest", "list", &att]);
        let resolved = run_ok(&store, "resolve", &[&a]);
        assert!(resolved.contains("same-as"));
    }

    #[test]
    fn bodies_with_floats_or_non_objects_are_refused() {
        let store = scratch("refuse");
        let float = run(
            &store,
            "cir",
            vec!["mint".into(), "x".into(), r#"{"n":1.5}"#.into()],
        );
        assert!(float.is_err());
        let list = run(&store, "cir", vec!["mint".into(), "x".into(), "[1]".into()]);
        assert!(list.is_err());
    }

    #[test]
    fn attesting_a_non_cir_is_refused() {
        let store = scratch("notcir");
        let td = last_word(&run_ok(&store, "tdr", &["mint", "x", "{}"]));
        let blob = store.store.put(b"raw").unwrap();
        let result = run(
            &store,
            "attest",
            vec!["content".into(), blob.to_string(), td, blob.to_string()],
        );
        assert!(result.is_err());
    }

    #[test]
    fn hashes_parse_with_or_without_a_prefix() {
        let hash = BlobHash::of(b"x");
        assert_eq!(parse_hash(&format!("ci:{hash}")).unwrap(), hash);
        assert_eq!(parse_hash(&hash.to_string()).unwrap(), hash);
        assert!(parse_hash("ci:nope").is_err());
    }

    #[test]
    fn store_flag_is_split_from_anywhere() {
        let (rest, store) = split_store_flag(
            ["a", "--store", "/tmp/x", "b"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        )
        .unwrap();
        assert_eq!(rest, vec!["a", "b"]);
        assert_eq!(store.as_deref(), Some("/tmp/x"));
    }

    #[test]
    fn lock_pins_a_known_identity_and_run_reports_the_missing_runtime() {
        let store = scratch("transform");
        let ci = last_word(&run_ok(&store, "cir", &["mint", "item", r#"{"name":"A"}"#]));
        let td = last_word(&run_ok(
            &store,
            "tdr",
            &["mint", "flac-encode", r#"{"variant":"flac"}"#],
        ));
        let blob = store.store.put(b"flac").unwrap();
        let att = last_word(&run_ok(
            &store,
            "attest",
            &["content", &ci, &td, &blob.to_string()],
        ));
        run_ok(&store, "collection", &["add", "songs", &ci]);
        run_ok(&store, "collection", &["attest", "songs", &att]);
        let file =
            std::env::temp_dir().join(format!("spirit-cli-transform-{}.json", std::process::id()));
        std::fs::write(
            &file,
            format!(r#"{{"kind":"wasm-transform","body":{{"module":"blob:{blob}","inputs":{{"src":{{"ci":"{ci}"}}}}}}}}"#),
        )
        .unwrap();
        let locked = run_ok(&store, "lock", &[&format!("@{}", file.display())]);
        assert!(locked.starts_with("td:"));
        assert!(locked.contains("1 input(s) pinned, 0 fetched"));
        let locked_td = locked.lines().next().unwrap().to_string();
        let refused = run(&store, "run", vec![locked_td, "--ci".into(), ci]).unwrap_err();
        assert!(refused.to_string().contains("no transform runner"));
        let _ = std::fs::remove_file(file);
    }
}
