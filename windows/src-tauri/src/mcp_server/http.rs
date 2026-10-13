// Just enough HTTP/1.1 for one JSON endpoint on the loopback interface.
//
// One request per connection, answered with `Connection: close`: the clients (Claude Code, Codex)
// open a new connection when they have to, and a server that never keeps one alive has no pipelining
// or leftover bytes to get wrong. Everything the parser does not understand is refused, never
// guessed: chunked bodies, a repeated header that decides who the caller is, a head over 16 KiB,
// a body over 1 MiB.

use tokio::io::{AsyncRead, AsyncReadExt};

/// The request line and the headers together.
pub const MAX_HEAD: usize = 16 * 1024;
/// A JSON-RPC message is far smaller; this only bounds what a caller can make us buffer.
pub const MAX_BODY: usize = 1 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadError {
    /// The peer closed before it sent a byte.
    Closed,
    /// The head or the body is over its limit.
    TooLarge,
    Malformed(&'static str),
    /// A transfer coding (`chunked`) this server does not read.
    Unsupported,
    Io,
}

/// The head of a request: everything but the body. Header names are lower-cased.
#[derive(Debug, PartialEq, Eq)]
pub struct Head {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub content_length: usize,
    pub expects_continue: bool,
}

/// Headers that decide who the caller is or what its body is: a second copy is refused.
const SINGLE: [&str; 5] = ["host", "authorization", "content-length", "content-type", "origin"];

fn is_token(text: &str) -> bool {
    !text.is_empty()
        && text.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// `head` is the bytes before the blank line: the request line and the header lines, each ended by
/// CRLF but the last.
pub fn parse_head(head: &[u8]) -> Result<Head, ReadError> {
    let text = std::str::from_utf8(head).map_err(|_| ReadError::Malformed("the head is not text"))?;
    if text.bytes().any(|b| b == b'\0' || (b < 0x20 && b != b'\r' && b != b'\n' && b != b'\t') || b == 0x7f) {
        return Err(ReadError::Malformed("a control character in the head"));
    }
    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    // Lines end in CRLF only: a bare CR or LF inside one is how a header gets smuggled in.
    if text.replace("\r\n", "").bytes().any(|b| b == b'\r' || b == b'\n') {
        return Err(ReadError::Malformed("a bare line break in the head"));
    }
    let mut parts = request_line.split(' ');
    let (Some(method), Some(target), Some(version), None) = (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(ReadError::Malformed("the request line"));
    };
    if !is_token(method) {
        return Err(ReadError::Malformed("the method"));
    }
    if version != "HTTP/1.1" && version != "HTTP/1.0" {
        return Err(ReadError::Malformed("the HTTP version"));
    }
    if !target.starts_with('/') || target.contains('#') {
        return Err(ReadError::Malformed("the request target"));
    }
    let path = target.split('?').next().unwrap_or_default().to_string();

    let mut headers: Vec<(String, String)> = Vec::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err(ReadError::Malformed("a header line"));
        };
        // A line folded under the one before (a leading space) puts the name in doubt.
        if !is_token(name) {
            return Err(ReadError::Malformed("a header name"));
        }
        let name = name.to_ascii_lowercase();
        if SINGLE.contains(&name.as_str()) && headers.iter().any(|(n, _)| *n == name) {
            return Err(ReadError::Malformed("a header given twice"));
        }
        headers.push((name, value.trim_matches(|c| c == ' ' || c == '\t').to_string()));
    }
    let find = |name: &str| headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str());

    if find("transfer-encoding").is_some() {
        return Err(ReadError::Unsupported);
    }
    let content_length = match find("content-length") {
        None => 0,
        Some(v) if !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()) => {
            v.parse::<usize>().map_err(|_| ReadError::TooLarge)?
        }
        Some(_) => return Err(ReadError::Malformed("content-length")),
    };
    if content_length > MAX_BODY {
        return Err(ReadError::TooLarge);
    }
    let expects_continue = find("expect").is_some_and(|v| v.eq_ignore_ascii_case("100-continue"));
    Ok(Head { method: method.to_string(), path, headers, content_length, expects_continue })
}

/// The position of the blank line that ends the head, in `buf`.
fn head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Reads the head (up to `MAX_HEAD`) and says what it is. `leftover` holds the bytes of the body
/// that came with the last read.
pub async fn read_head<R: AsyncRead + Unpin>(stream: &mut R) -> Result<(Head, Vec<u8>), ReadError> {
    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0u8; 2048];
    let end = loop {
        if let Some(end) = head_end(&buf) {
            break end;
        }
        if buf.len() > MAX_HEAD {
            return Err(ReadError::TooLarge);
        }
        match stream.read(&mut chunk).await {
            Ok(0) if buf.is_empty() => return Err(ReadError::Closed),
            Ok(0) => return Err(ReadError::Malformed("the head ended early")),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => return Err(ReadError::Io),
        }
    };
    if end > MAX_HEAD {
        return Err(ReadError::TooLarge);
    }
    let head = parse_head(&buf[..end])?;
    let leftover = buf[end + 4..].to_vec();
    Ok((head, leftover))
}

/// The body of `head`: `leftover` and what is still to come, exactly `content_length` bytes.
pub async fn read_body<R: AsyncRead + Unpin>(
    stream: &mut R,
    head: &Head,
    mut leftover: Vec<u8>,
) -> Result<Vec<u8>, ReadError> {
    if leftover.len() > head.content_length {
        // Bytes after the body: a second request on a connection that is closed after one.
        return Err(ReadError::Malformed("more bytes than content-length"));
    }
    let mut chunk = [0u8; 4096];
    while leftover.len() < head.content_length {
        let want = (head.content_length - leftover.len()).min(chunk.len());
        match stream.read(&mut chunk[..want]).await {
            Ok(0) => return Err(ReadError::Malformed("the body ended early")),
            Ok(n) => leftover.extend_from_slice(&chunk[..n]),
            Err(_) => return Err(ReadError::Io),
        }
    }
    Ok(leftover)
}

fn reason(status: u16) -> &'static str {
    match status {
        100 => "Continue",
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        411 => "Length Required",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        501 => "Not Implemented",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

/// A complete response. `body` of `Some` is JSON.
pub fn response(status: u16, extra: &[(&str, &str)], body: Option<&[u8]>) -> Vec<u8> {
    let mut head = format!("HTTP/1.1 {status} {}\r\n", reason(status));
    if body.is_some() {
        head.push_str("Content-Type: application/json\r\n");
    }
    head.push_str(&format!("Content-Length: {}\r\n", body.map_or(0, <[u8]>::len)));
    head.push_str("Connection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n");
    for (name, value) in extra {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    let mut out = head.into_bytes();
    if let Some(body) = body {
        out.extend_from_slice(body);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `text` as the lines of a head; the CRLF after the last one is what `read_head` cuts off.
    fn head(text: &str) -> Result<Head, ReadError> {
        parse_head(text.strip_suffix("\r\n").unwrap_or(text).as_bytes())
    }

    #[test]
    fn a_post_head_gives_its_method_path_headers_and_length() {
        let h = head("POST /mcp?x=1 HTTP/1.1\r\nHost: 127.0.0.1:1\r\nContent-Length: 12\r\nAuthorization: Bearer  t \r\n").unwrap();
        assert_eq!(h.method, "POST");
        assert_eq!(h.path, "/mcp");
        assert_eq!(h.content_length, 12);
        assert!(!h.expects_continue);
        assert!(h.headers.contains(&("host".into(), "127.0.0.1:1".into())));
        // Names are lower-cased, values trimmed of the spaces around them only.
        assert!(h.headers.contains(&("authorization".into(), "Bearer  t".into())));
    }

    #[test]
    fn a_head_without_a_body_has_no_length() {
        let h = head("GET /mcp HTTP/1.1\r\nHost: localhost:1\r\n").unwrap();
        assert_eq!(h.content_length, 0);
    }

    #[test]
    fn expect_100_continue_is_noticed() {
        let h = head("POST /mcp HTTP/1.1\r\nExpect: 100-Continue\r\nContent-Length: 1\r\n").unwrap();
        assert!(h.expects_continue);
    }

    #[test]
    fn chunked_bodies_are_not_read() {
        assert_eq!(head("POST /mcp HTTP/1.1\r\nTransfer-Encoding: chunked\r\n"), Err(ReadError::Unsupported));
    }

    #[test]
    fn what_decides_the_caller_may_not_be_given_twice() {
        for name in ["Host: a", "Authorization: Bearer a", "Content-Length: 1", "Content-Type: x", "Origin: a"] {
            let text = format!("POST /mcp HTTP/1.1\r\n{name}\r\n{name}\r\n");
            assert_eq!(head(&text), Err(ReadError::Malformed("a header given twice")), "{name}");
        }
    }

    #[test]
    fn a_bad_content_length_is_refused() {
        for value in ["-1", "1.5", "0x10", "", "1 2"] {
            let text = format!("POST /mcp HTTP/1.1\r\nContent-Length: {value}\r\n");
            assert_eq!(head(&text), Err(ReadError::Malformed("content-length")), "{value:?}");
        }
        let text = format!("POST /mcp HTTP/1.1\r\nContent-Length: {}\r\n", MAX_BODY + 1);
        assert_eq!(head(&text), Err(ReadError::TooLarge));
        assert_eq!(head("POST /mcp HTTP/1.1\r\nContent-Length: 99999999999999999999999\r\n"), Err(ReadError::TooLarge));
    }

    #[test]
    fn a_malformed_head_is_refused() {
        for text in [
            "POST /mcp\r\n",
            "POST /mcp HTTP/2\r\n",
            "POST mcp HTTP/1.1\r\n",
            "POST /mcp#x HTTP/1.1\r\n",
            "PO ST /mcp HTTP/1.1\r\n",
            "POST /mcp HTTP/1.1\r\nno colon here\r\n",
            "POST /mcp HTTP/1.1\r\n folded: value\r\n",
            "POST /mcp HTTP/1.1\r\nBad Name: v\r\n",
            "POST /mcp HTTP/1.1\r\nX: a\0b\r\n",
            "POST /mcp HTTP/1.1\r\nX: a\nAuthorization: Bearer b\r\n",
            "POST /mcp HTTP/1.1\r\nX: a\rb\r\n",
        ] {
            assert!(matches!(head(text), Err(ReadError::Malformed(_))), "{text:?}");
        }
    }

    fn run<T>(f: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
    }

    #[test]
    fn a_whole_request_is_read_even_when_it_arrives_in_pieces() {
        let wire = b"POST /mcp HTTP/1.1\r\nContent-Length: 5\r\nHost: h\r\n\r\nhello".to_vec();
        run(async move {
            // A reader that gives out three bytes at a time.
            struct Dribble(Vec<u8>, usize);
            impl AsyncRead for Dribble {
                fn poll_read(
                    mut self: std::pin::Pin<&mut Self>,
                    _: &mut std::task::Context<'_>,
                    buf: &mut tokio::io::ReadBuf<'_>,
                ) -> std::task::Poll<std::io::Result<()>> {
                    let start = self.1;
                    let n = 3.min(self.0.len() - start).min(buf.remaining());
                    buf.put_slice(&self.0[start..start + n]);
                    self.1 += n;
                    std::task::Poll::Ready(Ok(()))
                }
            }
            let mut reader = Dribble(wire, 0);
            let (head, leftover) = read_head(&mut reader).await.unwrap();
            let body = read_body(&mut reader, &head, leftover).await.unwrap();
            assert_eq!(head.method, "POST");
            assert_eq!(body, b"hello");
            assert!(head.headers.contains(&("host".into(), "h".into())));
        });
    }

    #[test]
    fn a_closed_connection_and_a_cut_head_are_told_apart() {
        run(async {
            let mut empty: &[u8] = b"";
            assert_eq!(read_head(&mut empty).await.unwrap_err(), ReadError::Closed);
            let mut cut: &[u8] = b"POST /mcp HTTP/1.1\r\nHost";
            assert_eq!(read_head(&mut cut).await.unwrap_err(), ReadError::Malformed("the head ended early"));
        });
    }

    #[test]
    fn a_head_over_the_limit_is_refused_before_it_is_all_read() {
        run(async {
            let mut wire = b"POST /mcp HTTP/1.1\r\nX: ".to_vec();
            wire.extend(std::iter::repeat(b'a').take(MAX_HEAD * 2));
            let mut reader: &[u8] = &wire;
            assert_eq!(read_head(&mut reader).await.unwrap_err(), ReadError::TooLarge);
        });
    }

    #[test]
    fn a_body_shorter_or_longer_than_its_length_is_refused() {
        run(async {
            let head = parse_head(b"POST /mcp HTTP/1.1\r\nContent-Length: 4").unwrap();
            let mut nothing: &[u8] = b"";
            assert_eq!(
                read_body(&mut nothing, &head, b"ab".to_vec()).await.unwrap_err(),
                ReadError::Malformed("the body ended early")
            );
            let mut nothing: &[u8] = b"";
            assert_eq!(
                read_body(&mut nothing, &head, b"abcde".to_vec()).await.unwrap_err(),
                ReadError::Malformed("more bytes than content-length")
            );
        });
    }

    #[test]
    fn a_response_carries_its_length_and_closes() {
        let text = String::from_utf8(response(401, &[("WWW-Authenticate", "Bearer")], Some(b"{}"))).unwrap();
        assert!(text.starts_with("HTTP/1.1 401 Unauthorized\r\n"));
        assert!(text.contains("Content-Length: 2\r\n"));
        assert!(text.contains("Connection: close\r\n"));
        assert!(text.contains("WWW-Authenticate: Bearer\r\n"));
        assert!(text.ends_with("\r\n\r\n{}"));
        let accepted = String::from_utf8(response(202, &[], None)).unwrap();
        assert!(accepted.contains("Content-Length: 0\r\n"));
        assert!(!accepted.contains("Content-Type:"));
    }
}
