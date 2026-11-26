//! Lossless, read-only S-expression tree over the original text.
//!
//! Every byte of the input belongs either to a token (an atom or a list's
//! parentheses) or to the whitespace in front of a token, the whitespace in
//! front of a closing parenthesis, or the trailing whitespace of the file.
//! [`Tree::to_text`] rebuilds the text from those pieces, so an unmodified
//! tree writes back byte for byte. Nodes are stored in a flat arena holding
//! byte offsets only; atom values are decoded on demand.
//!
//! Parsing is a single linear pass with an explicit stack, so input size is
//! bounded by memory and nesting depth by [`ParseOptions::max_depth`], never
//! by the call stack.

use std::borrow::Cow;

use crate::error::{line_col, Error, ErrorKind};
use crate::value::{Atom, Value};

/// Parser limits.
#[derive(Debug, Clone, Copy)]
pub struct ParseOptions {
    /// Maximum list nesting depth. KiCad files nest about a dozen levels.
    pub max_depth: usize,
}

impl Default for ParseOptions {
    fn default() -> Self {
        ParseOptions { max_depth: 512 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    List,
    /// Unquoted atom.
    Symbol,
    /// Double-quoted atom.
    String,
}

#[derive(Debug, Clone, Copy)]
struct RawNode {
    /// Start of the whitespace in front of the token.
    lead: u32,
    /// First byte of the token (`(` for lists).
    start: u32,
    /// One past the last byte of the token (`)` for lists).
    end: u32,
    /// Range into `Tree::children` (lists only).
    first: u32,
    count: u32,
    kind: NodeKind,
}

/// A parsed file. Owns the text; typed models borrow from it while reading.
#[derive(Debug, Clone)]
pub struct Tree {
    text: String,
    nodes: Vec<RawNode>,
    children: Vec<u32>,
    top_first: u32,
    top_count: u32,
    trailing: u32,
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

fn is_delim(b: u8) -> bool {
    is_ws(b) || matches!(b, b'(' | b')' | b'"')
}

impl Tree {
    /// Parses with default options.
    pub fn parse(text: impl Into<String>) -> Result<Tree, Error> {
        Self::parse_with(text, ParseOptions::default())
    }

    pub fn parse_with(text: impl Into<String>, opts: ParseOptions) -> Result<Tree, Error> {
        let text: String = text.into();
        if u32::try_from(text.len()).is_err() {
            return Err(Error::unpositioned(ErrorKind::TooLarge));
        }
        let bytes = text.as_bytes();
        let len = bytes.len();
        let mut nodes: Vec<RawNode> = Vec::with_capacity(len / 8);
        let mut children: Vec<u32> = Vec::with_capacity(len / 8);
        // Child ids of all open lists, innermost last.
        let mut scratch: Vec<u32> = Vec::new();
        // (node id, scratch length when the list opened)
        let mut stack: Vec<(u32, usize)> = Vec::new();
        let mut i = 0usize;
        let mut lead = 0usize;

        // `len` fits in u32 (checked above), so every offset below does too.
        let off = |x: usize| x as u32;

        while i < len {
            let b = bytes[i];
            if is_ws(b) {
                i += 1;
                continue;
            }
            match b {
                b'(' => {
                    if stack.len() >= opts.max_depth {
                        return Err(Error::at(
                            &text,
                            i,
                            ErrorKind::TooDeep {
                                limit: opts.max_depth,
                            },
                        ));
                    }
                    let id = off(nodes.len());
                    nodes.push(RawNode {
                        lead: off(lead),
                        start: off(i),
                        end: off(i + 1),
                        first: 0,
                        count: 0,
                        kind: NodeKind::List,
                    });
                    scratch.push(id);
                    stack.push((id, scratch.len()));
                    i += 1;
                    lead = i;
                }
                b')' => {
                    let Some((id, mark)) = stack.pop() else {
                        return Err(Error::at(&text, i, ErrorKind::UnexpectedClose));
                    };
                    let first = off(children.len());
                    children.extend(scratch.drain(mark..));
                    let node = &mut nodes[id as usize];
                    node.first = first;
                    node.count = off(children.len()) - first;
                    node.end = off(i + 1);
                    i += 1;
                    lead = i;
                }
                b'"' => {
                    let start = i;
                    i += 1;
                    loop {
                        match bytes.get(i) {
                            None => {
                                return Err(Error::at(&text, start, ErrorKind::UnterminatedString))
                            }
                            Some(b'\\') => i += 2,
                            Some(b'"') => {
                                i += 1;
                                break;
                            }
                            Some(_) => i += 1,
                        }
                    }
                    // A trailing backslash can step past the end.
                    if i > len {
                        return Err(Error::at(&text, start, ErrorKind::UnterminatedString));
                    }
                    scratch.push(off(nodes.len()));
                    nodes.push(RawNode {
                        lead: off(lead),
                        start: off(start),
                        end: off(i),
                        first: 0,
                        count: 0,
                        kind: NodeKind::String,
                    });
                    lead = i;
                }
                _ => {
                    let start = i;
                    while i < len && !is_delim(bytes[i]) {
                        i += 1;
                    }
                    scratch.push(off(nodes.len()));
                    nodes.push(RawNode {
                        lead: off(lead),
                        start: off(start),
                        end: off(i),
                        first: 0,
                        count: 0,
                        kind: NodeKind::Symbol,
                    });
                    lead = i;
                }
            }
        }
        if let Some(&(id, _)) = stack.first() {
            let open = nodes[id as usize].start as usize;
            return Err(Error::at(&text, open, ErrorKind::UnclosedList { open }));
        }
        let top_first = off(children.len());
        children.append(&mut scratch);
        let top_count = off(children.len()) - top_first;
        Ok(Tree {
            text,
            nodes,
            children,
            top_first,
            top_count,
            trailing: off(lead),
        })
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn into_text(self) -> String {
        self.text
    }

    /// Number of nodes (atoms and lists).
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Top-level nodes in file order.
    pub fn top(&self) -> Children<'_> {
        Children {
            tree: self,
            ids: &self.children
                [self.top_first as usize..(self.top_first + self.top_count) as usize],
        }
    }

    /// The first top-level list, e.g. `(kicad_pcb ...)`.
    pub fn root(&self) -> Option<Node<'_>> {
        self.top().find(|n| n.is_list())
    }

    /// The first top-level list, or an error if there is none.
    pub fn require_root(&self) -> Result<Node<'_>, Error> {
        self.root()
            .ok_or_else(|| Error::at(&self.text, 0, ErrorKind::ExpectedList))
    }

    /// 1-based line and column of a byte offset.
    pub fn line_col(&self, offset: usize) -> (usize, usize) {
        line_col(&self.text, offset)
    }

    /// Rebuilds the text from the node arena. For a tree produced by
    /// [`Tree::parse`] this equals the input.
    pub fn to_text(&self) -> String {
        let mut out = String::with_capacity(self.text.len());
        for n in self.top() {
            self.write_node(n.id, &mut out);
        }
        out.push_str(&self.text[self.trailing as usize..]);
        out
    }

    fn write_node(&self, id: u32, out: &mut String) {
        // Explicit stack: (list id, index of next child to write).
        let t = &self.text;
        let mut stack: Vec<(u32, u32)> = Vec::new();
        let n = &self.nodes[id as usize];
        out.push_str(&t[n.lead as usize..n.start as usize]);
        if n.kind != NodeKind::List {
            out.push_str(&t[n.start as usize..n.end as usize]);
            return;
        }
        out.push('(');
        stack.push((id, 0));
        while let Some(top) = stack.last_mut() {
            let list = self.nodes[top.0 as usize];
            if top.1 == list.count {
                let close_lead = if list.count == 0 {
                    list.start + 1
                } else {
                    let last = self.children[(list.first + list.count - 1) as usize];
                    self.nodes[last as usize].end
                };
                out.push_str(&t[close_lead as usize..list.end as usize]);
                stack.pop();
                continue;
            }
            let child_id = self.children[(list.first + top.1) as usize];
            top.1 += 1;
            let c = &self.nodes[child_id as usize];
            out.push_str(&t[c.lead as usize..c.start as usize]);
            if c.kind == NodeKind::List {
                out.push('(');
                stack.push((child_id, 0));
            } else {
                out.push_str(&t[c.start as usize..c.end as usize]);
            }
        }
    }

    fn node(&self, id: u32) -> Node<'_> {
        Node { tree: self, id }
    }
}

/// Iterator over the children of a list.
#[derive(Clone)]
pub struct Children<'a> {
    tree: &'a Tree,
    ids: &'a [u32],
}

impl<'a> Iterator for Children<'a> {
    type Item = Node<'a>;
    fn next(&mut self) -> Option<Node<'a>> {
        let (&first, rest) = self.ids.split_first()?;
        self.ids = rest;
        Some(self.tree.node(first))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.ids.len(), Some(self.ids.len()))
    }
}

impl ExactSizeIterator for Children<'_> {}

impl<'a> DoubleEndedIterator for Children<'a> {
    fn next_back(&mut self) -> Option<Node<'a>> {
        let (&last, rest) = self.ids.split_last()?;
        self.ids = rest;
        Some(self.tree.node(last))
    }
}

/// A borrowed node of a [`Tree`].
#[derive(Clone, Copy)]
pub struct Node<'a> {
    tree: &'a Tree,
    id: u32,
}

impl std::fmt::Debug for Node<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let raw = self.raw();
        let shown: String = raw.chars().take(60).collect();
        f.debug_struct("Node")
            .field("kind", &self.kind())
            .field("offset", &self.offset())
            .field("text", &shown)
            .finish()
    }
}

impl<'a> Node<'a> {
    fn raw_node(&self) -> &'a RawNode {
        &self.tree.nodes[self.id as usize]
    }

    pub fn kind(&self) -> NodeKind {
        self.raw_node().kind
    }

    pub fn is_list(&self) -> bool {
        self.kind() == NodeKind::List
    }

    pub fn is_atom(&self) -> bool {
        !self.is_list()
    }

    /// Byte offset of the token.
    pub fn offset(&self) -> usize {
        self.raw_node().start as usize
    }

    /// Byte range of the token in the source text.
    pub fn span(&self) -> std::ops::Range<usize> {
        let n = self.raw_node();
        n.start as usize..n.end as usize
    }

    /// 1-based line and column of the token.
    pub fn line_col(&self) -> (usize, usize) {
        self.tree.line_col(self.offset())
    }

    /// The token's original text: for lists everything from `(` to `)`, for
    /// strings including the quotes.
    pub fn raw(&self) -> &'a str {
        let n = self.raw_node();
        &self.tree.text[n.start as usize..n.end as usize]
    }

    /// An error positioned at this node.
    pub fn error(&self, kind: ErrorKind) -> Error {
        Error::at(&self.tree.text, self.offset(), kind)
    }

    /// Decoded atom text; `None` for lists. Strings are unescaped.
    pub fn atom(&self) -> Option<Cow<'a, str>> {
        match self.kind() {
            NodeKind::List => None,
            NodeKind::Symbol => Some(Cow::Borrowed(self.raw())),
            NodeKind::String => {
                let raw = self.raw();
                Some(crate::escape::unescape(&raw[1..raw.len() - 1]))
            }
        }
    }

    /// Children of a list; empty for atoms.
    pub fn children(&self) -> Children<'a> {
        let n = self.raw_node();
        let ids = if n.kind == NodeKind::List {
            &self.tree.children[n.first as usize..(n.first + n.count) as usize]
        } else {
            &[][..]
        };
        Children {
            tree: self.tree,
            ids,
        }
    }

    pub fn len(&self) -> usize {
        self.children().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn child(&self, index: usize) -> Option<Node<'a>> {
        self.children().nth(index)
    }

    /// The head symbol of a list: its first element when that is an unquoted
    /// atom.
    pub fn head(&self) -> Option<&'a str> {
        let first = self.children().next()?;
        (first.kind() == NodeKind::Symbol).then(|| first.raw())
    }

    pub fn is(&self, head: &str) -> bool {
        self.head() == Some(head)
    }

    /// First child list with the given head.
    pub fn find(&self, head: &str) -> Option<Node<'a>> {
        self.children().find(|c| c.is(head))
    }

    /// First child list with the given head, or a positioned error.
    pub fn require(&self, head: &str) -> Result<Node<'a>, Error> {
        self.children().find(|c| c.is(head)).ok_or_else(|| {
            self.error(ErrorKind::MissingList {
                head: head.to_string(),
            })
        })
    }

    /// All child lists with the given head, in order.
    pub fn find_all<'h>(&self, head: &'h str) -> impl Iterator<Item = Node<'a>> + 'h
    where
        'a: 'h,
    {
        self.children().filter(move |c| c.is(head))
    }

    /// Child lists, skipping atoms.
    pub fn lists(&self) -> impl Iterator<Item = Node<'a>> {
        self.children().filter(|c| c.is_list())
    }

    /// True if an unquoted atom child equals `symbol` (flags like `locked`).
    pub fn has_symbol(&self, symbol: &str) -> bool {
        self.children()
            .any(|c| c.kind() == NodeKind::Symbol && c.raw() == symbol)
    }

    fn atom_child(&self, index: usize) -> Result<Node<'a>, Error> {
        if !self.is_list() {
            return Err(self.error(ErrorKind::ExpectedList));
        }
        let c = self
            .child(index)
            .ok_or_else(|| self.error(ErrorKind::MissingChild { index }))?;
        if c.is_list() {
            return Err(c.error(ErrorKind::ExpectedAtom { index }));
        }
        Ok(c)
    }

    /// Decoded text of the atom at `index` of this list.
    pub fn str_at(&self, index: usize) -> Result<Cow<'a, str>, Error> {
        let c = self.atom_child(index)?;
        c.atom()
            .ok_or_else(|| c.error(ErrorKind::ExpectedAtom { index }))
    }

    /// The atom at `index` parsed as a finite `f64`.
    pub fn f64_at(&self, index: usize) -> Result<f64, Error> {
        let c = self.atom_child(index)?;
        let text = c.atom().unwrap_or_default();
        parse_f64(&text).ok_or_else(|| {
            c.error(ErrorKind::InvalidNumber {
                text: text.into_owned(),
            })
        })
    }

    /// The atom at `index` parsed as `i64`.
    pub fn i64_at(&self, index: usize) -> Result<i64, Error> {
        let c = self.atom_child(index)?;
        let text = c.atom().unwrap_or_default();
        text.parse::<i64>().map_err(|_| {
            c.error(ErrorKind::InvalidNumber {
                text: text.into_owned(),
            })
        })
    }

    /// Converts this subtree to an owned [`Value`], keeping quoting.
    pub fn to_value(&self) -> Value {
        // Recursion is bounded by the parse depth limit.
        match self.kind() {
            NodeKind::List => Value::List(self.children().map(|c| c.to_value()).collect()),
            NodeKind::Symbol => Value::Atom(Atom::symbol(self.raw())),
            NodeKind::String => Value::Atom(Atom::string(self.atom().unwrap_or_default())),
        }
    }
}

/// Parses a decimal number as KiCad writes them. Rejects `inf`, `nan` and
/// other spellings Rust would accept but KiCad never writes.
pub fn parse_f64(text: &str) -> Option<f64> {
    let first = *text.as_bytes().first()?;
    if !(first.is_ascii_digit() || matches!(first, b'-' | b'+' | b'.')) {
        return None;
    }
    let v: f64 = text.parse().ok()?;
    v.is_finite().then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserves_bytes() {
        let cases = [
            "",
            "  \n",
            "(a)",
            "(kicad_pcb\n\t(version 20241229)\n\t(generator \"pcbnew\")\n)\n",
            "(a\r\n  (b 1.50 -0 +3 1e-3)\r\n  \"q \\\" \\\\ \\n\" sym)\r\n",
            "( a  ( ) (  ) )",
            "(µ \"Ω 100µF\")",
            "x y (z)",
            "(a \"\")(b)",
        ];
        for c in cases {
            let t = Tree::parse(c).unwrap();
            assert_eq!(t.to_text(), c, "case {c:?}");
        }
    }

    #[test]
    fn structure_and_queries() {
        let t = Tree::parse(
            "(pad \"1\" smd rect (at 1.5 -2 90) (size 1 2) (layers \"F.Cu\" \"F.Mask\") locked)",
        )
        .unwrap();
        let pad = t.root().unwrap();
        assert_eq!(pad.head(), Some("pad"));
        assert_eq!(pad.str_at(1).unwrap(), "1");
        assert_eq!(pad.str_at(2).unwrap(), "smd");
        let at = pad.find("at").unwrap();
        assert_eq!(at.f64_at(1).unwrap(), 1.5);
        assert_eq!(at.f64_at(2).unwrap(), -2.0);
        assert_eq!(at.i64_at(3).unwrap(), 90);
        assert!(pad.has_symbol("locked"));
        assert!(!pad.has_symbol("smdx"));
        let layers: Vec<_> = pad
            .find("layers")
            .unwrap()
            .children()
            .skip(1)
            .map(|c| c.atom().unwrap().into_owned())
            .collect();
        assert_eq!(layers, ["F.Cu", "F.Mask"]);
        assert_eq!(pad.find_all("size").count(), 1);
        assert!(pad.find("drill").is_none());
        assert_eq!(pad.lists().count(), 3);
    }

    #[test]
    fn string_escapes_decode() {
        let t = Tree::parse(r#"(t "a\"b\\c\nd" "plain")"#).unwrap();
        let r = t.root().unwrap();
        assert_eq!(r.str_at(1).unwrap(), "a\"b\\c\nd");
        assert!(matches!(r.str_at(2).unwrap(), Cow::Borrowed("plain")));
    }

    #[test]
    fn query_errors_carry_position() {
        let t = Tree::parse("(a\n  (at x 2))").unwrap();
        let at = t.root().unwrap().find("at").unwrap();
        let e = at.f64_at(1).unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidNumber { text: "x".into() });
        assert_eq!((e.line, e.column, e.offset), (2, 7, 9));
        let e = at.f64_at(5).unwrap_err();
        assert_eq!(e.kind, ErrorKind::MissingChild { index: 5 });
        assert_eq!((e.line, e.column), (2, 3));
        let e = t.root().unwrap().str_at(1).unwrap_err();
        assert_eq!(e.kind, ErrorKind::ExpectedAtom { index: 1 });
        let e = t.root().unwrap().require("size").unwrap_err();
        assert_eq!(
            e.kind,
            ErrorKind::MissingList {
                head: "size".into()
            }
        );
        assert!(e.to_string().contains("1:1"));
    }

    #[test]
    fn numbers_reject_non_kicad_spellings() {
        assert_eq!(parse_f64("1.6"), Some(1.6));
        assert_eq!(parse_f64("-0.5"), Some(-0.5));
        assert_eq!(parse_f64("1e-5"), Some(1e-5));
        assert_eq!(parse_f64(".5"), Some(0.5));
        assert_eq!(parse_f64("inf"), None);
        assert_eq!(parse_f64("nan"), None);
        assert_eq!(parse_f64("1e999"), None);
        assert_eq!(parse_f64(""), None);
    }

    #[test]
    fn unbalanced_input_errors() {
        let e = Tree::parse("(a (b)").unwrap_err();
        assert_eq!(e.kind, ErrorKind::UnclosedList { open: 0 });
        assert_eq!((e.line, e.column), (1, 1));

        let e = Tree::parse("(a))").unwrap_err();
        assert_eq!(e.kind, ErrorKind::UnexpectedClose);
        assert_eq!(e.offset, 3);

        let e = Tree::parse("(a\n \"open)").unwrap_err();
        assert_eq!(e.kind, ErrorKind::UnterminatedString);
        assert_eq!((e.line, e.column), (2, 2));

        let e = Tree::parse("(a \"x\\").unwrap_err();
        assert_eq!(e.kind, ErrorKind::UnterminatedString);
    }

    #[test]
    fn depth_limit_is_an_error_not_a_stack_overflow() {
        let deep = "(".repeat(100_000) + &")".repeat(100_000);
        let e = Tree::parse(&deep).unwrap_err();
        assert_eq!(e.kind, ErrorKind::TooDeep { limit: 512 });
        let ok = "(".repeat(400) + &")".repeat(400);
        let t = Tree::parse(&ok).unwrap();
        assert_eq!(t.to_text(), ok);
    }

    #[test]
    fn large_flat_file_parses() {
        let mut s = String::from("(kicad_pcb\n");
        for i in 0..200_000 {
            s.push_str(&format!(
                "\t(segment (start {i} 1.5) (end 2 3) (width 0.2) (layer \"F.Cu\"))\n"
            ));
        }
        s.push(')');
        let t = Tree::parse(s.clone()).unwrap();
        let root = t.root().unwrap();
        assert_eq!(root.find_all("segment").count(), 200_000);
        assert_eq!(t.to_text(), s);
    }

    #[test]
    fn to_value_keeps_quoting() {
        let t = Tree::parse("(a \"b\" c \"d e\")").unwrap();
        let v = t.root().unwrap().to_value();
        assert_eq!(v.to_canonical(), "(a \"b\" c \"d e\")");
    }
}
