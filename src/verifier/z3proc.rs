//! Persistent z3 subprocess speaking SMT-LIB2 over pipes (ADR-005).
//!
//! Protocol: `(set-option :print-success true)` makes z3 answer every command
//! with exactly one response — `success`, `sat`/`unsat`/`unknown` for
//! check-sat, an s-expression for get-value, or `(error "...")` — which gives
//! us a deterministic, checkable wire format (Online Z3 Guide, "Basic
//! Commands"). One long-lived process per session; queries run inside
//! `(push 1)` ... `(pop 1)` scopes.

use std::io::{BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

/// Result of a solve round-trip.
pub enum Answer {
    Unsat,
    Unknown,
    /// SAT; carries the requested values (empty if none were requested).
    Sat(Vec<(String, u64)>),
}

pub struct Z3Session {
    bin_path: String,
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    queries: u64,
}

impl Z3Session {
    pub fn new() -> Result<Z3Session, String> {
        Z3Session::new_with_timeout(60_000)
    }

    /// A session with a per-query timeout (ms). The MCMC chains use a short
    /// one: `unknown` on a candidate means "cannot confirm within budget",
    /// never "inequivalent" — final results are re-verified long-timeout.
    pub fn new_with_timeout(ms: u64) -> Result<Z3Session, String> {
        let bin = find_z3()?;
        let mut s = Z3Session::spawn(&bin)?;
        s.bin_path = bin;
        let timeout_ms = ms.to_string();
        let _ = s.send_raw(format!("(set-option :timeout {})", timeout_ms).as_str());
        Ok(s)
    }

    /// Send without treating `(error ...)`/`unsupported` as fatal (init
    /// options only).
    fn send_raw(&mut self, cmd: &str) -> Result<String, String> {
        self.stdin
            .write_all(cmd.as_bytes())
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush())
            .map_err(|e| format!("z3 stdin write failed: {}", e))?;
        self.read_response()
    }

    fn spawn(bin: &str) -> Result<Z3Session, String> {
        let mut child = Command::new(bin)
            .arg("-in")
            .arg("-smt2")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("failed to spawn z3 at `{}`: {}", bin, e))?;
        let stdin = child.stdin.take().ok_or("no stdin on z3 child")?;
        let stdout = child.stdout.take().ok_or("no stdout on z3 child")?;
        let mut s = Z3Session {
            bin_path: bin.to_string(),
            child,
            stdin,
            stdout: BufReader::new(stdout),
            queries: 0,
        };
        // Init sequence. print-success is required for the response protocol;
        // the others are conveniences and must not fail init if unsupported.
        s.send("(set-option :print-success true)")?;
        if let Err(e) = s.send("(set-option :timeout 60000)") {
            eprintln!("[z3proc] warning: no timeout option: {}", e);
        }
        s.send("(set-logic QF_AUFBV)")?;
        Ok(s)
    }

    /// Recover from a poisoned session by respawning.
    pub fn restart(&mut self) -> Result<(), String> {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let fresh = Z3Session::spawn(&self.bin_path)?;
        *self = fresh;
        Ok(())
    }

    pub fn queries(&self) -> u64 {
        self.queries
    }

    fn send(&mut self, cmd: &str) -> Result<String, String> {
        self.stdin
            .write_all(cmd.as_bytes())
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush())
            .map_err(|e| format!("z3 stdin write failed: {}", e))?;
        let resp = self.read_response()?;
        if resp.starts_with("(error") {
            return Err(format!("z3 rejected `{}`: {}", truncate(cmd), resp));
        }
        if resp == "unsupported" {
            // z3's answer to unknown commands (observed: `unsupported` plus a
            // `; name line: L ...` comment line). A verifier must never
            // silently accept an unsupported command.
            return Err(format!("z3 does not support command `{}`", truncate(cmd)));
        }
        Ok(resp)
    }

    /// Read exactly one response: a balanced s-expression or a bare atom.
    /// `;`-comment lines (z3 emits them for some warnings) are skipped.
    fn read_response(&mut self) -> Result<String, String> {
        // skip whitespace and comment lines
        let first = loop {
            let mut b = [0u8; 1];
            self.stdout
                .read_exact(&mut b)
                .map_err(|e| format!("z3 stdout closed: {}", e))?;
            if b[0] == b';' {
                // consume through end of line
                let mut c = [0u8; 1];
                loop {
                    self.stdout
                        .read_exact(&mut c)
                        .map_err(|e| format!("z3 stdout closed in comment: {}", e))?;
                    if c[0] == b'\n' {
                        break;
                    }
                }
                continue;
            }
            if !b[0].is_ascii_whitespace() {
                break b[0];
            }
        };
        let mut buf = vec![first];
        if first == b'(' {
            let mut depth = 1usize;
            while depth > 0 {
                let mut b = [0u8; 1];
                self.stdout
                    .read_exact(&mut b)
                    .map_err(|e| format!("z3 stdout closed mid-expression: {}", e))?;
                if b[0] == b'(' {
                    depth += 1;
                } else if b[0] == b')' {
                    depth -= 1;
                }
                buf.push(b[0]);
            }
        } else {
            loop {
                let mut b = [0u8; 1];
                self.stdout
                    .read_exact(&mut b)
                    .map_err(|e| format!("z3 stdout closed mid-atom: {}", e))?;
                if b[0].is_ascii_whitespace() {
                    break;
                }
                buf.push(b[0]);
            }
        }
        String::from_utf8(buf).map_err(|e| format!("z3 response not utf8: {}", e))
    }

    /// Run one query: push scope, run `commands` (declares/asserts), check-sat,
    /// optionally get-value, pop. Every z3 error is a hard failure — a
    /// verifier must never swallow a solver error.
    pub fn solve(
        &mut self,
        commands: &[String],
        want: Option<&[String]>,
    ) -> Result<Answer, String> {
        self.queries += 1;
        self.send("(push 1)")?;
        let run = (|| -> Result<Answer, String> {
            for c in commands {
                self.send(c)?;
            }
            let res = self.send("(check-sat)")?;
            match res.as_str() {
                "unsat" => Ok(Answer::Unsat),
                "unknown" => Ok(Answer::Unknown),
                "sat" => {
                    if let Some(names) = want {
                        if !names.is_empty() {
                            let cmd = format!("(get-value ({}))", names.join(" "));
                            let resp = self.send(&cmd)?;
                            let vals = parse_values(&resp)?;
                            if vals.len() != names.len() {
                                return Err(format!(
                                    "get-value returned {} of {} requested values",
                                    vals.len(),
                                    names.len()
                                ));
                            }
                            return Ok(Answer::Sat(vals));
                        }
                    }
                    Ok(Answer::Sat(vec![]))
                }
                other => Err(format!("unexpected check-sat response `{}`", other)),
            }
        })();
        // Always try to restore the stack before reporting the outcome.
        let pop = self.send("(pop 1)");
        match (run, pop) {
            (Ok(a), Ok(_)) => Ok(a),
            (Ok(_), Err(e)) => Err(format!("pop failed after query: {}", e)),
            (Err(e), _) => Err(e),
        }
    }
}

impl Drop for Z3Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn truncate(s: &str) -> &str {
    if s.len() > 120 {
        &s[..120]
    } else {
        s
    }
}

/// Parse a `(get-value ...)` response: `((name #x...) (name2 true) ...)`.
/// The response is one outer list of `(term value)` pairs; we scan for pairs
/// at depth 2 (inside the outer wrapper) and split each on its first
/// whitespace into identifier and literal value.
fn parse_values(resp: &str) -> Result<Vec<(String, u64)>, String> {
    let b = resp.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut depth = 0usize;
    while i < b.len() {
        match b[i] {
            b'(' => {
                depth += 1;
                if depth == 2 {
                    // start of a `(term value)` pair: read to its matching ')'
                    let mut d = 1usize;
                    let mut j = i + 1;
                    while j < b.len() && d > 0 {
                        if b[j] == b'(' {
                            d += 1;
                        } else if b[j] == b')' {
                            d -= 1;
                        }
                        j += 1;
                    }
                    if d != 0 {
                        return Err(format!("unbalanced get-value response: {}", truncate(resp)));
                    }
                    let inner = &resp[i + 1..j - 1];
                    let trimmed = inner.trim();
                    if trimmed.starts_with("error") {
                        return Err(format!("get-value error: {}", trimmed));
                    }
                    if let Some(space) = trimmed.find(|c: char| c.is_whitespace()) {
                        let name = trimmed[..space].trim();
                        let value_s = trimmed[space..].trim();
                        let v = parse_bv_or_bool(value_s)
                            .map_err(|e| format!("{} (in response `{}`)", e, truncate(resp)))?;
                        out.push((name.to_string(), v));
                    }
                    i = j;
                    depth -= 1; // the pair's ')' is consumed: back to depth 1
                    continue;
                }
                i += 1;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                i += 1;
            }
            _ => i += 1,
        }
    }
    if out.is_empty() {
        return Err(format!("could not parse get-value response: {}", truncate(resp)));
    }
    Ok(out)
}

fn parse_bv_or_bool(s: &str) -> Result<u64, String> {
    if let Some(hex) = s.strip_prefix("#x") {
        u64::from_str_radix(hex, 16).map_err(|e| format!("bad hex value `{}`: {}", s, e))
    } else if let Some(bin) = s.strip_prefix("#b") {
        u64::from_str_radix(bin, 2).map_err(|e| format!("bad bin value `{}`: {}", s, e))
    } else if s == "true" {
        Ok(1)
    } else if s == "false" {
        Ok(0)
    } else {
        s.parse::<u64>().map_err(|e| format!("unexpected value token `{}`: {}", s, e))
    }
}

/// Locate the z3 binary: `ISUPEROPT_Z3` first, then `z3` on PATH.
pub fn find_z3() -> Result<String, String> {
    if let Ok(p) = std::env::var("ISUPEROPT_Z3") {
        if std::path::Path::new(&p).exists() {
            return Ok(p);
        }
        return Err(format!("ISUPEROPT_Z3={} does not exist", p));
    }
    if let Ok(out) = Command::new("z3").arg("--version").output() {
        if out.status.success() {
            return Ok("z3".to_string());
        }
    }
    Err("z3 not found: set ISUPEROPT_Z3 or install z3 on PATH".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_unsat() {
        let mut z = Z3Session::new().expect("z3 must be available for tests");
        // x=5 and x+3=9 are contradictory: unsat.
        let cmds = vec![
            "(declare-const x (_ BitVec 64))".to_string(),
            "(assert (= x #x0000000000000005))".to_string(),
            "(assert (= (bvadd x #x0000000000000003) #x0000000000000009))".to_string(),
        ];
        match z.solve(&cmds, None).unwrap() {
            Answer::Unsat => {}
            _ => panic!("contradictory constraints must be unsat"),
        }
    }

    #[test]
    fn session_sat_unsat_and_values() {
        let mut z = Z3Session::new().expect("z3 must be available for tests");
        let cmds = vec![
            "(declare-const x (_ BitVec 64))".to_string(),
            "(assert (= x #x0000000000000005))".to_string(),
        ];
        let names = vec!["x".to_string()];
        match z.solve(&cmds, Some(&names)).unwrap() {
            Answer::Sat(vals) => {
                assert_eq!(vals, vec![("x".to_string(), 5)]);
            }
            _ => panic!("expected sat with x=5"),
        }
        // Multiple names must all come back (regression: the pair scanner once
        // consumed only the first pair).
        let cmds = vec![
            "(declare-const a (_ BitVec 64))".to_string(),
            "(declare-const b (_ BitVec 64))".to_string(),
            "(declare-const c (_ BitVec 64))".to_string(),
            "(declare-const p Bool)".to_string(),
            "(assert (= a #x0000000000000001))".to_string(),
            "(assert (= b #x0000000000000002))".to_string(),
            "(assert (= c #x0000000000000003))".to_string(),
            "(assert (= p (bvult a b)))".to_string(),
        ];
        let names = vec!["a".to_string(), "b".to_string(), "c".to_string(), "p".to_string()];
        match z.solve(&cmds, Some(&names)).unwrap() {
            Answer::Sat(vals) => {
                assert_eq!(vals.len(), 4);
                let m: std::collections::HashMap<_, _> = vals.into_iter().collect();
                assert_eq!(m["a"], 1);
                assert_eq!(m["b"], 2);
                assert_eq!(m["c"], 3);
                assert_eq!(m["p"], 1); // 1 < 2
            }
            _ => panic!("expected sat"),
        }
        // Scopes must have been restored: a contradictory assert at top level
        // in a *new* push scope must be unsat, and the stack must not leak.
        let cmds = vec![
            "(declare-const y (_ BitVec 64))".to_string(),
            "(assert (= y #x0000000000000001))".to_string(),
            "(assert (= y #x0000000000000002))".to_string(),
        ];
        match z.solve(&cmds, None).unwrap() {
            Answer::Unsat => {}
            _ => panic!("expected unsat for y=1 and y=2"),
        }
    }

    #[test]
    fn error_responses_are_hard_failures() {
        let mut z = Z3Session::new().expect("z3 must be available for tests");
        let bad = vec!["(declare-const x (_ BitVec 65))".to_string(), "(bogus-command)".to_string()];
        let r = z.solve(&bad, None);
        assert!(r.is_err(), "solver errors must surface, got {:?}", r.is_ok());
        // Session must still be usable after the error was reported.
        let cmds = vec!["(declare-const q (_ BitVec 8))".to_string()];
        assert!(z.solve(&cmds, None).is_ok());
    }
}
