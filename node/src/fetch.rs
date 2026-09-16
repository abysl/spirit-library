use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

pub const TIMEOUT: Duration = Duration::from_secs(10);
pub const MAX_BODY: usize = 1 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub host: String,
    pub port: u16,
    pub path: String,
}

impl Target {
    pub fn authority(&self) -> String {
        if self.port == 80 {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

pub fn target(url: &str) -> Result<Target, String> {
    let rest = match url.strip_prefix("http://") {
        Some(rest) => rest,
        None if url.starts_with("https://") => {
            return Err(
                "https gateways need a fetcher from the caller (Mesh::set_fetch); spirit-node carries no TLS"
                    .into(),
            )
        }
        None => return Err(format!("not an http url: {url}")),
    };
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, "/"),
    };
    if authority.is_empty() {
        return Err(format!("no host in {url}"));
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !host.contains(']') || host.ends_with(']') => {
            let port = port
                .parse::<u16>()
                .map_err(|_| format!("bad port in {url}"))?;
            (host, port)
        }
        _ => (authority, 80),
    };
    Ok(Target {
        host: host.trim_matches(|c| c == '[' || c == ']').to_string(),
        port,
        path: path.to_string(),
    })
}

pub fn get(url: &str) -> Result<String, String> {
    let target = target(url)?;
    let addr = (target.host.as_str(), target.port)
        .to_socket_addrs()
        .map_err(|error| format!("resolving {}: {error}", target.host))?
        .next()
        .ok_or_else(|| format!("{} resolves to nothing", target.host))?;
    let mut stream = TcpStream::connect_timeout(&addr, TIMEOUT)
        .map_err(|error| format!("connecting to {addr}: {error}"))?;
    let _ = stream.set_read_timeout(Some(TIMEOUT));
    let _ = stream.set_write_timeout(Some(TIMEOUT));
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nAccept: application/json\r\nUser-Agent: spirit-node\r\nConnection: close\r\n\r\n",
        target.path,
        target.authority()
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("sending to {addr}: {error}"))?;
    let mut raw = Vec::new();
    stream
        .take(MAX_BODY as u64 + 64 * 1024)
        .read_to_end(&mut raw)
        .map_err(|error| format!("reading from {addr}: {error}"))?;
    let (status, body) = parse_response(&raw)?;
    if status != 200 {
        return Err(format!("{url} answered {status}"));
    }
    String::from_utf8(body).map_err(|_| format!("{url} answered with non-utf8 body"))
}

pub fn parse_response(raw: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or("no header terminator in the reply")?;
    let head = std::str::from_utf8(&raw[..split]).map_err(|_| "non-utf8 headers")?;
    let body = &raw[split + 4..];
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| format!("bad status line {status_line:?}"))?;
    let mut chunked = false;
    let mut length: Option<usize> = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "transfer-encoding" => chunked = value.to_ascii_lowercase().contains("chunked"),
            "content-length" => length = value.parse().ok(),
            _ => {}
        }
    }
    let body = if chunked {
        dechunk(body)?
    } else {
        let take = length.unwrap_or(body.len()).min(body.len());
        body[..take].to_vec()
    };
    if body.len() > MAX_BODY {
        return Err("reply body too large".into());
    }
    Ok((status, body))
}

pub fn dechunk(mut body: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let line_end = body
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or("truncated chunk size")?;
        let size_text = std::str::from_utf8(&body[..line_end]).map_err(|_| "bad chunk size")?;
        let size_text = size_text.split(';').next().unwrap_or_default().trim();
        let size = usize::from_str_radix(size_text, 16).map_err(|_| "bad chunk size")?;
        body = &body[line_end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if body.len() < size {
            return Err("truncated chunk".into());
        }
        out.extend_from_slice(&body[..size]);
        body = &body[size..];
        body = body.strip_prefix(b"\r\n").unwrap_or(body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_http_url_parses_to_host_port_and_path() {
        assert_eq!(
            target("http://gateway.local").unwrap(),
            Target {
                host: "gateway.local".into(),
                port: 80,
                path: "/".into()
            }
        );
        assert_eq!(
            target("http://127.0.0.1:8090/gateway/status").unwrap(),
            Target {
                host: "127.0.0.1".into(),
                port: 8090,
                path: "/gateway/status".into()
            }
        );
        assert_eq!(
            target("http://[::1]:8090/x").unwrap(),
            Target {
                host: "::1".into(),
                port: 8090,
                path: "/x".into()
            }
        );
        assert_eq!(target("http://x:99").unwrap().authority(), "x:99");
        assert_eq!(target("http://x").unwrap().authority(), "x");
        assert!(target("https://gateway.local")
            .unwrap_err()
            .contains("set_fetch"));
        assert!(target("ftp://gateway.local").is_err());
        assert!(target("http://").is_err());
        assert!(target("http://x:notaport").is_err());
    }

    #[test]
    fn a_content_length_reply_yields_its_body_and_status() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 5\r\n\r\n{\"a\":1}";
        let (status, body) = parse_response(raw).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, b"{\"a\":");
        let (status, body) = parse_response(b"HTTP/1.1 404 Not Found\r\n\r\nnope").unwrap();
        assert_eq!(status, 404);
        assert_eq!(body, b"nope");
        assert!(parse_response(b"garbage").is_err());
    }

    #[test]
    fn a_chunked_reply_is_reassembled() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n{\"a\"\r\n3;ext=1\r\n:1}\r\n0\r\n\r\n";
        let (status, body) = parse_response(raw).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, b"{\"a\":1}");
        assert!(dechunk(b"5\r\nab").is_err());
        assert!(dechunk(b"zz\r\n").is_err());
    }

    #[test]
    fn get_fetches_from_a_local_listener() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let read = socket.read(&mut request).unwrap();
            let text = String::from_utf8_lossy(&request[..read]).into_owned();
            let body = r#"{"node_id":"abc"}"#;
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(reply.as_bytes()).unwrap();
            text
        });
        let body = get(&format!("http://127.0.0.1:{port}/gateway/status")).unwrap();
        assert_eq!(body, r#"{"node_id":"abc"}"#);
        let request = server.join().unwrap();
        assert!(request.starts_with("GET /gateway/status HTTP/1.1\r\n"));
        assert!(request.contains(&format!("Host: 127.0.0.1:{port}\r\n")));
    }

    #[test]
    fn a_non_200_reply_is_an_error() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = socket.read(&mut request).unwrap();
            socket
                .write_all(b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
        });
        let error = get(&format!("http://127.0.0.1:{port}/gateway/status")).unwrap_err();
        assert!(error.contains("503"), "{error}");
    }
}
