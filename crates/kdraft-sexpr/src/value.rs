//! Owned S-expression values and the canonical printer.
//!
//! Canonical form: one top-level value per line group, two-space indent.
//! A list whose elements are all atoms prints on one line. A list with list
//! elements prints its leading atoms on the first line, then each remaining
//! element on its own line one level deeper, then `)` on its own line.
//! Numbers print as the shortest decimal that round-trips, without exponent
//! or trailing zeros. Atoms are quoted only when needed unless created with
//! [`Atom::string`].

use crate::error::{Error, ErrorKind};
use crate::escape::{escape_into, needs_quotes};
use crate::tree::{parse_f64, Tree};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Atom {
    text: String,
    quoted: bool,
}

impl Atom {
    /// An atom quoted only if its text requires it.
    pub fn symbol(text: impl Into<String>) -> Atom {
        let text = text.into();
        let quoted = needs_quotes(&text);
        Atom { text, quoted }
    }

    /// An atom that is always quoted.
    pub fn string(text: impl Into<String>) -> Atom {
        Atom {
            text: text.into(),
            quoted: true,
        }
    }

    pub fn number(v: f64) -> Result<Atom, Error> {
        Ok(Atom::symbol(format_number(v)?))
    }

    pub fn int(v: i64) -> Atom {
        Atom::symbol(v.to_string())
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_quoted(&self) -> bool {
        self.quoted
    }
}

/// Formats a finite number canonically: shortest round-trip decimal, no
/// exponent, no trailing zeros, `-0` written as `0`.
pub fn format_number(v: f64) -> Result<String, Error> {
    if !v.is_finite() {
        return Err(Error::unpositioned(ErrorKind::NonFinite));
    }
    if v == 0.0 {
        return Ok("0".to_string());
    }
    // Rust's `Display` for f64 is shortest-round-trip and never uses an
    // exponent.
    Ok(format!("{v}"))
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Value {
    Atom(Atom),
    List(Vec<Value>),
}

impl Value {
    pub fn sym(text: impl Into<String>) -> Value {
        Value::Atom(Atom::symbol(text))
    }

    pub fn str(text: impl Into<String>) -> Value {
        Value::Atom(Atom::string(text))
    }

    pub fn num(v: f64) -> Result<Value, Error> {
        Ok(Value::Atom(Atom::number(v)?))
    }

    pub fn int(v: i64) -> Value {
        Value::Atom(Atom::int(v))
    }

    pub fn list(items: impl IntoIterator<Item = Value>) -> Value {
        Value::List(items.into_iter().collect())
    }

    /// `(head items...)`
    pub fn node(head: &str, items: impl IntoIterator<Item = Value>) -> Value {
        let mut v = vec![Value::sym(head)];
        v.extend(items);
        Value::List(v)
    }

    pub fn as_atom(&self) -> Option<&Atom> {
        match self {
            Value::Atom(a) => Some(a),
            Value::List(_) => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(l) => Some(l),
            Value::Atom(_) => None,
        }
    }

    pub fn as_list_mut(&mut self) -> Option<&mut Vec<Value>> {
        match self {
            Value::List(l) => Some(l),
            Value::Atom(_) => None,
        }
    }

    /// Head symbol of a list (first element, unquoted atom).
    pub fn head(&self) -> Option<&str> {
        match self.as_list()?.first()? {
            Value::Atom(a) if !a.quoted => Some(&a.text),
            _ => None,
        }
    }

    pub fn is(&self, head: &str) -> bool {
        self.head() == Some(head)
    }

    /// Elements after the head.
    pub fn args(&self) -> &[Value] {
        match self.as_list() {
            Some([_, rest @ ..]) => rest,
            _ => &[],
        }
    }

    pub fn find(&self, head: &str) -> Option<&Value> {
        self.as_list()?.iter().find(|v| v.is(head))
    }

    pub fn find_all<'s>(&'s self, head: &'s str) -> impl Iterator<Item = &'s Value> + 's {
        self.as_list()
            .unwrap_or(&[])
            .iter()
            .filter(move |v| v.is(head))
    }

    pub fn has_symbol(&self, symbol: &str) -> bool {
        self.as_list()
            .unwrap_or(&[])
            .iter()
            .any(|v| matches!(v, Value::Atom(a) if !a.quoted && a.text == symbol))
    }

    pub fn atom_text(&self) -> Option<&str> {
        self.as_atom().map(|a| a.text())
    }

    /// Text of the element at `index` (0 is the head).
    pub fn str_at(&self, index: usize) -> Result<&str, Error> {
        let list = self
            .as_list()
            .ok_or_else(|| Error::unpositioned(ErrorKind::ExpectedList))?;
        let v = list
            .get(index)
            .ok_or_else(|| Error::unpositioned(ErrorKind::MissingChild { index }))?;
        v.atom_text()
            .ok_or_else(|| Error::unpositioned(ErrorKind::ExpectedAtom { index }))
    }

    pub fn f64_at(&self, index: usize) -> Result<f64, Error> {
        let t = self.str_at(index)?;
        parse_f64(t).ok_or_else(|| {
            Error::unpositioned(ErrorKind::InvalidNumber {
                text: t.to_string(),
            })
        })
    }

    pub fn i64_at(&self, index: usize) -> Result<i64, Error> {
        let t = self.str_at(index)?;
        t.parse().map_err(|_| {
            Error::unpositioned(ErrorKind::InvalidNumber {
                text: t.to_string(),
            })
        })
    }

    /// Canonical text of this value, without a trailing newline.
    pub fn to_canonical(&self) -> String {
        let mut out = String::new();
        write_value(self, 0, &mut out);
        out
    }
}

/// Canonical text of a sequence of top-level values, each followed by `\n`.
pub fn to_canonical(values: &[Value]) -> String {
    let mut out = String::new();
    for v in values {
        write_value(v, 0, &mut out);
        out.push('\n');
    }
    out
}

/// Parses text into owned values (all top-level nodes).
pub fn parse_values(text: &str) -> Result<Vec<Value>, Error> {
    let tree = Tree::parse(text)?;
    Ok(tree.top().map(|n| n.to_value()).collect())
}

fn write_atom(a: &Atom, out: &mut String) {
    if a.quoted {
        out.push('"');
        escape_into(&a.text, out);
        out.push('"');
    } else {
        out.push_str(&a.text);
    }
}

fn indent(level: usize, out: &mut String) {
    for _ in 0..level {
        out.push_str("  ");
    }
}

fn write_value(v: &Value, level: usize, out: &mut String) {
    let items = match v {
        Value::Atom(a) => return write_atom(a, out),
        Value::List(items) => items,
    };
    out.push('(');
    let split = items
        .iter()
        .position(|i| matches!(i, Value::List(_)))
        .unwrap_or(items.len());
    for (n, item) in items[..split].iter().enumerate() {
        if n > 0 {
            out.push(' ');
        }
        if let Value::Atom(a) = item {
            write_atom(a, out);
        }
    }
    if split == items.len() {
        out.push(')');
        return;
    }
    for item in &items[split..] {
        out.push('\n');
        indent(level + 1, out);
        write_value(item, level + 1, out);
    }
    out.push('\n');
    indent(level, out);
    out.push(')');
}

impl From<Atom> for Value {
    fn from(a: Atom) -> Value {
        Value::Atom(a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_formatting() {
        let cases = [
            (1.0, "1"),
            (1.5, "1.5"),
            (-0.0, "0"),
            (0.1, "0.1"),
            (1e-7, "0.0000001"),
            (1e21, "1000000000000000000000"),
            (-12.25, "-12.25"),
            (0.1 + 0.2, "0.30000000000000004"),
        ];
        for (v, s) in cases {
            assert_eq!(format_number(v).unwrap(), s);
            assert_eq!(s.parse::<f64>().unwrap(), if v == 0.0 { 0.0 } else { v });
        }
        assert!(format_number(f64::NAN).is_err());
        assert!(format_number(f64::INFINITY).is_err());
    }

    #[test]
    fn canonical_layout() {
        let v = Value::node(
            "kdraft",
            [
                Value::node("version", [Value::int(1)]),
                Value::node(
                    "page",
                    [
                        Value::node("id", [Value::str("p1")]),
                        Value::node(
                            "at",
                            [Value::num(148.0).unwrap(), Value::num(105.5).unwrap()],
                        ),
                        Value::node("empty", []),
                    ],
                ),
                Value::sym("trailing"),
                Value::sym("needs quote"),
            ],
        );
        let expected = "(kdraft\n  (version 1)\n  (page\n    (id \"p1\")\n    (at 148 105.5)\n    (empty)\n  )\n  trailing\n  \"needs quote\"\n)";
        assert_eq!(v.to_canonical(), expected);
        let back = parse_values(expected).unwrap();
        assert_eq!(back, vec![v]);
    }

    #[test]
    fn accessors() {
        let v = parse_values("(at 1.5 x \"q\" (n))").unwrap().remove(0);
        assert_eq!(v.head(), Some("at"));
        assert_eq!(v.f64_at(1).unwrap(), 1.5);
        assert_eq!(v.str_at(3).unwrap(), "q");
        assert!(v.f64_at(2).is_err());
        assert!(v.str_at(4).is_err());
        assert!(v.str_at(9).is_err());
        assert!(v.has_symbol("x"));
        assert!(!v.has_symbol("q"));
        assert_eq!(v.args().len(), 4);
        assert!(v.find("n").is_some());
    }
}
