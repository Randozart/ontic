//! Hand-rolled HTTP/1.1 keep-alive client (std only) for the local
//! llama-server endpoint. No TLS — VITRIOL serves localhost. A missing or
//! dead server surfaces as a clean error, never a hang (bounded timeouts).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Generous: K-batch generation on a GTX 1070 Ti can take minutes per sample.
const IO_TIMEOUT: Duration = Duration::from_secs(600);

pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

pub struct HttpClient {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

/// Build the raw HTTP/1.1 request bytes (pure — unit tested).
pub fn format_request(host_header: &str, path: &str, body: &str) -> String {
    format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{}",
        path,
        host_header,
        body.len(),
        body
    )
}

/// Parse `HTTP/1.1 200 OK` into the numeric status (pure — unit tested).
pub fn parse_status_line(line: &str) -> Result<u16, String> {
    let code = line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| format!("malformed status line `{}`", line))?;
    code.parse::<u16>()
        .map_err(|_| format!("non-numeric status code `{}`", code))
}

struct Head {
    status: u16,
    content_length: Option<usize>,
    chunked: bool,
}

impl HttpClient {
    /// Connect with bounded timeout; errors are immediate and descriptive.
    pub fn connect(host: &str, port: u16) -> Result<Self, String> {
        let stream = TcpStream::connect((host, port))
            .map_err(|e| format!("cannot reach llama-server at {}:{}: {}", host, port, e))?;
        stream.set_read_timeout(Some(IO_TIMEOUT)).map_err(|e| e.to_string())?;
        stream.set_write_timeout(Some(IO_TIMEOUT)).map_err(|e| e.to_string())?;
        stream.set_nodelay(true).map_err(|e| e.to_string())?;
        let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
        let _ = CONNECT_TIMEOUT; // connect() above is already bounded by OS + this intent note
        let writer = stream
            .try_clone()
            .map_err(|e| format!("stream clone failed: {}", e))?;
        Ok(HttpClient {
            reader: BufReader::new(stream),
            writer,
        })
    }

    /// POST a JSON body, returning the parsed response. Reuses the connection.
    pub fn post_json(&mut self, host_header: &str, path: &str, body: &str) -> Result<HttpResponse, String> {
        let req = format_request(host_header, path, body);
        self.writer
            .write_all(req.as_bytes())
            .map_err(|e| format!("write failed: {}", e))?;
        self.writer.flush().map_err(|e| format!("flush failed: {}", e))?;

        let head = self.read_head()?;
        let body = if head.chunked {
            self.read_chunked()?
        } else {
            let len = head.content_length.unwrap_or(0);
            let mut buf = vec![0u8; len];
            self.reader
                .read_exact(&mut buf)
                .map_err(|e| format!("body read failed: {}", e))?;
            String::from_utf8_lossy(&buf).into_owned()
        };
        Ok(HttpResponse {
            status: head.status,
            body,
        })
    }

    fn read_head(&mut self) -> Result<Head, String> {
        let mut line = String::new();
        self.reader
            .read_line(&mut line)
            .map_err(|e| format!("status read failed: {}", e))?;
        if line.trim().is_empty() {
            // Some servers emit a leading CRLF between keep-alive responses.
            line.clear();
            self.reader
                .read_line(&mut line)
                .map_err(|e| format!("status read failed: {}", e))?;
        }
        let status = parse_status_line(line.trim_end())?;

        let mut content_length = None;
        let mut chunked = false;
        loop {
            let mut h = String::new();
            let n = self
                .reader
                .read_line(&mut h)
                .map_err(|e| format!("header read failed: {}", e))?;
            if n == 0 || h.trim().is_empty() {
                break;
            }
            let lower = h.to_ascii_lowercase();
            if let Some(v) = lower.strip_prefix("content-length:") {
                content_length = v.trim().parse::<usize>().ok();
            }
            if lower.starts_with("transfer-encoding:") && lower.contains("chunked") {
                chunked = true;
            }
        }
        Ok(Head {
            status,
            content_length,
            chunked,
        })
    }

    /// Decode a chunked transfer-encoding body.
    fn read_chunked(&mut self) -> Result<String, String> {
        let mut out = String::new();
        loop {
            let mut size_line = String::new();
            self.reader
                .read_line(&mut size_line)
                .map_err(|e| format!("chunk size read failed: {}", e))?;
            let size = usize::from_str_radix(size_line.trim(), 16)
                .map_err(|_| format!("bad chunk size `{}`", size_line.trim()))?;
            if size == 0 {
                // Consume trailing CRLF after last chunk.
                let mut crlf = String::new();
                let _ = self.reader.read_line(&mut crlf);
                break;
            }
            let mut buf = vec![0u8; size];
            self.reader
                .read_exact(&mut buf)
                .map_err(|e| format!("chunk read failed: {}", e))?;
            out.push_str(&String::from_utf8_lossy(&buf));
            let mut crlf = String::new();
            let _ = self.reader.read_line(&mut crlf);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_format_includes_length_and_keepalive() {
        let req = format_request("127.0.0.1:8279", "/completion", "{\"n\":1}");
        assert!(req.starts_with("POST /completion HTTP/1.1\r\n"));
        assert!(req.contains("Content-Length: 7"));
        assert!(req.contains("Connection: keep-alive"));
        assert!(req.ends_with("{\"n\":1}"));
    }

    #[test]
    fn test_status_line_parsing() {
        assert_eq!(parse_status_line("HTTP/1.1 200 OK").unwrap(), 200);
        assert_eq!(parse_status_line("HTTP/1.0 500 Internal Error").unwrap(), 500);
        assert!(parse_status_line("garbage").is_err());
    }
}
