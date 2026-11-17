//! `when` clauses: the conditions manifests put on views and menus, such as
//! `myExt.ready && !isWeb` or `resourceLangId == rust`.
//!
//! They are evaluated against the context keys extensions set with the
//! `setContext` command. A key nobody set is false, and so is anything this
//! does not understand (a regular expression, say): the caller then treats
//! the thing as not shown, and has to cope with that.
//!
//! Supported: `!`, `&&`, `||`, parentheses, `==` `!=` (also `===` `!==`),
//! `<` `<=` `>` `>=` on numbers, `true` and `false`.

use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Open,
    Close,
    Not,
    And,
    Or,
    /// A comparison: `==`, `!=`, `<`, `<=`, `>`, `>=`, or one we do not
    /// evaluate (`=~`, `in`).
    Op(&'static str),
    /// A key, a number or an unquoted value.
    Word(String),
    /// `'a quoted value'`.
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
        // A word runs until something above starts.
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

/// A context value as the text `==` compares it with.
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
    /// Something could not be read: the whole clause is false.
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
            // Both sides are always read: the position has to move on.
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

    /// `key`, or `key <op> value`.
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
            // A key nobody set equals nothing, and so differs from everything.
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

/// Is the clause true, with `get` giving the value of a context key? An
/// empty clause is: it is what "no condition" looks like.
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
        // `&&` binds tighter than `||`.
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
        // A key nobody set equals nothing.
        assert!(!check("nobodySetThis == x"));
        assert!(check("nobodySetThis != x"));
    }

    #[test]
    fn clauses_from_real_manifests() {
        // Claude Code: the same view offered in two places, one of them shown.
        assert!(!check("claude-code:doesNotSupportSecondarySidebar"));
        assert!(check("!claude-code:doesNotSupportSecondarySidebar"));
        assert!(!check("claude-vscode.sessionsListEnabled"));
        // GitHub Pull Requests.
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
