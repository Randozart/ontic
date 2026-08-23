//! Cloud HTTPS transport via system `curl`.
//!
//! Doctrine note: Ontic stays dependency-light; TLS comes from the platform
//! curl binary (ubiquitous on Linux/macOS/Windows 10+), the same way MLIR
//! toolchain binaries are consumed. The API key NEVER touches argv — it is
//! written to a 0600-permission header file consumed via `-H @file`, which
//! is deleted before results are parsed.

use std::path::PathBuf;
use std::process::Command;

pub struct Response {
    pub status: u16,
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AuthStyle {
    /// `Authorization: Bearer <key>` (OpenAI-compatible providers)
    Bearer,
    /// `x-goog-api-key: <key>` (Google generative language API)
    XGoogApiKey,
}



/// Pure builder for the protected header file contents (unit tested):
/// one Content-Type line plus the credential line.
pub fn header_file_contents(style: AuthStyle, api_key: &str) -> String {
    let value = match style {
        AuthStyle::Bearer => format!("Authorization: Bearer {}", api_key),
        AuthStyle::XGoogApiKey => format!("x-goog-api-key: {}", api_key),
    };
    format!(
        "Content-Type: application/json\n{}\n",
        value
    )
}

struct TempFile {
    path: PathBuf,
}

impl TempFile {
    fn create(dir: &PathBuf, name: &str, contents: &str, mode_600: bool) -> Result<TempFile, String> {
        let path = dir.join(name);
        std::fs::write(&path, contents).map_err(|e| format!("write {}: {}", name, e))?;
        if mode_600 {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                    .map_err(|e| format!("chmod {}: {}", name, e))?;
            }
        }
        Ok(TempFile { path })
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// POST a JSON body over HTTPS via curl.
///
/// `extra_headers` are raw `-H` payloads supplied by callers that contain no
/// secrets (the credential goes through the protected file).
pub fn post_json(
    url: &str,
    api_key: Option<(&str, AuthStyle)>,
    extra_headers: &[String],
    body: &str,
    timeout_secs: u64,
) -> Result<Response, String> {
    if curl_missing() {
        return Err(
            "curl not found on PATH — required for cloud sampler transports".to_string(),
        );
    }
    let dir = scratch_dir();
    let body_f = TempFile::create(&dir, "body.json", body, false)?;
    // Credential never enters argv: header file with 0600 perms, deleted on drop.
    let hdr = match api_key {
        Some((key, style)) => Some(TempFile::create(
            &dir,
            "auth.txt",
            &header_file_contents(style, key),
            true,
        )?),
        None => None,
    };

    let out_path = dir.join("resp.txt");
    let mut cmd = Command::new("curl");
    cmd.arg("-sS")
        .arg("--max-time")
        .arg(timeout_secs.to_string())
        .arg("-X")
        .arg("POST")
        .arg("-H")
        .arg("Content-Type: application/json");
    if let Some(hf) = &hdr {
        cmd.arg("-H").arg(format!("@{}", hf.path.display()));
    }
    for h in extra_headers {
        cmd.arg("-H").arg(h);
    }
    cmd.arg("--data")
        .arg(format!("@{}", body_f.path.display()))
        .arg(url)
        .arg("-o")
        .arg(out_path.to_str().ok_or("bad out path")?)
        .arg("-w")
        .arg("%{http_code}");

    let out = cmd.output().map_err(|e| format!("curl spawn failed: {}", e))?;
    // hdr drops here (deleted) before anything else reads responses.
    if !out.status.success() {
        return Err(format!(
            "curl failed: {} {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let code_str = String::from_utf8_lossy(&out.stdout);
    let status: u16 = code_str
        .trim()
        .parse()
        .map_err(|_| format!("bad curl status `{}`", code_str.trim()))?;
    let body = std::fs::read_to_string(&out_path)
        .map_err(|e| format!("read response: {}", e))?;
    Ok(Response { status, body })
}

fn curl_missing() -> bool {
    which::none()
}

mod which {
    /// Minimal PATH probe for the curl executable.
    pub fn none() -> bool {
        let path = match std::env::var("PATH") {
            Ok(p) => p,
            Err(_) => return false,
        };
        for dir in path.split(':') {
            let p = std::path::Path::new(dir).join("curl");
            if p.exists() {
                return false;
            }
        }
        true
    }
}

fn scratch_dir() -> PathBuf {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ontic-cloud-{}-{}", std::process::id(), n));
    std::fs::create_dir_all(&dir).expect("cloud scratch dir");
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_contents_bearer_and_goog() {
        assert_eq!(
            header_file_contents(AuthStyle::Bearer, "K1"),
            "Content-Type: application/json\nAuthorization: Bearer K1\n"
        );
        assert_eq!(
            header_file_contents(AuthStyle::XGoogApiKey, "K2"),
            "Content-Type: application/json\nx-goog-api-key: K2\n"
        );
    }

    #[test]
    fn test_tempfile_deleted_on_drop() {
        let dir = scratch_dir();
        let p = {
            let f = TempFile::create(&dir, "secret.txt", "k", true).unwrap();
            assert!(f.path.exists());
            f.path.clone()
        };
        assert!(!p.exists(), "0600 credential file must be gone after drop");
    }
}
