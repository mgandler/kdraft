//! Bill of materials: lines from `kicad-cli sch export bom` CSV, or a
//! fallback derived from the PCB's footprints.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::model::Board;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BomSource {
    /// `kicad-cli sch export bom`.
    Schematic,
    /// Footprints of the board (no kicad-cli or no schematic).
    Pcb,
}

impl BomSource {
    /// Tag stored with item numbers in documents.
    pub fn tag(self) -> &'static str {
        match self {
            BomSource::Schematic => "schematic",
            BomSource::Pcb => "pcb",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BomLine {
    /// References in natural order (`R2` before `R10`).
    pub refs: Vec<String>,
    pub qty: usize,
    pub value: String,
    pub footprint: String,
    pub dnp: bool,
    /// Other exported fields by column label.
    pub fields: BTreeMap<String, String>,
}

impl BomLine {
    /// Key identifying the group across updates: value and footprint (and
    /// DNP), as the export groups them.
    pub fn key(&self) -> String {
        format!(
            "{}|{}{}",
            self.value,
            self.footprint,
            if self.dnp { "|DNP" } else { "" }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Bom {
    pub source: BomSource,
    pub lines: Vec<BomLine>,
    /// Labels of extra columns, in export order.
    pub extra_columns: Vec<String>,
}

/// Splits a reference into prefix and number for natural ordering.
fn ref_key(r: &str) -> (String, u64, String) {
    let p: String = r.chars().take_while(|c| !c.is_ascii_digit()).collect();
    let rest = &r[p.len()..];
    let d: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let n = d.parse().unwrap_or(0);
    (p, n, rest[d.len()..].to_string())
}

pub fn sort_refs(refs: &mut [String]) {
    refs.sort_by_key(|r| ref_key(r));
}

/// `R1, R2, R3, R4, R7` -> `R1-R4, R7` (runs of three or more).
pub fn compress_refs(refs: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < refs.len() {
        let (p, n, s) = ref_key(&refs[i]);
        let mut j = i;
        while j + 1 < refs.len() {
            let (p2, n2, s2) = ref_key(&refs[j + 1]);
            if p2 == p && s2.is_empty() && s.is_empty() && n2 == n + (j + 1 - i) as u64 {
                j += 1;
            } else {
                break;
            }
        }
        if j - i >= 2 {
            out.push(format!("{}-{}", refs[i], refs[j]));
        } else {
            out.extend(refs[i..=j].iter().cloned());
        }
        i = j + 1;
    }
    out
}

/// Characters per line of a BOM reference cell.
pub const REF_WRAP_CHARS: usize = 36;

/// References compressed into ranges and wrapped at `max_chars` per line
/// (`, ` between entries; an entry is never split).
pub fn wrap_refs(refs: &[String], max_chars: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    let parts = compress_refs(refs);
    let n = parts.len();
    for (i, p) in parts.into_iter().enumerate() {
        let piece = if i + 1 < n { format!("{p},") } else { p };
        if !cur.is_empty() && cur.len() + 1 + piece.len() > max_chars {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(&piece);
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(cur);
    }
    lines
}

/// Item numbers for `lines`: stored numbers by group key, new groups get
/// the next numbers after the largest stored one, in line order.
pub fn number_lines(lines: &[BomLine], stored: &BTreeMap<String, usize>) -> Vec<usize> {
    let mut next = stored.values().copied().max().unwrap_or(0) + 1;
    lines
        .iter()
        .map(|l| match stored.get(&l.key()) {
            Some(n) => *n,
            None => {
                let n = next;
                next += 1;
                n
            }
        })
        .collect()
}

/// Fallback BOM from the board: footprints grouped by value and library
/// footprint id; `exclude_from_bom` and `board_only` parts left out; DNP
/// parts in groups of their own, marked.
pub fn from_board(board: &Board) -> Bom {
    let mut groups: BTreeMap<(bool, String, String), Vec<String>> = BTreeMap::new();
    for f in &board.footprints {
        if f.attributes.exclude_from_bom || f.attributes.board_only {
            continue;
        }
        let Some(r) = f.reference.clone().filter(|r| !r.is_empty()) else {
            continue;
        };
        groups
            .entry((
                f.attributes.dnp,
                f.value.clone().unwrap_or_default(),
                f.lib_id.clone(),
            ))
            .or_default()
            .push(r);
    }
    let mut lines: Vec<BomLine> = groups
        .into_iter()
        .map(|((dnp, value, footprint), mut refs)| {
            sort_refs(&mut refs);
            refs.dedup();
            BomLine {
                qty: refs.len(),
                refs,
                value,
                footprint,
                dnp,
                fields: BTreeMap::new(),
            }
        })
        .collect();
    sort_lines(&mut lines);
    Bom {
        source: BomSource::Pcb,
        lines,
        extra_columns: Vec::new(),
    }
}

/// Lines ordered by their first reference (natural order), DNP last.
pub fn sort_lines(lines: &mut [BomLine]) {
    lines.sort_by(|a, b| {
        a.dnp.cmp(&b.dnp).then_with(|| {
            ref_key(a.refs.first().map_or("", String::as_str))
                .cmp(&ref_key(b.refs.first().map_or("", String::as_str)))
        })
    });
}

/// Splits one CSV line (`,` or the given delimiter; `"` quoting with `""`
/// escapes). Tolerant: an unterminated quote runs to the end.
pub fn csv_fields(line: &str, delim: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    cur.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                cur.push(c);
            }
        } else if c == '"' {
            quoted = true;
        } else if c == delim {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out.into_iter().map(|s| s.trim().to_string()).collect()
}

fn find_col(header: &[String], names: &[&str]) -> Option<usize> {
    header.iter().position(|h| {
        let h = h
            .trim_start_matches("${")
            .trim_end_matches('}')
            .to_ascii_lowercase();
        names.iter().any(|n| h == *n)
    })
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BomError {
    #[error("BOM CSV has no header line")]
    Empty,
    #[error("BOM CSV has no reference column (header: {0})")]
    NoReferences(String),
}

/// Parses the CSV written by `kicad-cli sch export bom` as kdraft asks for
/// it (references, value, footprint, quantity, DNP; any extra columns).
/// Column names are matched loosely (`Refs`/`Reference`, `Qty`/
/// `QUANTITY`/`${QUANTITY}`, ...). Reference lists may use `,` or spaces
/// and ranges `R1-R4`, which are expanded. Groups are kept as exported.
pub fn parse_csv(text: &str) -> Result<Bom, BomError> {
    let text = text.trim_start_matches('\u{feff}');
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header_line = lines.next().ok_or(BomError::Empty)?;
    let delim = if header_line.contains(';') && !header_line.contains(',') {
        ';'
    } else if header_line.contains('\t') && !header_line.contains(',') {
        '\t'
    } else {
        ','
    };
    let header = csv_fields(header_line, delim);
    let ci = find_col(&header, &["refs", "reference", "references", "designator"])
        .ok_or_else(|| BomError::NoReferences(header_line.to_string()))?;
    let vi = find_col(&header, &["value"]);
    let fi = find_col(&header, &["footprint"]);
    let qi = find_col(&header, &["qty", "quantity"]);
    let di = find_col(&header, &["dnp", "do not populate"]);
    let known = [Some(ci), vi, fi, qi, di];
    let extra: Vec<(usize, String)> = header
        .iter()
        .enumerate()
        .filter(|(i, _)| !known.contains(&Some(*i)))
        .map(|(i, h)| (i, h.clone()))
        .collect();
    let mut out = Vec::new();
    for l in lines {
        let f = csv_fields(l, delim);
        let get = |i: Option<usize>| i.and_then(|i| f.get(i)).cloned().unwrap_or_default();
        let mut refs = expand_refs(&get(Some(ci)));
        if refs.is_empty() {
            continue;
        }
        sort_refs(&mut refs);
        let qty = get(qi).parse().unwrap_or(refs.len());
        let dnp_s = get(di).to_ascii_lowercase();
        let dnp = !dnp_s.is_empty() && !matches!(dnp_s.as_str(), "0" | "no" | "false" | "-");
        out.push(BomLine {
            refs,
            qty,
            value: get(vi),
            footprint: get(fi),
            dnp,
            fields: extra
                .iter()
                .map(|(i, h)| (h.clone(), f.get(*i).cloned().unwrap_or_default()))
                .collect(),
        });
    }
    Ok(Bom {
        source: BomSource::Schematic,
        lines: out,
        extra_columns: extra.into_iter().map(|(_, h)| h).collect(),
    })
}

/// `R1,R2 R5-R7` -> R1, R2, R5, R6, R7.
fn expand_refs(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    for tok in s
        .split([',', ' ', ';'])
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        if let Some((a, b)) = tok.split_once('-') {
            let (pa, na, sa) = ref_key(a);
            let (pb, nb, sb) = ref_key(b);
            if pa == pb
                && sa.is_empty()
                && sb.is_empty()
                && nb >= na
                && nb - na < 10_000
                && !pa.is_empty()
            {
                for n in na..=nb {
                    out.push(format!("{pa}{n}"));
                }
                continue;
            }
        }
        out.push(tok.to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_parse_with_quotes_ranges_and_dnp() {
        let csv = "\"Refs\",\"Value\",\"Footprint\",\"Qty\",\"DNP\",\"MPN\"\n\
                   \"R1,R2,R10\",\"10k\",\"Resistor_SMD:R_0603\",\"3\",\"\",\"RC0603\"\n\
                   \"C1-C3\",\"100n, 50V\",\"C_0402\",\"3\",\"\",\"\"\n\
                   \"U1\",\"MCU \"\"rev B\"\"\",\"QFN\",\"1\",\"DNP\",\"X\"\n";
        let b = parse_csv(csv).unwrap();
        assert_eq!(b.lines.len(), 3);
        assert_eq!(b.lines[0].refs, vec!["R1", "R2", "R10"]);
        assert_eq!(b.lines[1].refs, vec!["C1", "C2", "C3"]);
        assert_eq!(b.lines[1].value, "100n, 50V");
        assert_eq!(b.lines[2].value, "MCU \"rev B\"");
        assert!(b.lines[2].dnp && !b.lines[0].dnp);
        assert_eq!(b.extra_columns, vec!["MPN"]);
        assert_eq!(b.lines[0].fields["MPN"], "RC0603");
        assert!(parse_csv("").is_err());
        assert!(parse_csv("a,b\n1,2").is_err());
    }

    #[test]
    fn wrapping_and_numbers() {
        let refs: Vec<String> = (1..=40)
            .filter(|n| n % 3 != 0)
            .map(|n| format!("C{n}"))
            .collect();
        let w = wrap_refs(&refs, 20);
        assert!(w.len() > 1);
        assert!(w.iter().all(|l| l.len() <= 20), "{w:?}");
        assert_eq!(
            w.join(" ").matches('C').count(),
            compress_refs(&refs).len()
                + compress_refs(&refs)
                    .iter()
                    .filter(|p| p.contains('-'))
                    .count()
        );
        let line = |v: &str| BomLine {
            refs: vec!["R1".into()],
            qty: 1,
            value: v.into(),
            footprint: "F".into(),
            dnp: false,
            fields: BTreeMap::new(),
        };
        let lines = vec![line("a"), line("b"), line("c")];
        let mut stored = BTreeMap::new();
        stored.insert("b|F".to_string(), 7);
        assert_eq!(number_lines(&lines, &stored), vec![8, 7, 9]);
    }

    #[test]
    fn ranges() {
        let r: Vec<String> = ["R1", "R2", "R3", "R4", "R7", "R9", "R10", "C1"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(compress_refs(&r), vec!["R1-R4", "R7", "R9", "R10", "C1"]);
        let mut s: Vec<String> = ["R10", "R2", "C3", "R1"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        sort_refs(&mut s);
        assert_eq!(s, vec!["C3", "R1", "R2", "R10"]);
    }

    #[test]
    fn fallback_from_fixture() {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/kicad9-minimal.kicad_pcb");
        let b = Board::load(p).unwrap();
        let bom = from_board(&b);
        assert_eq!(bom.source, BomSource::Pcb);
        let refs: Vec<&str> = bom
            .lines
            .iter()
            .flat_map(|l| l.refs.iter().map(String::as_str))
            .collect();
        assert!(refs.contains(&"R1"), "{refs:?}");
        // Mounting holes are excluded from the BOM in the fixture.
        assert!(!refs.contains(&"H1"), "{refs:?}");
    }
}
