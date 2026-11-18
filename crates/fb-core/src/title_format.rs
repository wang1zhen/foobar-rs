//! Subset of foobar2000 title formatting used by columns and panel titles.
//!
//! Supported syntax:
//! - `%field%` expands a field; `%%` is a literal `%`.
//! - `$if(cond,a,b)`, `$if2(a,b)`, `$num(x,n)`, `$left(s,n)`, `$right(s,n)`,
//!   `$lower(s)`, `$upper(s)`, `$replace(s,from,to)`, `$meta(name)`.
//! - Functions nest and names are case-insensitive; missing fields expand to "".
//!
//! Semantics kept deliberately small: `$if` treats "" and "0" as false; `$num`
//! keeps the integer part, zero-pads it to the requested digit count (1..=32,
//! sign counts toward the width, decimals truncated) and prints "" for an
//! unparsable value;
//! `$left`/`$right` count Unicode chars with a non-negative `n` (invalid `n`
//! means 0). Nesting is limited to [`MAX_NESTING_DEPTH`] and rendered output to
//! [`MAX_OUTPUT_BYTES`], truncated on a char boundary. [`format`] returns the
//! template unchanged when parsing fails; [`validate`] reports the parse error
//! at the failing byte offset.

use std::collections::BTreeMap;

/// Maximum function nesting accepted by the parser.
pub const MAX_NESTING_DEPTH: usize = 32;
/// Maximum rendered output; anything longer is truncated on a char boundary.
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024;
/// Maximum zero-padding length accepted by `$num`.
const MAX_NUM_LENGTH: usize = 32;

/// Looks up a metadata field by normalized name.
pub trait FieldLookup {
    fn field(&self, name: &str) -> Option<&str>;
}

/// Simple in-memory field source for tests and callers without a `Track` handle.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FieldMap(BTreeMap<String, String>);

impl FieldMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.0.insert(name.into(), value.into());
    }
}

impl FieldLookup for FieldMap {
    fn field(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid title format at byte {position}: {message}")]
pub struct FormatError {
    pub position: usize,
    pub message: String,
}

/// Formats `template`. Output is capped at [`MAX_OUTPUT_BYTES`] (truncated on a
/// char boundary); invalid syntax is returned verbatim, so a bad user template
/// never loses text or panics.
pub fn format(template: &str, fields: &dyn FieldLookup) -> String {
    match parse(template) {
        Ok(nodes) => {
            let mut buffer = String::with_capacity(template.len().min(MAX_OUTPUT_BYTES));
            let mut out = Output {
                buffer: &mut buffer,
                limit: MAX_OUTPUT_BYTES,
            };
            render(&nodes, fields, &mut out);
            buffer
        }
        Err(_) => template.to_owned(),
    }
}

/// Parses `template` without evaluating it, rejecting unknown functions,
/// wrong arity and unbalanced syntax.
pub fn validate(template: &str) -> Result<(), FormatError> {
    parse(template).map(|_| ())
}

enum Node {
    Text(String),
    Field(String),
    Func { name: String, args: Vec<Vec<Node>> },
}

fn parse(template: &str) -> Result<Vec<Node>, FormatError> {
    let mut parser = Parser {
        src: template,
        pos: 0,
        depth: 0,
    };
    let (nodes, _) = parser.parse_nodes(false)?;
    Ok(nodes)
}

struct Parser<'a> {
    src: &'a str,
    pos: usize,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn parse_nodes(&mut self, in_args: bool) -> Result<(Vec<Node>, Option<char>), FormatError> {
        let mut nodes = Vec::new();
        let mut text = String::new();

        loop {
            let Some(ch) = self.peek() else {
                if in_args {
                    return Err(error(self.pos, "unclosed function argument list"));
                }
                flush(&mut nodes, &mut text);
                return Ok((nodes, None));
            };

            match ch {
                '%' => {
                    flush(&mut nodes, &mut text);
                    nodes.push(self.parse_field()?);
                }
                '$' => {
                    flush(&mut nodes, &mut text);
                    nodes.push(self.parse_function()?);
                }
                ')' if in_args => {
                    self.pos += 1;
                    flush(&mut nodes, &mut text);
                    return Ok((nodes, Some(')')));
                }
                ',' if in_args => {
                    self.pos += 1;
                    flush(&mut nodes, &mut text);
                    return Ok((nodes, Some(',')));
                }
                _ => {
                    text.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    fn parse_field(&mut self) -> Result<Node, FormatError> {
        let start = self.pos;
        self.pos += 1;
        let Some(end) = self.src[self.pos..].find('%') else {
            return Err(error(start, "unclosed %field%"));
        };
        let name = &self.src[self.pos..self.pos + end];
        self.pos += end + 1;

        if name.is_empty() {
            Ok(Node::Text("%".into()))
        } else {
            Ok(Node::Field(name.trim().to_owned()))
        }
    }

    fn parse_function(&mut self) -> Result<Node, FormatError> {
        let start = self.pos;
        self.pos += 1;

        let name_start = self.pos;
        while matches!(self.peek(), Some(ch) if ch.is_ascii_alphanumeric() || ch == '_') {
            self.pos += 1;
        }
        let raw_name = &self.src[name_start..self.pos];
        if raw_name.is_empty() {
            return Err(error(start, "expected a function name after '$'"));
        }
        if self.peek() != Some('(') {
            return Err(error(start, format!("expected '(' after ${raw_name}")));
        }
        self.pos += 1;

        if self.depth >= MAX_NESTING_DEPTH {
            return Err(error(
                start,
                format!("nesting deeper than {MAX_NESTING_DEPTH} levels"),
            ));
        }
        self.depth += 1;
        let args = self.parse_args(raw_name, start);
        self.depth -= 1;
        let args = args?;

        let name = raw_name.to_ascii_lowercase();
        let Some(expected) = arity(&name) else {
            return Err(error(start, format!("unknown function ${name}")));
        };
        if args.len() != expected {
            return Err(error(
                start,
                format!("${name} expects {expected} argument(s), got {}", args.len()),
            ));
        }
        Ok(Node::Func { name, args })
    }

    fn parse_args(&mut self, raw_name: &str, start: usize) -> Result<Vec<Vec<Node>>, FormatError> {
        let mut args = Vec::new();
        loop {
            let (nodes, terminator) = self.parse_nodes(true)?;
            args.push(nodes);
            match terminator {
                Some(',') => {}
                Some(')') => break,
                _ => return Err(error(start, format!("unclosed ${raw_name}("))),
            }
        }
        Ok(args)
    }
}

fn arity(name: &str) -> Option<usize> {
    match name {
        "if" => Some(3),
        "if2" => Some(2),
        "num" | "left" | "right" => Some(2),
        "lower" | "upper" | "meta" => Some(1),
        "replace" => Some(3),
        _ => None,
    }
}

fn error(position: usize, message: impl Into<String>) -> FormatError {
    FormatError {
        position,
        message: message.into(),
    }
}

fn flush(nodes: &mut Vec<Node>, text: &mut String) {
    if !text.is_empty() {
        nodes.push(Node::Text(std::mem::take(text)));
    }
}

/// Bounded string sink. Refuses to grow past `limit` and truncates on a char
/// boundary, so a template cannot amplify itself into an unbounded title.
struct Output<'a> {
    buffer: &'a mut String,
    limit: usize,
}

impl Output<'_> {
    fn is_full(&self) -> bool {
        self.buffer.len() >= self.limit
    }

    fn push_str(&mut self, value: &str) {
        let remaining = self.limit.saturating_sub(self.buffer.len());
        if value.len() <= remaining {
            self.buffer.push_str(value);
            return;
        }
        let mut end = remaining;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        self.buffer.push_str(&value[..end]);
    }

    fn push(&mut self, ch: char) {
        if self.buffer.len() + ch.len_utf8() <= self.limit {
            self.buffer.push(ch);
        }
    }
}

fn render(nodes: &[Node], fields: &dyn FieldLookup, out: &mut Output) {
    for node in nodes {
        match node {
            Node::Text(text) => out.push_str(text),
            Node::Field(name) => out.push_str(fields.field(name).unwrap_or("")),
            Node::Func { name, args } => render_func(name, args, fields, out),
        }
    }
}

fn eval(nodes: &[Node], fields: &dyn FieldLookup) -> String {
    let mut buffer = String::new();
    let mut out = Output {
        buffer: &mut buffer,
        limit: MAX_OUTPUT_BYTES,
    };
    render(nodes, fields, &mut out);
    buffer
}

fn count_arg(nodes: &[Node], fields: &dyn FieldLookup) -> usize {
    eval(nodes, fields).trim().parse().unwrap_or(0)
}

/// `$num` keeps the integer part and zero-pads it to `length` digits; the sign
/// counts toward the width. Negative numbers keep their sign, decimals truncate
/// toward 0.
fn format_number(value: f64, length: usize) -> String {
    let integer = value.trunc();
    if !integer.is_finite() {
        return String::new();
    }

    let number = integer as i64;
    let digits = number.unsigned_abs().to_string();
    let sign_width = usize::from(number < 0);
    let padding = length.saturating_sub(digits.len() + sign_width);

    let mut out = String::with_capacity(length + 1);
    if number < 0 {
        out.push('-');
    }
    out.push_str(&"0".repeat(padding));
    out.push_str(&digits);
    out
}

fn push_replaced(out: &mut Output, value: &str, from: &str, to: &str) {
    if from.is_empty() {
        out.push_str(value);
        return;
    }

    let mut rest = value;
    while let Some(index) = rest.find(from) {
        out.push_str(&rest[..index]);
        out.push_str(to);
        if out.is_full() {
            return;
        }
        rest = &rest[index + from.len()..];
    }
    out.push_str(rest);
}

fn render_func(name: &str, args: &[Vec<Node>], fields: &dyn FieldLookup, out: &mut Output) {
    match name {
        "if" => {
            let condition = eval(&args[0], fields);
            let branch = if !condition.is_empty() && condition != "0" {
                &args[1]
            } else {
                &args[2]
            };
            render(branch, fields, out);
        }
        "if2" => {
            for arg in args {
                let value = eval(arg, fields);
                if !value.is_empty() {
                    out.push_str(&value);
                    break;
                }
            }
        }
        "num" => {
            let value = eval(&args[0], fields);
            let length = count_arg(&args[1], fields).clamp(1, MAX_NUM_LENGTH);
            if let Ok(number) = value.trim().parse::<f64>() {
                out.push_str(&format_number(number, length));
            }
        }
        "left" => {
            let value = eval(&args[0], fields);
            let count = count_arg(&args[1], fields);
            for ch in value.chars().take(count) {
                out.push(ch);
            }
        }
        "right" => {
            let value = eval(&args[0], fields);
            let count = count_arg(&args[1], fields);
            let total = value.chars().count();
            for ch in value.chars().skip(total.saturating_sub(count)) {
                out.push(ch);
            }
        }
        "lower" => out.push_str(&eval(&args[0], fields).to_lowercase()),
        "upper" => out.push_str(&eval(&args[0], fields).to_uppercase()),
        "replace" => {
            let value = eval(&args[0], fields);
            let from = eval(&args[1], fields);
            let to = eval(&args[2], fields);
            push_replaced(out, &value, &from, &to);
        }
        "meta" => {
            let field = eval(&args[0], fields);
            out.push_str(fields.field(field.trim()).unwrap_or(""));
        }
        _ => unreachable!("arity() rejects unknown functions"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields() -> FieldMap {
        let mut fields = FieldMap::new();
        fields.insert("title", "Song Title");
        fields.insert("artist", "Band");
        fields.insert("album", "Record");
        fields.insert("album_artist", "Various");
        fields.insert("track", "03");
        fields.insert("disc", "1");
        fields.insert("year", "1999");
        fields.insert("duration", "213");
        fields.insert("format", "FLAC");
        fields.insert("path", "/music/a.flac");
        fields
    }

    #[test]
    fn expands_fields_and_missing_ones_as_empty() {
        assert_eq!(
            format("%artist% - %title% (%year%)", &fields()),
            "Band - Song Title (1999)"
        );
        assert_eq!(format("[%missing%]", &fields()), "[]");
        assert_eq!(format("%title% % unknown %", &fields()), "Song Title ");
    }

    #[test]
    fn double_percent_is_a_literal_percent() {
        assert_eq!(format("100%% %artist%", &fields()), "100% Band");
        assert_eq!(format("%%title%%", &fields()), "%title%");
        assert_eq!(format("%%", &fields()), "%");
    }

    #[test]
    fn if_uses_empty_and_zero_as_false() {
        assert_eq!(
            format("$if(%artist%,has artist,unknown)", &fields()),
            "has artist"
        );
        assert_eq!(
            format("$if(%missing%,has artist,unknown)", &fields()),
            "unknown"
        );
        assert_eq!(format("$if(0,yes,no)", &fields()), "no");
        assert_eq!(format("$if(1,yes,no)", &fields()), "yes");
    }

    #[test]
    fn if2_picks_the_first_non_empty_value() {
        assert_eq!(
            format("$if2(%missing%,%album_artist%)", &fields()),
            "Various"
        );
        assert_eq!(format("$if2(%missing%,%nope%)", &fields()), "");
    }

    #[test]
    fn number_pads_integers_and_keeps_sign_and_truncates_decimals() {
        assert_eq!(format("$num(3,2)", &fields()), "03");
        assert_eq!(format("$num(123,2)", &fields()), "123");
        assert_eq!(format("$num(-3,3)", &fields()), "-03");
        assert_eq!(format("$num(-123,2)", &fields()), "-123");
        assert_eq!(format("$num(3.14159,2)", &fields()), "03");
        assert_eq!(format("$num(-1.5,1)", &fields()), "-1");
    }

    #[test]
    fn number_length_is_clamped_and_bad_values_are_empty() {
        assert_eq!(format("$num(7,0)", &fields()), "7");
        assert_eq!(
            format("$num(7,99)", &fields()),
            format!("{}7", "0".repeat(31))
        );
        assert_eq!(format("$num(%missing%,2)", &fields()), "");
        assert_eq!(format("$num(abc,2)", &fields()), "");
        assert_eq!(format("$num(inf,2)", &fields()), "");
        assert_eq!(format("$num(NaN,2)", &fields()), "");
    }

    #[test]
    fn left_right_and_case_work_on_unicode() {
        assert_eq!(format("$left(%title%,4)", &fields()), "Song");
        assert_eq!(format("$right(%title%,5)", &fields()), "Title");
        assert_eq!(format("$left(%title%,99)", &fields()), "Song Title");
        assert_eq!(format("$left(中文标题,2)", &fields()), "中文");
        assert_eq!(format("$right(中文标题,2)", &fields()), "标题");
        assert_eq!(format("$upper($left(%title%,4))", &fields()), "SONG");
        assert_eq!(format("$lower(%format%)", &fields()), "flac");
    }

    #[test]
    fn replace_and_meta() {
        assert_eq!(format("$replace(%title%, ,_)", &fields()), "Song_Title");
        assert_eq!(format("$replace(%title%,,x)", &fields()), "Song Title");
        assert_eq!(format("$meta(artist)", &fields()), "Band");
        assert_eq!(format("$meta($lower(ARTIST))", &fields()), "Band");
        assert_eq!(format("$meta(missing)", &fields()), "");
    }

    #[test]
    fn nested_calls_and_commas_in_nested_arguments() {
        assert_eq!(
            format("$if(%artist%,$upper($left(%album%,3)),none)", &fields()),
            "REC"
        );
        assert_eq!(
            format("$replace($lower(%album%),c,$upper(o))", &fields()),
            "reOord"
        );
    }

    #[test]
    fn validate_accepts_valid_templates() {
        assert!(validate("%artist% - %title%").is_ok());
        assert!(validate("$if(%artist%,%% %title%,none)").is_ok());
        assert!(validate("").is_ok());
    }

    #[test]
    fn validate_rejects_invalid_syntax_with_position() {
        assert!(validate("%title").is_err());
        assert!(validate("$left(%title%,2").is_err());
        assert!(validate("$nope(x)").is_err());
        assert!(validate("$lower(a,b)").is_err());
        assert!(validate("$").is_err());
        assert!(validate("$lower").is_err());

        let err = validate("$nope(x)").unwrap_err();
        assert_eq!(err.position, 0);
        assert!(err.to_string().contains("unknown function"));
    }

    #[test]
    fn literal_parentheses_are_text() {
        assert_eq!(format("(Live) [%year%]", &fields()), "(Live) [1999]");
        assert_eq!(format("(unbalanced", &fields()), "(unbalanced");
        assert!(validate("Track (Live)").is_ok());
    }

    #[test]
    fn format_returns_the_template_unchanged_on_error() {
        assert_eq!(format("%title", &fields()), "%title");
        assert_eq!(format("$nope(x)", &fields()), "$nope(x)");
    }

    #[test]
    fn nesting_depth_is_limited() {
        fn nested(depth: usize) -> String {
            let mut template = "x".to_owned();
            for _ in 0..depth {
                template = format!("$lower({template})");
            }
            template
        }

        let deepest = nested(MAX_NESTING_DEPTH);
        assert!(validate(&deepest).is_ok());
        assert_eq!(format(&deepest, &fields()), "x");

        let too_deep = nested(MAX_NESTING_DEPTH + 1);
        let err = validate(&too_deep).unwrap_err();
        assert!(err.to_string().contains("nesting"));
        assert_eq!(format(&too_deep, &fields()), too_deep);
    }

    #[test]
    fn output_is_truncated_at_the_byte_limit() {
        let long_text = "a".repeat(MAX_OUTPUT_BYTES + 100);
        let output = format(&long_text, &fields());
        assert_eq!(output.len(), MAX_OUTPUT_BYTES);

        let mut fields = FieldMap::new();
        fields.insert("big", "a".repeat(1000));
        let replacement = "b".repeat(1000);
        let template = format!("$replace(%big%,a,{replacement})");
        let output = format(&template, &fields);
        assert_eq!(output.len(), MAX_OUTPUT_BYTES);
        assert!(output.chars().all(|ch| ch == 'b'));
    }

    #[test]
    fn truncation_keeps_utf8_char_boundaries() {
        let template = "中".repeat(MAX_OUTPUT_BYTES / 3 + 10);
        let output = format(&template, &fields());

        assert!(output.len() <= MAX_OUTPUT_BYTES);
        assert_eq!(output.len() % 3, 0);
        assert!(output.chars().all(|ch| ch == '中'));
    }
}
