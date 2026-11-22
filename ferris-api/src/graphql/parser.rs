//! The lexer and recursive-descent parser for the supported query subset.
//!
//! Both are hand-written because a full GraphQL engine is not available
//! offline. The parser is recursive descent, so the nesting bound
//! ([`MAX_DEPTH`]) is load-bearing: without it one tiny request —
//! `{a{a{a{...}}}}` — overflows the stack and aborts the process (V3-API-1).

use serde_json::{Map as JsonMap, Value as JsonValue, json};

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Token {
    Name(String),
    String(String),
    Number(f64),
    Variable(String),
    Spread,
    Punct(char),
}

#[derive(Debug, Clone)]
pub(super) struct Field {
    pub(super) alias: Option<String>,
    pub(super) name: String,
    pub(super) args: Vec<(String, JsonValue)>,
    pub(super) selection: Vec<Field>,
}

pub(super) fn tokenize(input: &str) -> std::result::Result<Vec<Token>, String> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() || c == ',' {
            i += 1;
            continue;
        }
        if c == '#' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '"' {
            i += 1;
            let mut text = String::new();
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                    text.push(match chars[i] {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        other => other,
                    });
                } else {
                    text.push(chars[i]);
                }
                i += 1;
            }
            if i >= chars.len() {
                return Err("unterminated string".to_string());
            }
            i += 1;
            tokens.push(Token::String(text));
            continue;
        }
        if c == '$' {
            i += 1;
            let start = i;
            while i < chars.len() && is_name_char(chars[i]) {
                i += 1;
            }
            if start == i {
                return Err("expected variable name after `$`".to_string());
            }
            tokens.push(Token::Variable(chars[start..i].iter().collect()));
            continue;
        }
        if c == '.' && chars.get(i + 1) == Some(&'.') && chars.get(i + 2) == Some(&'.') {
            tokens.push(Token::Spread);
            i += 3;
            continue;
        }
        if c.is_ascii_digit() || (c == '-' && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit()))
        {
            let start = i;
            i += 1;
            while i < chars.len()
                && (chars[i].is_ascii_digit() || matches!(chars[i], '.' | 'e' | 'E' | '+' | '-'))
            {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            let number = text
                .parse::<f64>()
                .map_err(|_| format!("invalid number `{text}`"))?;
            tokens.push(Token::Number(number));
            continue;
        }
        if is_name_start(c) {
            let start = i;
            while i < chars.len() && is_name_char(chars[i]) {
                i += 1;
            }
            tokens.push(Token::Name(chars[start..i].iter().collect()));
            continue;
        }
        if "{}()[]:@!".contains(c) {
            tokens.push(Token::Punct(c));
            i += 1;
            continue;
        }
        return Err(format!("unexpected character `{c}`"));
    }

    Ok(tokens)
}

fn is_name_start(c: char) -> bool {
    c == '_' || c.is_ascii_alphabetic()
}

fn is_name_char(c: char) -> bool {
    c == '_' || c.is_ascii_alphanumeric()
}

/// Maximum nesting the parser follows before refusing the query.
///
/// The parser is recursive-descent, so nested selection sets (and nested
/// list/object literals) become native stack frames. Without a bound a single
/// small request — `{a{a{a{...}}}}` — overflows the stack and aborts the whole
/// process (V3-API-1). 64 is far above any legitimate query and far below the
/// depth that can exhaust a request thread's stack.
pub(super) const MAX_DEPTH: usize = 64;

pub(super) struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    variables: JsonMap<String, JsonValue>,
    /// Current nesting depth, checked against [`MAX_DEPTH`] on every descent.
    depth: usize,
}

impl Parser {
    pub(super) fn new(tokens: Vec<Token>, variables: JsonMap<String, JsonValue>) -> Self {
        Self {
            tokens,
            pos: 0,
            variables,
            depth: 0,
        }
    }

    /// Descend one nesting level, refusing to go past [`MAX_DEPTH`].
    fn enter_nesting(&mut self) -> std::result::Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(format!(
                "query exceeds the maximum nesting depth of {MAX_DEPTH}"
            ));
        }
        Ok(())
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        token
    }

    fn expect_punct(&mut self, expected: char) -> std::result::Result<(), String> {
        match self.advance() {
            Some(Token::Punct(c)) if c == expected => Ok(()),
            other => Err(format!("expected `{expected}`, found {other:?}")),
        }
    }

    fn expect_name(&mut self) -> std::result::Result<String, String> {
        match self.advance() {
            Some(Token::Name(name)) => Ok(name),
            other => Err(format!("expected a name, found {other:?}")),
        }
    }

    pub(super) fn parse_document(&mut self) -> std::result::Result<Vec<Field>, String> {
        if let Some(Token::Name(kind)) = self.peek() {
            if matches!(kind.as_str(), "query" | "mutation" | "subscription") {
                let kind = kind.clone();
                if kind != "query" {
                    return Err("only queries are supported".to_string());
                }
                self.advance();
                if matches!(self.peek(), Some(Token::Name(_))) {
                    self.advance();
                }
            }
        }
        self.parse_selection_set()
    }

    fn parse_selection_set(&mut self) -> std::result::Result<Vec<Field>, String> {
        self.enter_nesting()?;
        self.expect_punct('{')?;
        let mut fields = Vec::new();
        while !matches!(self.peek(), Some(Token::Punct('}'))) {
            if self.peek().is_none() {
                return Err("unterminated selection set".to_string());
            }
            fields.push(self.parse_field()?);
        }
        self.expect_punct('}')?;
        Ok(fields)
    }

    fn parse_field(&mut self) -> std::result::Result<Field, String> {
        let first = self.expect_name()?;
        let (alias, name) = if matches!(self.peek(), Some(Token::Punct(':'))) {
            self.advance();
            (Some(first), self.expect_name()?)
        } else {
            (None, first)
        };

        let mut args = Vec::new();
        if matches!(self.peek(), Some(Token::Punct('('))) {
            self.advance();
            while !matches!(self.peek(), Some(Token::Punct(')'))) {
                let arg_name = self.expect_name()?;
                self.expect_punct(':')?;
                args.push((arg_name, self.parse_value()?));
            }
            self.expect_punct(')')?;
        }

        // Skip directives (`@deprecated(...)`) without interpreting them.
        while matches!(self.peek(), Some(Token::Punct('@'))) {
            self.advance();
            let _ = self.expect_name()?;
            if matches!(self.peek(), Some(Token::Punct('('))) {
                let mut depth = 0;
                loop {
                    match self.advance() {
                        Some(Token::Punct('(')) => depth += 1,
                        Some(Token::Punct(')')) => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        Some(_) => {}
                        None => return Err("unterminated directive".to_string()),
                    }
                }
            }
        }

        let selection = if matches!(self.peek(), Some(Token::Punct('{'))) {
            self.parse_selection_set()?
        } else {
            Vec::new()
        };

        Ok(Field {
            alias,
            name,
            args,
            selection,
        })
    }

    fn parse_value(&mut self) -> std::result::Result<JsonValue, String> {
        match self.advance() {
            Some(Token::String(s)) => Ok(json!(s)),
            Some(Token::Number(n)) => Ok(json!(n)),
            Some(Token::Variable(name)) => self
                .variables
                .get(&name)
                .cloned()
                .ok_or_else(|| format!("undefined variable `${name}`")),
            Some(Token::Name(name)) => match name.as_str() {
                "true" => Ok(json!(true)),
                "false" => Ok(json!(false)),
                "null" => Ok(JsonValue::Null),
                _ => Ok(json!(name)),
            },
            Some(Token::Punct('[')) => {
                self.enter_nesting()?;
                let mut items = Vec::new();
                while !matches!(self.peek(), Some(Token::Punct(']'))) {
                    items.push(self.parse_value()?);
                }
                self.expect_punct(']')?;
                Ok(JsonValue::Array(items))
            }
            Some(Token::Punct('{')) => {
                self.enter_nesting()?;
                let mut object = JsonMap::new();
                while !matches!(self.peek(), Some(Token::Punct('}'))) {
                    let key = self.expect_name()?;
                    self.expect_punct(':')?;
                    object.insert(key, self.parse_value()?);
                }
                self.expect_punct('}')?;
                Ok(JsonValue::Object(object))
            }
            other => Err(format!("unexpected value {other:?}")),
        }
    }
}
