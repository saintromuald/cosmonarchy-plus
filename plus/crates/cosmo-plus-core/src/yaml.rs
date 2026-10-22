//! A deliberately tiny YAML subset parser for `plus.yml`.
//!
//! Supports what the config needs and nothing more:
//!   * comments (`#` to end of line) and blank lines,
//!   * block mappings (`key: value`, `key:` with a nested block),
//!   * block sequences (`- item`, `- key: value` list-of-maps),
//!   * scalars: bare strings, `"quoted"` / `'quoted'` strings, integers.
//!
//! No anchors, aliases, multi-line strings, flow collections, or tags. Indentation is
//! tracked by leading-spaces column; tabs are rejected. Unknown structure parses into a
//! tree the caller walks, so an unexpected key is a caller decision, not a parse error.

#[derive(Debug, Clone, PartialEq)]
pub enum Yaml {
    Null,
    Scalar(String),
    List(Vec<Yaml>),
    Map(Vec<(String, Yaml)>),
}

impl Yaml {
    pub fn get(&self, key: &str) -> Option<&Yaml> {
        match self {
            Yaml::Map(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Integer value if this is a scalar that parses as an i64.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Yaml::Scalar(s) => s.trim().parse::<i64>().ok(),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Yaml::Scalar(s) => Some(s.as_str()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Line<'a> {
    indent: usize,
    text: &'a str,
}

/// Strip a trailing comment (`#` at start or after whitespace) and trim the line.
fn strip_comment(raw: &str) -> &str {
    for (i, b) in raw.bytes().enumerate() {
        if b == b'#' && (i == 0 || raw.as_bytes()[i - 1].is_ascii_whitespace()) {
            return raw[..i].trim_end();
        }
    }
    raw.trim_end()
}

fn indent_of(s: &str) -> Result<usize, String> {
    let mut n = 0;
    for b in s.bytes() {
        match b {
            b' ' => n += 1,
            b'\t' => return Err("tabs are not allowed for indentation".to_string()),
            _ => break,
        }
    }
    Ok(n)
}

fn preprocess(input: &str) -> Result<Vec<Line<'_>>, String> {
    let mut out = Vec::new();
    for raw in input.lines() {
        let text = strip_comment(raw);
        if text.trim().is_empty() {
            continue;
        }
        let indent = indent_of(text)?;
        out.push(Line {
            indent,
            text: text.trim(),
        });
    }
    Ok(out)
}

/// Parse a scalar token: strip quotes, inline lists become lists, otherwise the bare
/// string. Inline lists support the `[a, b]` and `[a,b]` spellings used by the draft
/// option tables (`units: [41, 41]`).
fn parse_scalar(tok: &str) -> Yaml {
    let t = tok.trim();
    if t.len() >= 2 {
        let b = t.as_bytes();
        let (open, close) = (b[0], b[t.len() - 1]);
        if (open == b'"' && close == b'"') || (open == b'\'' && close == b'\'') {
            return Yaml::Scalar(t[1..t.len() - 1].to_string());
        }
        if open == b'[' && close == b']' {
            let inner = &t[1..t.len() - 1];
            let items: Vec<Yaml> = inner
                .split(',')
                .map(|part| parse_scalar(part.trim()))
                .filter(|y| !matches!(y, Yaml::Scalar(s) if s.is_empty()))
                .collect();
            return Yaml::List(items);
        }
    }
    Yaml::Scalar(t.to_string())
}

pub fn parse(input: &str) -> Result<Yaml, String> {
    let lines = preprocess(input)?;
    let mut pos = 0usize;
    let root = parse_block(&lines, &mut pos, 0)?;
    // Any trailing content must have consumed all lines.
    if pos != lines.len() {
        return Err(format!("unexpected content at line {}", pos + 1));
    }
    Ok(root)
}

/// Parse a block starting at the current line, whose first entry is indented at least
/// `min_indent`. Advances `pos` past the block.
fn parse_block<'a>(
    lines: &[Line<'a>],
    pos: &mut usize,
    min_indent: usize,
) -> Result<Yaml, String> {
    if *pos >= lines.len() {
        return Ok(Yaml::Null);
    }

    // A mapping is a sequence of `key:` or `key: value` lines at the same indent.
    if lines[*pos].text.starts_with("- ") || lines[*pos].text == "-" {
        return parse_list(lines, pos, min_indent);
    }

    // Otherwise assume a mapping block.
    let block_indent = lines[*pos].indent;
    if block_indent < min_indent {
        return Err(format!("bad indentation at line {}", *pos + 1));
    }

    let mut entries: Vec<(String, Yaml)> = Vec::new();
    while *pos < lines.len() && lines[*pos].indent == block_indent {
        let line = &lines[*pos];
        let text = line.text;
        if text.starts_with("- ") || text == "-" {
            // A sequence nested at this mapping's indent is malformed here.
            break;
        }
        let Some(colon) = text.find(':') else {
            return Err(format!("expected 'key: value' at line {}, got '{text}'", *pos + 1));
        };
        let key = text[..colon].trim().trim_matches('"').trim_matches('\'').to_string();
        let rest = text[colon + 1..].trim();
        *pos += 1;

        if rest.is_empty() {
            // Nested block follows (deeper indent), or Null if nothing follows.
            if *pos < lines.len() && lines[*pos].indent > block_indent {
                let child = parse_block(lines, pos, block_indent + 1)?;
                entries.push((key, child));
            } else {
                entries.push((key, Yaml::Null));
            }
        } else {
            entries.push((key, parse_scalar(rest)));
        }
    }
    Ok(Yaml::Map(entries))
}

/// Parse a `- ` sequence. Each item is either a scalar (`- x`), a nested map (`- k: v`
/// followed by more-indented continuation), or a nested block (`-` newline + deeper).
fn parse_list<'a>(
    lines: &[Line<'a>],
    pos: &mut usize,
    min_indent: usize,
) -> Result<Yaml, String> {
    let list_indent = lines[*pos].indent;
    if list_indent < min_indent {
        return Err(format!("bad indentation at line {}", *pos + 1));
    }
    let mut items = Vec::new();
    while *pos < lines.len() && lines[*pos].indent == list_indent {
        let text = lines[*pos].text;
        if text == "-" {
            // `-` alone: nested block follows.
            *pos += 1;
            if *pos < lines.len() && lines[*pos].indent > list_indent {
                items.push(parse_block(lines, pos, list_indent + 1)?);
            } else {
                items.push(Yaml::Null);
            }
            continue;
        }
        if !text.starts_with("- ") {
            break;
        }
        let rest = text[2..].trim();
        *pos += 1;

        if rest.is_empty() {
            if *pos < lines.len() && lines[*pos].indent > list_indent {
                items.push(parse_block(lines, pos, list_indent + 1)?);
            } else {
                items.push(Yaml::Null);
            }
            continue;
        }

        if let Some(colon) = rest.find(':') {
            // `- key: value` list-of-maps item.
            let key = rest[..colon].trim().trim_matches('"').trim_matches('\'').to_string();
            let val = rest[colon + 1..].trim();
            let mut entries: Vec<(String, Yaml)> = Vec::new();
            if val.is_empty() {
                // `- key:` with a deeper nested block under this item.
                if *pos < lines.len() && lines[*pos].indent > list_indent {
                    entries.push((key, parse_block(lines, pos, list_indent + 1)?));
                } else {
                    entries.push((key, Yaml::Null));
                }
            } else {
                entries.push((key, parse_scalar(val)));
            }
            // Continuation keys at the item's indent (list_indent + 2 for `- `).
            while *pos < lines.len() && lines[*pos].indent > list_indent {
                let line = &lines[*pos];
                let cont = line.text;
                let Some(colon) = cont.find(':') else {
                    break;
                };
                let ckey = cont[..colon].trim().trim_matches('"').trim_matches('\'').to_string();
                let cval = cont[colon + 1..].trim();
                *pos += 1;
                if cval.is_empty() {
                    if *pos < lines.len() && lines[*pos].indent > line.indent {
                        entries.push((ckey, parse_block(lines, pos, line.indent + 1)?));
                    } else {
                        entries.push((ckey, Yaml::Null));
                    }
                } else {
                    entries.push((ckey, parse_scalar(cval)));
                }
            }
            items.push(Yaml::Map(entries));
        } else {
            items.push(parse_scalar(rest));
        }
    }
    Ok(Yaml::List(items))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalars_and_comments() {
        let y = parse("key: value  # comment\nother: 42\n").unwrap();
        assert_eq!(y.get("key").unwrap().as_str(), Some("value"));
        assert_eq!(y.get("other").unwrap().as_i64(), Some(42));
    }

    #[test]
    fn quoted_scalars() {
        let y = parse("a: \"hello world\"\nb: '42'\n").unwrap();
        assert_eq!(y.get("a").unwrap().as_str(), Some("hello world"));
        // '42' is quoted, so it stays a string, not an int.
        assert_eq!(y.get("b").unwrap().as_str(), Some("42"));
    }

    #[test]
    fn nested_map() {
        let y = parse("saturation:\n  tier1: 8\n  tier2: 4\n").unwrap();
        let sat = y.get("saturation").unwrap();
        assert_eq!(sat.get("tier1").unwrap().as_i64(), Some(8));
        assert_eq!(sat.get("tier2").unwrap().as_i64(), Some(4));
    }

    #[test]
    fn list_of_maps() {
        let y = parse(
            "overrides:\n  - unit: Terran Mason\n    minerals: 50\n  - unit: Zerg X\n    gas: 300\n",
        )
        .unwrap();
        let ov = y.get("overrides").unwrap();
        let Yaml::List(items) = ov else { panic!("expected list") };
        assert_eq!(items.len(), 2);
        let Yaml::Map(m0) = &items[0] else { panic!("expected map") };
        assert_eq!(m0.iter().find(|(k, _)| k == "unit").unwrap().1.as_str(), Some("Terran Mason"));
        assert_eq!(m0.iter().find(|(k, _)| k == "minerals").unwrap().1.as_i64(), Some(50));
    }

    #[test]
    fn negative_numbers_and_quoted_keys() {
        let y = parse("gas_tier_deltas:\n  \"1\":\n    minerals: -100\n    build_time_seconds: -10\n  2:\n    minerals: -150\n").unwrap();
        let d = y.get("gas_tier_deltas").unwrap();
        assert_eq!(d.get("1").unwrap().get("minerals").unwrap().as_i64(), Some(-100));
        assert_eq!(d.get("1").unwrap().get("build_time_seconds").unwrap().as_i64(), Some(-10));
        assert_eq!(d.get("2").unwrap().get("minerals").unwrap().as_i64(), Some(-150));
    }

    #[test]
    fn tab_indent_rejected() {
        assert!(parse("key:\n\tchild: 1\n").is_err());
    }
}
