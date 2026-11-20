use std::time::Duration;

use serde_json::Value;

use super::npmrc::{Secret, NPM};
use super::{Facts, Heard, Registry};

const AGENT: &str = "hawk-canvas (https://github.com/sebastian642431/code-canvas)";
const MOST: u64 = 64 * 1024 * 1024;

pub fn reach(url: &str) -> String {
    moved(url, std::env::var("CODE_CANVAS_REGISTRY").ok())
}

fn moved(url: &str, base: Option<String>) -> String {
    match base.filter(|base| !base.trim().is_empty()) {
        Some(base) => {
            let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
            format!("{}/{rest}", base.trim().trim_end_matches('/'))
        }
        None => url.to_string(),
    }
}

enum Failure {
    NotFound,
    Refused,
    Other(String),
}

fn get(url: &str, auth: Option<&Secret>, accept: &str) -> Result<String, Failure> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut request = agent.get(&reach(url)).header("User-Agent", AGENT).header("Accept", accept);
    if let Some(secret) = auth {
        request = request.header("Authorization", secret.header());
    }
    let mut response = request.call().map_err(|e| Failure::Other(e.to_string()))?;
    match response.status().as_u16() {
        200..=299 => {
            response.body_mut().with_config().limit(MOST).read_to_string().map_err(|e| Failure::Other(e.to_string()))
        }
        404 | 410 => Err(Failure::NotFound),
        401 | 403 => Err(Failure::Refused),
        status => Err(Failure::Other(format!("the registry answered {status}"))),
    }
}

pub fn crates_index_path(name: &str) -> String {
    let name = name.to_lowercase();
    match name.len() {
        1 => format!("1/{name}"),
        2 => format!("2/{name}"),
        3 => format!("3/{}/{name}", &name[..1]),
        _ => format!("{}/{}/{name}", &name[..2], &name[2..4]),
    }
}

pub fn python_name(name: &str) -> String {
    let mut out = String::new();
    for c in name.to_lowercase().chars() {
        if "-_.".contains(c) {
            if !out.ends_with('-') {
                out.push('-');
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn go_path(module: &str) -> String {
    let mut out = String::new();
    for c in module.chars() {
        if c.is_ascii_uppercase() {
            out.push('!');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

pub fn url(registry: Registry, base: Option<&str>, name: &str) -> String {
    match registry {
        Registry::Npm => format!("{}{}", base.unwrap_or(NPM), name.replace('/', "%2f")),
        Registry::Crates => format!("https://index.crates.io/{}", crates_index_path(name)),
        Registry::PyPI => format!("https://pypi.org/pypi/{}/json", python_name(name)),
        Registry::NuGet => format!("https://api.nuget.org/v3-flatcontainer/{}/index.json", name.to_lowercase()),
        Registry::Go => format!("https://proxy.golang.org/{}/@v/list", go_path(name)),
        Registry::Packagist => format!("https://repo.packagist.org/p2/{}.json", name.to_lowercase()),
        Registry::Pub => format!("https://pub.dev/api/packages/{name}"),
    }
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map_or_else(Vec::new, |items| items.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
}

pub fn npm_facts(body: &str) -> Option<Facts> {
    let json: Value = serde_json::from_str(body).ok()?;
    let versions = json.get("versions")?.as_object()?.keys().cloned();
    let latest = json.pointer("/dist-tags/latest").and_then(Value::as_str).map(str::to_string);
    Some(Facts::new(versions, latest))
}

pub fn crates_facts(body: &str) -> Option<Facts> {
    let versions = body
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|v| v.get("yanked").and_then(Value::as_bool) != Some(true))
        .filter_map(|v| v.get("vers").and_then(Value::as_str).map(str::to_string));
    Some(Facts::new(versions, None))
}

pub fn pypi_facts(body: &str) -> Option<Facts> {
    let json: Value = serde_json::from_str(body).ok()?;
    let releases = json.get("releases")?.as_object()?;
    let kept = releases.iter().filter(|(_, files)| {
        let files = files.as_array().map(Vec::as_slice).unwrap_or_default();
        files.is_empty() || !files.iter().all(|f| f.get("yanked").and_then(Value::as_bool) == Some(true))
    });
    Some(Facts::new(kept.map(|(version, _)| version.clone()), None))
}

pub fn nuget_facts(body: &str) -> Option<Facts> {
    let json: Value = serde_json::from_str(body).ok()?;
    Some(Facts::new(strings(json.get("versions")).into_iter(), None))
}

pub fn go_facts(list: &str, latest: Option<String>) -> Facts {
    let versions = list.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string);
    Facts::new(versions.chain(latest), None)
}

pub fn packagist_facts(body: &str, name: &str) -> Option<Facts> {
    let json: Value = serde_json::from_str(body).ok()?;
    let packages = json.get("packages")?.as_object()?;
    let entries = packages.get(name).or_else(|| packages.get(&name.to_lowercase()))?.as_array()?;
    let versions = entries
        .iter()
        .filter_map(|e| e.get("version").and_then(Value::as_str))
        .filter(|v| !v.starts_with("dev-") && !v.ends_with("-dev"))
        .map(str::to_string);
    Some(Facts::new(versions, None))
}

pub fn pub_facts(body: &str) -> Option<Facts> {
    let json: Value = serde_json::from_str(body).ok()?;
    let versions = json
        .get("versions")?
        .as_array()?
        .iter()
        .filter(|v| v.get("retracted").and_then(Value::as_bool) != Some(true))
        .filter_map(|v| v.get("version").and_then(Value::as_str).map(str::to_string));
    let latest = json.pointer("/latest/version").and_then(Value::as_str).map(str::to_string);
    Some(Facts::new(versions, latest))
}

const JSON: &str = "application/json";
const NPM_SHORT: &str = "application/vnd.npm.install-v1+json; q=1.0, application/json; q=0.8, */*";

pub fn fetch(registry: Registry, base: Option<&str>, name: &str, auth: Option<&Secret>) -> Heard {
    let address = url(registry, base, name);
    let accept = if registry == Registry::Npm { NPM_SHORT } else { JSON };
    let body = match get(&address, auth, accept) {
        Ok(body) => body,
        Err(Failure::NotFound) => return Heard::NotFound,
        Err(Failure::Refused) => return Heard::Refused,
        Err(Failure::Other(why)) => {
            log::debug!("versions: {registry:?} {name}: {why}");
            return Heard::Failed;
        }
    };
    let facts = match registry {
        Registry::Npm => npm_facts(&body),
        Registry::Crates => crates_facts(&body),
        Registry::PyPI => pypi_facts(&body),
        Registry::NuGet => nuget_facts(&body),
        Registry::Packagist => packagist_facts(&body, name),
        Registry::Pub => pub_facts(&body),
        Registry::Go => {
            let latest = match body.trim().is_empty() {
                true => {
                    let at = format!("https://proxy.golang.org/{}/@latest", go_path(name));
                    get(&at, None, JSON).ok().and_then(|b| {
                        serde_json::from_str::<Value>(&b).ok()?.get("Version")?.as_str().map(str::to_string)
                    })
                }
                false => None,
            };
            Some(go_facts(&body, latest))
        }
    };
    match facts {
        Some(facts) => Heard::Facts(facts),
        None => {
            log::debug!("versions: {registry:?} {name}: the answer could not be read");
            Heard::Failed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(facts: &Facts) -> (Vec<&str>, Option<&str>) {
        (facts.releases.iter().map(|r| r.text.as_str()).collect(), facts.latest.as_ref().map(|r| r.text.as_str()))
    }

    #[test]
    fn addresses() {
        assert_eq!(url(Registry::Npm, None, "@types/node"), "https://registry.npmjs.org/@types%2fnode");
        assert_eq!(
            url(Registry::Npm, Some("https://npm.pkg.github.com/"), "@latin-securities/widgets"),
            "https://npm.pkg.github.com/@latin-securities%2fwidgets"
        );
        assert_eq!(url(Registry::Crates, None, "Serde"), "https://index.crates.io/se/rd/serde");
        assert_eq!(crates_index_path("a"), "1/a");
        assert_eq!(crates_index_path("io"), "2/io");
        assert_eq!(crates_index_path("syn"), "3/s/syn");
        assert_eq!(url(Registry::PyPI, None, "Typing_Extensions"), "https://pypi.org/pypi/typing-extensions/json");
        assert_eq!(
            url(Registry::NuGet, None, "Newtonsoft.Json"),
            "https://api.nuget.org/v3-flatcontainer/newtonsoft.json/index.json"
        );
        assert_eq!(
            url(Registry::Go, None, "github.com/BurntSushi/toml"),
            "https://proxy.golang.org/github.com/!burnt!sushi/toml/@v/list"
        );
        assert_eq!(
            url(Registry::Packagist, None, "Monolog/Monolog"),
            "https://repo.packagist.org/p2/monolog/monolog.json"
        );
        assert_eq!(url(Registry::Pub, None, "http"), "https://pub.dev/api/packages/http");
    }

    #[test]
    fn npm_answers() {
        let body = r#"{"name":"demo","dist-tags":{"latest":"1.4.0","next":"2.0.0-rc.1"},"versions":{"1.0.0":{},"1.4.0":{},"1.10.0-beta.1":{},"2.0.0-rc.1":{},"0.9.0":{}}}"#;
        let facts = npm_facts(body).unwrap();
        assert_eq!(texts(&facts), (vec!["0.9.0", "1.0.0", "1.4.0", "1.10.0-beta.1", "2.0.0-rc.1"], Some("1.4.0")));
        assert!(npm_facts("not json").is_none());
    }

    #[test]
    fn crates_index_answers() {
        let body = r#"{"name":"demo","vers":"0.9.0","yanked":false}
{"name":"demo","vers":"1.0.0","yanked":false}
{"name":"demo","vers":"1.10.0","yanked":false}
{"name":"demo","vers":"1.9.3","yanked":false}
{"name":"demo","vers":"2.0.0","yanked":true}
{"name":"demo","vers":"2.0.0-rc.1","yanked":false}
"#;
        let facts = crates_facts(body).unwrap();
        assert_eq!(texts(&facts), (vec!["0.9.0", "1.0.0", "1.9.3", "1.10.0", "2.0.0-rc.1"], Some("1.10.0")));
        assert_eq!(texts(&crates_facts("").unwrap()), (vec![], None));
    }

    #[test]
    fn pypi_answers() {
        let body = r#"{"info":{"version":"2.31.0"},"releases":{"2.28.1":[{"yanked":false}],"2.31.0":[{"yanked":false}],"2.32.0":[{"yanked":true},{"yanked":true}],"3.0.0b1":[{"yanked":false}],"1.0":[]}}"#;
        let facts = pypi_facts(body).unwrap();
        assert_eq!(texts(&facts), (vec!["1.0", "2.28.1", "2.31.0", "3.0.0b1"], Some("2.31.0")));
    }

    #[test]
    fn nuget_go_packagist_and_pub_answers() {
        let nuget = nuget_facts(r#"{"versions":["12.0.3","13.0.1","13.0.3","14.0.1-beta1"]}"#).unwrap();
        assert_eq!(texts(&nuget).1, Some("13.0.3"));
        let go = go_facts("v1.6.0\nv1.7.0\nv2.0.0-rc.1\n", None);
        assert_eq!(texts(&go), (vec!["v1.6.0", "v1.7.0", "v2.0.0-rc.1"], Some("v1.7.0")));
        let pseudo = go_facts("", Some("v0.0.0-20230101000000-abcdef123456".into()));
        assert_eq!(texts(&pseudo).1, Some("v0.0.0-20230101000000-abcdef123456"));
        let body = r#"{"packages":{"monolog/monolog":[{"version":"3.5.0"},{"version":"v3.4.0"},{"version":"dev-main"},{"version":"4.0.0-RC1"}]}}"#;
        let packagist = packagist_facts(body, "monolog/monolog").unwrap();
        assert_eq!(texts(&packagist), (vec!["v3.4.0", "3.5.0", "4.0.0-RC1"], Some("3.5.0")));
        let body = r#"{"name":"http","latest":{"version":"1.1.2"},"versions":[{"version":"1.1.0"},{"version":"1.1.2"},{"version":"1.1.1","retracted":true},{"version":"1.2.0-beta"}]}"#;
        assert_eq!(texts(&pub_facts(body).unwrap()), (vec!["1.1.0", "1.1.2", "1.2.0-beta"], Some("1.1.2")));
    }

    #[test]
    fn the_registry_can_be_moved_for_tests() {
        assert_eq!(moved("https://registry.npmjs.org/react", None), "https://registry.npmjs.org/react");
        assert_eq!(
            moved("https://npm.pkg.github.com/@a%2fb", Some("http://127.0.0.1:9000/".into())),
            "http://127.0.0.1:9000/npm.pkg.github.com/@a%2fb"
        );
    }
}
