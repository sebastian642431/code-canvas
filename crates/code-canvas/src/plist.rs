use serde_json::{Map, Value};

pub fn to_json(xml: &str) -> Option<Value> {
    let start = xml.find("<plist")?;
    let mut p = Parser { s: xml, at: start + xml[start..].find('>')? + 1 };
    p.value()
}

struct Parser<'a> {
    s: &'a str,
    at: usize,
}

impl<'a> Parser<'a> {
    fn rest(&self) -> &'a str {
        &self.s[self.at..]
    }

    fn skip(&mut self) -> Option<()> {
        loop {
            let rest = self.rest();
            let trimmed = rest.trim_start();
            self.at += rest.len() - trimmed.len();
            let close = match () {
                _ if trimmed.starts_with("<!--") => "-->",
                _ if trimmed.starts_with("<?") => "?>",
                _ if trimmed.starts_with("<!") => ">",
                _ => return Some(()),
            };
            self.at += trimmed.find(close)? + close.len();
        }
    }

    fn tag(&mut self) -> Option<(&'a str, bool)> {
        self.skip()?;
        let (s, rest) = (self.s, self.rest());
        let inner = rest.strip_prefix('<')?;
        let end = inner.find('>')?;
        let body = &s[self.at + 1..self.at + 1 + end];
        self.at += end + 2;
        let empty = body.ends_with('/');
        let name = body.trim_end_matches('/').split_whitespace().next().unwrap_or("");
        Some((name, empty))
    }

    fn text(&mut self, name: &str) -> Option<String> {
        let close = format!("</{name}>");
        let end = self.rest().find(&close)?;
        let raw = &self.s[self.at..self.at + end];
        self.at += end + close.len();
        Some(unescape(raw))
    }

    fn value(&mut self) -> Option<Value> {
        let (name, empty) = self.tag()?;
        let name = name.to_string();
        match (name.as_str(), empty) {
            ("dict", true) => Some(Value::Object(Map::new())),
            ("array", true) => Some(Value::Array(Vec::new())),
            ("string" | "data" | "date", true) => Some(Value::String(String::new())),
            ("true", _) => Some(Value::Bool(true)),
            ("false", _) => Some(Value::Bool(false)),
            ("dict", false) => {
                let mut map = Map::new();
                loop {
                    self.skip()?;
                    if self.rest().starts_with("</dict") {
                        self.tag()?;
                        return Some(Value::Object(map));
                    }
                    let (key, empty) = self.tag()?;
                    if key != "key" {
                        return None;
                    }
                    let key = if empty { String::new() } else { self.text("key")? };
                    let value = self.value()?;
                    map.insert(key, value);
                }
            }
            ("array", false) => {
                let mut list = Vec::new();
                loop {
                    self.skip()?;
                    if self.rest().starts_with("</array") {
                        self.tag()?;
                        return Some(Value::Array(list));
                    }
                    list.push(self.value()?);
                }
            }
            ("string" | "data" | "date", false) => Some(Value::String(self.text(&name)?)),
            ("integer" | "real", false) => {
                let text = self.text(&name)?;
                let number = text.trim().parse::<i64>().ok().map(Value::from);
                number.or_else(|| text.trim().parse::<f64>().ok().map(Value::from))
            }
            _ => None,
        }
    }
}

fn unescape(raw: &str) -> String {
    let raw = raw.strip_prefix("<![CDATA[").and_then(|r| r.strip_suffix("]]>")).map_or_else(
        || {
            let mut out = String::with_capacity(raw.len());
            let mut rest = raw;
            while let Some(at) = rest.find('&') {
                out.push_str(&rest[..at]);
                rest = &rest[at..];
                let Some(end) = rest.find(';').filter(|&end| end <= 10) else {
                    out.push('&');
                    rest = &rest[1..];
                    continue;
                };
                let entity = &rest[1..end];
                let decoded = match entity {
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "amp" => Some('&'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    _ => entity
                        .strip_prefix("#x")
                        .map(|hex| u32::from_str_radix(hex, 16))
                        .or_else(|| entity.strip_prefix('#').map(str::parse::<u32>))
                        .and_then(Result::ok)
                        .and_then(char::from_u32),
                };
                match decoded {
                    Some(c) => {
                        out.push(c);
                        rest = &rest[end + 1..];
                    }
                    None => {
                        out.push('&');
                        rest = &rest[1..];
                    }
                }
            }
            out.push_str(rest);
            out
        },
        str::to_string,
    );
    raw
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grammar_in_a_property_list_reads_as_json() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <!-- a comment -->
    <key>scopeName</key>
    <string>source.zed</string>
    <key>fileTypes</key>
    <array><string>zed</string></array>
    <key>patterns</key>
    <array>
        <dict>
            <key>match</key>
            <string>\b(a&amp;b|&lt;x&gt;)\b</string>
            <key>name</key>
            <string>keyword.zed</string>
        </dict>
        <dict/>
    </array>
    <key>hidden</key>
    <true/>
    <key>version</key>
    <integer>3</integer>
    <key>empty</key>
    <string/>
</dict>
</plist>"#;
        let v = to_json(xml).unwrap();
        assert_eq!(v["scopeName"], "source.zed");
        assert_eq!(v["fileTypes"][0], "zed");
        assert_eq!(v["patterns"][0]["match"], "\\b(a&b|<x>)\\b");
        assert_eq!(v["patterns"][1], Value::Object(Map::new()));
        assert_eq!(v["hidden"], true);
        assert_eq!(v["version"], 3);
        assert_eq!(v["empty"], "");
        assert!(to_json("<plist><dict><string>x</string></dict></plist>").is_none());
    }
}
