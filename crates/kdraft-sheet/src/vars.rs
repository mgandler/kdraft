//! Text variables: `${NAME}` expansion and legacy `%` codes.

use std::collections::BTreeMap;

use kdraft_board::TitleBlock;

/// Variable values. Lookup order: built-ins set by [`TextVars::set`]
/// (title block, page, file), then project variables.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextVars {
    builtin: BTreeMap<String, String>,
    project: BTreeMap<String, String>,
}

/// Page and file context for the built-in variables.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PageInfo {
    pub page_number: usize,
    pub page_count: usize,
    pub paper: String,
    pub sheet_name: Option<String>,
    pub file_name: Option<String>,
    pub project_name: Option<String>,
    /// Value for `${KICAD_VERSION}`; unset leaves the variable verbatim.
    pub kicad_version: Option<String>,
}

impl TextVars {
    pub fn new() -> TextVars {
        TextVars::default()
    }

    pub fn set(&mut self, name: &str, value: impl Into<String>) {
        self.builtin.insert(name.to_string(), value.into());
    }

    pub fn set_project(&mut self, vars: &BTreeMap<String, String>) {
        self.project = vars.clone();
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.builtin
            .get(name)
            .or_else(|| self.project.get(name))
            .map(String::as_str)
    }

    /// Built-ins from a title block and page context.
    pub fn from_title_block(tb: &TitleBlock, page: &PageInfo) -> TextVars {
        let mut v = TextVars::new();
        let mut opt = |k: &str, val: &Option<String>| {
            if let Some(x) = val {
                v.set(k, x.clone());
            } else {
                v.set(k, "");
            }
        };
        opt("TITLE", &tb.title);
        opt("REVISION", &tb.rev);
        opt("ISSUE_DATE", &tb.date);
        opt("COMPANY", &tb.company);
        for i in 1..=9u32 {
            v.set(
                &format!("COMMENT{i}"),
                tb.comments.get(&i).cloned().unwrap_or_default(),
            );
        }
        v.set("#", page.page_number.to_string());
        v.set("##", page.page_count.to_string());
        v.set("PAPER", page.paper.clone());
        let sheet = page.sheet_name.clone().unwrap_or_default();
        v.set("SHEETNAME", sheet.clone());
        v.set("SHEETPATH", sheet);
        if let Some(f) = &page.file_name {
            v.set("FILENAME", f.clone());
        }
        if let Some(p) = &page.project_name {
            v.set("PROJECTNAME", p.clone());
        }
        if let Some(k) = &page.kicad_version {
            v.set("KICAD_VERSION", k.clone());
        }
        v
    }

    /// Expands `${NAME}` references, recursively up to a fixed depth.
    /// Unknown variables stay verbatim.
    pub fn expand(&self, text: &str) -> String {
        let mut cur = text.to_string();
        for _ in 0..8 {
            let next = self.expand_once(&cur);
            if next == cur {
                break;
            }
            cur = next;
        }
        cur
    }

    fn expand_once(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(i) = rest.find("${") {
            out.push_str(&rest[..i]);
            let after = &rest[i + 2..];
            match after.find('}') {
                Some(j) => {
                    let name = &after[..j];
                    match self.get(name) {
                        Some(v) => out.push_str(v),
                        None => {
                            out.push_str("${");
                            out.push_str(name);
                            out.push('}');
                        }
                    }
                    rest = &after[j + 1..];
                }
                None => {
                    out.push_str(&rest[i..]);
                    rest = "";
                }
            }
        }
        out.push_str(rest);
        out
    }
}

/// Converts legacy `%` codes to `${...}` variables.
pub fn legacy_to_vars(text: &str) -> String {
    let mut out = String::new();
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let Some(&k) = it.peek() else {
            out.push('%');
            break;
        };
        let rep = match k {
            '%' => Some("%".to_string()),
            'K' => Some("${KICAD_VERSION}".to_string()),
            'Z' => Some("${PAPER}".to_string()),
            'Y' => Some("${COMPANY}".to_string()),
            'D' => Some("${ISSUE_DATE}".to_string()),
            'R' => Some("${REVISION}".to_string()),
            'S' => Some("${#}".to_string()),
            'N' => Some("${##}".to_string()),
            'F' => Some("${FILENAME}".to_string()),
            'P' => Some("${SHEETPATH}".to_string()),
            'T' => Some("${TITLE}".to_string()),
            'C' => None,
            _ => {
                out.push('%');
                continue;
            }
        };
        it.next();
        match rep {
            Some(r) => out.push_str(&r),
            None => match it.peek().and_then(|d| d.to_digit(10)) {
                // %C0 is the first comment.
                Some(d) => {
                    it.next();
                    out.push_str(&format!("${{COMMENT{}}}", d + 1));
                }
                None => out.push_str("%C"),
            },
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expansion_recursive_and_unknown_verbatim() {
        let mut v = TextVars::new();
        v.set("TITLE", "Board ${PART}");
        let mut p = BTreeMap::new();
        p.insert("PART".to_string(), "X1".to_string());
        p.insert("TITLE".to_string(), "ignored".to_string());
        v.set_project(&p);
        assert_eq!(v.expand("T: ${TITLE} ${NOPE} ${"), "T: Board X1 ${NOPE} ${");
        let mut loopy = TextVars::new();
        loopy.set("A", "${A}x");
        assert!(loopy.expand("${A}").ends_with('x'));
    }

    #[test]
    fn legacy_codes() {
        assert_eq!(legacy_to_vars("Id: %S/%N"), "Id: ${#}/${##}");
        assert_eq!(legacy_to_vars("%C0 %C3 %%"), "${COMMENT1} ${COMMENT4} %");
        assert_eq!(legacy_to_vars("50% off %q"), "50% off %q");
    }

    #[test]
    fn title_block_builtins() {
        let mut tb = TitleBlock {
            title: Some("T".into()),
            ..Default::default()
        };
        tb.comments.insert(2, "c2".into());
        let v = TextVars::from_title_block(
            &tb,
            &PageInfo {
                page_number: 2,
                page_count: 3,
                paper: "A3".into(),
                ..Default::default()
            },
        );
        assert_eq!(
            v.expand("${TITLE}|${COMMENT2}|${COMMENT1}|${#}/${##}|${PAPER}|${KICAD_VERSION}"),
            "T|c2||2/3|A3|${KICAD_VERSION}"
        );
    }
}
