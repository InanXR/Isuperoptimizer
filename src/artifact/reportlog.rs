//! Machine-readable run log (JSONL) — the artifact of record.
//!
//! Why this exists: an early prose report mis-transcribed a counterexample
//! input while quoting the true model's *outputs* — an internally inconsistent
//! artifact that hand-verification correctly rejected. The fix is structural:
//! the harness writes every report-quotable fact here (verdicts with full
//! counterexample details, mode summaries with the real numbers), reports
//! paste from this file instead of retyping, and `recheck --from-log`
//! re-verifies every logged counterexample against the concrete evaluator.
//!
//! Zero dependencies (ADR-005), so this module contains a tiny JSON *emitter*
//! and a strict parser for exactly the subset we emit: flat objects with
//! string / u64 / bool / string-array values, one per line. Anything outside
//! that subset fails loudly — log-format drift must never be silent.

use std::fs::{create_dir_all, File};
use std::io::{BufWriter, Write};
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub enum JVal {
    S(String),
    U(u64),
    B(bool),
    ArrS(Vec<String>),
}

pub use JVal::S as s_str; // convenience alias

fn escape_into(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
}

fn encode(v: &JVal) -> String {
    match v {
        JVal::S(s) => {
            let mut out = String::with_capacity(s.len() + 2);
            escape_into(&mut out, s);
            format!("\"{}\"", out)
        }
        JVal::U(n) => format!("{}", n),
        JVal::B(b) => format!("{}", b),
        JVal::ArrS(a) => {
            let inner: Vec<String> = a
                .iter()
                .map(|s| {
                    let mut out = String::with_capacity(s.len() + 2);
                    escape_into(&mut out, s);
                    format!("\"{}\"", out)
                })
                .collect();
            format!("[{}]", inner.join(","))
        }
    }
}

pub struct ReportLog {
    out: Option<BufWriter<File>>,
    pub path: String,
}

impl ReportLog {
    /// Open (truncate) the log file. On failure, warn loudly and run
    /// disabled — the checks still run, but the summary states the path.
    pub fn open(path: &str) -> ReportLog {
        if let Some(parent) = Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                let _ = create_dir_all(parent);
            }
        }
        match File::create(path) {
            Ok(f) => ReportLog { out: Some(BufWriter::new(f)), path: path.to_string() },
            Err(e) => {
                eprintln!(
                    "[reportlog] WARNING: cannot open `{}` for writing ({}); \
                     running WITHOUT an artifact of record",
                    path, e
                );
                ReportLog { out: None, path: path.to_string() }
            }
        }
    }

    pub fn event(&mut self, fields: &[(&str, JVal)]) {
        if let Some(w) = self.out.as_mut() {
            let mut line = String::from("{");
            for (i, (k, v)) in fields.iter().enumerate() {
                if i > 0 {
                    line.push(',');
                }
                let mut kesc = String::new();
                escape_into(&mut kesc, k);
                line.push_str(&format!("\"{}\":{}", kesc, encode(v)));
            }
            line.push('}');
            let _ = writeln!(w, "{}", line);
        }
    }

    pub fn flush(&mut self) {
        if let Some(w) = self.out.as_mut() {
            let _ = w.flush();
        }
    }
}

// ---------------------------------------------------------------- parser

/// Strict parser for the subset `ReportLog` emits: one flat JSON object per
/// line with S/U/B/ArrS values. Returns an error on anything else.
pub fn parse_jsonl_line(line: &str) -> Result<Vec<(String, JVal)>, String> {
    let mut p = P { s: line.as_bytes(), i: 0 };
    p.ws();
    p.eat(b'{')?;
    let mut out = Vec::new();
    p.ws();
    if p.peek() == Some(b'}') {
        p.i += 1;
        p.ws();
        if p.i != p.s.len() {
            return Err(format!("trailing bytes after object: {:?}", &line[p.i..]));
        }
        return Ok(out);
    }
    loop {
        p.ws();
        let key = p.string()?;
        p.ws();
        p.eat(b':')?;
        p.ws();
        let val = p.value()?;
        out.push((key, val));
        p.ws();
        match p.peek() {
            Some(b',') => {
                p.i += 1;
            }
            Some(b'}') => {
                p.i += 1;
                break;
            }
            other => return Err(format!("expected ',' or '}}', got {:?}", other)),
        }
    }
    p.ws();
    if p.i != p.s.len() {
        return Err(format!("trailing bytes after object: {:?}", &line[p.i..]));
    }
    Ok(out)
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> P<'a> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn eat(&mut self, c: u8) -> Result<(), String> {
        match self.peek() {
            Some(x) if x == c => {
                self.i += 1;
                Ok(())
            }
            other => Err(format!("expected `{}`, got {:?}", c as char, other)),
        }
    }
    fn string(&mut self) -> Result<String, String> {
        self.eat(b'"')?;
        let mut buf: Vec<u8> = Vec::new();
        loop {
            match self.peek() {
                None => return Err("unterminated string".into()),
                Some(b'"') => {
                    self.i += 1;
                    return String::from_utf8(buf).map_err(|e| e.to_string());
                }
                Some(b'\\') => {
                    self.i += 1;
                    match self.peek() {
                        Some(b'n') => { buf.push(b'\n'); self.i += 1; }
                        Some(b't') => { buf.push(b'\t'); self.i += 1; }
                        Some(b'r') => { buf.push(b'\r'); self.i += 1; }
                        Some(b'"') => { buf.push(b'"'); self.i += 1; }
                        Some(b'\\') => { buf.push(b'\\'); self.i += 1; }
                        Some(b'u') => {
                            self.i += 1;
                            if self.i + 4 > self.s.len() {
                                return Err("truncated \\u escape".into());
                            }
                            let hex = std::str::from_utf8(&self.s[self.i..self.i + 4])
                                .map_err(|e| e.to_string())?;
                            let cp = u32::from_str_radix(hex, 16)
                                .map_err(|e| format!("bad \\u escape: {}", e))?;
                            let ch = char::from_u32(cp).ok_or("bad \\u codepoint")?;
                            let mut b = [0u8; 4];
                            buf.extend_from_slice(ch.encode_utf8(&mut b).as_bytes());
                            self.i += 4;
                        }
                        other => return Err(format!("bad escape {:?}", other)),
                    }
                }
                Some(_) => {
                    buf.push(self.s[self.i]);
                    self.i += 1;
                }
            }
        }
    }
    fn value(&mut self) -> Result<JVal, String> {
        match self.peek() {
            Some(b'"') => Ok(JVal::S(self.string()?)),
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.peek() == Some(b']') {
                    self.i += 1;
                    return Ok(JVal::ArrS(items));
                }
                loop {
                    self.ws();
                    items.push(self.string()?);
                    self.ws();
                    match self.peek() {
                        Some(b',') => { self.i += 1; }
                        Some(b']') => { self.i += 1; break; }
                        other => return Err(format!("expected ',' or ']' in array, got {:?}", other)),
                    }
                }
                Ok(JVal::ArrS(items))
            }
            Some(b't') => {
                if self.s[self.i..].starts_with(b"true") {
                    self.i += 4;
                    Ok(JVal::B(true))
                } else {
                    Err("bad literal".into())
                }
            }
            Some(b'f') => {
                if self.s[self.i..].starts_with(b"false") {
                    self.i += 5;
                    Ok(JVal::B(false))
                } else {
                    Err("bad literal".into())
                }
            }
            Some(c) if c.is_ascii_digit() => {
                let start = self.i;
                while self.i < self.s.len() && self.s[self.i].is_ascii_digit() {
                    self.i += 1;
                }
                let txt = std::str::from_utf8(&self.s[start..self.i]).map_err(|e| e.to_string())?;
                txt.parse::<u64>().map(JVal::U).map_err(|e| e.to_string())
            }
            other => Err(format!("unsupported value at offset {}: {:?}", self.i, other)),
        }
    }
}

/// Field accessor for parsed lines.
pub fn get<'a>(fields: &'a [(String, JVal)], key: &str) -> Option<&'a JVal> {
    fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emit_then_parse_roundtrip() {
        let dir = std::env::temp_dir().join("isuperopt-reportlog-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("l.jsonl");
        let path_str = path.to_str().unwrap();
        {
            let mut log = ReportLog::open(path_str);
            log.event(&[
                ("kind", JVal::S("pair_verdict".into())),
                ("target", JVal::S("movq %rax, %rcx\nshrq $1, %rcx".into())),
                ("live_outs", JVal::ArrS(vec!["rcx".into(), "r8".into()])),
                ("n", JVal::U(18446744073709551615)),
                ("ok", JVal::B(true)),
            ]);
            log.event(&[("kind", JVal::S("empty_obj".into()))]);
            log.flush();
        }
        let content = std::fs::read_to_string(&path).unwrap();
        let mut lines = content.lines();
        let f1 = parse_jsonl_line(lines.next().unwrap()).unwrap();
        assert_eq!(get(&f1, "kind"), Some(&JVal::S("pair_verdict".into())));
        assert_eq!(get(&f1, "target"), Some(&JVal::S("movq %rax, %rcx\nshrq $1, %rcx".into())));
        assert_eq!(get(&f1, "live_outs"), Some(&JVal::ArrS(vec!["rcx".into(), "r8".into()])));
        assert_eq!(get(&f1, "n"), Some(&JVal::U(u64::MAX)));
        assert_eq!(get(&f1, "ok"), Some(&JVal::B(true)));
        let f2 = parse_jsonl_line(lines.next().unwrap()).unwrap();
        assert_eq!(f2, vec![("kind".to_string(), JVal::S("empty_obj".into()))]);
        assert!(lines.next().is_none());
    }

    #[test]
    fn rejects_foreign_shapes() {
        assert!(parse_jsonl_line("[1,2]").is_err());
        assert!(parse_jsonl_line("{\"a\":{\"b\":1}}").is_err()); // nested object
        assert!(parse_jsonl_line("{\"a\":1} trailing").is_err());
        assert!(parse_jsonl_line("{\"a\":-1}").is_err()); // negatives not emitted
    }
}
