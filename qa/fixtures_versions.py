import os


def write(root, rel, text):
    path = os.path.join(root, *rel.split("/"))
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf8", newline="") as out:
        out.write(text)


PACKAGE_JSON = """{
  "name": "shop",
  "version": "1.0.0",
  "private": true,
  "dependencies": {
    "react": "^18.2.0",
    "@latin-securities/widgets": "^1.0.0",
    "left-pad": "1.0.0",
    "lodash": "^4.17.21"
  },
  "devDependencies": {
    "vite": "~5.0.0"
  }
}
"""

NPMRC = """@latin-securities:registry=https://npm.pkg.github.com
//npm.pkg.github.com/:_authToken=${QA_NPM_TOKEN}
"""

CARGO_TOML = """[package]
name = "shop"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1.0.100", features = ["derive"] }
log = "0.3"

[dev-dependencies]
anyhow = "1"
"""

REQUIREMENTS = "requests==2.28.1\nDjango>=4.2,<5\n"

GO_MOD = "module example.com/shop\n\ngo 1.21\n\nrequire github.com/spf13/cobra v1.6.0\n"

CSPROJ = """<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <PackageReference Include="Newtonsoft.Json" Version="13.0.1" />
  </ItemGroup>
</Project>
"""


def versions_manifests(root):
    """One manifest of each kind the app reads versions from, with dependencies the
    fake registries (qa/fake/registry.py) know: some current, some within their
    range, some behind it, and a package of a scope that .npmrc sends to a private
    registry with a token taken from QA_NPM_TOKEN."""
    write(root, "package.json", PACKAGE_JSON)
    write(root, ".npmrc", NPMRC)
    write(root, "Cargo.toml", CARGO_TOML)
    write(root, "requirements.txt", REQUIREMENTS)
    write(root, "go.mod", GO_MOD)
    write(root, "Shop.csproj", CSPROJ)
    write(root, "src/index.ts", 'export const shop = "open";\n')


FIXTURES = {"versions-manifests": versions_manifests}
