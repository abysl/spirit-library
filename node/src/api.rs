use crate::gateway::{Request, Response, Service};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const SOCKET_FILE: &str = "api.sock";
const MAX_FRAME: usize = 256 << 20;

const MAX_SOCKET_PATH: usize = 100;

pub fn path(dir: &Path) -> PathBuf {
    let preferred = dir.join(SOCKET_FILE);
    if preferred.as_os_str().len() < MAX_SOCKET_PATH {
        return preferred;
    }
    let key = spirit_core::BlobHash::of(dir.as_os_str().as_encoded_bytes()).to_string();
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    runtime.join(format!("spirit-{}.sock", &key[..16]))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiRequest {
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub query: String,
    #[serde(default, with = "serde_bytes_vec")]
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiReply {
    pub status: u16,
    pub content_type: String,
    #[serde(default, with = "serde_bytes_vec")]
    pub body: Vec<u8>,
}

mod serde_bytes_vec {
    use serde::de::{SeqAccess, Visitor};
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(bytes)
    }

    struct BytesVisitor;

    impl<'de> Visitor<'de> for BytesVisitor {
        type Value = Vec<u8>;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a byte string")
        }

        fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Vec<u8>, E> {
            Ok(v.to_vec())
        }

        fn visit_byte_buf<E: serde::de::Error>(self, v: Vec<u8>) -> Result<Vec<u8>, E> {
            Ok(v)
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<u8>, A::Error> {
            let mut out = Vec::new();
            while let Some(byte) = seq.next_element::<u8>()? {
                out.push(byte);
            }
            Ok(out)
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        deserializer.deserialize_byte_buf(BytesVisitor)
    }
}

pub fn encode_frame<T: Serialize>(value: &T) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut payload = Vec::new();
    ciborium::into_writer(value, &mut payload)?;
    let mut frame = (payload.len() as u32).to_be_bytes().to_vec();
    frame.extend(payload);
    Ok(frame)
}

pub fn call(dir: &Path, request: &ApiRequest) -> Result<ApiReply, Box<dyn Error>> {
    #[cfg(unix)]
    {
        use std::io::{Read, Write};
        let mut stream = std::os::unix::net::UnixStream::connect(path(dir))?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(120)))?;
        stream.write_all(&encode_frame(request)?)?;
        let mut len = [0u8; 4];
        stream.read_exact(&mut len)?;
        let len = u32::from_be_bytes(len) as usize;
        if len > MAX_FRAME {
            return Err("reply frame too large".into());
        }
        let mut payload = vec![0u8; len];
        stream.read_exact(&mut payload)?;
        Ok(ciborium::from_reader(payload.as_slice())?)
    }
    #[cfg(not(unix))]
    {
        let _ = (dir, request);
        Err("the local API socket needs a unix platform".into())
    }
}

pub fn alive(dir: &Path) -> bool {
    call(
        dir,
        &ApiRequest {
            method: "GET".into(),
            path: "/gateway/status".into(),
            query: String::new(),
            body: Vec::new(),
        },
    )
    .map(|reply| reply.status == 200)
    .unwrap_or(false)
}

#[cfg(unix)]
pub async fn serve(service: Arc<Service>) -> Result<PathBuf, Box<dyn Error>> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let socket = path(service.dir());
    let _ = std::fs::remove_file(&socket);
    let listener = tokio::net::UnixListener::bind(&socket)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    }
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let service = service.clone();
            tokio::spawn(async move {
                loop {
                    let mut len = [0u8; 4];
                    if stream.read_exact(&mut len).await.is_err() {
                        break;
                    }
                    let len = u32::from_be_bytes(len) as usize;
                    if len > MAX_FRAME {
                        break;
                    }
                    let mut payload = vec![0u8; len];
                    if stream.read_exact(&mut payload).await.is_err() {
                        break;
                    }
                    let reply = match ciborium::from_reader::<ApiRequest, _>(payload.as_slice()) {
                        Ok(request) => {
                            let response = service
                                .dispatch(
                                    Request {
                                        method: request.method,
                                        path: request.path,
                                        query: request.query,
                                        bearer: None,
                                        body: request.body,
                                    },
                                    true,
                                )
                                .await;
                            reply_of(response)
                        }
                        Err(error) => ApiReply {
                            status: 400,
                            content_type: "application/json".into(),
                            body: format!("{{\"error\":\"bad frame: {error}\"}}").into_bytes(),
                        },
                    };
                    let Ok(frame) = encode_frame(&reply) else {
                        break;
                    };
                    if stream.write_all(&frame).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    Ok(socket)
}

#[cfg(not(unix))]
pub async fn serve(service: Arc<Service>) -> Result<PathBuf, Box<dyn Error>> {
    let _ = service;
    Err("the local API socket needs a unix platform".into())
}

fn reply_of(response: Response) -> ApiReply {
    ApiReply {
        status: response.status,
        content_type: response.content_type.to_string(),
        body: response.body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_store_path_gets_a_short_socket_elsewhere() {
        let short = Path::new("/tmp/s");
        assert_eq!(path(short), Path::new("/tmp/s/api.sock"));
        let long = PathBuf::from(format!("/tmp/{}", "x".repeat(120)));
        let socket = path(&long);
        assert!(socket.as_os_str().len() < MAX_SOCKET_PATH);
        assert!(socket.to_string_lossy().contains("spirit-"));
        assert_eq!(path(&long), socket);
    }

    #[test]
    fn frames_round_trip_with_binary_bodies() {
        let request = ApiRequest {
            method: "POST".into(),
            path: "/gateway/blob".into(),
            query: String::new(),
            body: vec![0, 159, 146, 150],
        };
        let frame = encode_frame(&request).unwrap();
        let len = u32::from_be_bytes(frame[..4].try_into().unwrap()) as usize;
        assert_eq!(len, frame.len() - 4);
        let back: ApiRequest = ciborium::from_reader(&frame[4..]).unwrap();
        assert_eq!(back.body, request.body);
        assert_eq!(back.path, "/gateway/blob");
    }
}
