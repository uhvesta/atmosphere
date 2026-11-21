use serde_json::{Map, Number, Value};

use crate::ParseError;

#[derive(Debug, Clone)]
struct Line<'a> {
    num: usize,
    indent: usize,
    raw: &'a str,
    content: &'a str,
}

pub fn parse_yaml_to_value(input: &str) -> Result<Value, ParseError> {
    let mut lines = Vec::new();
    for (idx, raw_line) in input.lines().enumerate() {
        let num = idx + 1;
        let raw = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        let mut indent = 0usize;
        for b in raw.bytes() {
            match b {
                b' ' => indent += 1,
                b'\t' => {
                    return Err(ParseError::Yaml {
                        line: num,
                        message: "tabs are not allowed for indentation".to_string(),
                    });
                }
                _ => break,
            }
        }
        let content = &raw[indent..];
        lines.push(Line {
            num,
            indent,
            raw,
            content,
        });
    }

    let mut parser = Parser { lines, pos: 0 };
    parser.skip_ignorable();

    if let Some(line) = parser.peek() {
        if line.indent == 0 && is_doc_start(line.content) {
            parser.pos += 1;
            parser.skip_ignorable();
        }
    }

    if parser.is_at_end() {
        return Err(ParseError::Yaml {
            line: 1,
            message: "empty YAML document".to_string(),
        });
    }

    let base_indent = parser.peek().unwrap().indent;
    let value = parser.parse_block(base_indent)?;
    parser.skip_ignorable();

    if let Some(line) = parser.peek() {
        if line.indent == 0 && is_doc_end(line.content) {
            parser.pos += 1;
            parser.skip_ignorable();
        }
    }

    if let Some(line) = parser.peek() {
        return Err(ParseError::Yaml {
            line: line.num,
            message: format!("unexpected trailing content: {}", line.content),
        });
    }

    Ok(value)
}

fn is_doc_start(s: &str) -> bool {
    s == "---" || s.starts_with("--- ") || s.starts_with("---\t")
}

fn is_doc_end(s: &str) -> bool {
    s == "..." || s.starts_with("... ") || s.starts_with("...\t")
}

fn is_ignorable(content: &str) -> bool {
    content.is_empty() || content.starts_with('#')
}

struct Parser<'a> {
    lines: Vec<Line<'a>>,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Line<'a>> {
        self.lines.get(self.pos)
    }

    fn is_at_end(&self) -> bool {
        self.pos >= self.lines.len()
    }

    fn skip_ignorable(&mut self) {
        while let Some(line) = self.peek() {
            if is_ignorable(line.content) {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn parse_block(&mut self, expected_indent: usize) -> Result<Value, ParseError> {
        self.skip_ignorable();
        let line = self.peek().ok_or_else(|| ParseError::Yaml {
            line: self.lines.len().max(1),
            message: "unexpected end of document".to_string(),
        })?;

        if line.indent != expected_indent {
            return Err(ParseError::Yaml {
                line: line.num,
                message: format!(
                    "expected indentation {}, found {}",
                    expected_indent, line.indent
                ),
            });
        }

        if is_sequence_item(line.content) {
            self.parse_sequence(expected_indent)
        } else if line.content.starts_with('[') || line.content.starts_with('{') {
            let line_num = line.num;
            let content = strip_inline_comment(line.content).trim();
            self.pos += 1;
            parse_flow_value(content, line_num)
        } else if split_mapping_entry(line.content).is_some() {
            self.parse_mapping(expected_indent)
        } else {
            let line_num = line.num;
            let content = line.content;
            self.pos += 1;
            parse_scalar(content, line_num)
        }
    }

    fn parse_sequence(&mut self, indent: usize) -> Result<Value, ParseError> {
        let mut items = Vec::new();
        loop {
            self.skip_ignorable();
            let Some(line) = self.peek().cloned() else {
                break;
            };
            if line.indent == 0 && is_doc_end(line.content) {
                break;
            }
            if line.indent < indent {
                break;
            }
            if line.indent > indent {
                return Err(ParseError::Yaml {
                    line: line.num,
                    message: format!(
                        "unexpected indentation {} inside sequence at indent {}",
                        line.indent, indent
                    ),
                });
            }
            if !is_sequence_item(line.content) {
                break;
            }

            let rest = line.content[1..].trim_start_matches(' ');
            let item_col = line.indent + (line.content.len() - rest.len());
            self.pos += 1;

            if is_ignorable(rest) {
                self.skip_ignorable();
                if let Some(next) = self.peek() {
                    if next.indent > indent {
                        let child_indent = next.indent;
                        items.push(self.parse_block(child_indent)?);
                    } else {
                        items.push(Value::Null);
                    }
                } else {
                    items.push(Value::Null);
                }
            } else if let Some(header) = parse_block_scalar_header(rest) {
                let s = self.parse_block_scalar(indent, header, line.num)?;
                items.push(Value::String(s));
            } else if let Some((key_raw, val_raw)) = split_mapping_entry(rest) {
                let map = self.parse_inline_mapping_item(item_col, line.num, key_raw, val_raw)?;
                items.push(Value::Object(map));
            } else if is_sequence_item(rest) {
                return Err(ParseError::Yaml {
                    line: line.num,
                    message: "nested inline sequences (`- -`) are not supported".to_string(),
                });
            } else {
                items.push(parse_inline_value(rest, line.num)?);
            }
        }
        Ok(Value::Array(items))
    }

    fn parse_inline_mapping_item(
        &mut self,
        map_indent: usize,
        first_line_num: usize,
        first_key_raw: &str,
        first_val_raw: &str,
    ) -> Result<Map<String, Value>, ParseError> {
        let mut map = Map::new();
        let first_key = parse_key(first_key_raw, first_line_num)?;
        let first_val = self.parse_mapping_value(map_indent, first_line_num, first_val_raw)?;
        map.insert(first_key, first_val);
        self.parse_mapping_entries_into(map_indent, &mut map)?;
        Ok(map)
    }

    fn parse_mapping(&mut self, indent: usize) -> Result<Value, ParseError> {
        let mut map = Map::new();
        self.parse_mapping_entries_into(indent, &mut map)?;
        Ok(Value::Object(map))
    }

    fn parse_mapping_entries_into(
        &mut self,
        indent: usize,
        map: &mut Map<String, Value>,
    ) -> Result<(), ParseError> {
        loop {
            self.skip_ignorable();
            let Some(line) = self.peek().cloned() else {
                break;
            };
            if line.indent == 0 && is_doc_end(line.content) {
                break;
            }
            if line.indent < indent {
                break;
            }
            if line.indent > indent {
                return Err(ParseError::Yaml {
                    line: line.num,
                    message: format!(
                        "unexpected indentation {} inside mapping at indent {}",
                        line.indent, indent
                    ),
                });
            }
            if is_sequence_item(line.content) {
                return Err(ParseError::Yaml {
                    line: line.num,
                    message: "unexpected sequence item inside mapping".to_string(),
                });
            }

            let (key_raw, val_raw) =
                split_mapping_entry(line.content).ok_or_else(|| ParseError::Yaml {
                    line: line.num,
                    message: format!("expected `key: value` mapping entry, found: {}", line.content),
                })?;

            let key = parse_key(key_raw, line.num)?;
            if map.contains_key(&key) {
                return Err(ParseError::Yaml {
                    line: line.num,
                    message: format!("duplicate key `{key}`"),
                });
            }

            self.pos += 1;
            let val = self.parse_mapping_value(indent, line.num, val_raw)?;
            map.insert(key, val);
        }
        Ok(())
    }

    fn parse_mapping_value(
        &mut self,
        parent_indent: usize,
        line_num: usize,
        val_raw: &str,
    ) -> Result<Value, ParseError> {
        let trimmed = val_raw.trim_start_matches(' ');
        if is_ignorable(trimmed) {
            self.skip_ignorable();
            if let Some(next) = self.peek() {
                if next.indent > parent_indent {
                    let child_indent = next.indent;
                    return self.parse_block(child_indent);
                }
                if next.indent == parent_indent && is_sequence_item(next.content) {
                    // YAML allows `-` sequence items at the same indent as the mapping key.
                    return self.parse_sequence(parent_indent);
                }
            }
            return Ok(Value::Null);
        }

        if let Some(header) = parse_block_scalar_header(trimmed) {
            let s = self.parse_block_scalar(parent_indent, header, line_num)?;
            return Ok(Value::String(s));
        }

        parse_inline_value(trimmed, line_num)
    }

    fn parse_block_scalar(
        &mut self,
        parent_indent: usize,
        header: BlockScalarHeader,
        _header_line: usize,
    ) -> Result<String, ParseError> {
        let mut raw_lines: Vec<Option<&'a str>> = Vec::new();
        let mut content_indent: Option<usize> = None;

        while let Some(line) = self.peek() {
            if line.content.is_empty() {
                raw_lines.push(None);
                self.pos += 1;
                continue;
            }
            if line.indent <= parent_indent {
                break;
            }
            let ci = match content_indent {
                Some(ci) => ci,
                None => {
                    content_indent = Some(line.indent);
                    line.indent
                }
            };
            if line.indent < ci {
                break;
            }
            let slice = if line.raw.len() >= ci {
                &line.raw[ci..]
            } else {
                ""
            };
            raw_lines.push(Some(slice));
            self.pos += 1;
        }

        // Count trailing blank lines
        let mut trailing_blanks = 0usize;
        while matches!(raw_lines.last(), Some(None)) {
            raw_lines.pop();
            trailing_blanks += 1;
        }

        if raw_lines.is_empty() {
            return match header.chomp {
                Chomp::Keep => Ok("\n".repeat(trailing_blanks)),
                Chomp::Clip | Chomp::Strip => Ok(String::new()),
            };
        }

        let mut out = String::new();
        match header.style {
            BlockStyle::Literal => {
                for (i, item) in raw_lines.iter().enumerate() {
                    if i > 0 {
                        out.push('\n');
                    }
                    if let Some(text) = item {
                        out.push_str(text);
                    }
                }
            }
            BlockStyle::Folded => {
                let mut prev_was_blank = false;
                let mut prev_was_indented = false;
                for (i, item) in raw_lines.iter().enumerate() {
                    match item {
                        None => {
                            out.push('\n');
                            prev_was_blank = true;
                            prev_was_indented = false;
                        }
                        Some(text) => {
                            let is_indented = text.starts_with(' ');
                            if i > 0 && !prev_was_blank {
                                if is_indented || prev_was_indented {
                                    out.push('\n');
                                } else {
                                    out.push(' ');
                                }
                            }
                            out.push_str(text);
                            prev_was_blank = false;
                            prev_was_indented = is_indented;
                        }
                    }
                }
            }
        }

        match header.chomp {
            Chomp::Strip => {}
            Chomp::Clip => out.push('\n'),
            Chomp::Keep => {
                out.push('\n');
                for _ in 0..trailing_blanks {
                    out.push('\n');
                }
            }
        }

        Ok(out)
    }
}

#[derive(Clone, Copy)]
enum BlockStyle {
    Literal,
    Folded,
}

#[derive(Clone, Copy)]
enum Chomp {
    Clip,
    Strip,
    Keep,
}

#[derive(Clone, Copy)]
struct BlockScalarHeader {
    style: BlockStyle,
    chomp: Chomp,
}

fn parse_block_scalar_header(s: &str) -> Option<BlockScalarHeader> {
    let s = strip_inline_comment(s).trim();
    match s {
        "|" => Some(BlockScalarHeader {
            style: BlockStyle::Literal,
            chomp: Chomp::Clip,
        }),
        "|-" => Some(BlockScalarHeader {
            style: BlockStyle::Literal,
            chomp: Chomp::Strip,
        }),
        "|+" => Some(BlockScalarHeader {
            style: BlockStyle::Literal,
            chomp: Chomp::Keep,
        }),
        ">" => Some(BlockScalarHeader {
            style: BlockStyle::Folded,
            chomp: Chomp::Clip,
        }),
        ">-" => Some(BlockScalarHeader {
            style: BlockStyle::Folded,
            chomp: Chomp::Strip,
        }),
        ">+" => Some(BlockScalarHeader {
            style: BlockStyle::Folded,
            chomp: Chomp::Keep,
        }),
        _ => None,
    }
}

fn is_sequence_item(content: &str) -> bool {
    content == "-" || content.starts_with("- ")
}

fn split_mapping_entry(s: &str) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_single {
            if b == b'\'' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                    i += 2;
                    continue;
                }
                in_single = false;
            }
            i += 1;
            continue;
        }
        if in_double {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == b'"' {
                in_double = false;
            }
            i += 1;
            continue;
        }
        match b {
            b'\'' => in_single = true,
            b'"' => in_double = true,
            b'#' => {
                if i == 0 || bytes[i - 1] == b' ' {
                    return None;
                }
            }
            b':' => {
                let next_is_boundary = i + 1 == bytes.len() || bytes[i + 1] == b' ';
                if next_is_boundary {
                    let key = s[..i].trim();
                    if key.is_empty() {
                        return None;
                    }
                    let val = &s[i + 1..];
                    return Some((key, val));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn parse_key(raw: &str, line: usize) -> Result<String, ParseError> {
    let trimmed = raw.trim();
    if trimmed.starts_with('"') {
        parse_double_quoted(trimmed, line)
    } else if trimmed.starts_with('\'') {
        parse_single_quoted(trimmed, line)
    } else {
        Ok(trimmed.to_string())
    }
}

fn parse_inline_value(raw: &str, line: usize) -> Result<Value, ParseError> {
    let trimmed = strip_inline_comment(raw).trim();
    if trimmed.starts_with('[') || trimmed.starts_with('{') {
        parse_flow_value(trimmed, line)
    } else {
        parse_scalar(raw, line)
    }
}

fn parse_scalar(raw: &str, line: usize) -> Result<Value, ParseError> {
    let trimmed = raw.trim_start();
    if trimmed.starts_with('"') {
        let (s, rest) = parse_double_quoted_prefix(trimmed, line)?;
        let rest = strip_inline_comment(rest).trim();
        if !rest.is_empty() {
            return Err(ParseError::Yaml {
                line,
                message: format!("unexpected trailing characters after quoted string: {rest}"),
            });
        }
        return Ok(Value::String(s));
    }
    if trimmed.starts_with('\'') {
        let (s, rest) = parse_single_quoted_prefix(trimmed, line)?;
        let rest = strip_inline_comment(rest).trim();
        if !rest.is_empty() {
            return Err(ParseError::Yaml {
                line,
                message: format!("unexpected trailing characters after quoted string: {rest}"),
            });
        }
        return Ok(Value::String(s));
    }

    let unquoted = strip_inline_comment(trimmed).trim();
    match unquoted {
        "" | "null" | "~" => Ok(Value::Null),
        "true" => Ok(Value::Bool(true)),
        "false" => Ok(Value::Bool(false)),
        other => {
            if let Ok(n) = other.parse::<i64>() {
                Ok(Value::Number(Number::from(n)))
            } else {
                Ok(Value::String(other.to_string()))
            }
        }
    }
}

fn strip_inline_comment(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_single {
            if b == b'\'' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                    i += 2;
                    continue;
                }
                in_single = false;
            }
            i += 1;
            continue;
        }
        if in_double {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == b'"' {
                in_double = false;
            }
            i += 1;
            continue;
        }
        match b {
            b'\'' => in_single = true,
            b'"' => in_double = true,
            b'#' if i == 0 || bytes[i - 1] == b' ' => return &s[..i],
            _ => {}
        }
        i += 1;
    }
    s
}

fn parse_single_quoted(s: &str, line: usize) -> Result<String, ParseError> {
    let (out, rest) = parse_single_quoted_prefix(s, line)?;
    if !rest.trim().is_empty() {
        return Err(ParseError::Yaml {
            line,
            message: format!("unexpected trailing characters: {}", rest.trim()),
        });
    }
    Ok(out)
}

fn parse_single_quoted_prefix(s: &str, line: usize) -> Result<(String, &str), ParseError> {
    let Some(mut rest) = s.strip_prefix('\'') else {
        return Err(ParseError::Yaml {
            line,
            message: "expected opening single quote".to_string(),
        });
    };
    let mut out = String::new();
    while !rest.is_empty() {
        let mut chars = rest.chars();
        let ch = chars.next().unwrap();
        rest = chars.as_str();
        if ch == '\'' {
            if let Some(after_second) = rest.strip_prefix('\'') {
                out.push('\'');
                rest = after_second;
            } else {
                return Ok((out, rest));
            }
        } else {
            out.push(ch);
        }
    }
    Err(ParseError::Yaml {
        line,
        message: "unterminated single-quoted string".to_string(),
    })
}

fn parse_double_quoted(s: &str, line: usize) -> Result<String, ParseError> {
    let (out, rest) = parse_double_quoted_prefix(s, line)?;
    if !rest.trim().is_empty() {
        return Err(ParseError::Yaml {
            line,
            message: format!("unexpected trailing characters: {}", rest.trim()),
        });
    }
    Ok(out)
}

fn parse_double_quoted_prefix(s: &str, line: usize) -> Result<(String, &str), ParseError> {
    let Some(mut rest) = s.strip_prefix('"') else {
        return Err(ParseError::Yaml {
            line,
            message: "expected opening double quote".to_string(),
        });
    };
    let mut out = String::new();
    while !rest.is_empty() {
        let mut chars = rest.chars();
        let ch = chars.next().unwrap();
        rest = chars.as_str();
        match ch {
            '"' => return Ok((out, rest)),
            '\\' => {
                let mut esc_chars = rest.chars();
                let esc = esc_chars.next().ok_or_else(|| ParseError::Yaml {
                    line,
                    message: "unterminated escape sequence".to_string(),
                })?;
                rest = esc_chars.as_str();
                match esc {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    '/' => out.push('/'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    '0' => out.push('\0'),
                    'u' => {
                        if rest.len() < 4 {
                            return Err(ParseError::Yaml {
                                line,
                                message: "invalid \\u escape".to_string(),
                            });
                        }
                        let hex = &rest[..4];
                        rest = &rest[4..];
                        let code = u32::from_str_radix(hex, 16).map_err(|_| ParseError::Yaml {
                            line,
                            message: format!("invalid unicode escape \\u{hex}"),
                        })?;
                        let c = char::from_u32(code).ok_or_else(|| ParseError::Yaml {
                            line,
                            message: format!("invalid unicode scalar \\u{hex}"),
                        })?;
                        out.push(c);
                    }
                    other => {
                        return Err(ParseError::Yaml {
                            line,
                            message: format!("unsupported escape sequence \\{other}"),
                        });
                    }
                }
            }
            c => out.push(c),
        }
    }
    Err(ParseError::Yaml {
        line,
        message: "unterminated double-quoted string".to_string(),
    })
}

fn parse_flow_value(s: &str, line: usize) -> Result<Value, ParseError> {
    let mut fp = FlowParser { s: s.trim(), line };
    let val = fp.parse_value()?;
    fp.skip_ws();
    if !fp.s.is_empty() {
        return Err(ParseError::Yaml {
            line,
            message: format!("unexpected trailing characters in flow value: {}", fp.s),
        });
    }
    Ok(val)
}

struct FlowParser<'a> {
    s: &'a str,
    line: usize,
}

impl<'a> FlowParser<'a> {
    fn skip_ws(&mut self) {
        self.s = self.s.trim_start_matches([' ', '\t']);
    }

    fn parse_value(&mut self) -> Result<Value, ParseError> {
        self.skip_ws();
        if self.s.starts_with('[') {
            self.parse_array()
        } else if self.s.starts_with('{') {
            self.parse_object()
        } else if self.s.starts_with('"') {
            let (s, rest) = parse_double_quoted_prefix(self.s, self.line)?;
            self.s = rest;
            Ok(Value::String(s))
        } else if self.s.starts_with('\'') {
            let (s, rest) = parse_single_quoted_prefix(self.s, self.line)?;
            self.s = rest;
            Ok(Value::String(s))
        } else {
            let end = self
                .s
                .find([',', ']', '}'])
                .unwrap_or(self.s.len());
            let token = &self.s[..end];
            self.s = &self.s[end..];
            parse_scalar(token, self.line)
        }
    }

    fn parse_array(&mut self) -> Result<Value, ParseError> {
        self.s = &self.s[1..];
        let mut items = Vec::new();
        self.skip_ws();
        if let Some(rest) = self.s.strip_prefix(']') {
            self.s = rest;
            return Ok(Value::Array(items));
        }
        loop {
            items.push(self.parse_value()?);
            self.skip_ws();
            if let Some(rest) = self.s.strip_prefix(',') {
                self.s = rest;
                self.skip_ws();
                if let Some(after) = self.s.strip_prefix(']') {
                    self.s = after;
                    return Ok(Value::Array(items));
                }
            } else if let Some(rest) = self.s.strip_prefix(']') {
                self.s = rest;
                return Ok(Value::Array(items));
            } else {
                return Err(ParseError::Yaml {
                    line: self.line,
                    message: "expected `,` or `]` in flow sequence".to_string(),
                });
            }
        }
    }

    fn parse_object(&mut self) -> Result<Value, ParseError> {
        self.s = &self.s[1..];
        let mut map = Map::new();
        self.skip_ws();
        if let Some(rest) = self.s.strip_prefix('}') {
            self.s = rest;
            return Ok(Value::Object(map));
        }
        loop {
            self.skip_ws();
            let key = if self.s.starts_with('"') {
                let (k, rest) = parse_double_quoted_prefix(self.s, self.line)?;
                self.s = rest;
                k
            } else if self.s.starts_with('\'') {
                let (k, rest) = parse_single_quoted_prefix(self.s, self.line)?;
                self.s = rest;
                k
            } else {
                let colon = self.s.find(':').ok_or_else(|| ParseError::Yaml {
                    line: self.line,
                    message: "expected `:` in flow mapping".to_string(),
                })?;
                let k = self.s[..colon].trim().to_string();
                self.s = &self.s[colon..];
                k
            };
            self.skip_ws();
            let Some(rest) = self.s.strip_prefix(':') else {
                return Err(ParseError::Yaml {
                    line: self.line,
                    message: "expected `:` after key in flow mapping".to_string(),
                });
            };
            self.s = rest;
            if map.contains_key(&key) {
                return Err(ParseError::Yaml {
                    line: self.line,
                    message: format!("duplicate key `{key}`"),
                });
            }
            let val = self.parse_value()?;
            map.insert(key, val);
            self.skip_ws();
            if let Some(rest) = self.s.strip_prefix(',') {
                self.s = rest;
                self.skip_ws();
                if let Some(after) = self.s.strip_prefix('}') {
                    self.s = after;
                    return Ok(Value::Object(map));
                }
            } else if let Some(rest) = self.s.strip_prefix('}') {
                self.s = rest;
                return Ok(Value::Object(map));
            } else {
                return Err(ParseError::Yaml {
                    line: self.line,
                    message: "expected `,` or `}` in flow mapping".to_string(),
                });
            }
        }
    }
}
