//! A data-only reader for Lua table literals (no Lua VM).
//!
//! The game keeps its object definitions in Lua source files as literal
//! tables (`tileslist = { object000 = { name = "baba", ... }, ... }`). This
//! module tokenises a Lua file and extracts every *top-level* assignment
//! `name = <value>` whose value is a literal: `nil`, booleans, numbers,
//! strings, and table constructors nesting those. Anything else (function
//! bodies, calls, arithmetic) is skipped and represented as
//! [`Value::Function`] or [`Value::Expr`]; code is never executed.
//!
//! ```
//! use baba_formats::lua::{LuaChunk, Value};
//! let chunk = LuaChunk::parse("t = { a = 1, \"x\", [5] = true, f = function() end }").unwrap();
//! let t = chunk.get("t").unwrap().as_table().unwrap();
//! assert_eq!(t.get("a").and_then(Value::as_i64), Some(1));
//! assert_eq!(t.index(1).and_then(Value::as_str), Some("x"));
//! assert!(matches!(t.get("f"), Some(Value::Function)));
//! ```

use std::path::Path;

use crate::error::{Error, Result, read_text};

/// A Lua value as far as a data-only reader can know it.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// `nil`
    Nil,
    /// `true` / `false`
    Bool(bool),
    /// Any number (Lua integers fit exactly up to 2^53).
    Num(f64),
    /// A string literal, escapes resolved.
    Str(String),
    /// A table constructor.
    Table(Table),
    /// A `function ... end` expression (body skipped).
    Function,
    /// Any other expression, kept as space-joined token text.
    Expr(String),
}

impl Value {
    /// The string, if this is a string literal.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    /// The number, if numeric.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }
    /// The number as an integer, if numeric and integral.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Num(n) if n.fract() == 0.0 && n.is_finite() => Some(*n as i64),
            _ => None,
        }
    }
    /// The boolean, if boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    /// The table, if a table.
    pub fn as_table(&self) -> Option<&Table> {
        match self {
            Value::Table(t) => Some(t),
            _ => None,
        }
    }
    /// A two-integer array `{a, b}`.
    pub fn as_int_pair(&self) -> Option<(i64, i64)> {
        let t = self.as_table()?;
        Some((t.index(1)?.as_i64()?, t.index(2)?.as_i64()?))
    }
    /// An array of integers `{a, b, ...}`.
    pub fn as_int_list(&self) -> Option<Vec<i64>> {
        self.as_table()?.array().map(Value::as_i64).collect()
    }
    /// An array of strings `{"a", "b", ...}`.
    pub fn as_str_list(&self) -> Option<Vec<String>> {
        self.as_table()?
            .array()
            .map(|v| v.as_str().map(str::to_string))
            .collect()
    }
}

/// A table key.
#[derive(Clone, Debug, PartialEq)]
pub enum Key {
    /// Integer key: positional items (1, 2, ...) and `[n] = v`.
    Index(i64),
    /// String key: `name = v` and `["name"] = v`.
    Name(String),
    /// Any other bracketed key (`[1.5]`, `[true]`, an expression).
    Other(Box<Value>),
}

/// A table constructor's fields in source order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    /// Fields in source order.
    pub entries: Vec<(Key, Value)>,
}

impl Table {
    /// Value of the string key `name` (last occurrence wins, as in Lua).
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.entries.iter().rev().find_map(|(k, v)| match k {
            Key::Name(n) if n == name => Some(v),
            _ => None,
        })
    }
    /// Value at integer key `i`.
    pub fn index(&self, i: i64) -> Option<&Value> {
        self.entries.iter().rev().find_map(|(k, v)| match k {
            Key::Index(n) if *n == i => Some(v),
            _ => None,
        })
    }
    /// The array part: values at 1, 2, 3, ... up to the first gap.
    pub fn array(&self) -> impl Iterator<Item = &Value> {
        let mut i = 0;
        std::iter::from_fn(move || {
            i += 1;
            self.index(i)
        })
    }
    /// Number of items in [`Table::array`].
    pub fn len(&self) -> usize {
        self.array().count()
    }
    /// True if the table has no fields at all.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    /// String-keyed fields in source order.
    pub fn named(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.entries.iter().filter_map(|(k, v)| match k {
            Key::Name(n) => Some((n.as_str(), v)),
            _ => None,
        })
    }
}

/// The top-level assignments of one Lua file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LuaChunk {
    /// `(name, value)` in source order (a name can repeat; [`LuaChunk::get`]
    /// returns the last).
    pub globals: Vec<(String, Value)>,
}

impl LuaChunk {
    /// Parses Lua source and collects its top-level assignments.
    pub fn parse(src: &str) -> Result<LuaChunk> {
        let tokens = lex(src)?;
        Parser { t: &tokens, i: 0 }.chunk()
    }

    /// Reads and parses a file.
    pub fn load(path: &Path) -> Result<LuaChunk> {
        LuaChunk::parse(&read_text(path)?).map_err(|e| e.at(path))
    }

    /// The last top-level assignment to `name`.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.globals
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v)
    }
}

// ---------------------------------------------------------------- lexer

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Name(String),
    Str(String),
    Num(f64),
    Sym(&'static str),
    Eof,
}

#[derive(Clone, Debug)]
struct Token {
    tok: Tok,
    line: u32,
}

const SYMS: [&str; 33] = [
    "...", "..", "==", "~=", "<=", ">=", "//", "::", "<<", ">>", "+", "-", "*", "/", "%", "^", "#",
    "&", "~", "|", "<", ">", "=", "(", ")", "{", "}", "[", "]", ";", ":", ",", ".",
];

fn lex(src: &str) -> Result<Vec<Token>> {
    let b = src.as_bytes();
    let mut i = 0usize;
    let mut line = 1u32;
    let mut out = Vec::new();
    while i < b.len() {
        let c = b[i];
        if c == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == b'-' && b.get(i + 1) == Some(&b'-') {
            i += 2;
            if let Some(level) = long_bracket_level(b, i) {
                let (_, end, lines) = long_bracket_body(b, i, level, line)?;
                line += lines;
                i = end;
            } else {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            continue;
        }
        let start_line = line;
        if c == b'['
            && let Some(level) = long_bracket_level(b, i)
        {
            let (s, end, lines) = long_bracket_body(b, i, level, line)?;
            line += lines;
            i = end;
            out.push(Token {
                tok: Tok::Str(s),
                line: start_line,
            });
            continue;
        }
        if c == b'"' || c == b'\'' {
            let (s, end, lines) = quoted_string(b, i, line)?;
            line += lines;
            i = end;
            out.push(Token {
                tok: Tok::Str(s),
                line: start_line,
            });
            continue;
        }
        if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            let (n, end) = number(b, i).ok_or_else(|| Error::lua(line, "bad number"))?;
            i = end;
            out.push(Token {
                tok: Tok::Num(n),
                line,
            });
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' || c >= 0x80 {
            let s = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] >= 0x80) {
                i += 1;
            }
            out.push(Token {
                tok: Tok::Name(String::from_utf8_lossy(&b[s..i]).into_owned()),
                line,
            });
            continue;
        }
        let sym = SYMS
            .iter()
            .find(|s| b[i..].starts_with(s.as_bytes()))
            .ok_or_else(|| Error::lua(line, format!("unexpected character {:?}", c as char)))?;
        i += sym.len();
        out.push(Token {
            tok: Tok::Sym(sym),
            line,
        });
    }
    out.push(Token {
        tok: Tok::Eof,
        line,
    });
    Ok(out)
}

/// If `b[i..]` starts a long bracket `[[` / `[==[`, returns its level.
fn long_bracket_level(b: &[u8], i: usize) -> Option<usize> {
    if b.get(i) != Some(&b'[') {
        return None;
    }
    let mut j = i + 1;
    while b.get(j) == Some(&b'=') {
        j += 1;
    }
    (b.get(j) == Some(&b'[')).then_some(j - i - 1)
}

/// Body of a long bracket starting at `i`. Returns (text, end index, newlines).
fn long_bracket_body(b: &[u8], i: usize, level: usize, line: u32) -> Result<(String, usize, u32)> {
    let open = level + 2;
    let mut close = vec![b']'];
    close.extend(std::iter::repeat_n(b'=', level));
    close.push(b']');
    let mut start = i + open;
    // A newline right after the opening bracket is skipped.
    if b.get(start) == Some(&b'\r') {
        start += 1;
    }
    if b.get(start) == Some(&b'\n') {
        start += 1;
    }
    let rel = b[start..]
        .windows(close.len())
        .position(|w| w == close.as_slice())
        .ok_or_else(|| Error::lua(line, "unterminated long string or comment"))?;
    let body = &b[start..start + rel];
    let end = start + rel + close.len();
    let lines = b[i..end].iter().filter(|&&c| c == b'\n').count() as u32;
    Ok((String::from_utf8_lossy(body).into_owned(), end, lines))
}

fn quoted_string(b: &[u8], i: usize, line: u32) -> Result<(String, usize, u32)> {
    let q = b[i];
    let mut j = i + 1;
    let mut out: Vec<u8> = Vec::new();
    let mut lines = 0;
    loop {
        let c = *b
            .get(j)
            .ok_or_else(|| Error::lua(line, "unterminated string"))?;
        j += 1;
        if c == q {
            break;
        }
        if c == b'\n' {
            return Err(Error::lua(line, "newline in string"));
        }
        if c != b'\\' {
            out.push(c);
            continue;
        }
        let e = *b
            .get(j)
            .ok_or_else(|| Error::lua(line, "unterminated string"))?;
        j += 1;
        match e {
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'r' => out.push(b'\r'),
            b'a' => out.push(7),
            b'b' => out.push(8),
            b'f' => out.push(12),
            b'v' => out.push(11),
            b'\\' | b'"' | b'\'' => out.push(e),
            b'\n' => {
                out.push(b'\n');
                lines += 1;
            }
            b'z' => {
                while j < b.len() && b[j].is_ascii_whitespace() {
                    if b[j] == b'\n' {
                        lines += 1;
                    }
                    j += 1;
                }
            }
            b'x' => {
                let h = b
                    .get(j..j + 2)
                    .and_then(|h| std::str::from_utf8(h).ok())
                    .and_then(|h| u8::from_str_radix(h, 16).ok())
                    .ok_or_else(|| Error::lua(line, "bad \\x escape"))?;
                out.push(h);
                j += 2;
            }
            b'u' => {
                let close = b[j..]
                    .iter()
                    .position(|&c| c == b'}')
                    .ok_or_else(|| Error::lua(line, "bad \\u escape"))?;
                let hex = std::str::from_utf8(&b[j + 1..j + close]).unwrap_or("");
                let ch = u32::from_str_radix(hex, 16)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or_else(|| Error::lua(line, "bad \\u escape"))?;
                let mut buf = [0u8; 4];
                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                j += close + 1;
            }
            d if d.is_ascii_digit() => {
                let mut v = (d - b'0') as u32;
                for _ in 0..2 {
                    match b.get(j) {
                        Some(&n) if n.is_ascii_digit() => {
                            v = v * 10 + (n - b'0') as u32;
                            j += 1;
                        }
                        _ => break,
                    }
                }
                out.push(u8::try_from(v).map_err(|_| Error::lua(line, "bad decimal escape"))?);
            }
            _ => return Err(Error::lua(line, format!("bad escape \\{}", e as char))),
        }
    }
    Ok((String::from_utf8_lossy(&out).into_owned(), j, lines))
}

fn number(b: &[u8], i: usize) -> Option<(f64, usize)> {
    if b[i] == b'0' && matches!(b.get(i + 1), Some(b'x' | b'X')) {
        let mut j = i + 2;
        while j < b.len() && b[j].is_ascii_hexdigit() {
            j += 1;
        }
        let s = std::str::from_utf8(&b[i + 2..j]).ok()?;
        return Some((u64::from_str_radix(s, 16).ok()? as f64, j));
    }
    let mut j = i;
    while j < b.len() && (b[j].is_ascii_digit() || b[j] == b'.') {
        j += 1;
    }
    if j < b.len() && (b[j] == b'e' || b[j] == b'E') {
        j += 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
    }
    let s = std::str::from_utf8(&b[i..j]).ok()?;
    Some((s.parse().ok()?, j))
}

// ---------------------------------------------------------------- parser

struct Parser<'a> {
    t: &'a [Token],
    i: usize,
}

fn is_block_open(t: &Tok) -> bool {
    matches!(t, Tok::Name(n) if matches!(n.as_str(), "function" | "if" | "do" | "repeat"))
}

fn is_block_close(t: &Tok) -> bool {
    matches!(t, Tok::Name(n) if matches!(n.as_str(), "end" | "until"))
}

impl Parser<'_> {
    fn tok(&self) -> &Tok {
        &self.t[self.i].tok
    }
    fn peek(&self, n: usize) -> &Tok {
        &self.t[(self.i + n).min(self.t.len() - 1)].tok
    }
    fn line(&self) -> u32 {
        self.t[self.i].line
    }
    fn bump(&mut self) {
        if self.i < self.t.len() - 1 {
            self.i += 1;
        }
    }
    fn is_sym(&self, s: &str) -> bool {
        matches!(self.tok(), Tok::Sym(x) if *x == s)
    }

    fn chunk(mut self) -> Result<LuaChunk> {
        let mut globals = Vec::new();
        let mut block = 0i32;
        let mut paren = 0i32;
        while *self.tok() != Tok::Eof {
            let tok = self.tok().clone();
            if block == 0
                && paren == 0
                && let Tok::Name(name) = &tok
            {
                let prev_ok = self.i == 0
                    || !matches!(self.t[self.i - 1].tok, Tok::Sym("." | ":" | "," | "["));
                let is_kw = matches!(
                    name.as_str(),
                    "local" | "return" | "function" | "if" | "do" | "end" | "then" | "else"
                );
                if prev_ok && !is_kw && matches!(self.peek(1), Tok::Sym("=")) {
                    self.bump();
                    self.bump();
                    let v = self.value(true)?;
                    globals.push((name.clone(), v));
                    continue;
                }
            }
            match &tok {
                Tok::Sym("(" | "{" | "[") => paren += 1,
                Tok::Sym(")" | "}" | "]") => paren -= 1,
                t if is_block_open(t) => block += 1,
                t if is_block_close(t) => block -= 1,
                _ => {}
            }
            self.bump();
        }
        Ok(LuaChunk { globals })
    }

    /// Parses one value. `top` = statement level (an expression ends at a
    /// line break instead of a `,` / `}`).
    fn value(&mut self, top: bool) -> Result<Value> {
        let start = self.i;
        let lit = match self.tok().clone() {
            Tok::Sym("{") => Some(Value::Table(self.table()?)),
            Tok::Str(s) => {
                self.bump();
                Some(Value::Str(s))
            }
            Tok::Num(n) => {
                self.bump();
                Some(Value::Num(n))
            }
            Tok::Sym("-") if matches!(self.peek(1), Tok::Num(_)) => {
                let Tok::Num(n) = self.peek(1).clone() else {
                    unreachable!()
                };
                self.bump();
                self.bump();
                Some(Value::Num(-n))
            }
            Tok::Name(n) if n == "true" || n == "false" || n == "nil" => {
                self.bump();
                Some(match n.as_str() {
                    "true" => Value::Bool(true),
                    "false" => Value::Bool(false),
                    _ => Value::Nil,
                })
            }
            Tok::Name(n) if n == "function" => {
                self.skip_block()?;
                Some(Value::Function)
            }
            _ => None,
        };
        if let Some(v) = lit
            && self.at_value_end()
        {
            return Ok(v);
        }
        self.i = start;
        self.skip_expr(top)?;
        let text: Vec<String> = self.t[start..self.i]
            .iter()
            .map(|t| match &t.tok {
                Tok::Name(n) => n.clone(),
                Tok::Str(s) => format!("{s:?}"),
                Tok::Num(n) => n.to_string(),
                Tok::Sym(s) => s.to_string(),
                Tok::Eof => String::new(),
            })
            .collect();
        Ok(Value::Expr(text.join(" ")))
    }

    /// True if the literal just parsed is complete (not the start of a
    /// longer expression such as `1 + 2` or `"a" .. b`).
    fn at_value_end(&self) -> bool {
        match self.tok() {
            Tok::Eof => true,
            Tok::Sym("," | ";" | "}" | "]" | ")") => true,
            Tok::Sym(_) => false,
            // A name/literal cannot continue an expression that ended in a
            // literal: at statement level it starts the next statement, inside
            // a table it is a syntax error reported by the caller.
            _ => true,
        }
    }

    /// Skips `function ... end` (current token is `function`).
    fn skip_block(&mut self) -> Result<()> {
        let line = self.line();
        let mut depth = 0i32;
        loop {
            let t = self.tok();
            if *t == Tok::Eof {
                return Err(Error::lua(line, "unterminated block"));
            }
            if is_block_open(t) {
                depth += 1;
            } else if is_block_close(t) {
                depth -= 1;
                if depth == 0 {
                    self.bump();
                    return Ok(());
                }
            }
            self.bump();
        }
    }

    /// Skips an arbitrary expression.
    fn skip_expr(&mut self, top: bool) -> Result<()> {
        let line = self.line();
        let mut paren = 0i32;
        let mut block = 0i32;
        loop {
            let t = self.tok().clone();
            if t == Tok::Eof {
                return Ok(());
            }
            if paren == 0 && block == 0 {
                if matches!(t, Tok::Sym("," | ";" | "}" | ")" | "]")) {
                    return Ok(());
                }
                if top && self.line() > line && self.i > 0 {
                    // A new line at statement level starts a new statement
                    // unless the previous token clearly continues the expression.
                    let prev = &self.t[self.i - 1].tok;
                    let continues = matches!(prev, Tok::Sym(s) if !matches!(*s, ")" | "]" | "}"));
                    if !continues {
                        return Ok(());
                    }
                }
            }
            match &t {
                Tok::Sym("(" | "{" | "[") => paren += 1,
                Tok::Sym(")" | "}" | "]") => paren -= 1,
                t if is_block_open(t) => block += 1,
                t if is_block_close(t) => block -= 1,
                _ => {}
            }
            self.bump();
        }
    }

    fn table(&mut self) -> Result<Table> {
        let open_line = self.line();
        self.bump(); // {
        let mut entries = Vec::new();
        let mut pos = 0i64;
        loop {
            match self.tok().clone() {
                Tok::Sym("}") => {
                    self.bump();
                    return Ok(Table { entries });
                }
                Tok::Eof => return Err(Error::lua(open_line, "unterminated table")),
                Tok::Sym("," | ";") => {
                    self.bump();
                    continue;
                }
                Tok::Sym("[") => {
                    self.bump();
                    let k = self.value(false)?;
                    if !self.is_sym("]") {
                        return Err(Error::lua(self.line(), "expected ]"));
                    }
                    self.bump();
                    if !self.is_sym("=") {
                        return Err(Error::lua(self.line(), "expected = after [key]"));
                    }
                    self.bump();
                    let v = self.value(false)?;
                    let key = match k {
                        Value::Num(n) if n.fract() == 0.0 => Key::Index(n as i64),
                        Value::Str(s) => Key::Name(s),
                        other => Key::Other(Box::new(other)),
                    };
                    entries.push((key, v));
                }
                Tok::Name(n) if matches!(self.peek(1), Tok::Sym("=")) => {
                    self.bump();
                    self.bump();
                    let v = self.value(false)?;
                    entries.push((Key::Name(n), v));
                }
                _ => {
                    let v = self.value(false)?;
                    pos += 1;
                    entries.push((Key::Index(pos), v));
                }
            }
            // After a field: `,` `;` or `}`.
            if !matches!(self.tok(), Tok::Sym("," | ";" | "}")) {
                return Err(Error::lua(self.line(), "expected , or } in table"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals() {
        let c = LuaChunk::parse(
            r#"
            -- comment
            --[[ long
                 comment ]]
            a = 1
            b = -2.5
            c = "x\"y\n\65"
            d = [[long
string]]
            e = { 1, 2, three = 3, [10] = "ten", ["k k"] = { nested = true }; 4 }
            local f = 0x10
            "#,
        )
        .unwrap();
        assert_eq!(c.get("a"), Some(&Value::Num(1.0)));
        assert_eq!(c.get("b"), Some(&Value::Num(-2.5)));
        assert_eq!(c.get("c").unwrap().as_str(), Some("x\"y\nA"));
        assert_eq!(c.get("d").unwrap().as_str(), Some("long\nstring"));
        let e = c.get("e").unwrap().as_table().unwrap();
        assert_eq!(e.len(), 3);
        assert_eq!(e.index(3).unwrap().as_i64(), Some(4));
        assert_eq!(e.get("three").unwrap().as_i64(), Some(3));
        assert_eq!(e.index(10).unwrap().as_str(), Some("ten"));
        assert_eq!(
            e.get("k k").unwrap().as_table().unwrap().get("nested"),
            Some(&Value::Bool(true))
        );
        assert_eq!(c.get("f").unwrap().as_i64(), Some(16));
    }

    #[test]
    fn skips_code() {
        let c = LuaChunk::parse(
            r#"
            function foo(x)
                local inner = { a = 1 }
                if x then inner = 2 end
                for i = 1, 3 do x = i end
                return x
            end
            t = {
                f = function(u) local q = { 1 } return q end,
                g = mmf.newObject(3),
                h = 1 + 2,
                k = "after",
            }
            x.y = 5
            z = a .. "b"
            w = 7
            "#,
        )
        .unwrap();
        let names: Vec<_> = c.globals.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["t", "z", "w"]);
        let t = c.get("t").unwrap().as_table().unwrap();
        assert_eq!(t.get("f"), Some(&Value::Function));
        assert!(matches!(t.get("g"), Some(Value::Expr(_))));
        assert!(matches!(t.get("h"), Some(Value::Expr(_))));
        assert_eq!(t.get("k").unwrap().as_str(), Some("after"));
        assert!(matches!(c.get("z"), Some(Value::Expr(_))));
        assert_eq!(c.get("w").unwrap().as_i64(), Some(7));
    }

    #[test]
    fn errors_have_lines() {
        let e = LuaChunk::parse("a = {\n1,\n2 3 }").unwrap_err();
        assert!(
            matches!(e.kind, crate::ErrorKind::Lua { line: 3, .. }),
            "{e}"
        );
    }
}
