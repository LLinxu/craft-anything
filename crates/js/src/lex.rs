//! A small, forgiving JavaScript tokenizer.
//!
//! It never fails: unterminated strings, comments and templates just end at the end of input. It
//! serves two jobs: estimating how deeply a source nests before we hand it to the real parser
//! (whose recursion is unbounded), and extracting facts when parsing is refused or fails.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokKind {
    Ident,
    Str,
    /// One chunk of a template literal (between backticks / `${` / `}`).
    Template,
    Number,
    Punct,
    Regex,
    Comment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token<'a> {
    pub kind: TokKind,
    /// Raw source text of the token (quotes included for strings).
    pub text: &'a str,
    pub offset: usize,
    /// A `/…/` after `yield`, where the same text is a regex in a generator and division in
    /// sloppy code; the nesting estimate counts the brackets inside it, since they may be code.
    pub ambiguous_regex: bool,
}

const PUNCTS: &[&str] = &[
    ">>>=", "...", "===", "!==", "**=", "<<=", ">>=", ">>>", "&&=", "||=", "??=", "=>", "==", "!=", "<=", ">=", "&&", "||", "??", "?.", "++", "--", "+=", "-=",
    "*=", "/=", "%=", "&=", "|=", "^=", "**", "<<", ">>",
];

/// Words after which `/` starts a regex. `yield` is handled separately: in a generator it starts
/// a regex, but in sloppy code it is an identifier and `/` divides, so a regex there is read only
/// when one closes on the line and its brackets are counted too.
const REGEX_AFTER_KEYWORDS: &[&str] = &["return", "typeof", "instanceof", "in", "of", "new", "delete", "void", "throw", "case", "do", "else", "await"];

/// Words after which `++`/`--` starts an expression instead of updating a value. Missing one
/// would under-count, so this list errs towards treating a word as expression-starting.
const EXPRESSION_KEYWORDS: &[&str] = &[
    "return",
    "throw",
    "typeof",
    "void",
    "delete",
    "await",
    "new",
    "in",
    "instanceof",
    "of",
    "yield",
    "case",
    "do",
    "else",
    "if",
    "for",
    "while",
    "with",
    "var",
    "let",
    "const",
    "class",
    "extends",
    "function",
    "default",
    "import",
    "export",
];

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$' || b == b'\\' || b >= 0x80
}

fn is_ident_part(b: u8) -> bool {
    is_ident_start(b) || b.is_ascii_digit()
}

/// Byte length of the JavaScript whitespace character at `offset` that isn't plain ASCII
/// whitespace (the vertical tab, no-break space, byte-order mark and the other space separators),
/// or 0.
fn other_space_len(src: &str, offset: usize) -> usize {
    match src.as_bytes().get(offset) {
        Some(0x0b) => 1,
        Some(&c) if c >= 0x80 => src
            .get(offset..)
            .and_then(|rest| rest.chars().next())
            .filter(|c| matches!(c, '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'))
            .map_or(0, char::len_utf8),
        _ => 0,
    }
}

fn line_terminator_len(src: &str, offset: usize) -> usize {
    match src.as_bytes().get(offset) {
        Some(b'\n' | b'\r') => 1,
        Some(0xe2) if src.get(offset..).is_some_and(|rest| rest.starts_with('\u{2028}') || rest.starts_with('\u{2029}')) => 3,
        _ => 0,
    }
}

/// The same for a byte slice: LF, CR, U+2028 or U+2029 at `i`; 0 when there is none.
fn line_terminator_bytes(bytes: &[u8], i: usize) -> usize {
    match bytes.get(i) {
        Some(b'\n' | b'\r') => 1,
        Some(0xe2) if bytes.get(i + 1) == Some(&0x80) && matches!(bytes.get(i + 2), Some(0xa8 | 0xa9)) => 3,
        _ => 0,
    }
}

/// Tokenize `src`, stopping after `max_tokens` (the flag says whether it stopped early).
pub fn tokenize(src: &str, max_tokens: usize) -> (Vec<Token<'_>>, bool) {
    let mut out = Vec::new();
    let truncated = scan(src, max_tokens, |t| out.push(t));
    (out, truncated)
}

/// What a scan found besides the tokens.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Scanned {
    /// `max_tokens` stopped the scan.
    pub truncated: bool,
    /// The shared `yield /…/` lookahead budget ran out before a line ended, so whether those
    /// slashes are regexes or division is unknown; admission fails closed.
    pub lookahead_exhausted: bool,
    /// A `yield /…/` read as a regex hides division token boundaries: a quote or backtick inside
    /// it, or a following `/`, `++` or `--`; admission fails closed.
    pub ambiguous_boundaries: bool,
}

/// Stream tokens to `emit` without storing them. Returns true when `max_tokens` stopped it.
pub fn scan<'a>(src: &'a str, max_tokens: usize, emit: impl FnMut(Token<'a>)) -> bool {
    scan_checked(src, max_tokens, emit).truncated
}

pub(crate) fn scan_checked<'a>(src: &'a str, max_tokens: usize, mut emit: impl FnMut(Token<'a>)) -> Scanned {
    let b = src.as_bytes();
    let mut lookahead_exhausted = false;
    let mut ambiguous_boundaries = false;
    // Set after a `yield /…/` read as a regex: the division reading ends in a `/` token instead,
    // so the next `/`, `++` or `--` would be classified differently.
    let mut after_ambiguous_regex = false;
    let mut count = 0usize;
    let mut i = 0usize;
    // true = this brace opened a template substitution `${`.
    let mut braces: Vec<bool> = Vec::new();
    let slice = |a: usize, z: usize| src.get(a..z).unwrap_or("");
    let mut last_sig: Option<(TokKind, &str)> = None;
    // Whether the last token was a name after `.`/`?.`: `x.of /` divides, it does not start a regex.
    let mut last_member = false;
    // Bytes left for `yield /…/` lookahead across this source; keeps hostile input linear.
    let mut lookahead_left = b.len();
    while i < b.len() {
        let c = b.get(i).copied().unwrap_or(0);
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let line_end = line_terminator_len(src, i);
        if line_end != 0 {
            i += line_end;
            continue;
        }
        let space = other_space_len(src, i);
        if space != 0 {
            i += space;
            continue;
        }
        if count >= max_tokens {
            return Scanned { truncated: true, lookahead_exhausted, ambiguous_boundaries };
        }
        count += 1;
        let start = i;
        let next = b.get(i + 1).copied().unwrap_or(0);
        // Comments.
        if c == b'/' && next == b'/' {
            while i < b.len() && line_terminator_len(src, i) == 0 {
                i += 1;
            }
            emit(Token { kind: TokKind::Comment, text: slice(start, i), offset: start, ambiguous_regex: false });
            continue;
        }
        if c == b'/' && next == b'*' {
            i += 2;
            while i < b.len() && !(b.get(i) == Some(&b'*') && b.get(i + 1) == Some(&b'/')) {
                i += 1;
            }
            i = (i + 2).min(b.len());
            emit(Token { kind: TokKind::Comment, text: slice(start, i), offset: start, ambiguous_regex: false });
            continue;
        }
        if after_ambiguous_regex {
            // Comments above keep the flag: the next real token decides. The flag is replaced by
            // this token's own state at the end of the iteration.
            let diverges = (c == b'/' && next != b'/' && next != b'*') || ((c == b'+' || c == b'-') && next == c);
            ambiguous_boundaries |= diverges;
        }
        let tok_kind;
        let mut ambiguous_regex = false;
        let slash = if c == b'/' { regex_start(last_sig, last_member) } else { RegexStart::No };
        let ambiguous_close = if slash == RegexStart::Ambiguous {
            match regex_closes(b, i + 1, &mut lookahead_left) {
                RegexCloses::Closes => true,
                RegexCloses::No => false,
                RegexCloses::Exhausted => {
                    lookahead_exhausted = true;
                    false
                }
            }
        } else {
            false
        };
        if c == b'"' || c == b'\'' {
            i += 1;
            while let Some(&d) = b.get(i) {
                i += 1;
                if d == b'\\' {
                    i += if b.get(i) == Some(&b'\r') && b.get(i + 1) == Some(&b'\n') { 2 } else { 1 };
                } else if d == c || d == b'\n' {
                    break;
                }
            }
            i = i.min(b.len());
            tok_kind = TokKind::Str;
        } else if c == b'`' || (c == b'}' && braces.last() == Some(&true)) {
            if c == b'}' {
                braces.pop();
            }
            i += 1;
            loop {
                match b.get(i) {
                    None => break,
                    Some(b'\\') => i += 2,
                    Some(b'`') => {
                        i += 1;
                        break;
                    }
                    Some(b'$') if b.get(i + 1) == Some(&b'{') => {
                        i += 2;
                        braces.push(true);
                        break;
                    }
                    Some(_) => i += 1,
                }
            }
            i = i.min(b.len());
            tok_kind = TokKind::Template;
        } else if is_ident_start(c) {
            while b.get(i).is_some_and(|&d| is_ident_part(d) && (d < 0x80 || (line_terminator_len(src, i) == 0 && other_space_len(src, i) == 0))) {
                i += 1;
            }
            tok_kind = TokKind::Ident;
        } else if c.is_ascii_digit() || (c == b'.' && next.is_ascii_digit()) {
            let radix = c == b'0' && matches!(next, b'x' | b'X' | b'b' | b'B' | b'o' | b'O');
            while let Some(&d) = b.get(i) {
                let exp_sign =
                    !radix && matches!(d, b'+' | b'-') && matches!(b.get(i.wrapping_sub(1)), Some(b'e' | b'E')) && b.get(i + 1).is_some_and(u8::is_ascii_digit);
                if d.is_ascii_alphanumeric() || d == b'.' || d == b'_' || exp_sign {
                    i += 1;
                } else {
                    break;
                }
            }
            tok_kind = TokKind::Number;
        } else if slash != RegexStart::No && (slash == RegexStart::Yes || ambiguous_close) {
            ambiguous_regex = slash == RegexStart::Ambiguous;
            i += 1;
            let mut in_class = false;
            while let Some(&d) = b.get(i) {
                let term = line_terminator_bytes(b, i);
                if term != 0 {
                    i += term;
                    break;
                }
                i += 1;
                match d {
                    b'\\' => {
                        if line_terminator_bytes(b, i) != 0 {
                            break;
                        }
                        i += 1;
                    }
                    b'[' => in_class = true,
                    b']' => in_class = false,
                    b'/' if !in_class => {
                        // In sloppy code this closing `/` divides, and a `*` or `/` right after it
                        // opens a comment the regex reading passes.
                        if ambiguous_regex && matches!(b.get(i), Some(b'*' | b'/')) {
                            ambiguous_boundaries = true;
                        }
                        break;
                    }
                    _ => {}
                }
            }
            while b.get(i).is_some_and(|&d| d.is_ascii_alphabetic()) {
                i += 1;
            }
            i = i.min(b.len());
            tok_kind = TokKind::Regex;
        } else {
            let rest = b.get(i..).unwrap_or_default();
            let len = PUNCTS.iter().find(|p| rest.starts_with(p.as_bytes())).map_or(1, |p| p.len());
            // Never split a multi-byte character.
            let mut z = i + len;
            while z < b.len() && !src.is_char_boundary(z) {
                z += 1;
            }
            if c == b'{' {
                braces.push(false);
            } else if c == b'}' {
                braces.pop();
            }
            i = z;
            tok_kind = TokKind::Punct;
        }
        let text = slice(start, i);
        last_member = tok_kind == TokKind::Ident && matches!(last_sig, Some((TokKind::Punct, "." | "?.")));
        last_sig = Some((tok_kind, text));
        emit(Token { kind: tok_kind, text, offset: start, ambiguous_regex });
        if ambiguous_regex {
            // In sloppy code a quote here starts a string whose end the regex reading may pass, and
            // an unbalanced bracket leaves the two readings with different bracket scopes.
            if text.bytes().any(|b| matches!(b, b'\'' | b'"' | b'`')) || !brackets_balance(text) {
                ambiguous_boundaries = true;
            }
        }
        after_ambiguous_regex = ambiguous_regex;
    }
    Scanned { truncated: false, lookahead_exhausted, ambiguous_boundaries }
}

/// Whether a `/` starts a regex, divides, or is the ambiguous `yield /…`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegexStart {
    No,
    Yes,
    Ambiguous,
}

fn regex_start(prev: Option<(TokKind, &str)>, after_member: bool) -> RegexStart {
    if after_member {
        // `x.of /`, `x.in /`: a property name is a value, so `/` divides.
        return RegexStart::No;
    }
    match prev {
        None => RegexStart::Yes,
        Some((TokKind::Punct, ")" | "]" | "}" | "++" | "--")) => RegexStart::No,
        Some((TokKind::Punct, _)) => RegexStart::Yes,
        // `yield /re/` in a generator, or `yield / 2` in sloppy code: read a regex only when one
        // closes on the line, and count its brackets too, as they may be code rather than a class.
        Some((TokKind::Ident, "yield")) => RegexStart::Ambiguous,
        Some((TokKind::Ident, w)) if REGEX_AFTER_KEYWORDS.contains(&w) => RegexStart::Yes,
        Some(_) => RegexStart::No,
    }
}

/// What a `yield /…/` lookahead found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegexCloses {
    /// A closing `/` on the same line.
    Closes,
    /// A line ended first: the slash divides.
    No,
    /// The shared lookahead budget ran out before the line did: unknown, so admission fails closed.
    Exhausted,
}

/// Whether a `/`, starting at `from`, closes a regex on the same line. Escapes and `[…]` classes
/// are respected, as in the scanner above, and every JavaScript line terminator ends the search
/// (even after a backslash). `budget` is the bytes this scan may still look at across the whole
/// source, which keeps repeated lookahead on hostile `yield /[…` input linear.
fn regex_closes(bytes: &[u8], mut i: usize, budget: &mut usize) -> RegexCloses {
    let mut in_class = false;
    while let Some(&d) = bytes.get(i) {
        if line_terminator_bytes(bytes, i) != 0 {
            return RegexCloses::No;
        }
        if *budget == 0 {
            return RegexCloses::Exhausted;
        }
        *budget -= 1;
        i += 1;
        match d {
            b'\\' => {
                if line_terminator_bytes(bytes, i) != 0 {
                    return RegexCloses::No;
                }
                i += 1;
            }
            b'[' => in_class = true,
            b']' => in_class = false,
            b'/' if !in_class => return RegexCloses::Closes,
            _ => {}
        }
    }
    RegexCloses::No
}

/// How much nesting the text of an ambiguous `yield /…/` may hold if it is sloppy-code division:
/// its brackets and every `!`, `+`, `-`, `~`, `*`, `%`, `?`, `&`, `|` or `^`, which chain one node
/// each. Counting a real regex's characters too over-estimates, which only sends the source to the
/// lexer fallback.
fn ambiguous_depth(text: &str) -> usize {
    text.bytes().filter(|b| matches!(b, b'(' | b'[' | b'{' | b'!' | b'+' | b'-' | b'~' | b'*' | b'%' | b'?' | b'&' | b'|' | b'^')).count()
}

/// Whether the brackets in `text` balance. An ambiguous token whose division reading leaves a
/// bracket open, or closes one it never opened, no longer shares the scanner's bracket scopes.
fn brackets_balance(text: &str) -> bool {
    let mut depth = 0usize;
    for b in text.bytes() {
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => match depth.checked_sub(1) {
                Some(d) => depth = d,
                None => return false,
            },
            _ => {}
        }
    }
    depth == 0
}

/// Decode the value of a string token (quotes stripped, common escapes resolved).
pub fn string_value(raw: &str) -> String {
    let inner = raw.get(1..).unwrap_or("");
    let inner = match raw.chars().next() {
        Some(q) if inner.ends_with(q) => inner.get(..inner.len().saturating_sub(1)).unwrap_or(inner),
        _ => inner,
    };
    unescape(inner)
}

/// Resolve JavaScript escape sequences (lenient: unknown escapes keep the character).
pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('b') => out.push('\u{8}'),
            Some('f') => out.push('\u{c}'),
            Some('v') => out.push('\u{b}'),
            Some('0') => out.push('\0'),
            Some('x') => {
                let h: String = it.by_ref().take(2).collect();
                if let Some(ch) = u32::from_str_radix(&h, 16).ok().and_then(char::from_u32) {
                    out.push(ch);
                }
            }
            Some('u') => {
                let hex: String = if it.peek() == Some(&'{') {
                    it.next();
                    it.by_ref().take_while(|c| *c != '}').take(8).collect()
                } else {
                    it.by_ref().take(4).collect()
                };
                let Ok(mut code) = u32::from_str_radix(&hex, 16) else { continue };
                if (0xD800..0xDC00).contains(&code) {
                    let mut ahead = it.clone();
                    if ahead.next() == Some('\\') && ahead.next() == Some('u') {
                        let lo: String = ahead.by_ref().take(4).collect();
                        if let Some(lo) = u32::from_str_radix(&lo, 16).ok().filter(|l| lo.len() == 4 && (0xDC00..0xE000).contains(l)) {
                            code = 0x10000 + ((code - 0xD800) << 10) + (lo - 0xDC00);
                            it = ahead;
                        }
                    }
                }
                out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
            }
            Some('\n') => {}
            Some('\r') => {
                if it.peek() == Some(&'\n') {
                    it.next();
                }
            }
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// An upper-bound style estimate of how deep the syntax tree for `tokens` will nest.
///
/// Brackets add a level; operators that nest expressions (member access, binary and assignment
/// operators, conditionals, arrows, unary prefixes) and nesting statement keywords add a level
/// until the next `,` or `;` at the same bracket depth. So do a call, index or template that
/// follows another (`f()()`, `a[0][1]`, a chain of tagged templates) and a label that follows
/// another (`a: b: c;`): each wraps what came before in one more node, though its brackets close.
/// A bracket that the expression goes on after (`((f)()).a`, `f(g(x)())()`, `(a + b) + c`) keeps
/// what was counted inside it, since what follows wraps all of it; of siblings such as
/// `(a) && (b)` only the deepest counts.
/// Comments and whitespace (non-ASCII spaces included) are skipped. Over-estimates are fine: they
/// only send a file to the lexer fallback.
pub fn nesting_estimate(tokens: &[Token<'_>]) -> usize {
    let mut n = Nesting::default();
    for t in tokens {
        n.feed(t);
    }
    n.max
}

/// What the last tokens were, as far as chains and labels need to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Recent {
    /// `)`, `]` or the end of a template: a `(`, `[` or template right after it continues a chain.
    PostfixEnd,
    /// A label's colon (or any colon): a name and a colon after it make another label.
    Colon,
    /// A name that follows a colon.
    NameAfterColon,
    Other,
}

/// Does `t`, right after a closing bracket, carry on the expression the bracket ended?
fn continues_after_close(t: &Token<'_>) -> bool {
    match t.kind {
        TokKind::Punct => !matches!(t.text, "," | ";" | "{" | ":"),
        TokKind::Template => true,
        TokKind::Ident => matches!(t.text, "in" | "instanceof"),
        _ => false,
    }
}

/// Streaming state for [`nesting_estimate`].
#[derive(Debug)]
pub struct Nesting {
    /// What each open bracket has counted so far, one per level.
    stack: Vec<usize>,
    /// The `else` links counted at each level: a block's links end with its `}`.
    chains: Vec<usize>,
    /// Whether the last token ended a statement at that level (`;` or `}`).
    ended: Vec<bool>,
    /// The expression depth at every level together.
    sum: usize,
    /// The `else` links at every level together. Kept apart from the expression level, which a
    /// comma clears: `if (a) b, c; else …` still nests one level per link.
    chain_total: usize,
    pub max: usize,
    recent: Recent,
    /// What a bracket that just closed had counted, for the expression it may go on in.
    carry: usize,
    /// Whether the last token can end an expression, so `++`/`--` after it is postfix.
    prev_value: bool,
    /// Whether the last token was `++`/`--`; a following one is prefix, never postfix.
    prev_update: bool,
    /// Whether the last token was `extends`, so a `class` after it is a nested heritage.
    prev_extends: bool,
}

impl Default for Nesting {
    fn default() -> Self {
        Nesting {
            stack: vec![0],
            chains: vec![0],
            ended: vec![false],
            sum: 0,
            chain_total: 0,
            max: 0,
            recent: Recent::Other,
            carry: 0,
            prev_value: false,
            prev_update: false,
            prev_extends: false,
        }
    }
}

impl Nesting {
    /// Take one token into the estimate.
    pub fn feed(&mut self, t: &Token<'_>) {
        if t.kind == TokKind::Comment {
            return;
        }
        let Nesting { stack, chains, ended, sum, chain_total, max, recent, carry, prev_value, prev_update, prev_extends } = self;
        // `if (a); else if (a); else …`: each link ends with `;`, but the next link nests inside
        // it. Count the links at their own level, apart from the expression level (which a comma
        // clears), so a `;` inside a block does not reset the chain that block is the consequent
        // of. Any other token after a statement end closes that level's chain.
        let level = stack.len().saturating_sub(1);
        if let Some(flag) = ended.get_mut(level)
            && std::mem::take(flag)
            && let Some(links) = chains.get_mut(level)
        {
            if t.kind == TokKind::Ident && t.text == "else" {
                *links += 1;
                *chain_total += 1;
            } else {
                *chain_total = chain_total.saturating_sub(*links);
                *links = 0;
            }
        }
        if t.ambiguous_regex {
            let inner = ambiguous_depth(t.text);
            // The division reading's depth is part of this expression, so later nesting adds to
            // it; the comma, `;` and closing-bracket handling above clear it like any other depth.
            if let Some(n) = stack.last_mut() {
                *n = n.saturating_add(inner);
            }
            *sum = sum.saturating_add(inner);
        }
        // `((f)())()`, `f(g(x)())()`, `(a + b) + c`: what follows a closed bracket wraps the
        // expression it ended, so that expression's depth goes on counting at this level. Siblings
        // (`(a) && (b)`) don't add up: the deepest one counts.
        let carried = std::mem::take(carry);
        if continues_after_close(t)
            && let Some(n) = stack.last_mut()
            && carried > *n
        {
            *sum += carried - *n;
            *n = carried;
        }
        // `f()(`, `a[0][`, a template after a template: another node around the chain so far.
        let chained = *recent == Recent::PostfixEnd
            && match t.kind {
                TokKind::Punct => matches!(t.text, "(" | "["),
                TokKind::Template => t.text.starts_with('`'),
                _ => false,
            };
        // `a: b:`: a label after a label. `default` is a case clause, not a label.
        let labelled = *recent == Recent::NameAfterColon && t.kind == TokKind::Punct && t.text == ":";
        *recent = match t.kind {
            TokKind::Punct if matches!(t.text, ")" | "]") => Recent::PostfixEnd,
            TokKind::Template if t.text.len() > 1 && t.text.ends_with('`') => Recent::PostfixEnd,
            TokKind::Punct if t.text == ":" => Recent::Colon,
            TokKind::Ident if *recent == Recent::Colon && t.text != "default" => Recent::NameAfterColon,
            _ => Recent::Other,
        };
        if (chained || labelled)
            && let Some(n) = stack.last_mut()
        {
            *n += 1;
            *sum += 1;
        }
        let closes = match t.kind {
            TokKind::Punct => matches!(t.text, ")" | "]" | "}"),
            TokKind::Template => t.text.starts_with('}'),
            _ => false,
        };
        let opens = match t.kind {
            TokKind::Punct => matches!(t.text, "(" | "[" | "{"),
            TokKind::Template => t.text.ends_with("${"),
            _ => false,
        };
        if closes
            && stack.len() > 1
            && let Some(n) = stack.pop()
        {
            *sum = sum.saturating_sub(n + 1);
            *carry = n;
            if let Some(links) = chains.pop() {
                *chain_total = chain_total.saturating_sub(links);
            }
            ended.pop();
            if t.kind == TokKind::Punct
                && t.text == "}"
                && let Some(flag) = ended.last_mut()
            {
                *flag = true;
            }
        }
        if opens {
            stack.push(0);
            chains.push(0);
            ended.push(false);
            *sum += 1;
        }
        if !opens && !closes {
            let nests = match t.kind {
                TokKind::Punct => !matches!(t.text, "," | ";" | ":") && !(matches!(t.text, "++" | "--") && *prev_value && !*prev_update),
                // `class extends class extends …`: each class after `extends` nests a heritage.
                // `extends` itself would accumulate across separate declarations, which don't nest.
                TokKind::Ident if t.text == "class" => *prev_extends,
                TokKind::Ident => matches!(
                    t.text,
                    "typeof" | "void" | "delete" | "await" | "new" | "if" | "else" | "for" | "while" | "do" | "with" | "in" | "instanceof" | "yield"
                ),
                _ => false,
            };
            if let Some(n) = stack.last_mut() {
                if matches!((t.kind, t.text), (TokKind::Punct, "," | ";")) {
                    *sum = sum.saturating_sub(*n);
                    *n = 0;
                    if t.text == ";"
                        && let Some(flag) = ended.last_mut()
                    {
                        *flag = true;
                    }
                } else if nests {
                    *n += 1;
                    *sum += 1;
                }
            }
        }
        *max = (*max).max(sum.saturating_add(*chain_total));
        // What the next token sees: `a++` is postfix, `++a` and `++++a` are prefix.
        *prev_value = match t.kind {
            TokKind::Number | TokKind::Str | TokKind::Regex => true,
            TokKind::Ident => !EXPRESSION_KEYWORDS.contains(&t.text),
            TokKind::Punct => matches!(t.text, ")" | "]" | "++" | "--"),
            TokKind::Template => t.text.len() > 1 && t.text.ends_with('`'),
            TokKind::Comment => false,
        };
        *prev_update = t.kind == TokKind::Punct && matches!(t.text, "++" | "--");
        *prev_extends = t.kind == TokKind::Ident && t.text == "extends";
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(s: &str) -> Vec<(TokKind, String)> {
        tokenize(s, 1000).0.into_iter().map(|t| (t.kind, t.text.to_string())).collect()
    }

    #[test]
    fn operators_before_non_ascii_identifier() {
        for (src, op) in [("a===日本", "==="), ("a>>日本", ">>"), ("a=>é", "=>"), ("a!==😀", "!==")] {
            let t: Vec<&str> = tokenize(src, 100).0.into_iter().map(|t| t.text).collect();
            assert_eq!(t.len(), 3, "{src}: {t:?}");
            assert_eq!(t[1], op, "{src}");
        }
    }

    #[test]
    fn whitespace_does_not_count_against_limit() {
        let (t, truncated) = tokenize("a b c d", 4);
        assert_eq!(t.len(), 4);
        assert!(!truncated);
        let (t, truncated) = tokenize("a b c d", 3);
        assert_eq!(t.len(), 3);
        assert!(truncated);
    }

    #[test]
    fn exponent_sign() {
        let texts = |s: &str| tokenize(s, 100).0.into_iter().map(|t| t.text.to_string()).collect::<Vec<_>>();
        assert_eq!(texts("x = 1e+5 + 2.5E-3;"), ["x", "=", "1e+5", "+", "2.5E-3", ";"]);
        assert_eq!(texts("0x1e+5"), ["0x1e", "+", "5"]);
    }

    #[test]
    fn crlf_line_continuation_stays_in_string() {
        let k = kinds("x = 'a\\\r\nb';");
        assert!(k.contains(&(TokKind::Str, "'a\\\r\nb'".into())), "{k:?}");
        assert!(k.contains(&(TokKind::Punct, ";".into())), "{k:?}");
    }

    #[test]
    fn unescape_crlf_line_continuation() {
        assert_eq!(unescape("a\\\r\nb"), "ab");
        assert_eq!(unescape("a\\\rb"), "ab");
        assert_eq!(unescape("a\\\nb"), "ab");
    }

    #[test]
    fn unescape_backspace_formfeed_vtab() {
        assert_eq!(unescape(r"a\fb\vc\bd"), "a\u{c}b\u{b}c\u{8}d");
        assert_eq!(string_value(r#""\b\f\v""#), "\u{8}\u{c}\u{b}");
    }

    #[test]
    fn unescape_combines_surrogate_pairs() {
        assert_eq!(unescape("\\uD83D\\uDE00"), "😀");
        assert_eq!(unescape("a\\uD83D\\u0041"), "a\u{FFFD}A");
        assert_eq!(unescape("\\uDE00x"), "\u{FFFD}x");
        assert_eq!(unescape("\\u{1F600}"), "😀");
    }

    #[test]
    fn tokenizes_strings_templates_regex() {
        let k = kinds(r#"a = "x\"y" + `t${b}u` / 2; r = /a\/b[/]c/g"#);
        assert!(k.contains(&(TokKind::Str, r#""x\"y""#.into())));
        assert!(k.contains(&(TokKind::Template, "`t${".into())));
        assert!(k.contains(&(TokKind::Template, "}u`".into())));
        assert!(k.contains(&(TokKind::Regex, "/a\\/b[/]c/g".into())));
        assert!(k.contains(&(TokKind::Punct, "/".into())));
        assert_eq!(string_value(r#""aA\x42\n""#), "aAB\n");
    }

    #[test]
    fn nesting_grows_with_depth() {
        let shallow = nesting_estimate(&tokenize("a(b, c); d[e];", 100).0);
        let deep_src = format!("{}1{}", "(".repeat(300), ")".repeat(300));
        let deep = nesting_estimate(&tokenize(&deep_src, 10_000).0);
        let chain = nesting_estimate(&tokenize(&"a+".repeat(500), 10_000).0);
        let nots = nesting_estimate(&tokenize(&format!("{}x", "!".repeat(400)), 10_000).0);
        assert!(shallow < 5, "{shallow}");
        assert!(deep >= 300 && chain >= 500 && nots >= 400, "{deep} {chain} {nots}");
    }

    #[test]
    fn nesting_counts_else_if_extends_prefix_update_and_hidden_brackets() {
        let deep = |src: String| nesting_estimate(&tokenize(&src, 100_000).0);
        let n = 5_000;
        let else_if = format!("if(a);{}", "else if(a);".repeat(n));
        assert!(deep(else_if) > n, "else-if chain");
        let extends = format!("x = class{};", " extends class".repeat(n) + " {}");
        assert!(deep(extends) > n, "class extends chain");
        let separate = "class A extends B {}".repeat(n);
        assert!(deep(separate) < 16, "separate extended classes do not add up");
        let prefix = format!("x = {}a;", "++".repeat(n));
        assert!(deep(prefix) > n, "prefix ++ run");
        let hidden = format!("x = a++ /{} / 1;", "[".repeat(n) + &"]".repeat(n));
        assert!(deep(hidden) > n, "brackets after an update operator");
        let commas = "if(a)b,c;else ".repeat(n) + "if(a)b,c;";
        assert!(deep(commas) > n, "else-if chain with comma expressions");
        let alternating = "if(a){b;}else if(a)c;else ".repeat(n) + "if(a){d;}";
        assert!(deep(alternating) > n, "block and unbraced consequents");
        let yield_regex = format!("x = yield /{} /;", "[".repeat(n) + &"]".repeat(n));
        assert!(deep(yield_regex) > n, "brackets in a regex after yield");
        let yield_nots = format!("x = yield /{}a / 1;", "!".repeat(n));
        assert!(deep(yield_nots) > n, "prefix operators in an ambiguous regex after yield");
        let adds = format!("x = yield /{}a / 1;", "a+".repeat(n));
        assert!(deep(adds) > n, "addition chain in an ambiguous regex after yield");
        let separated = format!("x = yield /{}a / 1;", "!a".repeat(n));
        assert!(deep(separated) > n, "separated prefix operators in an ambiguous regex after yield");
        let cond = format!("x = yield /{}a / 1;", "a?b:".repeat(n));
        assert!(deep(cond) > n, "conditional chain in an ambiguous regex after yield");
        for op in ["&", "|", "^"] {
            let bitwise = format!("x = yield /{}a / 1;", format!("a{op}").repeat(n));
            assert!(deep(bitwise) > n, "bitwise {op} chain in an ambiguous regex after yield");
        }
    }

    #[test]
    fn ambiguous_regex_lookahead_stops_at_its_budget() {
        let bytes = b"/[[[[[[";
        let mut budget = 4;
        assert_eq!(regex_closes(bytes, 1, &mut budget), RegexCloses::Exhausted);
        assert_eq!(budget, 0);
    }

    #[test]
    fn ambiguous_regex_stops_at_every_line_terminator() {
        for term in ["\n", "\r", "\u{2028}", "\u{2029}"] {
            let src = format!("yield / 2;{term} x = {}a; /re/;", "!".repeat(6_000));
            let toks = tokenize(&src, 100_000).0;
            let nots = toks.iter().filter(|t| t.kind == TokKind::Punct && t.text == "!").count();
            assert_eq!(nots, 6_000, "line terminator {term:?}");
            assert!(nesting_estimate(&toks) >= 6_000, "line terminator {term:?}");
        }
        // A backslash before the line end does not let the lookahead continue either.
        let escaped = format!("yield / 2;\\\n x = {}a; /re/;", "!".repeat(6_000));
        let toks = tokenize(&escaped, 100_000).0;
        assert_eq!(toks.iter().filter(|t| t.kind == TokKind::Punct && t.text == "!").count(), 6_000);
        assert!(nesting_estimate(&toks) >= 6_000);
    }

    #[test]
    fn division_follows_updates_and_members_but_yield_can_start_a_regex() {
        for src in ["a++ / b[0] / c", "x.of / b[0] / c", "x?.of / b[0] / c"] {
            let kinds: Vec<TokKind> = tokenize(src, 100).0.into_iter().map(|t| t.kind).collect();
            assert!(!kinds.contains(&TokKind::Regex), "{src}: {kinds:?}");
        }
        // `yield /a[0]/g` is a regex in a generator: recognized as one, flagged ambiguous, and
        // the code after its closing delimiter is still tokenized.
        let toks = tokenize("yield /a[0]/g; f(x);", 100).0;
        let regex = toks.iter().find(|t| t.kind == TokKind::Regex).expect("a regex after yield");
        assert_eq!(regex.text, "/a[0]/g");
        assert!(regex.ambiguous_regex);
        let texts: Vec<&str> = toks.iter().map(|t| t.text).collect();
        assert!(texts.contains(&";") && texts.contains(&"f") && texts.contains(&"("), "{texts:?}");
        // `yield* /re/` too, and `return /re/` as before.
        assert!(tokenize("yield* /re/g", 100).0.iter().any(|t| t.kind == TokKind::Regex));
        assert!(tokenize("return /a[0]/g", 100).0.iter().any(|t| t.kind == TokKind::Regex));
        // A compact generator regex stays a shallow regex, not a refused source.
        let boundary = tokenize("yield /a+b/;", 100).0;
        assert!(boundary.iter().any(|t| t.kind == TokKind::Regex));
        assert!(nesting_estimate(&boundary) < 16);
        // `yield / 2` with no closing `/` on the line is sloppy-code division: no regex is made,
        // and the brackets after it stay visible to the nesting estimate.
        let division = tokenize("yield / 2; g(h(k(i)));", 100).0;
        assert!(!division.iter().any(|t| t.kind == TokKind::Regex), "{division:?}");
        assert!(nesting_estimate(&division) >= 3);
    }

    #[test]
    fn never_panics_on_junk() {
        for s in ["\"", "`${", "/*", "'\\", "}}}}", "`${`${`${", "/[", "é/é/", "\u{1F600}=`\u{1F600}${é}`", "a/**/b//"] {
            let (t, _) = tokenize(s, 100);
            let _ = nesting_estimate(&t);
            for tok in t {
                let _ = string_value(tok.text);
            }
        }
    }
}
