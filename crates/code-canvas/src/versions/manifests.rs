use super::range::Flavor;
use super::{Dependency, Registry, Source};

pub fn is_manifest(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "package.json" | "cargo.toml" | "pyproject.toml" | "go.mod" | "composer.json" | "pubspec.yaml"
    ) || lower == "directory.packages.props"
        || (lower.starts_with("requirements") && lower.ends_with(".txt"))
        || [".csproj", ".fsproj", ".vbproj"].iter().any(|end| lower.ends_with(end))
}

pub fn dependencies(file_name: &str, text: &str) -> Vec<Dependency> {
    let lower = file_name.to_ascii_lowercase();
    match lower.as_str() {
        "package.json" => package_json(text),
        "composer.json" => composer_json(text),
        "cargo.toml" => toml_tables(text, Registry::Crates, Flavor::Cargo, cargo_table, &[]),
        "pyproject.toml" => {
            let mut out = toml_tables(text, Registry::PyPI, Flavor::Exact, poetry_table, &["python"]);
            out.extend(pep_621(text));
            out.sort_by_key(|d| (d.line, d.col));
            out
        }
        "go.mod" => go_mod(text),
        "pubspec.yaml" => pubspec(text),
        _ if lower.starts_with("requirements") && lower.ends_with(".txt") => requirements_txt(text),
        _ if is_manifest(&lower) => msbuild(text),
        _ => Vec::new(),
    }
}

fn dep(line: usize, col: usize, name: &str, wanted: &str, registry: Registry, flavor: Flavor) -> Dependency {
    Dependency {
        line: line as u32,
        col: col as u32,
        name: name.to_string(),
        wanted: wanted.to_string(),
        registry,
        flavor,
        source: Source::Public,
    }
}

fn chars(line: &str, byte: usize) -> usize {
    line[..byte].chars().count()
}

fn quoted(line: &str, from: usize) -> Option<(usize, &str)> {
    let rest = line.get(from..)?;
    let open = rest.find(['"', '\''])?;
    let quote = rest[open..].chars().next()?;
    let start = from + open + 1;
    let len = line[start..].find(quote)?;
    Some((start, &line[start..start + len]))
}

fn json_pair(line: &str) -> Option<(&str, usize, &str)> {
    let open = line.find('"')?;
    let name_len = line[open + 1..].find('"')?;
    let name = &line[open + 1..open + 1 + name_len];
    let after = open + 2 + name_len;
    let colon = after + line[after..].find(|c: char| !c.is_whitespace())?;
    if !line[colon..].starts_with(':') {
        return None;
    }
    let value = colon + 1 + line[colon + 1..].find(|c: char| !c.is_whitespace())?;
    if !line[value..].starts_with('"') {
        return None;
    }
    let len = line[value + 1..].find('"')?;
    Some((name, value + 1, &line[value + 1..value + 1 + len]))
}

fn json_sections(
    text: &str,
    sections: &[&str],
    registry: Registry,
    flavor: Flavor,
    keep: impl Fn(&str, &str) -> bool,
) -> Vec<Dependency> {
    let mut out = Vec::new();
    let mut inside = false;
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if !inside {
            let opens = trimmed.strip_prefix('"').and_then(|r| r.split_once('"'));
            inside = opens.is_some_and(|(name, rest)| sections.contains(&name) && rest.trim_end().ends_with('{'));
            continue;
        }
        if trimmed.starts_with('}') {
            inside = false;
            continue;
        }
        let Some((name, at, wanted)) = json_pair(line) else { continue };
        if keep(name, wanted) {
            out.push(dep(i, chars(line, at), name, wanted, registry, flavor));
        }
    }
    out
}

fn package_json(text: &str) -> Vec<Dependency> {
    const SECTIONS: [&str; 4] = ["dependencies", "devDependencies", "peerDependencies", "optionalDependencies"];
    json_sections(text, &SECTIONS, Registry::Npm, Flavor::Npm, |_, wanted| {
        let from_registry = wanted.chars().next().is_some_and(|c| c.is_ascii_digit() || "^~>=<v*xX".contains(c));
        from_registry && !wanted.contains(':') && !wanted.contains('/')
    })
}

fn composer_json(text: &str) -> Vec<Dependency> {
    json_sections(text, &["require", "require-dev"], Registry::Packagist, Flavor::Composer, |name, wanted| {
        name.contains('/') && !wanted.is_empty() && !wanted.starts_with("dev-") && !wanted.contains("as ")
    })
}

fn inline_value(line: &str, from: usize, key: &str) -> Option<(usize, String)> {
    let mut at = from;
    while let Some(found) = line.get(at..).and_then(|rest| rest.find(key)) {
        let start = at + found;
        let before_ok =
            line[..start].chars().next_back().is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '-'));
        let after = &line[start + key.len()..];
        if before_ok && after.trim_start().starts_with('=') {
            let eq = start + key.len() + after.find('=')?;
            let value = eq + 1 + line[eq + 1..].find(|c: char| !c.is_whitespace())?;
            if line[value..].starts_with(['"', '\'']) {
                return quoted(line, value).map(|(s, v)| (s, v.to_string()));
            }
            return None;
        }
        at = start + key.len();
    }
    None
}

enum Table {
    Listing,
    Single(String),
    Other,
}

fn cargo_table(header: &str) -> Table {
    if header.ends_with("dependencies") {
        return Table::Listing;
    }
    match header.rsplit_once("dependencies.") {
        Some((_, name)) => Table::Single(name.trim_matches(['"', '\'']).to_string()),
        None => Table::Other,
    }
}

fn poetry_table(header: &str) -> Table {
    let group = header.strip_prefix("tool.poetry.group.").is_some_and(|rest| rest.ends_with(".dependencies"));
    if header == "tool.poetry.dependencies" || header == "tool.poetry.dev-dependencies" || group {
        return Table::Listing;
    }
    match header.strip_prefix("tool.poetry.").and_then(|rest| rest.rsplit_once("dependencies.")) {
        Some((_, name)) => Table::Single(name.trim_matches(['"', '\'']).to_string()),
        None => Table::Other,
    }
}

fn toml_tables(
    text: &str,
    registry: Registry,
    flavor: Flavor,
    table: fn(&str) -> Table,
    skip: &[&str],
) -> Vec<Dependency> {
    let mut out = Vec::new();
    let mut current = Table::Other;
    for (i, line) in text.lines().enumerate() {
        let code = line.split('#').next().unwrap_or("");
        let trimmed = code.trim();
        if let Some(header) = trimmed.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
            current = table(header.trim_matches(|c| c == '[' || c == ']').trim());
            continue;
        }
        let Some(eq) = code.find('=') else { continue };
        let key = code[..eq].trim().trim_matches(['"', '\'']);
        let value_at = eq + 1;
        let found = match &current {
            Table::Listing if !key.contains('.') && !skip.contains(&key) => {
                let value = code[value_at..].trim_start();
                if value.starts_with(['"', '\'']) {
                    quoted(code, value_at).map(|(at, v)| (key.to_string(), at, v.to_string()))
                } else if value.starts_with('{') {
                    let elsewhere = ["path", "git", "registry", "url", "workspace"].iter().any(|k| {
                        code[value_at..].contains(&format!("{k} =")) || code[value_at..].contains(&format!("{k}="))
                    });
                    let name = inline_value(code, value_at, "package").map_or(key.to_string(), |(_, n)| n);
                    let version = inline_value(code, value_at, "version");
                    version.filter(|_| !elsewhere).map(|(at, v)| (name, at, v))
                } else {
                    None
                }
            }
            Table::Single(name) if key == "version" && !skip.contains(&name.as_str()) => {
                quoted(code, value_at).map(|(at, v)| (name.clone(), at, v.to_string()))
            }
            _ => None,
        };
        if let Some((name, at, wanted)) = found {
            out.push(dep(i, chars(line, at), &name, &wanted, registry, flavor));
        }
    }
    out
}

fn pep_508(text: &str) -> Option<(&str, usize, &str)> {
    let start = text.len() - text.trim_start().len();
    let rest = &text[start..];
    let name_len = rest.find(|c: char| !(c.is_ascii_alphanumeric() || "._-".contains(c))).unwrap_or(rest.len());
    if name_len == 0 {
        return None;
    }
    let name = &rest[..name_len];
    let mut at = start + name_len;
    let skip_space = |at: usize| at + text[at..].len() - text[at..].trim_start().len();
    at = skip_space(at);
    if text[at..].starts_with('[') {
        at += text[at..].find(']')? + 1;
        at = skip_space(at);
    }
    if text[at..].starts_with('(') {
        at = skip_space(at + 1);
    }
    let end = at + text[at..].find([';', '#', ')']).unwrap_or(text.len() - at);
    let spec = text[at..end].trim_end().trim_end_matches('\\').trim_end();
    let first = spec.chars().next()?;
    "<>=~!".contains(first).then_some((name, at, spec))
}

fn requirements_txt(text: &str) -> Vec<Dependency> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with(['#', '-']) || trimmed.contains("://") {
            continue;
        }
        if let Some((name, at, spec)) = pep_508(line) {
            out.push(dep(i, chars(line, at), name, spec, Registry::PyPI, Flavor::Exact));
        }
    }
    out
}

fn unquoted(code: &str) -> String {
    let mut out = String::new();
    let mut quote: Option<char> = None;
    for c in code.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None => out.push(c),
        }
    }
    out
}

fn pep_621(text: &str) -> Vec<Dependency> {
    let mut out = Vec::new();
    let mut header = String::new();
    let mut depth = 0i64;
    for (i, line) in text.lines().enumerate() {
        let code = line.split(" #").next().unwrap_or("");
        let mut from = 0;
        if depth <= 0 {
            depth = 0;
            let trimmed = code.trim();
            if let Some(h) = trimmed.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
                header = h.trim().to_string();
                continue;
            }
            let Some(eq) = code.find('=') else { continue };
            let key = code[..eq].trim().trim_matches(['"', '\'']);
            let wanted = match header.as_str() {
                "project" => key == "dependencies",
                "project.optional-dependencies" | "dependency-groups" => true,
                _ => false,
            };
            if !wanted || !code[eq + 1..].trim_start().starts_with('[') {
                continue;
            }
            from = eq + 1;
        }
        let mut at_from = from;
        while let Some((at, item)) = quoted(code, at_from) {
            if let Some((name, spec_at, spec)) = pep_508(item) {
                out.push(dep(i, chars(line, at + spec_at), name, spec, Registry::PyPI, Flavor::Exact));
            }
            at_from = at + item.len() + 1;
        }
        let bare = unquoted(&code[from..]);
        depth += bare.matches('[').count() as i64 - bare.matches(']').count() as i64;
    }
    out
}

fn attr<'a>(line: &'a str, name: &str) -> Option<(usize, &'a str)> {
    let mut from = 0;
    while let Some(found) = line[from..].find(name) {
        let start = from + found;
        let before_ok = line[..start].chars().next_back().is_some_and(char::is_whitespace);
        let after = line[start + name.len()..].trim_start();
        if before_ok && after.starts_with('=') {
            let eq = start + name.len() + line[start + name.len()..].find('=')?;
            return quoted(line, eq + 1);
        }
        from = start + name.len();
    }
    None
}

fn msbuild(text: &str) -> Vec<Dependency> {
    let mut out = Vec::new();
    let mut pending: Option<String> = None;
    for (i, line) in text.lines().enumerate() {
        let element =
            ["<PackageReference", "<PackageVersion", "<GlobalPackageReference"].iter().any(|e| line.contains(e));
        if element {
            let Some((_, name)) = attr(line, "Include").or_else(|| attr(line, "Update")) else { continue };
            match attr(line, "Version").or_else(|| attr(line, "VersionOverride")) {
                Some((at, version)) if !version.contains('$') && !version.is_empty() => {
                    out.push(dep(i, chars(line, at), name, version, Registry::NuGet, Flavor::Exact));
                }
                Some(_) => {}
                None if !line.trim_end().ends_with("/>") => pending = Some(name.to_string()),
                None => {}
            }
            continue;
        }
        if let (Some(name), Some(open)) = (pending.as_ref(), line.find("<Version>")) {
            let at = open + "<Version>".len();
            if let Some(len) = line[at..].find("</Version>") {
                let version = &line[at..at + len];
                if !version.contains('$') {
                    out.push(dep(i, chars(line, at), name, version.trim(), Registry::NuGet, Flavor::Exact));
                }
            }
            pending = None;
        } else if line.contains("</PackageReference>") || line.contains("</PackageVersion>") {
            pending = None;
        }
    }
    out
}

fn go_mod(text: &str) -> Vec<Dependency> {
    let mut out = Vec::new();
    let mut block = false;
    for (i, line) in text.lines().enumerate() {
        let code = line.split("//").next().unwrap_or("");
        let trimmed = code.trim();
        let entry = if block {
            if trimmed.starts_with(')') {
                block = false;
                continue;
            }
            trimmed
        } else if let Some(rest) = trimmed.strip_prefix("require") {
            let rest = rest.trim();
            if rest.starts_with('(') {
                block = true;
                continue;
            }
            rest
        } else {
            continue;
        };
        let mut words = entry.split_whitespace();
        let (Some(module), Some(version)) = (words.next(), words.next()) else { continue };
        if !version.starts_with('v') {
            continue;
        }
        let Some(at) = code.rfind(version) else { continue };
        out.push(dep(i, chars(line, at), module, version, Registry::Go, Flavor::Exact));
    }
    out
}

fn pubspec(text: &str) -> Vec<Dependency> {
    const SECTIONS: [&str; 3] = ["dependencies:", "dev_dependencies:", "dependency_overrides:"];
    let mut out = Vec::new();
    let mut inside = false;
    let mut child: Option<usize> = None;
    let mut pending: Option<String> = None;
    for (i, line) in text.lines().enumerate() {
        let code = line.split(" #").next().unwrap_or("").trim_end();
        if code.trim().is_empty() || code.trim_start().starts_with('#') {
            continue;
        }
        let indent = code.len() - code.trim_start().len();
        if indent == 0 {
            inside = SECTIONS.contains(&code);
            (child, pending) = (None, None);
            continue;
        }
        if !inside {
            continue;
        }
        let indent_of_child = *child.get_or_insert(indent);
        let Some((key, value)) = code.trim_start().split_once(':') else { continue };
        let (key, value) = (key.trim(), value.trim());
        if indent == indent_of_child {
            pending = None;
            if value.is_empty() {
                pending = Some(key.to_string());
                continue;
            }
            let at = code.len() - code[code.find(':').unwrap_or(0) + 1..].trim_start().len();
            let (at, wanted) = match quoted(code, at).filter(|_| value.starts_with(['"', '\''])) {
                Some((at, v)) => (at, v),
                None => (at, value),
            };
            if !wanted.starts_with('{') {
                out.push(dep(i, chars(line, at), key, wanted, Registry::Pub, Flavor::Exact));
            }
        } else if let (Some(name), "version") = (pending.as_ref(), key) {
            let at = code.len() - code[code.find(':').unwrap_or(0) + 1..].trim_start().len();
            let (at, wanted) = match quoted(code, at).filter(|_| value.starts_with(['"', '\''])) {
                Some((at, v)) => (at, v),
                None => (at, value),
            };
            out.push(dep(i, chars(line, at), name, wanted, Registry::Pub, Flavor::Exact));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(file: &str, text: &str) -> Vec<(u32, u32, String, String)> {
        let deps = dependencies(file, text);
        for d in &deps {
            let line = text.lines().nth(d.line as usize).unwrap_or("");
            let at: String = line.chars().skip(d.col as usize).take(d.wanted.chars().count()).collect();
            assert_eq!(at, d.wanted, "{file}: the column of {} points at its requirement", d.name);
        }
        deps.into_iter().map(|d| (d.line, d.col, d.name, d.wanted)).collect()
    }

    fn names(found: &[(u32, u32, String, String)]) -> Vec<(u32, &str, &str)> {
        found.iter().map(|(l, _, n, w)| (*l, n.as_str(), w.as_str())).collect()
    }

    #[test]
    fn what_counts_as_a_manifest() {
        for yes in [
            "package.json",
            "Cargo.toml",
            "pyproject.toml",
            "requirements.txt",
            "requirements-dev.txt",
            "App.csproj",
            "Lib.fsproj",
            "Directory.Packages.props",
            "go.mod",
            "composer.json",
            "pubspec.yaml",
        ] {
            assert!(is_manifest(yes), "{yes}");
        }
        for no in ["package-lock.json", "Cargo.lock", "go.sum", "README.md", "setup.py", "tsconfig.json"] {
            assert!(!is_manifest(no), "{no}");
        }
    }

    #[test]
    fn a_package_json() {
        let text = r#"{
  "name": "demo",
  "version": "1.0.0",
  "scripts": { "build": "vite build" },
  "dependencies": {
    "react": "^18.2.0",
    "@scope/pkg": "~1.4",
    "local": "file:../local",
    "forked": "github:me/forked",
    "tagged": "latest",
    "aliased": "npm:other@^1",
    "inside": "workspace:*"
  },
  "devDependencies": {
    "vite": "5.0.0"
  },
  "peerDependencies": {
    "react-dom": ">=17 <19"
  },
  "optionalDependencies": {
    "fsevents": "2.x"
  }
}"#;
        let found = seen("package.json", text);
        assert_eq!(
            names(&found),
            vec![
                (5, "react", "^18.2.0"),
                (6, "@scope/pkg", "~1.4"),
                (14, "vite", "5.0.0"),
                (17, "react-dom", ">=17 <19"),
                (20, "fsevents", "2.x")
            ]
        );
        assert_eq!(found[0].1, 14);
        assert!(dependencies("package.json", text).iter().all(|d| d.registry == Registry::Npm));
        assert!(dependencies("other.json", text).is_empty());
    }

    #[test]
    fn a_cargo_toml() {
        let text = r#"[package]
name = "demo"
version = "0.1.0"

[dependencies]
serde = { version = "1", features = ["derive"] }
log = "0.4" # logging
local = { path = "../local" }
shared.workspace = true
renamed = { package = "real-name", version = "2.1" }
inherited = { workspace = true }
forked = { git = "https://example.com/forked", version = "1" }

[dev-dependencies]
tempfile = "3"

[target.'cfg(windows)'.dependencies]
wry = "0.57"

[dependencies.tokio]
version = "1.35"
features = ["full"]

[build-dependencies]
cc = '1.0.83'

[workspace.dependencies]
anyhow = "1.0.75"

[features]
default = []
"#;
        assert_eq!(
            names(&seen("Cargo.toml", text)),
            vec![
                (5, "serde", "1"),
                (6, "log", "0.4"),
                (9, "real-name", "2.1"),
                (14, "tempfile", "3"),
                (17, "wry", "0.57"),
                (20, "tokio", "1.35"),
                (24, "cc", "1.0.83"),
                (27, "anyhow", "1.0.75"),
            ]
        );
        assert!(dependencies("Cargo.toml", text).iter().all(|d| d.registry == Registry::Crates));
    }

    #[test]
    fn a_requirements_txt() {
        let text = "# pinned\nrequests==2.28.1\nDjango >= 4.2, <5  # web\nnumpy\n-r base.txt\n--index-url https://example.com\nuvicorn[standard]~=0.23.0 ; python_version >= \"3.8\"\npkg @ https://example.com/pkg.whl\nflask==2.3.* \\\n    --hash=sha256:abc\n";
        assert_eq!(
            names(&seen("requirements-dev.txt", text)),
            vec![
                (1, "requests", "==2.28.1"),
                (2, "Django", ">= 4.2, <5"),
                (6, "uvicorn", "~=0.23.0"),
                (8, "flask", "==2.3.*"),
            ]
        );
    }

    #[test]
    fn a_pyproject_toml() {
        let text = r#"[project]
name = "demo"
dependencies = [
    "httpx>=0.25",
    "pydantic==2.4.2", # pinned
    "rich",
]

[project.optional-dependencies]
test = ["pytest>=7.4", "coverage[toml]~=7.3"]

[dependency-groups]
lint = [
  "ruff==0.1.5",
]

[tool.poetry.dependencies]
python = "^3.11"
fastapi = "^0.104.1"
sqlalchemy = { version = "~2.0", extras = ["asyncio"] }
mine = { path = "../mine" }

[tool.poetry.group.dev.dependencies]
black = "23.11.0"

[tool.ruff]
line-length = 100
"#;
        assert_eq!(
            names(&seen("pyproject.toml", text)),
            vec![
                (3, "httpx", ">=0.25"),
                (4, "pydantic", "==2.4.2"),
                (9, "pytest", ">=7.4"),
                (9, "coverage", "~=7.3"),
                (13, "ruff", "==0.1.5"),
                (18, "fastapi", "^0.104.1"),
                (19, "sqlalchemy", "~2.0"),
                (23, "black", "23.11.0"),
            ]
        );
        assert!(dependencies("pyproject.toml", text).iter().all(|d| d.registry == Registry::PyPI));
    }

    #[test]
    fn a_csproj_and_central_versions() {
        let text = r#"<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <PackageReference Include="Newtonsoft.Json" Version="13.0.1" />
    <PackageReference Version="[1.0,2.0)" Include="Ranged" />
    <PackageReference Include="FromProperty" Version="$(SomeVersion)" />
    <PackageReference Include="Serilog">
      <Version>3.0.1</Version>
    </PackageReference>
    <PackageReference Include="Central" />
    <PackageVersion Include="Polly" Version="7.2.4" />
  </ItemGroup>
</Project>
"#;
        assert_eq!(
            names(&seen("App.csproj", text)),
            vec![
                (2, "Newtonsoft.Json", "13.0.1"),
                (3, "Ranged", "[1.0,2.0)"),
                (6, "Serilog", "3.0.1"),
                (9, "Polly", "7.2.4")
            ]
        );
        assert_eq!(names(&seen("Directory.Packages.props", text)).len(), 4);
        assert!(dependencies("App.csproj", text).iter().all(|d| d.registry == Registry::NuGet));
    }

    #[test]
    fn a_go_mod() {
        let text = "module example.com/demo\n\ngo 1.21\n\nrequire github.com/pkg/errors v0.9.1\n\nrequire (\n\tgithub.com/spf13/cobra v1.7.0\n\tgolang.org/x/sys v0.0.0-20230101000000-abcdef123456 // indirect\n)\n\nreplace github.com/pkg/errors => ../errors\n";
        assert_eq!(
            names(&seen("go.mod", text)),
            vec![
                (4, "github.com/pkg/errors", "v0.9.1"),
                (7, "github.com/spf13/cobra", "v1.7.0"),
                (8, "golang.org/x/sys", "v0.0.0-20230101000000-abcdef123456"),
            ]
        );
    }

    #[test]
    fn a_composer_json() {
        let text = r#"{
    "require": {
        "php": "^8.1",
        "ext-json": "*",
        "monolog/monolog": "^3.0",
        "symfony/console": "~6.3.0",
        "acme/dev": "dev-main"
    },
    "require-dev": {
        "phpunit/phpunit": "^10.4"
    }
}"#;
        assert_eq!(
            names(&seen("composer.json", text)),
            vec![(4, "monolog/monolog", "^3.0"), (5, "symfony/console", "~6.3.0"), (9, "phpunit/phpunit", "^10.4")]
        );
    }

    #[test]
    fn a_pubspec_yaml() {
        let text = "name: demo\nenvironment:\n  sdk: '>=3.0.0 <4.0.0'\n\ndependencies:\n  flutter:\n    sdk: flutter\n  http: ^1.1.0\n  provider: \"6.0.5\" # state\n  hosted_one:\n    hosted: https://example.com\n    version: ^2.0.0\n\ndev_dependencies:\n  lints: ^2.1.1\n";
        assert_eq!(
            names(&seen("pubspec.yaml", text)),
            vec![
                (7, "http", "^1.1.0"),
                (8, "provider", "6.0.5"),
                (11, "hosted_one", "^2.0.0"),
                (14, "lints", "^2.1.1")
            ]
        );
    }
}
