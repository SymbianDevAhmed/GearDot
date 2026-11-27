//! "Check for updates" and the version list: asks GitHub for this project's releases, and on
//! request downloads a release's setup, verifies it and starts it.
//!
//! Integrity rules (nothing unverified is ever started):
//!  * every release asset must come from `https://github.com/<REPO>/releases/download/`, and
//!    every redirect must stay on HTTPS and on GitHub's own hosts;
//!  * the release must have SHA256SUMS.txt listing the setup, and the setup's SHA-256 must match;
//!  * when this build carries a release-signing public key (see scripts/new-signing-key.ps1),
//!    SHA256SUMS.txt must also carry a valid ECDSA P-256 signature (SHA256SUMS.txt.sig) made with
//!    the matching private key. Checksums from the same place as the file only catch a broken
//!    download; the signature also catches a release someone else uploaded;
//!  * the downloaded file stays open, with writing and deleting denied, from the hash check until
//!    the setup has started, so it cannot be swapped in between.
//!
//! Uses WinHTTP (part of Windows, loaded only when the user checks) and BCrypt.
use std::ffi::c_void;
use std::path::PathBuf;

use windows::Win32::Networking::WinHttp::*;
use windows::Win32::Security::Cryptography::*;
use windows::core::{HSTRING, PCWSTR};

/// GitHub repository the updates come from (owner/name).
pub const REPO: &str = "ferloom/Wireless-Device-Battery-Monitor";
/// Prefix of the setup file in a release ("BatteryHub-Taskbar-Setup-v1.1.exe").
const SETUP_PREFIX: &str = "BatteryHub-Taskbar-Setup-";
/// Largest download accepted (the setup is about 1 MB).
const MAX_BODY: usize = 64 * 1024 * 1024;
/// Public key (X || Y, 64 bytes, hex) of the release-signing key, set at build time from
/// `release-signing.pub` when that file exists. None: releases are checked with SHA-256 only.
const RELEASE_KEY_HEX: Option<&str> = option_env!("BH_RELEASE_KEY");

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// "1.1" (tag without the leading v)
    pub version: String,
    pub setup_name: String,
    pub setup_url: String,
    pub sums_url: Option<String>,
    pub sig_url: Option<String>,
    pub page_url: String,
    /// Published as a pre-release on GitHub.
    pub prerelease: bool,
}

#[derive(Clone, Debug)]
pub enum Check {
    UpToDate { latest: String },
    Available(Release),
    Failed(String),
}

/// Asks GitHub for the latest release and compares it with this build. Blocking (a second or so).
pub fn check() -> Check {
    match latest_release() {
        Ok(r) if crate::install::version_cmp(&r.version, crate::install::VERSION).is_gt() => Check::Available(r),
        Ok(r) => Check::UpToDate { latest: r.version },
        Err(e) => Check::Failed(e),
    }
}

fn latest_release() -> Result<Release, String> {
    let body = http_get(&format!("https://api.github.com/repos/{REPO}/releases/latest"))?;
    parse_release(&String::from_utf8_lossy(&body)).ok_or_else(|| "unexpected answer from GitHub".into())
}

/// Every release that has a setup, newest first (for installing a chosen, also older, version).
pub fn list_releases() -> Result<Vec<Release>, String> {
    let body = http_get(&format!("https://api.github.com/repos/{REPO}/releases?per_page=50"))?;
    let text = String::from_utf8_lossy(&body);
    let mut out: Vec<Release> = split_objects(&text).iter().filter_map(|o| parse_release(o)).collect();
    out.sort_by(|a, b| crate::install::version_cmp(&b.version, &a.version));
    out.dedup_by(|a, b| a.version == b.version);
    Ok(out)
}

/// The top-level objects of a JSON array (`[{...}, {...}]`), as text. Strings are skipped, so
/// braces inside release notes do not confuse it.
fn split_objects(json: &str) -> Vec<&str> {
    let b = json.as_bytes();
    let (mut out, mut depth, mut start, mut in_str, mut esc) = (Vec::new(), 0i32, 0usize, false, false);
    for (i, &c) in b.iter().enumerate() {
        if in_str {
            match c {
                _ if esc => esc = false,
                b'\\' => esc = true,
                b'"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match c {
            b'"' => in_str = true,
            b'{' => {
                if depth == 0 {
                    start = i;
                }
                depth += 1;
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    out.push(&json[start..=i]);
                }
            }
            _ => {}
        }
    }
    out
}

/// Picks the fields needed from GitHub's release JSON (a tiny scanner instead of a JSON library,
/// which would make the program several times larger). Asset URLs from anywhere but this
/// repository's release downloads are ignored.
fn parse_release(json: &str) -> Option<Release> {
    let tag = json_string(json, "tag_name")?;
    let version = tag.trim_start_matches(['v', 'V']).to_string();
    if version.is_empty() || !version.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let page_url = json_string(json, "html_url")
        .filter(|u| u.starts_with(&format!("https://github.com/{REPO}/releases/")))
        .unwrap_or_else(|| format!("https://github.com/{REPO}/releases/latest"));
    let urls: Vec<String> = json_strings(json, "browser_download_url").into_iter().filter(|u| is_release_asset(u)).collect();
    let setup_url = urls.iter().find(|u| file_name(u).starts_with(SETUP_PREFIX) && u.ends_with(".exe"))?.clone();
    let sums_url = urls.iter().find(|u| file_name(u) == "SHA256SUMS.txt").cloned();
    let sig_url = urls.iter().find(|u| file_name(u) == "SHA256SUMS.txt.sig").cloned();
    let prerelease = json.contains("\"prerelease\": true") || json.contains("\"prerelease\":true");
    Some(Release { version, setup_name: file_name(&setup_url).to_string(), setup_url, sums_url, sig_url, page_url, prerelease })
}

/// A download URL of one of this repository's releases (no other host, path or scheme).
pub fn is_release_asset(url: &str) -> bool {
    let prefix = format!("https://github.com/{REPO}/releases/download/");
    url.strip_prefix(&prefix).is_some_and(|rest| {
        let parts: Vec<&str> = rest.split('/').collect();
        parts.len() == 2
            && parts.iter().all(|p| !p.is_empty() && *p != "." && *p != "..")
            && rest.chars().all(|c| c.is_ascii_alphanumeric() || "._-/+".contains(c))
    })
}

fn file_name(url: &str) -> &str {
    url.rsplit('/').next().unwrap_or(url)
}

/// All string values of `"key": "value"` in `json` (no escapes are expected in these values).
fn json_strings(json: &str, key: &str) -> Vec<String> {
    let pat = format!("\"{key}\"");
    let mut out = Vec::new();
    let mut rest = json;
    while let Some(i) = rest.find(&pat) {
        rest = &rest[i + pat.len()..];
        let after = rest.trim_start();
        let Some(after) = after.strip_prefix(':') else { continue };
        let after = after.trim_start();
        let Some(after) = after.strip_prefix('"') else { continue };
        if let Some(end) = after.find('"') {
            out.push(after[..end].to_string());
        }
    }
    out
}

fn json_string(json: &str, key: &str) -> Option<String> {
    json_strings(json, key).into_iter().next()
}

/// The expected SHA-256 (lower-case hex) of `name` in a SHA256SUMS.txt.
pub fn expected_hash(sums: &str, name: &str) -> Option<String> {
    sums.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next()?, it.next()?.trim_start_matches('*')))
        })
        .find(|(_, n)| *n == name)
        .map(|(h, _)| h.to_lowercase())
        .filter(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()))
}

/// A verified setup on disk, kept open (other processes may read it, nobody may change or
/// delete it) until `start` ran it.
#[derive(Debug)]
pub struct Verified {
    pub path: PathBuf,
    _lock: std::fs::File,
}

impl Verified {
    /// Starts the setup with `args`; the file lock is released once the process runs.
    pub fn start(self, args: &[&str], no_window: bool) -> std::io::Result<()> {
        use std::os::windows::process::CommandExt;
        let mut cmd = std::process::Command::new(&self.path);
        cmd.args(args);
        if no_window {
            cmd.creation_flags(0x0800_0000);
        }
        cmd.spawn().map(|_| ())
    }
}

/// Downloads the setup of `r`, verifies it (see the module notes) and returns it, locked, from a
/// new folder under %TEMP%.
pub fn download(r: &Release) -> Result<Verified, String> {
    if !is_release_asset(&r.setup_url) {
        return Err("the release's setup is not on this project's GitHub release page".into());
    }
    let sums_url = r.sums_url.as_ref().ok_or("the release has no SHA256SUMS.txt, so the setup cannot be verified - it was not started")?;
    let sums_raw = http_get(sums_url)?;
    if let Some(key) = release_key() {
        let sig_url = r.sig_url.as_ref().ok_or("the release is not signed (no SHA256SUMS.txt.sig) - the setup was not started")?;
        let sig = http_get(sig_url)?;
        if !verify_signature(&sums_raw, &decode_sig(&sig).ok_or("the release signature is damaged")?, &key) {
            return Err("the release signature does not match this program's release key - the setup was not started".into());
        }
    }
    let sums = String::from_utf8_lossy(&sums_raw).to_string();
    let expected = expected_hash(&sums, &r.setup_name).ok_or("the release's SHA256SUMS.txt does not list the setup")?;
    let data = http_get(&r.setup_url)?;
    let actual = sha256_hex(&data)?;
    if actual != expected {
        return Err(format!("checksum mismatch (expected {expected}, got {actual}) - the setup was not started"));
    }
    // A fresh folder per download (no fixed, guessable path).
    let dir = std::env::temp_dir().join(format!("BatteryHub-Update-{}-{}", std::process::id(), crate::history::now_secs()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(&r.setup_name);
    std::fs::write(&path, &data).map_err(|e| e.to_string())?;
    // Lock it, then check what is on disk is still what was verified.
    let lock = open_locked(&path).map_err(|e| e.to_string())?;
    let on_disk = std::fs::read(&path).map_err(|e| e.to_string())?;
    if sha256_hex(&on_disk)? != expected {
        return Err("the saved setup changed after it was verified - it was not started".into());
    }
    Ok(Verified { path, _lock: lock })
}

/// Opens `path` for reading with only read sharing: no one can write, rename or delete it.
fn open_locked(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new().read(true).share_mode(0x1 /* FILE_SHARE_READ */).open(path)
}

pub fn sha256(data: &[u8]) -> Result<[u8; 32], String> {
    let mut hash = [0u8; 32];
    let st = unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, data, &mut hash) };
    if st.is_err() {
        return Err("SHA-256 failed".into());
    }
    Ok(hash)
}

pub fn sha256_hex(data: &[u8]) -> Result<String, String> {
    Ok(sha256(data)?.iter().map(|b| format!("{b:02x}")).collect())
}

// ---- release signatures ------------------------------------------------------------------------

fn hex_bytes(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

/// The release-signing public key (X || Y) built into this program, if any.
pub fn release_key() -> Option<[u8; 64]> {
    RELEASE_KEY_HEX.and_then(hex_bytes).and_then(|v| v.try_into().ok())
}

/// SHA256SUMS.txt.sig: the 64-byte signature (r || s), raw or as hex text.
fn decode_sig(b: &[u8]) -> Option<Vec<u8>> {
    if b.len() == 64 {
        return Some(b.to_vec());
    }
    hex_bytes(std::str::from_utf8(b).ok()?).filter(|v| v.len() == 64)
}

/// Checks an ECDSA P-256 signature (r || s) of SHA-256(`data`) with public key X || Y.
pub fn verify_signature(data: &[u8], sig: &[u8], key: &[u8; 64]) -> bool {
    let Ok(hash) = sha256(data) else { return false };
    let mut blob = Vec::with_capacity(8 + 64);
    blob.extend_from_slice(&BCRYPT_ECDSA_PUBLIC_P256_MAGIC.to_le_bytes());
    blob.extend_from_slice(&32u32.to_le_bytes());
    blob.extend_from_slice(key);
    unsafe {
        let mut k = BCRYPT_KEY_HANDLE::default();
        if BCryptImportKeyPair(BCRYPT_ECDSA_P256_ALG_HANDLE, None, BCRYPT_ECCPUBLIC_BLOB, &mut k, &blob, 0).is_err() {
            return false;
        }
        let ok = BCryptVerifySignature(k, None, &hash, sig, BCRYPT_FLAGS(0)).is_ok();
        let _ = BCryptDestroyKey(k);
        ok
    }
}

// ---- WinHTTP -----------------------------------------------------------------------------------

struct Handle(*mut c_void);
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

/// Hosts a download may be redirected to (GitHub's API, pages and release file servers).
pub fn trusted_host(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    h == "github.com" || h == "api.github.com" || h.ends_with(".githubusercontent.com") || h == "githubusercontent.com"
}

/// HTTPS GET. Redirects are followed by WinHTTP but never to plain HTTP, and the final address
/// must be on one of GitHub's hosts. Errors on non-200 answers and on bodies over MAX_BODY.
pub fn http_get(url: &str) -> Result<Vec<u8>, String> {
    let rest = url.strip_prefix("https://").ok_or("only https URLs are used")?;
    let (host, path) = rest.split_once('/').map(|(h, p)| (h, format!("/{p}"))).unwrap_or((rest, "/".into()));
    if !trusted_host(host) {
        return Err(format!("{host} is not one of GitHub's hosts"));
    }
    unsafe {
        let session = Handle(WinHttpOpen(
            &HSTRING::from(format!("BatteryHub/{}", crate::install::VERSION)),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ));
        if session.0.is_null() {
            return Err("WinHTTP is not available".into());
        }
        let _ = WinHttpSetTimeouts(session.0, 10_000, 10_000, 15_000, 30_000);
        // Never follow a redirect from HTTPS to HTTP (also WinHTTP's default; set explicitly).
        let policy: u32 = WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP;
        let _ = WinHttpSetOption(Some(session.0), WINHTTP_OPTION_REDIRECT_POLICY, Some(&policy.to_le_bytes()));
        let connect = Handle(WinHttpConnect(session.0, &HSTRING::from(host), 443, 0));
        if connect.0.is_null() {
            return Err(format!("cannot reach {host}"));
        }
        let request = Handle(WinHttpOpenRequest(
            connect.0,
            &HSTRING::from("GET"),
            &HSTRING::from(path.as_str()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ));
        if request.0.is_null() {
            return Err("cannot create the request".into());
        }
        let headers: Vec<u16> = "Accept: application/vnd.github+json\r\n".encode_utf16().collect();
        let _ = WinHttpAddRequestHeaders(request.0, &headers, WINHTTP_ADDREQ_FLAG_ADD);
        WinHttpSendRequest(request.0, None, None, 0, 0, 0).map_err(|e| format!("no connection ({})", e.message()))?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(|e| format!("no answer ({})", e.message()))?;
        // Where the redirects ended.
        let mut buf = vec![0u16; 2048];
        let mut len = (buf.len() * 2) as u32;
        if WinHttpQueryOption(request.0, WINHTTP_OPTION_URL, Some(buf.as_mut_ptr().cast()), &mut len).is_ok() {
            let final_url = String::from_utf16_lossy(&buf[..(len as usize / 2).min(buf.len())]);
            let final_url = final_url.trim_end_matches('\0');
            let ok = final_url.strip_prefix("https://").and_then(|r| r.split(['/', ':']).next()).is_some_and(trusted_host);
            if !ok {
                return Err(format!("the download was redirected to an unexpected address ({final_url})"));
            }
        }
        let mut status = 0u32;
        let mut len = 4u32;
        let mut index = 0u32;
        let _ = WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as *mut c_void),
            &mut len,
            &mut index,
        );
        if status != 200 {
            return Err(if status == 404 { "no release found".into() } else { format!("HTTP {status}") });
        }
        let mut body = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut read = 0u32;
            WinHttpReadData(request.0, buf.as_mut_ptr().cast(), buf.len() as u32, &mut read).map_err(|e| e.message().to_string())?;
            if read == 0 {
                break;
            }
            if body.len() + read as usize > MAX_BODY {
                return Err("the download is far larger than expected - stopped".into());
            }
            body.extend_from_slice(&buf[..read as usize]);
        }
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "html_url": "https://github.com/ferloom/Wireless-Device-Battery-Monitor/releases/tag/v1.5",
      "tag_name": "v1.5",
      "prerelease": false,
      "assets": [
        { "name": "BatteryHub-Taskbar-Setup-v1.5.exe",
          "browser_download_url": "https://github.com/ferloom/Wireless-Device-Battery-Monitor/releases/download/v1.5/BatteryHub-Taskbar-Setup-v1.5.exe" },
        { "name": "SHA256SUMS.txt",
          "browser_download_url": "https://github.com/ferloom/Wireless-Device-Battery-Monitor/releases/download/v1.5/SHA256SUMS.txt" }
      ]
    }"#;

    #[test]
    fn parses_a_github_release() {
        let r = parse_release(SAMPLE).unwrap();
        assert_eq!(r.version, "1.5");
        assert_eq!(r.setup_name, "BatteryHub-Taskbar-Setup-v1.5.exe");
        assert!(r.sums_url.unwrap().ends_with("/SHA256SUMS.txt"));
        assert!(r.page_url.ends_with("/tag/v1.5"));
        assert!(r.sig_url.is_none() && !r.prerelease);
    }

    #[test]
    fn release_without_a_setup_is_rejected() {
        assert!(parse_release(r#"{"tag_name": "v2.0", "assets": []}"#).is_none());
    }

    #[test]
    fn assets_from_elsewhere_are_ignored() {
        // A setup hosted anywhere else (another repository, another host, plain http) is not used.
        for bad in [
            "https://github.com/someone/else/releases/download/v9/BatteryHub-Taskbar-Setup-v9.exe",
            "http://github.com/ferloom/Wireless-Device-Battery-Monitor/releases/download/v9/BatteryHub-Taskbar-Setup-v9.exe",
            "https://evil.example/ferloom/Wireless-Device-Battery-Monitor/releases/download/v9/BatteryHub-Taskbar-Setup-v9.exe",
            "https://github.com/ferloom/Wireless-Device-Battery-Monitor/releases/download/../x/BatteryHub-Taskbar-Setup-v9.exe",
        ] {
            assert!(!is_release_asset(bad), "{bad}");
            let json = format!(r#"{{"tag_name": "v9", "assets": [{{"browser_download_url": "{bad}"}}]}}"#);
            assert!(parse_release(&json).is_none(), "{bad}");
        }
        assert!(is_release_asset("https://github.com/ferloom/Wireless-Device-Battery-Monitor/releases/download/v1.5/SHA256SUMS.txt"));
        assert!(parse_release(r#"{"tag_name": "v1.5; rm", "assets": []}"#).is_none());
    }

    #[test]
    fn checksum_lines() {
        let h = "a".repeat(64);
        let sums = format!("{h}  BatteryHub-Taskbar-Setup-v1.5.exe\n{}  other.exe\n", "b".repeat(64));
        assert_eq!(expected_hash(&sums, "BatteryHub-Taskbar-Setup-v1.5.exe"), Some(h.clone()));
        assert_eq!(expected_hash(&format!("{h} *x.exe"), "x.exe"), Some(h));
        assert_eq!(expected_hash(&sums, "missing.exe"), None);
        assert_eq!(expected_hash("zz  x.exe", "x.exe"), None); // not a SHA-256
    }

    #[test]
    fn a_release_without_checksums_is_never_installed() {
        let mut r = parse_release(SAMPLE).unwrap();
        r.sums_url = None;
        let err = download(&r).err().unwrap();
        assert!(err.contains("SHA256SUMS"), "{err}");
    }

    #[test]
    fn only_github_hosts() {
        assert!(trusted_host("github.com") && trusted_host("api.github.com") && trusted_host("release-assets.githubusercontent.com"));
        assert!(!trusted_host("github.com.evil.example") && !trusted_host("evilgithubusercontent.com") && !trusted_host("example.com"));
        assert!(http_get("https://example.com/x").is_err());
        assert!(http_get("http://github.com/x").is_err());
    }

    #[test]
    fn release_list_is_split_into_objects() {
        let json = format!(
            r#"[{SAMPLE}, {{"tag_name": "v1.4", "body": "notes with {{ braces }} and \"quotes\"", "browser_download_url": "https://github.com/ferloom/Wireless-Device-Battery-Monitor/releases/download/v1.4/BatteryHub-Taskbar-Setup-v1.4.exe"}}]"#
        );
        let objs = split_objects(&json);
        assert_eq!(objs.len(), 2);
        let rels: Vec<Release> = objs.iter().filter_map(|o| parse_release(o)).collect();
        assert_eq!(rels.iter().map(|r| r.version.as_str()).collect::<Vec<_>>(), vec!["1.5", "1.4"]);
    }

    #[test]
    fn sha256_known_value() {
        assert_eq!(sha256_hex(b"abc").unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn signatures_verify_and_tampering_is_caught() {
        // A throw-away key made here: sign with it, verify with its public half.
        unsafe {
            let mut k = BCRYPT_KEY_HANDLE::default();
            BCryptGenerateKeyPair(BCRYPT_ECDSA_P256_ALG_HANDLE, &mut k, 256, 0).ok().unwrap();
            BCryptFinalizeKeyPair(k, 0).ok().unwrap();
            let data = b"0123  BatteryHub-Taskbar-Setup-v9.exe\n";
            let hash = sha256(data).unwrap();
            let mut sig = vec![0u8; 64];
            let mut n = 0u32;
            BCryptSignHash(k, None, &hash, Some(&mut sig), &mut n, BCRYPT_FLAGS(0)).ok().unwrap();
            let mut blob = vec![0u8; 8 + 64];
            BCryptExportKey(k, None, BCRYPT_ECCPUBLIC_BLOB, Some(&mut blob), &mut n, 0).ok().unwrap();
            let _ = BCryptDestroyKey(k);
            let key: [u8; 64] = blob[8..72].try_into().unwrap();
            assert!(verify_signature(data, &sig, &key));
            assert!(!verify_signature(b"0123  BatteryHub-Taskbar-Setup-v6.exe\n", &sig, &key));
            let mut bad = sig.clone();
            bad[5] ^= 1;
            assert!(!verify_signature(data, &bad, &key));
            let hex: String = sig.iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(decode_sig(hex.as_bytes()), Some(sig));
        }
    }

    #[test]
    fn newer_versions_compare_correctly() {
        use crate::install::version_cmp;
        assert!(version_cmp("1.1", "1.0.1.1").is_gt());
        assert!(version_cmp("1.0.1.1", "1.0.1").is_gt());
        assert!(version_cmp("1.5", "1.10").is_lt());
    }
}
