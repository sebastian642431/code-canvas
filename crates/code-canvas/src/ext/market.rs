use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use serde_json::Value;

const OPEN_VSX: &str = "https://open-vsx.org/api";
const MAX_VSIX: u64 = 400 * 1024 * 1024;
/// The most a README, a changelog or a manifest may weigh.
const MAX_TEXT: u64 = 4 * 1024 * 1024;
const MAX_ICON: u64 = 2 * 1024 * 1024;

/// The registry's address: Open VSX, or the one `CODE_CANVAS_OPEN_VSX` names (the QA runs'
/// own registry on this computer).
pub fn api() -> String {
    std::env::var("CODE_CANVAS_OPEN_VSX")
        .ok()
        .map(|given| given.trim().trim_end_matches('/').to_string())
        .filter(|given| !given.is_empty())
        .unwrap_or_else(|| OPEN_VSX.to_string())
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MarketItem {
    pub namespace: String,
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub version: String,
    pub downloads: u64,
    pub verified: bool,
    /// The address of its icon, empty when it has none.
    pub icon: String,
    /// The average of its reviews' stars, when it has reviews.
    pub rating: Option<f32>,
    pub reviews: u64,
    pub deprecated: bool,
}

impl MarketItem {
    pub fn id(&self) -> String {
        format!("{}.{}", self.namespace, self.name).to_lowercase()
    }
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn get_json(url: &str) -> Result<Value, String> {
    let mut response = ureq::get(url).header("Accept", "application/json").call().map_err(|e| e.to_string())?;
    let body = response.body_mut().read_to_string().map_err(|e| e.to_string())?;
    serde_json::from_str(&body).map_err(|e| e.to_string())
}

fn encode(query: &str) -> String {
    let mut out = String::new();
    for b in query.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn search(query: &str) -> Result<Vec<MarketItem>, String> {
    let sort = if query.trim().is_empty() { "downloadCount" } else { "relevance" };
    let url = format!("{}/-/search?query={}&size=40&sortBy={sort}&sortOrder=desc", api(), encode(query.trim()));
    Ok(parse_search(&get_json(&url)?))
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .http_status_as_error(false)
        .build()
        .into()
}

/// The bytes at an address of the registry (it sends files on to where they are kept),
/// or why not.
pub fn fetch(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut response = agent().get(url).call().map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    if status == 404 {
        return Err("not on the registry".into());
    }
    if status >= 400 {
        return Err(format!("the registry answered {status}"));
    }
    response.body_mut().with_config().limit(limit).read_to_vec().map_err(|e| e.to_string())
}

pub fn fetch_text(url: &str) -> Result<String, String> {
    let bytes = fetch(url, MAX_TEXT)?;
    Ok(String::from_utf8_lossy(&bytes).trim_start_matches('\u{feff}').to_string())
}

pub fn fetch_json(url: &str) -> Result<Value, String> {
    serde_json::from_str(&fetch_text(url)?).map_err(|e| e.to_string())
}

/// An icon of the registry as a small picture, ready for a texture.
pub fn fetch_icon(url: &str) -> Result<egui::ColorImage, String> {
    let bytes = fetch(url, MAX_ICON)?;
    let small = image::load_from_memory(&bytes).map_err(|e| e.to_string())?.thumbnail(128, 128).into_rgba8();
    let size = [small.width() as usize, small.height() as usize];
    Ok(egui::ColorImage::from_rgba_unmultiplied(size, small.as_raw()))
}

fn number(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn file(v: &Value, key: &str) -> String {
    v.get("files").and_then(|f| f.get(key)).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn rating(v: &Value) -> Option<f32> {
    let stars = v.get("averageRating").and_then(Value::as_f64)?;
    (number(v, "reviewCount") > 0 || stars > 0.0).then_some(stars as f32)
}

fn item(e: &Value) -> MarketItem {
    MarketItem {
        namespace: text(e, "namespace"),
        name: text(e, "name"),
        display_name: Some(text(e, "displayName")).filter(|s| !s.is_empty()).unwrap_or_else(|| text(e, "name")),
        description: text(e, "description"),
        version: text(e, "version"),
        downloads: number(e, "downloadCount"),
        verified: flag(e, "verified"),
        icon: file(e, "icon"),
        rating: rating(e),
        reviews: number(e, "reviewCount"),
        deprecated: flag(e, "deprecated"),
    }
}

/// The extensions of an answer of `/-/search`.
pub fn parse_search(v: &Value) -> Vec<MarketItem> {
    let list = v.get("extensions").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    list.iter().map(item).filter(|e| !e.namespace.is_empty() && !e.name.is_empty()).collect()
}

/// What the registry tells of one extension: VS Code's page of an extension.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MarketDetails {
    pub item: MarketItem,
    /// Who publishes it, as the registry shows the name.
    pub publisher: String,
    /// When its latest version came out (an ISO 8601 time).
    pub released: String,
    pub license: String,
    pub repository: String,
    pub homepage: String,
    pub bugs: String,
    pub categories: Vec<String>,
    pub tags: Vec<String>,
    pub readme: String,
    pub changelog: String,
    pub manifest: String,
    pub license_file: String,
    pub versions: usize,
    /// The size of what is downloaded for this computer, when the registry says.
    pub size: Option<u64>,
    pub preview: bool,
    pub engine: String,
}

/// An answer of `/{namespace}/{name}`.
pub fn parse_details(v: &Value) -> Result<MarketDetails, String> {
    if let Some(error) = v.get("error").and_then(Value::as_str) {
        return Err(error.to_string());
    }
    let item = item(v);
    if item.namespace.is_empty() || item.name.is_empty() {
        return Err("the registry's answer names no extension".into());
    }
    let words = |key: &str| -> Vec<String> {
        let list = v.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        list.iter().filter_map(Value::as_str).filter(|t| !t.starts_with("__")).map(str::to_string).collect()
    };
    let sizes = v.get("downloadSizes");
    let size = [platform().as_str(), "universal"]
        .iter()
        .find_map(|key| sizes.and_then(|s| s.get(*key)).and_then(Value::as_u64))
        .or_else(|| sizes.and_then(Value::as_object).and_then(|all| all.values().find_map(Value::as_u64)));
    let publisher = [v.pointer("/publishedBy/fullName"), v.get("namespaceDisplayName"), v.get("namespace")]
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .find(|name| !name.trim().is_empty())
        .unwrap_or_default()
        .to_string();
    let versions = v
        .get("allVersions")
        .and_then(Value::as_object)
        .map_or(0, |all| all.keys().filter(|k| k.starts_with(|c: char| c.is_ascii_digit())).count());
    Ok(MarketDetails {
        publisher,
        released: text(v, "timestamp"),
        license: text(v, "license"),
        repository: link_of_repository(&text(v, "repository")),
        homepage: text(v, "homepage"),
        bugs: text(v, "bugs"),
        categories: words("categories"),
        tags: words("tags"),
        readme: file(v, "readme"),
        changelog: file(v, "changelog"),
        manifest: file(v, "manifest"),
        license_file: file(v, "license"),
        versions,
        size,
        preview: flag(v, "preview") || flag(v, "preRelease"),
        engine: v.pointer("/engines/vscode").and_then(Value::as_str).unwrap_or_default().to_string(),
        item,
    })
}

pub fn details(namespace: &str, name: &str) -> Result<MarketDetails, String> {
    let url = format!("{}/{}/{}", api(), encode(namespace), encode(name));
    parse_details(&fetch_json(&url)?)
}

/// Where the manifest (package.json) of a version is.
pub fn manifest_url(namespace: &str, name: &str, version: &str) -> String {
    format!("{}/{}/{}/{}/file/package.json", api(), encode(namespace), encode(name), encode(version))
}

/// The extension's page on the registry's site.
pub fn site_url(namespace: &str, name: &str) -> String {
    let base = api();
    let site = base.strip_suffix("/api").unwrap_or(&base);
    format!("{site}/extension/{}/{}", encode(namespace), encode(name))
}

/// When its first version on the registry came out: the oldest of its versions, then that
/// version's time.
pub fn first_released(namespace: &str, name: &str) -> Result<String, String> {
    let base = format!("{}/{}/{}", api(), encode(namespace), encode(name));
    let total = number(&fetch_json(&format!("{base}/versions?size=1"))?, "totalSize");
    if total == 0 {
        return Err("no versions".into());
    }
    let oldest = fetch_json(&format!("{base}/versions?size=1&offset={}", total - 1))?;
    let url = oldest
        .get("versions")
        .and_then(Value::as_object)
        .and_then(|all| all.values().next())
        .and_then(Value::as_str)
        .ok_or("no versions")?
        .to_string();
    let at = text(&fetch_json(&url)?, "timestamp");
    if at.is_empty() {
        return Err("no time".into());
    }
    Ok(at)
}

/// A web address for a manifest's `repository` (`git+https://…/x.git`, `github:a/b`,
/// `git@github.com:a/b.git`), or nothing.
pub fn link_of_repository(raw: &str) -> String {
    let raw = raw.trim();
    let raw = raw.strip_prefix("git+").unwrap_or(raw);
    let raw = raw.strip_suffix(".git").unwrap_or(raw);
    if let Some(path) = raw.strip_prefix("github:").or_else(|| raw.strip_prefix("git@github.com:")) {
        return format!("https://github.com/{path}");
    }
    if let Some(rest) = raw.strip_prefix("git://") {
        return format!("https://{rest}");
    }
    if raw.starts_with("https://") || raw.starts_with("http://") {
        return raw.to_string();
    }
    if !raw.is_empty() && !raw.contains(':') && raw.split('/').count() == 2 {
        return format!("https://github.com/{raw}");
    }
    String::new()
}

/// `2026-09-16T19:24:15Z` as `Sep 16, 2026`.
pub fn short_date(at: &str) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let mut parts = at.get(..10).unwrap_or_default().split('-').map(|p| p.parse::<usize>().ok());
    match (parts.next().flatten(), parts.next().flatten(), parts.next().flatten()) {
        (Some(year), Some(month @ 1..=12), Some(day)) => format!("{} {day}, {year}", MONTHS[month - 1]),
        _ => String::new(),
    }
}

/// A size in bytes as people read it.
pub fn size_words(bytes: u64) -> String {
    match bytes {
        0..=1023 => format!("{bytes} B"),
        1024..=1_048_575 => format!("{:.0} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

pub fn platform() -> String {
    let os = match std::env::consts::OS {
        "windows" => "win32",
        "macos" => "darwin",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "arm" => "armhf",
        other => other,
    };
    format!("{os}-{arch}")
}

const MAX_UNPACKED: u64 = 2 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;

pub struct Wanted<'a> {
    pub namespace: &'a str,
    pub name: &'a str,
    pub version: Option<&'a str>,
    pub sha256: Option<&'a str>,
}

pub fn install(namespace: &str, name: &str, version: Option<&str>, dir: &Path) -> Result<PathBuf, String> {
    install_wanted(&Wanted { namespace, name, version, sha256: None }, dir)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes).as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

pub fn install_wanted(wanted: &Wanted, dir: &Path) -> Result<PathBuf, String> {
    let (namespace, name, version) = (wanted.namespace, wanted.name, wanted.version);
    let which = version.map(encode).unwrap_or_else(|| "latest".to_string());
    let meta = get_json(&format!("{}/{}/{}/{which}", api(), encode(namespace), encode(name)))?;
    if let Some(error) = meta.get("error").and_then(Value::as_str) {
        return Err(error.to_string());
    }
    let downloads = meta.get("downloads");
    let pick = |key: &str| downloads.and_then(|d| d.get(key)).and_then(Value::as_str).map(str::to_string);
    let url = pick(&platform())
        .or_else(|| pick("universal"))
        .or_else(|| pick("web"))
        .or_else(|| meta.pointer("/files/download").and_then(Value::as_str).map(str::to_string))
        .ok_or("the registry lists no download for this extension")?;
    let mut response = ureq::get(&url).call().map_err(|e| e.to_string())?;
    let bytes = response.body_mut().with_config().limit(MAX_VSIX).read_to_vec().map_err(|e| e.to_string())?;
    if let Some(expected) = wanted.sha256 {
        let got = sha256_hex(&bytes);
        if !got.eq_ignore_ascii_case(expected) {
            return Err(format!("what was downloaded is not the file expected (SHA-256 {got}, not {expected})"));
        }
    }
    install_vsix_bytes(&bytes, dir, Some(wanted))
}

pub fn install_vsix(file: &Path, dir: &Path) -> Result<PathBuf, String> {
    let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
    install_vsix_bytes(&bytes, dir, None)
}

fn install_vsix_bytes(bytes: &[u8], dir: &Path, wanted: Option<&Wanted>) -> Result<PathBuf, String> {
    unpack_within(bytes, dir, wanted, (MAX_ENTRIES, MAX_UNPACKED))
}

fn unpack_within(
    bytes: &[u8],
    dir: &Path,
    wanted: Option<&Wanted>,
    (most_entries, most_bytes): (usize, u64),
) -> Result<PathBuf, String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("not a .vsix file: {e}"))?;
    if zip.len() > most_entries {
        return Err(format!("it holds {} files, more than the {most_entries} an extension may", zip.len()));
    }
    let mut declared: u64 = 0;
    for i in 0..zip.len() {
        declared = declared.saturating_add(zip.by_index_raw(i).map_err(|e| e.to_string())?.size());
    }
    if declared > most_bytes {
        return Err(format!(
            "it unpacks to {}, more than the {} an extension may",
            size_words(declared),
            size_words(most_bytes)
        ));
    }
    let manifest: Value = {
        let mut entry = zip.by_name("extension/package.json").map_err(|_| "no extension/package.json inside")?;
        let mut body = String::new();
        entry.read_to_string(&mut body).map_err(|e| e.to_string())?;
        serde_json::from_str(body.trim_start_matches('\u{feff}')).map_err(|e| format!("package.json: {e}"))?
    };
    let (publisher, name, version) =
        (text(&manifest, "publisher"), text(&manifest, "name"), text(&manifest, "version"));
    let safe = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.+".contains(c));
    if !safe(&publisher) || !safe(&name) || !safe(&version) || publisher.starts_with('.') {
        return Err("package.json has no valid publisher, name and version".into());
    }
    if let Some(wanted) = wanted {
        let asked = format!("{}.{}", wanted.namespace, wanted.name);
        if !publisher.eq_ignore_ascii_case(wanted.namespace) || !name.eq_ignore_ascii_case(wanted.name) {
            return Err(format!("the registry sent {publisher}.{name} for {asked}: it was not installed"));
        }
        if wanted.version.is_some_and(|v| v != version) {
            return Err(format!("the registry sent version {version} of {asked}, not the one asked for"));
        }
    }
    let id = format!("{publisher}.{name}").to_lowercase();
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let staging = dir.join(format!(".installing-{id}"));
    let _ = std::fs::remove_dir_all(&staging);

    let unpack = |zip: &mut zip::ZipArchive<Cursor<&[u8]>>| -> Result<(), String> {
        let mut left = most_bytes;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
            let Some(path) = entry.enclosed_name() else { continue };
            let Ok(rel) = path.strip_prefix("extension") else { continue };
            if rel.as_os_str().is_empty() {
                continue;
            }
            let out = staging.join(rel);
            if entry.is_dir() {
                std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
                continue;
            }
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut file = std::fs::File::create(&out).map_err(|e| format!("{}: {e}", out.display()))?;
            let wrote = std::io::copy(&mut (&mut entry).take(left + 1), &mut file).map_err(|e| e.to_string())?;
            if wrote > left {
                return Err(format!("it unpacks to more than the {} an extension may", size_words(most_bytes)));
            }
            left -= wrote;
            #[cfg(unix)]
            if let Some(mode) = entry.unix_mode() {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode));
            }
        }
        Ok(())
    };
    if let Err(e) = unpack(&mut zip) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }

    for old in installed_dirs(dir) {
        if dir_id(&old).as_deref() == Some(id.as_str()) {
            std::fs::remove_dir_all(&old).map_err(|e| format!("cannot replace the installed version: {e}"))?;
        }
    }
    let target = dir.join(format!("{id}-{version}"));
    std::fs::rename(&staging, &target).map_err(|e| e.to_string())?;
    Ok(target)
}

pub fn installed_dirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.join("package.json").is_file() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))
        })
        .collect();
    out.sort();
    out
}

pub fn dir_id(dir: &Path) -> Option<String> {
    let v: Value =
        serde_json::from_str(std::fs::read_to_string(dir.join("package.json")).ok()?.trim_start_matches('\u{feff}'))
            .ok()?;
    let (publisher, name) = (text(&v, "publisher"), text(&v, "name"));
    (!publisher.is_empty() && !name.is_empty()).then(|| format!("{publisher}.{name}").to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn vsix(files: &[(&str, &str)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for (name, body) in files {
            w.start_file(*name, options).unwrap();
            w.write_all(body.as_bytes()).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("code-canvas-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn unpacks_only_the_extension_folder() {
        let dir = temp("vsix");
        let manifest = r#"{"publisher":"Acme","name":"demo","version":"1.2.3"}"#;
        let bytes = vsix(&[
            ("extension.vsixmanifest", "<xml/>"),
            ("extension/package.json", manifest),
            ("extension/out/main.js", "exports.activate = () => {}"),
        ]);
        let target = install_vsix_bytes(&bytes, &dir, None).unwrap();
        assert_eq!(target, dir.join("acme.demo-1.2.3"));
        assert!(target.join("out/main.js").is_file());
        assert!(!target.join("extension.vsixmanifest").exists());
        assert_eq!(dir_id(&target).as_deref(), Some("acme.demo"));

        let newer = vsix(&[("extension/package.json", &manifest.replace("1.2.3", "2.0.0"))]);
        install_vsix_bytes(&newer, &dir, None).unwrap();
        assert_eq!(installed_dirs(&dir), vec![dir.join("acme.demo-2.0.0")]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_bad_archives() {
        let dir = temp("bad");
        assert!(install_vsix_bytes(b"not a zip", &dir, None).is_err());
        assert!(install_vsix_bytes(&vsix(&[("readme.md", "hi")]), &dir, None).is_err());
        let evil = vsix(&[("extension/package.json", r#"{"publisher":"..","name":"x","version":"1"}"#)]);
        assert!(install_vsix_bytes(&evil, &dir, None).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_the_registry_sends_must_be_what_was_asked_for() {
        let dir = temp("asked");
        let manifest = r#"{"publisher":"Acme","name":"demo","version":"1.2.3"}"#;
        let bytes = vsix(&[("extension/package.json", manifest)]);
        let asked = |namespace, name, version| Wanted { namespace, name, version, sha256: None };
        let other = install_vsix_bytes(&bytes, &dir, Some(&asked("acme", "other", None)));
        assert_eq!(other, Err("the registry sent Acme.demo for acme.other: it was not installed".to_string()));
        let imposter = install_vsix_bytes(&bytes, &dir, Some(&asked("evil", "demo", None)));
        assert!(imposter.is_err(), "another publisher");
        let older = install_vsix_bytes(&bytes, &dir, Some(&asked("acme", "demo", Some("1.2.2"))));
        assert!(older.unwrap_err().contains("version 1.2.3"));
        assert!(installed_dirs(&dir).is_empty(), "nothing was put in place");
        let right = install_vsix_bytes(&bytes, &dir, Some(&asked("ACME", "Demo", Some("1.2.3"))));
        assert_eq!(right, Ok(dir.join("acme.demo-1.2.3")), "case does not matter");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_archive_that_unpacks_too_big_or_into_too_many_files_is_refused() {
        let dir = temp("limits");
        let manifest = r#"{"publisher":"acme","name":"big","version":"1.0.0"}"#;
        let zeros = "0".repeat(5000);
        let bytes =
            vsix(&[("extension/package.json", manifest), ("extension/a.txt", &zeros), ("extension/b.txt", &zeros)]);
        assert!(unpack_within(&bytes, &dir, None, (2, 1 << 20)).unwrap_err().contains("3 files"));
        assert!(unpack_within(&bytes, &dir, None, (10, 8000)).unwrap_err().contains("unpacks to"));
        assert!(!dir.join(".installing-acme.big").exists(), "nothing is left half unpacked");
        assert!(unpack_within(&bytes, &dir, None, (10, 20_000)).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_download_is_checked_against_its_sha256() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn query_encoding() {
        assert_eq!(encode("c++ tools/é"), "c%2B%2B%20tools%2F%C3%A9");
    }

    #[test]
    fn search_results_with_icon_and_rating() {
        let v: Value = serde_json::from_str(
            r#"{"extensions": [
                {"namespace": "bruno-api-client", "name": "bruno", "displayName": "Bruno", "version": "6.0.0",
                 "description": "API client", "downloadCount": 82841, "verified": true,
                 "averageRating": 5.0, "reviewCount": 3,
                 "files": {"icon": "https://r/api/bruno-api-client/bruno/6.0.0/file/bruno-icon.png"}},
                {"namespace": "Missio", "name": "missio", "version": "0.7.6", "downloadCount": 2919, "files": {}},
                {"namespace": "", "name": "nameless"}
            ], "totalSize": 3}"#,
        )
        .unwrap();
        let found = parse_search(&v);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].id(), "bruno-api-client.bruno");
        assert_eq!(found[0].icon, "https://r/api/bruno-api-client/bruno/6.0.0/file/bruno-icon.png");
        assert_eq!((found[0].rating, found[0].reviews, found[0].verified), (Some(5.0), 3, true));
        assert_eq!(found[1].display_name, "missio");
        assert_eq!((found[1].icon.as_str(), found[1].rating, found[1].verified), ("", None, false));
    }

    #[test]
    fn details_of_one_extension() {
        let v: Value = serde_json::from_str(
            r#"{"namespace": "bruno-api-client", "name": "bruno", "displayName": "Bruno", "version": "6.0.0",
                "namespaceDisplayName": "bruno-api-client",
                "publishedBy": {"fullName": "Anoop M D", "loginName": "helloanoop"},
                "description": "API client", "downloadCount": 82833, "averageRating": 4.5, "reviewCount": 2,
                "verified": true, "timestamp": "2026-09-16T19:24:15.326253Z", "license": "MIT",
                "repository": "https://github.com/usebruno/bruno-vscode.git", "homepage": "https://www.usebruno.com",
                "bugs": "https://github.com/usebruno/bruno-vscode/issues",
                "categories": ["Testing", "Snippets"], "tags": ["__ext_bru", "REST", "HTTP"],
                "allVersions": {"latest": "x", "6.0.0": "x", "5.0.2": "x"},
                "downloadSizes": {"universal": 13908841}, "engines": {"vscode": "^1.80.0"},
                "files": {"readme": "https://r/readme.md", "changelog": "https://r/changelog.md",
                          "manifest": "https://r/package.json", "license": "https://r/LICENSE.txt",
                          "icon": "https://r/icon.png"}}"#,
        )
        .unwrap();
        let d = parse_details(&v).unwrap();
        assert_eq!(d.item.display_name, "Bruno");
        assert_eq!(d.publisher, "Anoop M D");
        assert_eq!((d.item.rating, d.item.reviews), (Some(4.5), 2));
        assert_eq!(d.repository, "https://github.com/usebruno/bruno-vscode");
        assert_eq!(d.tags, vec!["REST", "HTTP"]);
        assert_eq!(d.categories, vec!["Testing", "Snippets"]);
        assert_eq!((d.versions, d.size), (2, Some(13908841)));
        assert_eq!((d.readme.as_str(), d.manifest.as_str()), ("https://r/readme.md", "https://r/package.json"));
        assert_eq!(short_date(&d.released), "Sep 16, 2026");
        assert_eq!(d.engine, "^1.80.0");
        let missing: Value = serde_json::from_str(r#"{"error": "Extension not found: a.b"}"#).unwrap();
        assert_eq!(parse_details(&missing), Err("Extension not found: a.b".to_string()));
    }

    #[test]
    fn repository_links_and_words() {
        assert_eq!(link_of_repository("git+https://github.com/a/b.git"), "https://github.com/a/b");
        assert_eq!(link_of_repository("github:a/b"), "https://github.com/a/b");
        assert_eq!(link_of_repository("git@github.com:a/b.git"), "https://github.com/a/b");
        assert_eq!(link_of_repository("a/b"), "https://github.com/a/b");
        assert_eq!(link_of_repository("not a link"), "");
        assert_eq!(short_date("2025-01-03T00:00:00Z"), "Jan 3, 2025");
        assert_eq!(short_date("soon"), "");
        assert_eq!(size_words(13908841), "13.3 MB");
    }
}
