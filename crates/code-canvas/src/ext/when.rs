use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Open,
    Close,
    Not,
    And,
    Or,
    Op(&'static str),
    Word(String),
    Quoted(String),
}

fn tokens(text: &str) -> Vec<Token> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let at = |i: usize, s: &str| chars[i..].iter().take(s.chars().count()).copied().eq(s.chars());
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let fixed: &[(&str, Token)] = &[
            ("===", Token::Op("==")),
            ("!==", Token::Op("!=")),
            ("==", Token::Op("==")),
            ("!=", Token::Op("!=")),
            ("=~", Token::Op("=~")),
            ("<=", Token::Op("<=")),
            (">=", Token::Op(">=")),
            ("&&", Token::And),
            ("||", Token::Or),
            ("<", Token::Op("<")),
            (">", Token::Op(">")),
            ("(", Token::Open),
            (")", Token::Close),
            ("!", Token::Not),
        ];
        if let Some((s, token)) = fixed.iter().find(|(s, _)| at(i, s)) {
            out.push(token.clone());
            i += s.chars().count();
            continue;
        }
        if c == '\'' {
            let end = chars[i + 1..].iter().position(|&q| q == '\'').map_or(chars.len(), |p| i + 1 + p);
            out.push(Token::Quoted(chars[i + 1..end].iter().collect()));
            i = end + 1;
            continue;
        }
        let mut end = i;
        while end < chars.len() {
            let stop = chars[end].is_whitespace()
                || matches!(chars[end], '(' | ')' | '\'' | '<' | '>')
                || at(end, "&&")
                || at(end, "||")
                || at(end, "==")
                || at(end, "!=")
                || at(end, "=~");
            if stop {
                break;
            }
            end += 1;
        }
        let word: String = chars[i..end.max(i + 1)].iter().collect();
        i = end.max(i + 1);
        out.push(match word.as_str() {
            "in" => Token::Op("in"),
            _ => Token::Word(word),
        });
    }
    out
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0),
        Some(_) => true,
    }
}

fn as_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        other => Some(other.to_string()),
    }
}

struct Parser<'a> {
    tokens: Vec<Token>,
    at: usize,
    get: &'a dyn Fn(&str) -> Option<Value>,
    bad: bool,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.at).cloned();
        self.at += 1;
        token
    }

    fn or(&mut self) -> bool {
        let mut value = self.and();
        while self.peek() == Some(&Token::Or) {
            self.at += 1;
            let right = self.and();
            value = value || right;
        }
        value
    }

    fn and(&mut self) -> bool {
        let mut value = self.unary();
        while self.peek() == Some(&Token::And) {
            self.at += 1;
            let right = self.unary();
            value = value && right;
        }
        value
    }

    fn unary(&mut self) -> bool {
        match self.next() {
            Some(Token::Not) => !self.unary(),
            Some(Token::Open) => {
                let value = self.or();
                if self.next() != Some(Token::Close) {
                    self.bad = true;
                }
                value
            }
            Some(Token::Word(word)) => self.operand(&word),
            Some(Token::Quoted(text)) => !text.is_empty(),
            _ => {
                self.bad = true;
                false
            }
        }
    }

    fn operand(&mut self, key: &str) -> bool {
        let Some(Token::Op(op)) = self.peek().cloned() else {
            return match key {
                "true" => true,
                "false" => false,
                _ => truthy((self.get)(key).as_ref()),
            };
        };
        self.at += 1;
        let right = match self.next() {
            Some(Token::Word(w)) | Some(Token::Quoted(w)) => w,
            _ => {
                self.bad = true;
                return false;
            }
        };
        let left = (self.get)(key);
        let text = as_text(left.as_ref());
        let numbers = || Some((text.as_deref()?.parse::<f64>().ok()?, right.parse::<f64>().ok()?));
        match op {
            "==" => text.as_deref() == Some(right.as_str()),
            "!=" => text.as_deref() != Some(right.as_str()),
            "<" => numbers().is_some_and(|(a, b)| a < b),
            "<=" => numbers().is_some_and(|(a, b)| a <= b),
            ">" => numbers().is_some_and(|(a, b)| a > b),
            ">=" => numbers().is_some_and(|(a, b)| a >= b),
            _ => {
                self.bad = true;
                false
            }
        }
    }
}

pub fn eval(clause: &str, get: &dyn Fn(&str) -> Option<Value>) -> bool {
    let tokens = tokens(clause);
    if tokens.is_empty() {
        return true;
    }
    let mut parser = Parser { tokens, at: 0, get, bad: false };
    let value = parser.or();
    value && !parser.bad && parser.at == parser.tokens.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn check(clause: &str) -> bool {
        let context = json!({
            "ready": true, "off": false, "lang": "rust", "count": 3, "empty": "",
            "claude-code:doesNotSupportSecondarySidebar": false,
            "ReposManagerStateContext": "NeedsAuthentication"
        });
        eval(clause, &|key| context.get(key).cloned())
    }

    #[test]
    fn keys_and_their_absence() {
        assert!(check(""));
        assert!(check("   "));
        assert!(check("ready"));
        assert!(!check("off"));
        assert!(!check("nobodySetThis"));
        assert!(check("!nobodySetThis"));
        assert!(!check("empty"));
        assert!(check("count"));
        assert!(check("true"));
        assert!(!check("false"));
    }

    #[test]
    fn logic_follows_the_usual_precedence() {
        assert!(check("ready && !off"));
        assert!(!check("ready && off"));
        assert!(check("off || ready"));
        assert!(check("ready || off && off"));
        assert!(!check("(ready || off) && off"));
        assert!(check("!(off || nobodySetThis)"));
        assert!(check("!!ready"));
    }

    #[test]
    fn comparisons() {
        assert!(check("lang == rust"));
        assert!(check("lang == 'rust'"));
        assert!(check("lang === rust"));
        assert!(!check("lang == go"));
        assert!(check("lang != go"));
        assert!(check("ready == true"));
        assert!(check("count == 3"));
        assert!(check("count >= 3 && count < 4"));
        assert!(!check("count > 3"));
        assert!(!check("nobodySetThis == x"));
        assert!(check("nobodySetThis != x"));
    }

    #[test]
    fn clauses_from_real_manifests() {
        assert!(!check("claude-code:doesNotSupportSecondarySidebar"));
        assert!(check("!claude-code:doesNotSupportSecondarySidebar"));
        assert!(!check("claude-vscode.sessionsListEnabled"));
        assert!(check("(ReposManagerStateContext == NeedsAuthentication) && !isSessionsWindow"));
        assert!(!check(
            "(ReposManagerStateContext != NeedsAuthentication && !github:resolvingConflicts) && !isSessionsWindow"
        ));
        assert!(!check("(github:createPullRequest || github:revertPullRequest) && !isSessionsWindow"));
        assert!(!check("github:activePRCount <= 1"));
    }

    #[test]
    fn what_it_cannot_read_is_false() {
        assert!(!check("lang =~ /ru.*/"));
        assert!(!check("lang in list"));
        assert!(!check("ready &&"));
        assert!(!check("(ready"));
        assert!(!check("ready off"));
        assert!(!check("== rust"));
    }
}
