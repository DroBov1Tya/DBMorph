use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};

use crate::config;

#[derive(Debug, Clone)]
enum Token {
    Word(String),
    Str(String),
    Ident(String),
    LParen,
    RParen,
    Comma,
    Semi,
}

struct Tokenizer {
    reader: Option<BufReader<File>>,
    pushed: Option<u8>,
    peeked: Option<Token>,
}

impl Tokenizer {
    fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let file = File::open(path).with_context(|| format!("failed to open SQL dump {path:?}"))?;
        Ok(Self {
            reader: Some(BufReader::with_capacity(config::READ_BUFFER_BYTES, file)),
            pushed: None,
            peeked: None,
        })
    }

    /// A tokenizer with no backing file: yields EOF immediately.
    fn empty() -> Self {
        Self {
            reader: None,
            pushed: None,
            peeked: None,
        }
    }

    fn getb(&mut self) -> Result<Option<u8>> {
        if let Some(b) = self.pushed.take() {
            return Ok(Some(b));
        }
        let reader = match self.reader.as_mut() {
            Some(r) => r,
            None => return Ok(None),
        };
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            return Ok(None);
        }
        let b = chunk[0];
        reader.consume(1);
        Ok(Some(b))
    }

    fn ungetb(&mut self, b: u8) {
        self.pushed = Some(b);
    }

    fn next_token(&mut self) -> Result<Option<Token>> {
        if let Some(t) = self.peeked.take() {
            return Ok(Some(t));
        }
        self.lex_token()
    }

    fn peek_token(&mut self) -> Result<Option<&Token>> {
        if self.peeked.is_none() {
            self.peeked = self.lex_token()?;
        }
        Ok(self.peeked.as_ref())
    }

    fn lex_token(&mut self) -> Result<Option<Token>> {
        loop {
            let b = match self.getb()? {
                Some(b) => b,
                None => return Ok(None),
            };

            if b.is_ascii_whitespace() {
                continue;
            }

            match b {
                b'\'' => return Ok(Some(Token::Str(self.read_quoted(b'\'')?))),
                b'"' => return Ok(Some(Token::Str(self.read_quoted(b'"')?))),
                b'`' => return Ok(Some(Token::Ident(self.read_backtick()?))),
                b'[' => return Ok(Some(Token::Ident(self.read_until(b']')?))),
                b'(' => return Ok(Some(Token::LParen)),
                b')' => return Ok(Some(Token::RParen)),
                b',' => return Ok(Some(Token::Comma)),
                b';' => return Ok(Some(Token::Semi)),
                b'#' => {
                    self.skip_line()?;
                    continue;
                }
                b'-' => {
                    if let Some(n) = self.getb()? {
                        if n == b'-' {
                            self.skip_line()?;
                            continue;
                        }
                        self.ungetb(n);
                    }
                    return Ok(Some(self.read_word(b)?));
                }
                b'/' => {
                    if let Some(n) = self.getb()? {
                        if n == b'*' {
                            self.skip_block_comment()?;
                            continue;
                        }
                        self.ungetb(n);
                    }
                    continue;
                }
                _ => return Ok(Some(self.read_word(b)?)),
            }
        }
    }

    fn read_quoted(&mut self, quote: u8) -> Result<String> {
        let mut out: Vec<u8> = Vec::new();
        loop {
            let b = self
                .getb()?
                .ok_or_else(|| anyhow!("unterminated string literal"))?;
            if b == b'\\' {
                let n = self
                    .getb()?
                    .ok_or_else(|| anyhow!("unterminated escape in string literal"))?;
                match n {
                    b'n' => out.push(b'\n'),
                    b't' => out.push(b'\t'),
                    b'r' => out.push(b'\r'),
                    b'0' => out.push(0),
                    b'b' => out.push(8),
                    b'Z' => out.push(26),
                    b'\\' => out.push(b'\\'),
                    b'\'' => out.push(b'\''),
                    b'"' => out.push(b'"'),
                    other => out.push(other),
                }
                continue;
            }
            if b == quote {
                match self.getb()? {
                    Some(n) if n == quote => {
                        out.push(quote);
                        continue;
                    }
                    Some(n) => {
                        self.ungetb(n);
                        break;
                    }
                    None => break,
                }
            }
            out.push(b);
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    fn read_backtick(&mut self) -> Result<String> {
        let mut out: Vec<u8> = Vec::new();
        loop {
            let b = self
                .getb()?
                .ok_or_else(|| anyhow!("unterminated identifier"))?;
            if b == b'`' {
                match self.getb()? {
                    Some(b'`') => {
                        out.push(b'`');
                        continue;
                    }
                    Some(n) => {
                        self.ungetb(n);
                        break;
                    }
                    None => break,
                }
            }
            out.push(b);
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    fn read_until(&mut self, end: u8) -> Result<String> {
        let mut out: Vec<u8> = Vec::new();
        loop {
            let b = self
                .getb()?
                .ok_or_else(|| anyhow!("unterminated identifier"))?;
            if b == end {
                break;
            }
            out.push(b);
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    fn read_word(&mut self, first: u8) -> Result<Token> {
        let mut out: Vec<u8> = vec![first];
        loop {
            let b = match self.getb()? {
                Some(b) => b,
                None => break,
            };
            if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'$' | b'+' | b'-') {
                out.push(b);
            } else {
                self.ungetb(b);
                break;
            }
        }
        Ok(Token::Word(String::from_utf8_lossy(&out).into_owned()))
    }

    fn skip_line(&mut self) -> Result<()> {
        while let Some(b) = self.getb()? {
            if b == b'\n' {
                break;
            }
        }
        Ok(())
    }

    fn skip_block_comment(&mut self) -> Result<()> {
        let mut prev = 0u8;
        while let Some(b) = self.getb()? {
            if prev == b'*' && b == b'/' {
                break;
            }
            prev = b;
        }
        Ok(())
    }

    fn table_from_token(&mut self, first: Token) -> Result<String> {
        let mut name = ident_of(&first);
        loop {
            let dotted = matches!(self.peek_token()?, Some(Token::Word(w)) if w.starts_with('.'));
            if !dotted {
                break;
            }
            if let Some(Token::Word(w)) = self.next_token()? {
                if w == "." {
                    if let Some(nt) = self.next_token()? {
                        name = ident_of(&nt);
                    }
                } else {
                    name = w.trim_start_matches('.').to_string();
                }
            }
        }
        Ok(name)
    }

    fn skip_to_semi(&mut self) -> Result<()> {
        while let Some(t) = self.next_token()? {
            if matches!(t, Token::Semi) {
                break;
            }
        }
        Ok(())
    }
}

fn ident_of(token: &Token) -> String {
    let raw = match token {
        Token::Ident(s) | Token::Str(s) | Token::Word(s) => s.as_str(),
        _ => "",
    };
    let trimmed = raw
        .trim_matches('`')
        .trim_matches('"')
        .trim_matches(']')
        .trim_matches('[');
    trimmed.rsplit('.').next().unwrap_or(trimmed).to_string()
}

fn is_constraint_kw(word: &str) -> bool {
    matches!(
        word.to_ascii_uppercase().as_str(),
        "PRIMARY"
            | "UNIQUE"
            | "KEY"
            | "INDEX"
            | "CONSTRAINT"
            | "FOREIGN"
            | "CHECK"
            | "FULLTEXT"
            | "SPATIAL"
    )
}

fn read_create_table_name(tok: &mut Tokenizer) -> Result<Option<String>> {
    match tok.next_token()? {
        Some(Token::Word(w)) if w.eq_ignore_ascii_case("TABLE") => {}
        _ => return Ok(None),
    }

    let mut first = match tok.next_token()? {
        Some(t) => t,
        None => return Ok(None),
    };
    if let Token::Word(w) = &first
        && w.eq_ignore_ascii_case("IF")
    {
        let _ = tok.next_token()?;
        let _ = tok.next_token()?;
        first = match tok.next_token()? {
            Some(t) => t,
            None => return Ok(None),
        };
    }
    Ok(Some(tok.table_from_token(first)?))
}

fn parse_create_body(tok: &mut Tokenizer) -> Result<Option<Vec<String>>> {
    loop {
        match tok.next_token()? {
            Some(Token::LParen) => break,
            Some(Token::Semi) | None => return Ok(None),
            _ => continue,
        }
    }

    let mut cols: Vec<String> = Vec::new();
    let mut depth = 1usize;
    let mut expect_name = true;

    while let Some(t) = tok.next_token()? {
        match t {
            Token::LParen => depth += 1,
            Token::RParen => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Token::Comma if depth == 1 => expect_name = true,
            Token::Ident(s) | Token::Word(s) | Token::Str(s) if depth == 1 && expect_name => {
                expect_name = false;
                if !is_constraint_kw(&s) {
                    cols.push(ident_of(&Token::Word(s)));
                }
            }
            _ => {}
        }
    }

    Ok(Some(cols))
}

fn read_until_values(tok: &mut Tokenizer) -> Result<Option<Vec<String>>> {
    let mut collist: Option<Vec<String>> = None;

    while let Some(t) = tok.next_token()? {
        match t {
            Token::LParen => {
                let mut cols: Vec<String> = Vec::new();
                while let Some(inner) = tok.next_token()? {
                    match inner {
                        Token::RParen => break,
                        Token::Comma => continue,
                        Token::Ident(s) | Token::Word(s) | Token::Str(s) => {
                            cols.push(ident_of(&Token::Word(s)));
                        }
                        _ => {}
                    }
                }
                if !cols.is_empty() {
                    collist = Some(cols);
                }
            }
            Token::Word(w) if w.eq_ignore_ascii_case("VALUES") => break,
            Token::Semi => break,
            _ => {}
        }
    }

    Ok(collist)
}

/// Opens a SQL dump in a single pass: resolves the source table and its columns,
/// then returns an iterator positioned to stream that table's rows.
pub fn sql_open<P: AsRef<Path>>(
    path: P,
    wanted: Option<&str>,
) -> Result<(String, Vec<String>, SqlRowIter)> {
    let mut tok = Tokenizer::new(&path)?;
    let mut create_cols: HashMap<String, Vec<String>> = HashMap::new();

    while let Some(t) = tok.next_token()? {
        let word = match &t {
            Token::Word(w) => w.clone(),
            _ => continue,
        };

        if word.eq_ignore_ascii_case("CREATE") {
            let table = match read_create_table_name(&mut tok)? {
                Some(name) => name,
                None => continue,
            };
            if let Some(cols) = parse_create_body(&mut tok)?
                && !cols.is_empty()
            {
                create_cols.insert(table.to_ascii_lowercase(), cols);
            }
            continue;
        }

        if word.eq_ignore_ascii_case("INSERT") {
            match tok.next_token()? {
                Some(Token::Word(w)) if w.eq_ignore_ascii_case("INTO") => {}
                _ => continue,
            }
            let first = match tok.next_token()? {
                Some(t) => t,
                None => continue,
            };
            let table = tok.table_from_token(first)?;

            let is_target = match wanted {
                Some(w) => table.eq_ignore_ascii_case(w),
                None => true,
            };
            if !is_target {
                tok.skip_to_semi()?;
                continue;
            }

            let collist = read_until_values(&mut tok)?;

            let known = create_cols
                .get(&table.to_ascii_lowercase())
                .cloned()
                .or(collist);

            let (columns, pending) = match known {
                Some(cols) => (cols, None),
                None => {
                    let first_row = read_first_tuple(&mut tok)?;
                    let cols = (0..first_row.len()).map(|i| format!("c{i}")).collect();
                    (cols, Some(first_row))
                }
            };

            let iter = SqlRowIter {
                tok,
                target: table.clone(),
                in_values: true,
                pending,
            };
            return Ok((table, columns, iter));
        }
    }

    if let Some(w) = wanted
        && let Some(cols) = create_cols.get(&w.to_ascii_lowercase())
    {
        return Ok((
            w.to_string(),
            cols.clone(),
            SqlRowIter::empty(w.to_string()),
        ));
    }
    if let Some((table, cols)) = create_cols.into_iter().next() {
        return Ok((table.clone(), cols, SqlRowIter::empty(table)));
    }

    bail!("no INSERT or CREATE TABLE statement found in SQL dump");
}

/// Reads one VALUES tuple at its true width, opening `(` not yet consumed.
fn read_first_tuple(tok: &mut Tokenizer) -> Result<Vec<String>> {
    loop {
        match tok.next_token()? {
            Some(Token::LParen) => break,
            Some(Token::Semi) | None => bail!("no VALUES tuple found to infer columns"),
            _ => continue,
        }
    }
    read_tuple_fields(tok)
}

/// Streaming reader over `INSERT INTO <target> VALUES (...)` tuples; `pending`
/// holds a first tuple consumed early to infer the schema.
pub struct SqlRowIter {
    tok: Tokenizer,
    target: String,
    in_values: bool,
    pending: Option<Vec<String>>,
}

impl SqlRowIter {
    fn empty(target: String) -> Self {
        SqlRowIter {
            tok: Tokenizer::empty(),
            target,
            in_values: false,
            pending: None,
        }
    }

    fn enter_values_if_target(&mut self) -> Result<()> {
        match self.tok.next_token()? {
            Some(Token::Word(w)) if w.eq_ignore_ascii_case("INTO") => {}
            _ => return Ok(()),
        }
        let first = match self.tok.next_token()? {
            Some(t) => t,
            None => return Ok(()),
        };
        let table = self.tok.table_from_token(first)?;

        loop {
            match self.tok.next_token()? {
                Some(Token::Word(w)) if w.eq_ignore_ascii_case("VALUES") => {
                    if table.eq_ignore_ascii_case(&self.target) {
                        self.in_values = true;
                    } else {
                        self.tok.skip_to_semi()?;
                    }
                    return Ok(());
                }
                Some(Token::Semi) | None => return Ok(()),
                _ => continue,
            }
        }
    }
}

/// Reads one VALUES tuple at its true field width, opening `(` already consumed.
fn read_tuple_fields(tok: &mut Tokenizer) -> Result<Vec<String>> {
    let mut fields: Vec<String> = Vec::new();
    let mut cur: Vec<Token> = Vec::new();
    let mut depth = 1usize;

    loop {
        let t = tok
            .next_token()?
            .ok_or_else(|| anyhow!("unexpected EOF inside VALUES tuple"))?;
        match t {
            Token::LParen => {
                depth += 1;
                cur.push(Token::LParen);
            }
            Token::RParen => {
                if depth == 1 {
                    fields.push(finalize_field(&cur));
                    break;
                }
                depth -= 1;
                cur.push(Token::RParen);
            }
            Token::Comma if depth == 1 => {
                fields.push(finalize_field(&cur));
                cur.clear();
            }
            other => cur.push(other),
        }
    }

    Ok(fields)
}

fn finalize_field(tokens: &[Token]) -> String {
    if tokens.is_empty() {
        return String::new();
    }
    if let Some(s) = tokens.iter().rev().find_map(|t| match t {
        Token::Str(s) => Some(s.clone()),
        _ => None,
    }) {
        return s;
    }
    if tokens.len() == 1 {
        return match &tokens[0] {
            Token::Word(w) if w.eq_ignore_ascii_case("NULL") => String::new(),
            Token::Word(w) | Token::Ident(w) => w.clone(),
            _ => String::new(),
        };
    }
    tokens
        .iter()
        .map(|t| match t {
            Token::Word(w) | Token::Ident(w) => w.clone(),
            Token::LParen => "(".to_string(),
            Token::RParen => ")".to_string(),
            Token::Comma => ",".to_string(),
            Token::Semi => ";".to_string(),
            Token::Str(s) => s.clone(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

impl Iterator for SqlRowIter {
    type Item = Result<Vec<String>>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(row) = self.pending.take() {
            return Some(Ok(row));
        }
        loop {
            if self.in_values {
                match self.tok.next_token() {
                    Err(e) => return Some(Err(e)),
                    Ok(None) => return None,
                    Ok(Some(t)) => match t {
                        Token::LParen => return Some(read_tuple_fields(&mut self.tok)),
                        Token::Comma => continue,
                        Token::Semi => {
                            self.in_values = false;
                            continue;
                        }
                        _ => continue,
                    },
                }
            } else {
                match self.tok.next_token() {
                    Err(e) => return Some(Err(e)),
                    Ok(None) => return None,
                    Ok(Some(Token::Word(w))) if w.eq_ignore_ascii_case("INSERT") => {
                        if let Err(e) = self.enter_values_if_target() {
                            return Some(Err(e));
                        }
                    }
                    Ok(Some(_)) => continue,
                }
            }
        }
    }
}
