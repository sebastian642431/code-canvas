use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{json, Value};

const CLIENT_ID: &str = "Ov23liqzdSkACc9mnzin";
const SCOPE: &str = "gist";
const USER_AGENT: &str = concat!("code-canvas/", env!("CARGO_PKG_VERSION"));
const GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
pub const MARK: &str = "code-canvas.json";
const ABOUT: &str = "Code Canvas settings";

fn or_env(name: &str, otherwise: &str) -> String {
    std::env::var(name).ok().filter(|given| !given.trim().is_empty()).unwrap_or_else(|| otherwise.to_string())
}

pub fn client_id() -> String {
    or_env("CODE_CANVAS_GITHUB_CLIENT", CLIENT_ID)
}

fn site() -> String {
    or_env("CODE_CANVAS_GITHUB_SITE", "https://github.com")
}

fn api() -> String {
    or_env("CODE_CANVAS_GITHUB_API", "https://api.github.com")
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .build()
        .into()
}

#[derive(Clone, Debug, PartialEq)]
pub enum Refused {
    NotSignedIn,
    Other(String),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::NotSignedIn => f.write_str("GitHub no longer takes this sign-in"),
            Refused::Other(why) => f.write_str(why),
        }
    }
}

fn other(why: impl ToString) -> Refused {
    Refused::Other(why.to_string())
}

fn answer(sent: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Value, Refused> {
    let mut response = sent.map_err(other)?;
    let status = response.status().as_u16();
    let body = response.body_mut().read_to_string().map_err(other)?;
    let json: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    match status {
        200..=299 => Ok(json),
        401 => Err(Refused::NotSignedIn),
        _ => {
            let said = json.get("message").and_then(Value::as_str).unwrap_or("no reason given");
            Err(Refused::Other(format!("GitHub answered {status}: {said}")))
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Code {
    pub device: String,
    pub user: String,
    pub page: String,
    pub every: Duration,
    pub lasts: Duration,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Waited {
    Pending,
    Slower,
    Token(String),
}

pub fn parse_code(json: &Value) -> Option<Code> {
    let text = |key: &str| json.get(key).and_then(Value::as_str).map(str::to_string);
    let seconds = |key: &str, otherwise: u64| json.get(key).and_then(Value::as_u64).unwrap_or(otherwise);
    Some(Code {
        device: text("device_code")?,
        user: text("user_code")?,
        page: text("verification_uri")?,
        every: Duration::from_secs(seconds("interval", 5).clamp(1, 60)),
        lasts: Duration::from_secs(seconds("expires_in", 900)),
    })
}

pub fn parse_waited(json: &Value) -> Result<Waited, String> {
    if let Some(token) = json.get("access_token").and_then(Value::as_str).filter(|token| !token.is_empty()) {
        return Ok(Waited::Token(token.to_string()));
    }
    match json.get("error").and_then(Value::as_str) {
        Some("authorization_pending") => Ok(Waited::Pending),
        Some("slow_down") => Ok(Waited::Slower),
        Some("expired_token") => Err("the code ran out before it was entered".to_string()),
        Some("access_denied") => Err("the sign-in was cancelled on GitHub".to_string()),
        Some(other) => Err(json.get("error_description").and_then(Value::as_str).unwrap_or(other).to_string()),
        None => Err("GitHub gave no answer that can be read".to_string()),
    }
}

pub fn ask_code() -> Result<Code, String> {
    let sent = agent()
        .post(format!("{}/login/device/code", site()))
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json")
        .send_form([("client_id", client_id().as_str()), ("scope", SCOPE)]);
    let json = answer(sent).map_err(|why| why.to_string())?;
    parse_code(&json).ok_or_else(|| match json.get("error_description").and_then(Value::as_str) {
        Some(why) => why.to_string(),
        None => "GitHub gave no code".to_string(),
    })
}

pub fn ask_token(code: &Code) -> Result<Waited, String> {
    let sent = agent()
        .post(format!("{}/login/oauth/access_token", site()))
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json")
        .send_form([("client_id", client_id().as_str()), ("device_code", code.device.as_str()), ("grant_type", GRANT)]);
    parse_waited(&answer(sent).map_err(|why| why.to_string())?)
}

fn get(token: &str, path: &str) -> Result<Value, Refused> {
    let sent = agent()
        .get(format!("{}{path}", api()))
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .header("Authorization", format!("Bearer {token}"))
        .call();
    answer(sent)
}

fn write(token: &str, path: &str, new: bool, body: &Value) -> Result<Value, Refused> {
    let url = format!("{}{path}", api());
    let request = if new { agent().post(url) } else { agent().patch(url) };
    let sent = request
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .send(body.to_string());
    answer(sent)
}

pub fn who(token: &str) -> Result<String, Refused> {
    let json = get(token, "/user")?;
    json.get("login").and_then(Value::as_str).map(str::to_string).ok_or_else(|| other("GitHub did not say who this is"))
}

pub type Files = BTreeMap<String, String>;

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Gist {
    pub id: String,
    pub files: Files,
}

pub fn parse_gist(json: &Value) -> Option<Gist> {
    let id = json.get("id").and_then(Value::as_str)?.to_string();
    let files = json.get("files").and_then(Value::as_object)?;
    let whole = files.iter().filter(|(_, file)| file.get("truncated").and_then(Value::as_bool) != Some(true));
    let files = whole
        .filter_map(|(name, file)| Some((name.clone(), file.get("content").and_then(Value::as_str)?.to_string())))
        .collect();
    Some(Gist { id, files })
}

pub fn find_in(list: &Value) -> Option<String> {
    let ours =
        |gist: &&Value| gist.get("files").and_then(Value::as_object).is_some_and(|files| files.contains_key(MARK));
    list.as_array()?.iter().find(ours)?.get("id").and_then(Value::as_str).map(str::to_string)
}

pub fn read_gist(token: &str, id: &str) -> Result<Option<Gist>, Refused> {
    match get(token, &format!("/gists/{id}")) {
        Ok(json) => Ok(parse_gist(&json)),
        Err(Refused::Other(why)) if why.starts_with("GitHub answered 404") => Ok(None),
        Err(why) => Err(why),
    }
}

pub fn find_gist(token: &str) -> Result<Option<Gist>, Refused> {
    for page in 1..=10 {
        let list = get(token, &format!("/gists?per_page=100&page={page}"))?;
        if let Some(id) = find_in(&list) {
            return read_gist(token, &id);
        }
        if list.as_array().is_none_or(|gists| gists.len() < 100) {
            break;
        }
    }
    Ok(None)
}

fn body_of(files: &Files) -> Value {
    let files: serde_json::Map<String, Value> =
        files.iter().map(|(name, text)| (name.clone(), json!({ "content": text }))).collect();
    json!({ "description": ABOUT, "public": false, "files": files })
}

pub fn write_gist(token: &str, id: Option<&str>, files: &Files) -> Result<Gist, Refused> {
    let json = match id {
        Some(id) => write(token, &format!("/gists/{id}"), false, &body_of(files))?,
        None => write(token, "/gists", true, &body_of(files))?,
    };
    parse_gist(&json).ok_or_else(|| other("GitHub did not give the settings back"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_code_to_enter_is_read() {
        let json = json!({
            "device_code": "3584d83530557fdd1f46af8289938c8ef79f9dc5", "user_code": "WDJB-MJHT",
            "verification_uri": "https://github.com/login/device", "expires_in": 900, "interval": 5
        });
        let code = parse_code(&json).unwrap();
        assert_eq!((code.user.as_str(), code.page.as_str()), ("WDJB-MJHT", "https://github.com/login/device"));
        assert_eq!((code.every, code.lasts), (Duration::from_secs(5), Duration::from_secs(900)));
        assert_eq!(parse_code(&json!({ "error": "device_flow_disabled" })), None);
    }

    #[test]
    fn waiting_for_the_sign_in_knows_each_answer() {
        assert_eq!(parse_waited(&json!({ "error": "authorization_pending" })), Ok(Waited::Pending));
        assert_eq!(parse_waited(&json!({ "error": "slow_down", "interval": 10 })), Ok(Waited::Slower));
        let given = json!({ "access_token": "gho_16C7e42F292c6912E7710c838347Ae178B4a", "token_type": "bearer" });
        assert_eq!(parse_waited(&given), Ok(Waited::Token("gho_16C7e42F292c6912E7710c838347Ae178B4a".to_string())));
        assert!(parse_waited(&json!({ "error": "expired_token" })).unwrap_err().contains("ran out"));
        assert!(parse_waited(&json!({ "error": "access_denied" })).unwrap_err().contains("cancelled"));
        assert!(parse_waited(&json!({})).is_err());
    }

    #[test]
    fn the_settings_are_found_among_the_gists() {
        let list = json!([
            { "id": "aa", "files": { "notes.md": {} } },
            { "id": "bb", "files": { "prefs.json": {}, "code-canvas.json": {} } },
        ]);
        assert_eq!(find_in(&list), Some("bb".to_string()));
        assert_eq!(find_in(&json!([{ "id": "aa", "files": { "notes.md": {} } }])), None);
        let gist = json!({ "id": "bb", "files": {
            "prefs.json": { "content": "{}", "truncated": false },
            "huge.json": { "content": "{", "truncated": true },
        } });
        let read = parse_gist(&gist).unwrap();
        assert_eq!(read.files, Files::from([("prefs.json".to_string(), "{}".to_string())]));
        let sent = body_of(&read.files);
        assert_eq!(sent["public"], false);
        assert_eq!(sent["files"]["prefs.json"]["content"], "{}");
    }
}
