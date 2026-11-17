use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use serde_json::Value;

const API: &str = "https://open-vsx.org/api";
const MAX_VSIX: u64 = 400 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct MarketItem {
    pub namespace: String,
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub version: String,
    pub downloads: u64,
    pub verified: bool,
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
    let url = format!("{API}/-/search?query={}&size=40&sortBy={sort}&sortOrder=desc", encode(query.trim()));
    let v = get_json(&url)?;
    let list = v.get("extensions").and_then(Value::as_array).cloned().unwrap_or_default();
    Ok(list
        .iter()
        .map(|e| MarketItem {
            namespace: text(e, "namespace"),
            name: text(e, "name"),
            display_name: Some(text(e, "displayName")).filter(|s| !s.is_empty()).unwrap_or_else(|| text(e, "name")),
            description: text(e, "description"),
            version: text(e, "version"),
            downloads: e.get("downloadCount").and_then(Value::as_u64).unwrap_or(0),
            verified: e.get("verified").and_then(Value::as_bool).unwrap_or(false),
        })
        .filter(|e| !e.namespace.is_empty() && !e.name.is_empty())
        .collect())
}

fn platform() -> String {
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

pub fn install(namespace: &str, name: &str, dir: &Path) -> Result<PathBuf, String> {
    let meta = get_json(&format!("{API}/{}/{}/latest", encode(namespace), encode(name)))?;
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
    install_vsix_bytes(&bytes, dir)
}

pub fn install_vsix(file: &Path, dir: &Path) -> Result<PathBuf, String> {
    let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
    install_vsix_bytes(&bytes, dir)
}

fn install_vsix_bytes(bytes: &[u8], dir: &Path) -> Result<PathBuf, String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("not a .vsix file: {e}"))?;
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
    let id = format!("{publisher}.{name}").to_lowercase();
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let staging = dir.join(format!(".installing-{id}"));
    let _ = std::fs::remove_dir_all(&staging);

    let unpack = |zip: &mut zip::ZipArchive<Cursor<&[u8]>>| -> Result<(), String> {
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
            std::io::copy(&mut entry, &mut file).map_err(|e| e.to_string())?;
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
        let target = install_vsix_bytes(&bytes, &dir).unwrap();
        assert_eq!(target, dir.join("acme.demo-1.2.3"));
        assert!(target.join("out/main.js").is_file());
        assert!(!target.join("extension.vsixmanifest").exists());
        assert_eq!(dir_id(&target).as_deref(), Some("acme.demo"));

        let newer = vsix(&[("extension/package.json", &manifest.replace("1.2.3", "2.0.0"))]);
        install_vsix_bytes(&newer, &dir).unwrap();
        assert_eq!(installed_dirs(&dir), vec![dir.join("acme.demo-2.0.0")]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_bad_archives() {
        let dir = temp("bad");
        assert!(install_vsix_bytes(b"not a zip", &dir).is_err());
        assert!(install_vsix_bytes(&vsix(&[("readme.md", "hi")]), &dir).is_err());
        let evil = vsix(&[("extension/package.json", r#"{"publisher":"..","name":"x","version":"1"}"#)]);
        assert!(install_vsix_bytes(&evil, &dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn query_encoding() {
        assert_eq!(encode("c++ tools/é"), "c%2B%2B%20tools%2F%C3%A9");
    }
}
