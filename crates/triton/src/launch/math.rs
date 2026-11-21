//! `=` scope: evaluate an arithmetic expression and hand the board its two
//! lines — the normalized expression ("1,248 ÷ 4") and the result ("312").
//!
//! Grammar: `+ - * / % ^`, unary minus and parens, right-associative `^` that
//! binds tighter than unary minus (`-2^2 = -4`). Integer literals may use
//! spaces or commas as thousand separators ("1 248", "1,248").
//!
//! Callers pass the field's *raw* text: a leading `=` forces evaluation, so
//! `= 5` still evaluates, while a bare `5` in `All` scope stays an app query —
//! the expression must contain an operator or parens without the sigil.

/// Evaluate the math in a launcher query. Returns `(expr_pretty,
/// result_pretty)` — integers comma-separated in the expression, plain in the
/// result; decimals trimmed — or `None` when the text is not evaluable math.
pub fn math(query: &str) -> Option<(String, String)> {
    let q = query.trim_start();
    let (forced, body) = match q.strip_prefix('=') {
        Some(rest) => (true, rest.trim()),
        None => (false, q),
    };
    let mut parser = Parser::new(body);
    let node = parser.expr().ok()?;
    parser.skip_ws();
    if parser.pos != parser.input.len() {
        return None; // trailing junk is not math
    }
    if !forced && matches!(node, Node::Num(..)) {
        return None; // a lone literal is a query, not a calculation
    }
    let value = node.eval()?;
    Some((pretty(&node), fmt_result(value)))
}

/// A parsed expression. `Num` keeps its normalized literal so [`pretty`] can
/// re-separate it; `Paren` keeps the user's grouping for the same reason.
#[derive(Debug)]
enum Node {
    Num(f64, String),
    Neg(Box<Node>),
    Paren(Box<Node>),
    Add(Box<Node>, Box<Node>),
    Sub(Box<Node>, Box<Node>),
    Mul(Box<Node>, Box<Node>),
    Div(Box<Node>, Box<Node>),
    Mod(Box<Node>, Box<Node>),
    Pow(Box<Node>, Box<Node>),
}

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            input: s.as_bytes(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn at(&self, i: usize) -> Option<u8> {
        self.input.get(i).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t')) {
            self.pos += 1;
        }
    }

    /// `+`/`-` terms.
    fn expr(&mut self) -> Result<Node, ()> {
        let mut left = self.term()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'+') => {
                    self.pos += 1;
                    left = Node::Add(Box::new(left), Box::new(self.term()?));
                }
                Some(b'-') => {
                    self.pos += 1;
                    left = Node::Sub(Box::new(left), Box::new(self.term()?));
                }
                _ => return Ok(left),
            }
        }
    }

    /// `*`/`×`, `/`/`÷`, `%` factors.
    fn term(&mut self) -> Result<Node, ()> {
        let mut left = self.unary()?;
        loop {
            self.skip_ws();
            // `×` (U+00D7) and `÷` (U+00F7) share the 0xC3 lead byte.
            let op: Option<(fn(Box<Node>, Box<Node>) -> Node, usize)> = match self.peek() {
                Some(b'*') => Some((Node::Mul, 1)),
                Some(b'/') => Some((Node::Div, 1)),
                Some(b'%') => Some((Node::Mod, 1)),
                Some(0xC3) => match self.at(self.pos + 1) {
                    Some(0x97) => Some((Node::Mul, 2)),
                    Some(0xB7) => Some((Node::Div, 2)),
                    _ => None,
                },
                _ => None,
            };
            let Some((op, len)) = op else {
                return Ok(left);
            };
            self.pos += len;
            left = op(Box::new(left), Box::new(self.unary()?));
        }
    }

    /// `-x` binds looser than `^` but tighter than `*`: `-2^2 = -(2^2)`.
    fn unary(&mut self) -> Result<Node, ()> {
        self.skip_ws();
        if self.peek() == Some(b'-') {
            self.pos += 1;
            return Ok(Node::Neg(Box::new(self.unary()?)));
        }
        self.power()
    }

    /// `^`, right-associative (`2^3^2 = 2^(3^2)`), exponent may be unary.
    fn power(&mut self) -> Result<Node, ()> {
        let base = self.primary()?;
        self.skip_ws();
        if self.peek() == Some(b'^') {
            self.pos += 1;
            let exp = self.unary()?;
            return Ok(Node::Pow(Box::new(base), Box::new(exp)));
        }
        Ok(base)
    }

    fn primary(&mut self) -> Result<Node, ()> {
        self.skip_ws();
        match self.peek() {
            Some(b'(') => {
                self.pos += 1;
                let inner = self.expr()?;
                self.skip_ws();
                if self.peek() != Some(b')') {
                    return Err(());
                }
                self.pos += 1;
                Ok(Node::Paren(Box::new(inner)))
            }
            Some(c) if c.is_ascii_digit() || c == b'.' => self.number(),
            _ => Err(()),
        }
    }

    /// Digits with optional `.` fraction; `,` and ` ` inside the integer part
    /// are thousand separators ("1,248" / "1 248" both lex as `1248`).
    fn number(&mut self) -> Result<Node, ()> {
        let start = self.pos;
        let mut lit = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                lit.push(c as char);
                self.pos += 1;
            } else if matches!(c, b',' | b' ')
                && self.at(self.pos + 1).is_some_and(|d| d.is_ascii_digit())
            {
                self.pos += 1; // a separator, drop it
            } else {
                break;
            }
        }
        if self.peek() == Some(b'.') {
            lit.push('.');
            self.pos += 1;
            while self.peek().is_some_and(|d| d.is_ascii_digit()) {
                lit.push(self.input[self.pos] as char);
                self.pos += 1;
            }
        }
        if lit.is_empty() || lit == "." {
            self.pos = start;
            return Err(());
        }
        lit.parse::<f64>()
            .map(|v| Node::Num(v, lit))
            .map_err(|_| ())
    }
}

impl Node {
    /// `None` on division/modulo by zero and non-finite results
    /// (`0^0` yields 1 here, `0^-1` is rejected as non-finite).
    fn eval(&self) -> Option<f64> {
        let v = match self {
            Self::Num(v, _) => *v,
            Self::Neg(e) => -e.eval()?,
            Self::Paren(e) => e.eval()?,
            Self::Add(l, r) => l.eval()? + r.eval()?,
            Self::Sub(l, r) => l.eval()? - r.eval()?,
            Self::Mul(l, r) => l.eval()? * r.eval()?,
            Self::Div(l, r) => {
                let d = r.eval()?;
                if d == 0.0 {
                    return None;
                }
                l.eval()? / d
            }
            Self::Mod(l, r) => {
                let d = r.eval()?;
                if d == 0.0 {
                    return None;
                }
                l.eval()? % d
            }
            Self::Pow(l, r) => l.eval()?.powf(r.eval()?),
        };
        v.is_finite().then_some(v)
    }
}

/// Precedence for pretty-printing; a child renders parentheses when its own
/// level falls below the context it is drawn in.
fn prec(node: &Node) -> u8 {
    match node {
        Node::Num(..) | Node::Paren(_) => 5,
        Node::Pow(..) => 4,
        Node::Neg(_) => 3,
        Node::Mul(..) | Node::Div(..) | Node::Mod(..) => 2,
        Node::Add(..) | Node::Sub(..) => 1,
    }
}

/// The board's normalized expression: `×`/`÷` glyphs, comma-separated integer
/// literals, single spaces, parens kept where they matter.
fn pretty(node: &Node) -> String {
    fn write(node: &Node, min_prec: u8, out: &mut String) {
        if prec(node) < min_prec {
            out.push('(');
            write(node, 0, out);
            out.push(')');
            return;
        }
        match node {
            Node::Num(_, lit) => out.push_str(&commas(lit)),
            Node::Paren(inner) => {
                out.push('(');
                write(inner, 0, out);
                out.push(')');
            }
            Node::Neg(inner) => {
                out.push('-');
                write(inner, 4, out);
            }
            Node::Add(l, r) => binary(l, " + ", r, 1, 2, out),
            Node::Sub(l, r) => binary(l, " - ", r, 1, 2, out),
            Node::Mul(l, r) => binary(l, " × ", r, 2, 3, out),
            Node::Div(l, r) => binary(l, " ÷ ", r, 2, 3, out),
            Node::Mod(l, r) => binary(l, " % ", r, 2, 3, out),
            // `^` is right-associative: its right child needs no parens at the
            // same level, its left child must be a primary.
            Node::Pow(l, r) => binary(l, " ^ ", r, 5, 4, out),
        }
    }
    fn binary(l: &Node, op: &str, r: &Node, lp: u8, rp: u8, out: &mut String) {
        write(l, lp, out);
        out.push_str(op);
        write(r, rp, out);
    }
    let mut out = String::new();
    write(node, 0, &mut out);
    out
}

/// "1248.5" → "1,248.5": separators in the integer part only.
fn commas(lit: &str) -> String {
    let (int, frac) = match lit.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (lit, None),
    };
    let mut out = String::new();
    for (i, c) in int.char_indices() {
        // `as` casts below are on ASCII byte counts.
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if let Some(frac) = frac {
        out.push('.');
        out.push_str(frac);
    }
    out
}

/// The board's result line: integers unseparated ("312"), decimals trimmed
/// ("0.3" for `0.1 + 0.2`), magnitudes past `1e15` in exponent form.
fn fmt_result(v: f64) -> String {
    // Integral values — including -0 — take the unseparated integer path.
    if v.fract() == 0.0 && v.abs() < 1e15 {
        return (v as i64).to_string();
    }
    if v.abs() >= 1e15 {
        return format!("{v:.4e}");
    }
    let trimmed = format!("{v:.10}");
    let trimmed = trimmed.trim_end_matches('0').trim_end_matches('.');
    // Under 1e-10 the fixed format rounds to all zeros; go exponential.
    if trimmed.is_empty() || trimmed == "0" || trimmed == "-0" {
        format!("{v:.4e}")
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(query: &str) -> Option<String> {
        math(query).map(|(_, r)| r)
    }

    fn expr(query: &str) -> Option<String> {
        math(query).map(|(e, _)| e)
    }

    #[test]
    fn operators_and_precedence() {
        assert_eq!(result("1 + 2 * 3").as_deref(), Some("7"));
        assert_eq!(result("(1 + 2) * 3").as_deref(), Some("9"));
        assert_eq!(result("2 ^ 3 ^ 2").as_deref(), Some("512")); // right-assoc
        assert_eq!(result("10 % 3").as_deref(), Some("1"));
        assert_eq!(result("-2 ^ 2").as_deref(), Some("-4")); // ^ over unary
        assert_eq!(result("2 ^ -1").as_deref(), Some("0.5"));
        assert_eq!(result("--3").as_deref(), Some("3"));
    }

    #[test]
    fn the_board_example() {
        assert_eq!(
            math("1248 / 4"),
            Some(("1,248 ÷ 4".to_owned(), "312".to_owned()))
        );
    }

    #[test]
    fn thousand_separators() {
        assert_eq!(result("1 248 / 4").as_deref(), Some("312"));
        assert_eq!(result("1,248 / 4").as_deref(), Some("312"));
        assert_eq!(result("1,234,567 + 1").as_deref(), Some("1234568"));
    }

    #[test]
    fn equals_prefix_forces_math() {
        assert_eq!(math("= 5"), Some(("5".to_owned(), "5".to_owned())));
        assert_eq!(math("5"), None);
        assert_eq!(expr("= 1248/4").as_deref(), Some("1,248 ÷ 4"));
    }

    #[test]
    fn non_math_is_none() {
        for q in ["", "=", "con", "1 +", "(1 + 2", "1 / 0", "5 % 0", "abc(", "1 2 ("] {
            assert_eq!(math(q), None, "{q}");
        }
    }

    #[test]
    fn decimals_are_trimmed() {
        assert_eq!(result("0.1 + 0.2").as_deref(), Some("0.3"));
        assert_eq!(result("1.50 * 2").as_deref(), Some("3"));
        assert_eq!(result("1 / 3").as_deref(), Some("0.3333333333"));
    }

    #[test]
    fn pretty_normalizes_glyphs_and_spacing() {
        assert_eq!(expr("2*3+4").as_deref(), Some("2 × 3 + 4"));
        assert_eq!(expr("(1+2)*3").as_deref(), Some("(1 + 2) × 3"));
        assert_eq!(expr("10-2-3").as_deref(), Some("10 - 2 - 3"));
        assert_eq!(expr("10-(2-3)").as_deref(), Some("10 - (2 - 3)"));
        assert_eq!(expr("2*(3+4)").as_deref(), Some("2 × (3 + 4)"));
        assert_eq!(expr("-3+1").as_deref(), Some("-3 + 1"));
        assert_eq!(expr("-(2+3)").as_deref(), Some("-(2 + 3)"));
        assert_eq!(expr("12/3/2").as_deref(), Some("12 ÷ 3 ÷ 2"));
        assert_eq!(expr("12/(3/2)").as_deref(), Some("12 ÷ (3 ÷ 2)"));
        assert_eq!(expr("2^10").as_deref(), Some("2 ^ 10"));
    }
}
