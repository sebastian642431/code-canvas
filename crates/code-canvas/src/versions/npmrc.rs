use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::Source;

pub const NPM: &str = "https://registry.npmjs.org/";

#[derive(Clone, PartialEq, Eq)]
pub struct Secret(Arc<str>);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(hidden)")
    }
}

impl Secret {
    pub fn header(&self) -> &str {
        &self.0
    }
}

#[derive(Default)]
pub struct NpmConfig {
    registry: Option<String>,
    scopes: HashMap<String, String>,
    auth: Vec<(String, Secret)>,
}

fn expand(value: &str, env: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let mut out = String::new();
    let mut rest = value;
    while let Some(at) = rest.find("${") {
        out.push_str(&rest[..at]);
        let end = rest[at..].find('}')? + at;
        let name = rest[at + 2..end].trim_end_matches('?');
        out.push_str(&env(name).filter(|v| !v.is_empty())?);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

fn with_slash(url: &str) -> String {
    let url = url.trim();
    if url.ends_with('/') {
        url.to_string()
    } else {
        format!("{url}/")
    }
}

fn nerf(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    with_slash(&format!("//{rest}"))
}

impl NpmConfig {
    pub fn read(dir: &Path, root: &Path) -> NpmConfig {
        let mut config = NpmConfig::default();
        let env = |name: &str| std::env::var(name).ok();
        if let Some(user) = user_file() {
            if let Ok(text) = std::fs::read_to_string(user) {
                config.take(&text, &env);
            }
        }
        let mut folders: Vec<&Path> = dir.ancestors().take_while(|d| d.starts_with(root)).collect();
        folders.reverse();
        for folder in folders {
            if let Ok(text) = std::fs::read_to_string(folder.join(".npmrc")) {
                config.take(&text, &env);
            }
        }
        config
    }

    pub fn take(&mut self, text: &str, env: &dyn Fn(&str) -> Option<String>) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(['#', ';']) {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            let (key, value) = (key.trim(), value.trim().trim_matches(['"', '\'']));
            let Some(value) = expand(value, env) else {
                if let Some(at) = key.strip_suffix(":_authToken").or_else(|| key.strip_suffix(":_auth")) {
                    let at = with_slash(at);
                    self.auth.retain(|(prefix, _)| *prefix != at);
                }
                continue;
            };
            if key == "registry" {
                self.registry = Some(with_slash(&value));
            } else if let Some(scope) = key.strip_suffix(":registry").filter(|s| s.starts_with('@')) {
                self.scopes.insert(scope.to_string(), with_slash(&value));
            } else if key.starts_with("//") {
                let header = match (key.strip_suffix(":_authToken"), key.strip_suffix(":_auth")) {
                    (Some(at), _) => Some((at, format!("Bearer {value}"))),
                    (_, Some(at)) => Some((at, format!("Basic {value}"))),
                    _ => None,
                };
                if let Some((at, header)) = header {
                    let at = with_slash(at);
                    self.auth.retain(|(prefix, _)| *prefix != at);
                    self.auth.push((at, Secret(header.into())));
                }
            }
        }
    }

    fn secret_for(&self, registry: &str) -> Option<Secret> {
        let target = nerf(registry);
        let fits = self.auth.iter().filter(|(prefix, _)| target.starts_with(prefix.as_str()));
        fits.max_by_key(|(prefix, _)| prefix.len()).map(|(_, secret)| secret.clone())
    }

    pub fn source(&self, name: &str) -> Source {
        let scope = name.strip_prefix('@').and_then(|rest| rest.split_once('/')).map(|(scope, _)| format!("@{scope}"));
        let scoped = scope.and_then(|scope| self.scopes.get(&scope)).cloned();
        let from_scope = scoped.is_some();
        let base = scoped.or_else(|| self.registry.clone()).unwrap_or_else(|| NPM.to_string());
        match (self.secret_for(&base), from_scope) {
            (Some(secret), _) => Source::Private(base, secret),
            (None, true) => Source::NoToken(base),
            (None, false) if base == NPM => Source::Public,
            (None, false) => Source::Mirror(base),
        }
    }
}

fn user_file() -> Option<PathBuf> {
    for name in ["NPM_CONFIG_USERCONFIG", "npm_config_userconfig"] {
        if let Some(path) = std::env::var_os(name).filter(|p| !p.is_empty()) {
            return Some(PathBuf::from(path));
        }
    }
    dirs::home_dir().map(|home| home.join(".npmrc"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(text: &str, env: &[(&str, &str)]) -> NpmConfig {
        let env: HashMap<String, String> = env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let mut config = NpmConfig::default();
        config.take(text, &|name| env.get(name).cloned());
        config
    }

    #[test]
    fn a_scope_on_github_with_a_token_from_the_environment() {
        let text =
            "@latin-securities:registry=https://npm.pkg.github.com\n//npm.pkg.github.com/:_authToken=${GITHUB_TOKEN}\n";
        let with = config(text, &[("GITHUB_TOKEN", "ghp_secret")]);
        match with.source("@latin-securities/widgets") {
            Source::Private(base, secret) => {
                assert_eq!(base, "https://npm.pkg.github.com/");
                assert_eq!(secret.header(), "Bearer ghp_secret");
                assert!(!format!("{secret:?}").contains("ghp_secret"));
            }
            other => panic!("expected a private source, got {other:?}"),
        }
        assert_eq!(with.source("react"), Source::Public);
        assert_eq!(with.source("@other/thing"), Source::Public);
        let without = config(text, &[]);
        assert_eq!(without.source("@latin-securities/widgets"), Source::NoToken("https://npm.pkg.github.com/".into()));
        assert_eq!(
            config(text, &[("GITHUB_TOKEN", "")]).source("@latin-securities/x"),
            Source::NoToken("https://npm.pkg.github.com/".into())
        );
    }

    #[test]
    fn a_mirror_and_the_longest_matching_token() {
        let text = "registry=https://mirror.example.com/npm\n; comment\n//mirror.example.com/:_authToken=short\n//mirror.example.com/npm/:_authToken=long\n@corp:registry=https://corp.example.com/\n//corp.example.com/:_auth=dXNlcjpwYXNz\n";
        let c = config(text, &[]);
        match c.source("left-pad") {
            Source::Private(base, secret) => {
                assert_eq!(base, "https://mirror.example.com/npm/");
                assert_eq!(secret.header(), "Bearer long");
            }
            other => panic!("{other:?}"),
        }
        match c.source("@corp/lib") {
            Source::Private(_, secret) => assert_eq!(secret.header(), "Basic dXNlcjpwYXNz"),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            config("registry=https://mirror.example.com/\n", &[]).source("x"),
            Source::Mirror("https://mirror.example.com/".into())
        );
    }

    #[test]
    fn a_later_file_wins() {
        let mut c = config("@a:registry=https://one.example.com/\n", &[]);
        c.take("@a:registry=https://two.example.com/\n", &|_| None);
        assert_eq!(c.source("@a/b"), Source::NoToken("https://two.example.com/".into()));
    }
}
