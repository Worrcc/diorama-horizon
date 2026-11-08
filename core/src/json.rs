//! A small JSON reader and writer for the waypoint file (the core has no
//! dependencies). It reads any JSON text and writes it back pretty-printed
//! with two-space indents, as Gson's `setPrettyPrinting` does, so a file is
//! readable by hand. Objects keep their keys in order.

/// A JSON value. Numbers are `f64`: exact for integers up to 2^53, which
/// covers block coordinates, colours and millisecond times.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

/// Nesting deeper than this is refused (a waypoint file is two levels deep).
const MAX_DEPTH: usize = 32;

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }
    /// A whole number, if this is one that fits an `i64`.
    pub fn as_i64(&self) -> Option<i64> {
        let n = self.as_f64()?;
        (n.is_finite() && n.fract() == 0.0 && n.abs() < 9.2e18).then_some(n as i64)
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Reads one JSON value; text after it other than white space is an error.
    pub fn parse(text: &str) -> Result<Json, String> {
        let mut p = Parser {
            s: text.as_bytes(),
            at: 0,
        };
        let v = p.value(0)?;
        p.space();
        if p.at != p.s.len() {
            return Err(p.error("text after the value"));
        }
        Ok(v)
    }

    /// The value as pretty-printed JSON text.
    pub fn pretty(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out
    }

    fn write(&self, out: &mut String, indent: usize) {
        let pad = |out: &mut String, n: usize| {
            out.push('\n');
            for _ in 0..n {
                out.push_str("  ");
            }
        };
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => write_number(out, *n),
            Json::Str(s) => write_string(out, s),
            Json::Arr(items) if items.is_empty() => out.push_str("[]"),
            Json::Arr(items) => {
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    pad(out, indent + 1);
                    v.write(out, indent + 1);
                }
                pad(out, indent);
                out.push(']');
            }
            Json::Obj(fields) if fields.is_empty() => out.push_str("{}"),
            Json::Obj(fields) => {
                out.push('{');
                for (i, (k, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    pad(out, indent + 1);
                    write_string(out, k);
                    out.push_str(": ");
                    v.write(out, indent + 1);
                }
                pad(out, indent);
                out.push('}');
            }
        }
    }
}

fn write_number(out: &mut String, n: f64) {
    use std::fmt::Write;
    if !n.is_finite() {
        // JSON has no infinities; Gson refuses them too.
        out.push_str("null");
    } else if n.fract() == 0.0 && n.abs() < 9.0e15 {
        let _ = write!(out, "{}", n as i64);
    } else {
        let _ = write!(out, "{n}");
    }
}

fn write_string(out: &mut String, s: &str) {
    use std::fmt::Write;
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

struct Parser<'a> {
    s: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn error(&self, what: &str) -> String {
        format!("JSON: {what} at byte {}", self.at)
    }
    fn space(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.s.get(self.at) {
            self.at += 1;
        }
    }
    fn eat(&mut self, word: &str) -> bool {
        if self.s[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            true
        } else {
            false
        }
    }
    fn value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > MAX_DEPTH {
            return Err(self.error("nested too deep"));
        }
        self.space();
        match self.s.get(self.at) {
            None => Err(self.error("end of text")),
            Some(b'{') => {
                self.at += 1;
                let mut fields = Vec::new();
                self.space();
                if self.eat("}") {
                    return Ok(Json::Obj(fields));
                }
                loop {
                    self.space();
                    if self.s.get(self.at) != Some(&b'"') {
                        return Err(self.error("expected a key"));
                    }
                    let k = self.string()?;
                    self.space();
                    if !self.eat(":") {
                        return Err(self.error("expected ':'"));
                    }
                    let v = self.value(depth + 1)?;
                    fields.push((k, v));
                    self.space();
                    if self.eat(",") {
                        continue;
                    }
                    if self.eat("}") {
                        return Ok(Json::Obj(fields));
                    }
                    return Err(self.error("expected ',' or '}'"));
                }
            }
            Some(b'[') => {
                self.at += 1;
                let mut items = Vec::new();
                self.space();
                if self.eat("]") {
                    return Ok(Json::Arr(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    self.space();
                    if self.eat(",") {
                        continue;
                    }
                    if self.eat("]") {
                        return Ok(Json::Arr(items));
                    }
                    return Err(self.error("expected ',' or ']'"));
                }
            }
            Some(b'"') => self.string().map(Json::Str),
            Some(b't') if self.eat("true") => Ok(Json::Bool(true)),
            Some(b'f') if self.eat("false") => Ok(Json::Bool(false)),
            Some(b'n') if self.eat("null") => Ok(Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.error("unexpected character")),
        }
    }
    fn number(&mut self) -> Result<Json, String> {
        let start = self.at;
        while let Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') = self.s.get(self.at) {
            self.at += 1;
        }
        let text = std::str::from_utf8(&self.s[start..self.at]).unwrap_or("");
        text.parse::<f64>()
            .map(Json::Num)
            .map_err(|_| self.error("bad number"))
    }
    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self
            .s
            .get(self.at..self.at + 4)
            .and_then(|d| std::str::from_utf8(d).ok())
            .and_then(|d| u32::from_str_radix(d, 16).ok())
            .ok_or_else(|| self.error("bad \\u escape"))?;
        self.at += 4;
        Ok(digits)
    }
    fn string(&mut self) -> Result<String, String> {
        self.at += 1; // the opening quote
        let mut out = String::new();
        loop {
            let start = self.at;
            while let Some(&b) = self.s.get(self.at) {
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.at += 1;
            }
            // Runs between escapes are whole UTF-8 (the input is a &str and
            // the stops are ASCII).
            out.push_str(std::str::from_utf8(&self.s[start..self.at]).unwrap_or(""));
            match self.s.get(self.at) {
                Some(b'"') => {
                    self.at += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.at += 1;
                    let Some(&e) = self.s.get(self.at) else {
                        return Err(self.error("end of text in a string"));
                    };
                    self.at += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let c = if (0xD800..0xDC00).contains(&hi) && self.eat("\\u") {
                                let lo = self.hex4()?;
                                0x10000 + ((hi - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF)
                            } else {
                                hi
                            };
                            out.push(char::from_u32(c).unwrap_or('\u{FFFD}'));
                        }
                        _ => return Err(self.error("bad escape")),
                    }
                }
                _ => return Err(self.error("unterminated string")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_parses_and_writes_back() {
        let text =
            r#" { "a": [1, -2.5, true, null, "x\"y\\z\u00e9\ud83d\ude00"], "b": {}, "c": [] } "#;
        let v = Json::parse(text).unwrap();
        assert_eq!(
            v.get("a").and_then(|a| match a {
                Json::Arr(items) => items[4].as_str().map(str::to_owned),
                _ => None,
            }),
            Some("x\"y\\z\u{e9}\u{1F600}".to_owned())
        );
        let again = Json::parse(&v.pretty()).unwrap();
        assert_eq!(again, v);
        assert!(v.pretty().starts_with("{\n  \"a\": [\n    1,\n    -2.5,"));
    }

    #[test]
    fn json_refuses_broken_text() {
        for bad in [
            "",
            "[1,",
            "{\"a\" 1}",
            "[1] x",
            "\"abc",
            "[tru]",
            "{\"a\":1,}",
        ] {
            assert!(Json::parse(bad).is_err(), "{bad:?}");
        }
        let deep = "[".repeat(40) + &"]".repeat(40);
        assert!(Json::parse(&deep).is_err());
    }

    #[test]
    fn json_numbers_keep_integers_exact() {
        let v = Json::parse("[-1710003, 1762600000123, 4294967295]").unwrap();
        assert_eq!(
            v.pretty(),
            "[\n  -1710003,\n  1762600000123,\n  4294967295\n]"
        );
    }
}
