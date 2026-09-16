use crate::{resolve, Policy};
use ciborium::value::Value;
use spirit_core::record::{Attestation, Cir, Claim, Tdr};
use spirit_core::{clock, AttHash, BlobHash, BlobRef, BlobStore, CiHash, Identity, TdHash, Trust};
use spirit_index::Index;
use std::collections::BTreeMap;

pub const HTTP_GET: &str = "http-get";
pub const WASM_TRANSFORM: &str = "wasm-transform";
pub const MAX_ROUNDS: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Ci(CiHash),
    Pinned {
        ci: Option<CiHash>,
        url: Option<String>,
        blob: BlobHash,
    },
    Url(String),
    Query {
        kind: String,
        name: String,
    },
}

impl Input {
    pub fn blob(&self) -> Option<BlobHash> {
        match self {
            Input::Pinned { blob, .. } => Some(*blob),
            _ => None,
        }
    }

    fn to_value(&self) -> Value {
        let text = |s: &str| Value::Text(s.to_string());
        let mut entries: Vec<(Value, Value)> = Vec::new();
        match self {
            Input::Ci(ci) => entries.push((text("ci"), text(&ci.to_string()))),
            Input::Pinned { ci, url, blob } => {
                entries.push((text("blob"), text(&format!("blob:{blob}"))));
                if let Some(ci) = ci {
                    entries.push((text("ci"), text(&ci.to_string())));
                }
                if let Some(url) = url {
                    entries.push((text("url"), text(url)));
                }
            }
            Input::Url(url) => entries.push((text("url"), text(url))),
            Input::Query { kind, name } => entries.push((
                text("query"),
                Value::Map(vec![(text("kind"), text(kind)), (text("name"), text(name))]),
            )),
        }
        Value::Map(entries)
    }

    fn from_value(value: &Value) -> Option<Self> {
        let Value::Map(entries) = value else {
            return None;
        };
        let field = |name: &str| {
            entries.iter().find_map(|(key, item)| match key {
                Value::Text(key) if key == name => Some(item),
                _ => None,
            })
        };
        let text_of = |item: &Value| match item {
            Value::Text(text) => Some(text.clone()),
            _ => None,
        };
        let blob = field("blob")
            .and_then(text_of)
            .and_then(|text| BlobHash::parse(text.strip_prefix("blob:").unwrap_or(&text)));
        let ci = field("ci")
            .and_then(text_of)
            .and_then(|text| CiHash::parse(&text));
        let url = field("url").and_then(text_of);
        if let Some(blob) = blob {
            return Some(Input::Pinned { ci, url, blob });
        }
        if let Some(ci) = ci {
            return Some(Input::Ci(ci));
        }
        if let Some(url) = url {
            return Some(Input::Url(url));
        }
        if let Some(Value::Map(query)) = field("query") {
            let get = |name: &str| {
                query.iter().find_map(|(key, item)| match (key, item) {
                    (Value::Text(key), Value::Text(text)) if key == name => Some(text.clone()),
                    _ => None,
                })
            };
            return Some(Input::Query {
                kind: get("kind")?,
                name: get("name")?,
            });
        }
        None
    }
}

pub fn inputs(tdr: &Tdr) -> BTreeMap<String, Input> {
    let Value::Map(entries) = &tdr.body else {
        return BTreeMap::new();
    };
    let Some(Value::Map(declared)) = entries.iter().find_map(|(key, item)| match key {
        Value::Text(key) if key == "inputs" => Some(item),
        _ => None,
    }) else {
        return BTreeMap::new();
    };
    declared
        .iter()
        .filter_map(|(key, item)| match key {
            Value::Text(name) => Input::from_value(item).map(|input| (name.clone(), input)),
            _ => None,
        })
        .collect()
}

pub fn text_field(tdr: &Tdr, name: &str) -> Option<String> {
    let Value::Map(entries) = &tdr.body else {
        return None;
    };
    entries.iter().find_map(|(key, item)| match (key, item) {
        (Value::Text(key), Value::Text(text)) if key == name => Some(text.clone()),
        _ => None,
    })
}

pub fn is_locked(tdr: &Tdr) -> bool {
    let declared = inputs(tdr);
    let all_pinned = declared.values().all(|input| input.blob().is_some());
    let url_step = tdr.kind == HTTP_GET;
    all_pinned && (!url_step || declared.contains_key("url"))
}

fn with_fields(tdr: &Tdr, inputs: &BTreeMap<String, Input>, extra: &[(&str, String)]) -> Tdr {
    let mut entries: Vec<(Value, Value)> = match &tdr.body {
        Value::Map(entries) => entries
            .iter()
            .filter(|(key, _)| match key {
                Value::Text(key) => key != "inputs" && !extra.iter().any(|(name, _)| name == key),
                _ => true,
            })
            .cloned()
            .collect(),
        _ => Vec::new(),
    };
    for (name, value) in extra {
        entries.push((Value::Text((*name).into()), Value::Text(value.clone())));
    }
    if !inputs.is_empty() {
        entries.push((
            Value::Text("inputs".into()),
            Value::Map(
                inputs
                    .iter()
                    .map(|(name, input)| (Value::Text(name.clone()), input.to_value()))
                    .collect(),
            ),
        ));
    }
    Tdr {
        record: tdr.record.clone(),
        kind: tdr.kind.clone(),
        body: Value::Map(entries),
    }
}

pub trait Fetcher {
    fn get(&self, url: &str) -> Result<Vec<u8>, String>;
}

pub struct RunRequest {
    pub args: Option<Vec<u8>>,
    pub inputs: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunReply {
    Output(Vec<u8>),
    NeedInputs(Vec<String>),
}

pub trait TransformRunner {
    fn run(&self, module: &[u8], request: &RunRequest) -> Result<RunReply, String>;
}

#[derive(Debug)]
pub struct Locked {
    pub td: TdHash,
    pub tdr: Tdr,
    pub fetched: usize,
}

fn find_by_query(store: &BlobStore, index: &Index, kind: &str, name: &str) -> Option<CiHash> {
    let mut matches: Vec<CiHash> = index
        .cis()
        .filter(|(_, found)| *found == kind)
        .filter_map(|(ci, _)| {
            let cir = Cir::decode(&store.get(ci.hash()).ok()?).ok()?;
            let Value::Map(entries) = &cir.body else {
                return None;
            };
            let named = entries.iter().any(|(key, item)| {
                matches!((key, item), (Value::Text(key), Value::Text(text)) if key == "name" && text == name)
            });
            named.then_some(*ci)
        })
        .collect();
    matches.sort();
    matches.into_iter().next()
}

pub fn lock(
    store: &BlobStore,
    index: &Index,
    trust: &Trust,
    fetcher: Option<&dyn Fetcher>,
    tdr: &Tdr,
) -> Result<Locked, String> {
    let unlocked = tdr.address().map_err(|e| e.to_string())?;
    store
        .put(&tdr.encode().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let mut pinned = BTreeMap::new();
    let mut fetched = 0usize;
    let mut declared = inputs(tdr);
    if tdr.kind == HTTP_GET && !declared.contains_key("url") {
        if let Some(url) = text_field(tdr, "url") {
            declared.insert("url".into(), Input::Url(url));
        }
    }
    for (name, input) in declared {
        let resolved = match input {
            Input::Pinned { .. } => input,
            Input::Ci(ci) => {
                let found = resolve(store, index, trust, ci, Policy::default())
                    .ok_or_else(|| format!("input {name:?}: nothing trusted resolves {ci}"))?;
                Input::Pinned {
                    ci: Some(ci),
                    url: None,
                    blob: found.blob,
                }
            }
            Input::Query { kind, name: wanted } => {
                let ci = find_by_query(store, index, &kind, &wanted).ok_or_else(|| {
                    format!("input {name:?}: no {kind} named {wanted:?} in the index")
                })?;
                let found = resolve(store, index, trust, ci, Policy::default())
                    .ok_or_else(|| format!("input {name:?}: nothing trusted resolves {ci}"))?;
                Input::Pinned {
                    ci: Some(ci),
                    url: None,
                    blob: found.blob,
                }
            }
            Input::Url(url) => {
                let fetcher = fetcher.ok_or_else(|| {
                    format!("input {name:?} is a url and no fetcher is registered on this node")
                })?;
                let bytes = fetcher.get(&url)?;
                let blob = store.put(&bytes).map_err(|e| e.to_string())?;
                fetched += 1;
                Input::Pinned {
                    ci: None,
                    url: Some(url),
                    blob,
                }
            }
        };
        pinned.insert(name, resolved);
    }
    let mut extra = vec![("derived_from", unlocked.to_string())];
    if text_field(tdr, "snapshot").is_none() {
        extra.push(("snapshot", clock::now_rfc3339()));
    }
    let locked = with_fields(tdr, &pinned, &extra);
    let td = store
        .put(&locked.encode().map_err(|e| e.to_string())?)
        .map(TdHash::from_hash)
        .map_err(|e| e.to_string())?;
    Ok(Locked {
        td,
        tdr: locked,
        fetched,
    })
}

#[derive(Debug)]
pub struct Ran {
    pub td: TdHash,
    pub blob: BlobHash,
    pub att: AttHash,
    pub rounds: usize,
}

fn module_blob(tdr: &Tdr) -> Result<BlobHash, String> {
    let text = text_field(tdr, "module").ok_or("a wasm-transform names its module")?;
    BlobHash::parse(text.strip_prefix("blob:").unwrap_or(&text))
        .ok_or_else(|| format!("module {text:?} is not a blob hash"))
}

pub fn run(
    store: &BlobStore,
    identity: &Identity,
    runner: Option<&dyn TransformRunner>,
    fetcher: Option<&dyn Fetcher>,
    td: TdHash,
    output: CiHash,
) -> Result<Ran, String> {
    let tdr = Tdr::decode(&store.get(td.hash()).map_err(|e| e.to_string())?)
        .map_err(|e| format!("{td} is not a TDR: {e}"))?;
    if !is_locked(&tdr) {
        return Err(format!("{td} is not locked; lock it first"));
    }
    if !store.has(output.hash()) {
        return Err(format!("{output} is not in the store"));
    }
    let (blob, td, rounds) = match tdr.kind.as_str() {
        HTTP_GET => {
            let url = text_field(&tdr, "url")
                .or_else(|| match inputs(&tdr).get("url") {
                    Some(Input::Pinned { url, .. }) => url.clone(),
                    _ => None,
                })
                .ok_or("an http-get names its url")?;
            let fetcher =
                fetcher.ok_or("this node has no fetcher; http-get runs where one is registered")?;
            let bytes = fetcher.get(&url)?;
            let blob = store.put(&bytes).map_err(|e| e.to_string())?;
            (blob, td, 1)
        }
        WASM_TRANSFORM => {
            let runner = runner
                .ok_or("this node has no transform runner; wasm-transform runs where one is registered")?;
            let module = store
                .get(module_blob(&tdr)?)
                .map_err(|e| format!("module: {e}"))?;
            let args = text_field(&tdr, "args")
                .and_then(|text| BlobHash::parse(text.strip_prefix("blob:").unwrap_or(&text)))
                .map(|hash| store.get(hash).map_err(|e| e.to_string()))
                .transpose()?;
            let mut current = tdr.clone();
            let mut current_td = td;
            let mut rounds = 0usize;
            loop {
                rounds += 1;
                if rounds > MAX_ROUNDS {
                    return Err(format!("{td} asked for inputs {MAX_ROUNDS} times; giving up"));
                }
                let mut declared = inputs(&current);
                let mut bytes = BTreeMap::new();
                for (name, input) in &declared {
                    let blob = input.blob().ok_or_else(|| format!("input {name:?} is unpinned"))?;
                    bytes.insert(
                        name.clone(),
                        store.get(blob).map_err(|e| format!("input {name:?}: {e}"))?,
                    );
                }
                match runner.run(
                    &module,
                    &RunRequest {
                        args: args.clone(),
                        inputs: bytes,
                    },
                )? {
                    RunReply::Output(out) => {
                        let blob = store.put(&out).map_err(|e| e.to_string())?;
                        break (blob, current_td, rounds);
                    }
                    RunReply::NeedInputs(urls) => {
                        let fetcher = fetcher.ok_or(
                            "the transform needs inputs and this node has no fetcher",
                        )?;
                        for url in urls {
                            if declared.values().any(|input| matches!(input, Input::Pinned { url: Some(have), .. } if *have == url)) {
                                continue;
                            }
                            let fetched = fetcher.get(&url)?;
                            let blob = store.put(&fetched).map_err(|e| e.to_string())?;
                            declared.insert(
                                url.clone(),
                                Input::Pinned {
                                    ci: None,
                                    url: Some(url),
                                    blob,
                                },
                            );
                        }
                        current = with_fields(&current, &declared, &[]);
                        current_td = store
                            .put(&current.encode().map_err(|e| e.to_string())?)
                            .map(TdHash::from_hash)
                            .map_err(|e| e.to_string())?;
                    }
                }
            }
        }
        other => {
            return Err(format!(
                "{other:?} is a descriptive transform kind with no runtime; only {HTTP_GET} and {WASM_TRANSFORM} run"
            ))
        }
    };
    let attestation = Attestation::sign(
        Claim::content(output, td, BlobRef::from_hash(blob)),
        identity,
    )
    .map_err(|e| e.to_string())?;
    let att = store
        .put(&attestation.encode().map_err(|e| e.to_string())?)
        .map(AttHash::from_hash)
        .map_err(|e| e.to_string())?;
    Ok(Ran {
        td,
        blob,
        att,
        rounds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use spirit_core::collection::{self, Collection};
    use std::cell::RefCell;

    fn scratch(tag: &str) -> BlobStore {
        let dir =
            std::env::temp_dir().join(format!("spirit-transform-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        BlobStore::open(dir).unwrap()
    }

    struct Web(RefCell<Vec<String>>);

    impl Fetcher for Web {
        fn get(&self, url: &str) -> Result<Vec<u8>, String> {
            self.0.borrow_mut().push(url.to_string());
            Ok(format!("body of {url}").into_bytes())
        }
    }

    struct Concat;

    impl TransformRunner for Concat {
        fn run(&self, module: &[u8], request: &RunRequest) -> Result<RunReply, String> {
            assert_eq!(module, b"wasm bytes");
            if !request.inputs.contains_key("https://example.test/second") {
                return Ok(RunReply::NeedInputs(vec![
                    "https://example.test/second".into()
                ]));
            }
            let mut out = Vec::new();
            for (name, bytes) in &request.inputs {
                out.extend(name.as_bytes());
                out.push(b'=');
                out.extend(bytes);
                out.push(b'\n');
            }
            Ok(RunReply::Output(out))
        }
    }

    fn seed_song(store: &BlobStore, me: &Identity) -> (CiHash, BlobHash) {
        let cir = Cir::new(
            "item",
            &serde_json::json!({ "name": "Leaves from the Vine" }),
        )
        .unwrap();
        let ci = CiHash::from_hash(store.put(&cir.encode().unwrap()).unwrap());
        let flac = store.put(b"flac").unwrap();
        let td = TdHash::from_hash(
            store
                .put(
                    &Tdr::new("flac-encode", &serde_json::json!({ "variant": "flac" }))
                        .unwrap()
                        .encode()
                        .unwrap(),
                )
                .unwrap(),
        );
        let att = Attestation::sign(Claim::content(ci, td, BlobRef::from_hash(flac)), me).unwrap();
        let att = store.put(&att.encode().unwrap()).unwrap();
        collection::publish(
            store,
            &Collection::new("playlist", "songs", me.dgid()).with(Vec::new(), vec![ci.hash(), att]),
        )
        .unwrap();
        (ci, flac)
    }

    #[test]
    fn locking_pins_identities_queries_and_urls_and_records_provenance() {
        let store = scratch("lock");
        let me = Identity::from_secret([1; 32]);
        let (ci, flac) = seed_song(&store, &me);
        let index = Index::build(&store);
        let trust = Trust::new().with_own(me.dgid());
        let tdr = Tdr::new(
            "mp3-encode",
            &serde_json::json!({
                "variant": "mp3",
                "inputs": {
                    "source": { "ci": ci.to_string() },
                    "byname": { "query": { "kind": "item", "name": "Leaves from the Vine" } },
                    "cover": { "url": "https://example.test/cover.jpg" },
                    "preset": { "blob": format!("blob:{}", store.put(b"preset").unwrap()) }
                }
            }),
        )
        .unwrap();
        assert!(!is_locked(&tdr));
        let web = Web(RefCell::new(Vec::new()));
        let locked = lock(&store, &index, &trust, Some(&web), &tdr).unwrap();
        assert!(is_locked(&locked.tdr));
        assert_eq!(locked.fetched, 1);
        let pinned = inputs(&locked.tdr);
        assert_eq!(pinned["source"].blob(), Some(flac));
        assert_eq!(pinned["byname"].blob(), Some(flac));
        assert!(
            matches!(&pinned["cover"], Input::Pinned { url: Some(url), .. } if url.ends_with("cover.jpg"))
        );
        assert_eq!(
            text_field(&locked.tdr, "derived_from"),
            Some(tdr.address().unwrap().to_string())
        );
        assert!(text_field(&locked.tdr, "snapshot").is_some());
        assert_eq!(text_field(&locked.tdr, "variant").as_deref(), Some("mp3"));
        let again = lock(&store, &index, &trust, Some(&web), &locked.tdr).unwrap();
        assert_eq!(again.fetched, 0);
        assert!(lock(&store, &index, &trust, None, &tdr).is_err());
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn a_transform_asks_for_inputs_until_it_can_produce_and_the_output_is_attested() {
        let store = scratch("run");
        let me = Identity::from_secret([1; 32]);
        let (ci, _) = seed_song(&store, &me);
        let index = Index::build(&store);
        let trust = Trust::new().with_own(me.dgid());
        let module = store.put(b"wasm bytes").unwrap();
        let tdr = Tdr::new(
            WASM_TRANSFORM,
            &serde_json::json!({
                "module": format!("blob:{module}"),
                "inputs": { "first": { "url": "https://example.test/first" } }
            }),
        )
        .unwrap();
        let web = Web(RefCell::new(Vec::new()));
        let locked = lock(&store, &index, &trust, Some(&web), &tdr).unwrap();
        let ran = run(&store, &me, Some(&Concat), Some(&web), locked.td, ci).unwrap();
        assert_eq!(ran.rounds, 2);
        assert_ne!(ran.td, locked.td);
        let output = String::from_utf8(store.get(ran.blob).unwrap()).unwrap();
        assert!(output.contains("first=body of https://example.test/first"));
        assert!(output.contains("https://example.test/second=body of https://example.test/second"));
        let attestation = Attestation::decode(&store.get(ran.att.hash()).unwrap()).unwrap();
        assert_eq!(attestation.claim.ci, ci);
        assert_eq!(attestation.claim.td, Some(ran.td));
        assert_eq!(attestation.signer(), Some(me.dgid()));
        let final_tdr = Tdr::decode(&store.get(ran.td.hash()).unwrap()).unwrap();
        assert!(is_locked(&final_tdr));
        assert_eq!(inputs(&final_tdr).len(), 2);
        assert_eq!(web.0.borrow().len(), 2);
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn running_needs_a_lock_and_a_runtime() {
        let store = scratch("refuse");
        let me = Identity::from_secret([1; 32]);
        let (ci, _) = seed_song(&store, &me);
        let unlocked = Tdr::new(
            WASM_TRANSFORM,
            &serde_json::json!({ "module": "blob:00", "inputs": { "x": { "url": "https://example.test" } } }),
        )
        .unwrap();
        let td = TdHash::from_hash(store.put(&unlocked.encode().unwrap()).unwrap());
        assert!(run(&store, &me, Some(&Concat), None, td, ci)
            .unwrap_err()
            .contains("not locked"));
        let descriptive =
            Tdr::new("flac-encode", &serde_json::json!({ "variant": "flac" })).unwrap();
        let td = TdHash::from_hash(store.put(&descriptive.encode().unwrap()).unwrap());
        assert!(run(&store, &me, None, None, td, ci)
            .unwrap_err()
            .contains("no runtime"));
        let get = Tdr::new(
            HTTP_GET,
            &serde_json::json!({ "url": "https://example.test/x" }),
        )
        .unwrap();
        let index = Index::build(&store);
        let trust = Trust::new().with_own(me.dgid());
        let web = Web(RefCell::new(Vec::new()));
        let locked = lock(&store, &index, &trust, Some(&web), &get).unwrap();
        assert!(run(&store, &me, None, None, locked.td, ci)
            .unwrap_err()
            .contains("no fetcher"));
        let ran = run(&store, &me, None, Some(&web), locked.td, ci).unwrap();
        assert_eq!(
            store.get(ran.blob).unwrap(),
            b"body of https://example.test/x"
        );
        let _ = std::fs::remove_dir_all(store.root());
    }
}
