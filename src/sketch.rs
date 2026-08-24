//! The sketch language: the constrained surface the transformer may emit.
//! One grammar, two consumers: `GRAMMAR` (GBNF, constrains server-side
//! sampling) and `Parser` (Rust mirror, stage S1). They MUST change together.

/// Sketch value types. v1: Int (i64), F64, Bool, List<Int>, List<F64>.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ty {
    Int,
    F64,
    Bool,
    ListInt,
    ListF64,
}

impl Ty {
    /// Human-readable name used in signatures and errors.
    pub fn name(&self) -> &'static str {
        match self {
            Ty::Int => "Int",
            Ty::F64 => "F64",
            Ty::Bool => "Bool",
            Ty::ListInt => "List<Int>",
            Ty::ListF64 => "List<F64>",
        }
    }
}

/// Binary operators, ordered by precedence tier in the parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

/// Unary builtin operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Len,
    Index,
    Range,
    Sum,
    Max,
    Min,
    Sqrt,
    Exp,
    Log,
    Abs,
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

/// Candidate expression AST. Body of a candidate is a single expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    IntLit(i64),
    FloatLit(f64),
    BoolLit(bool),
    Var(String),
    BinOp(BinOp, Box<Expr>, Box<Expr>),
    UnOp(UnOp, Box<Expr>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    Let(String, Box<Expr>, Box<Expr>),
    ListLit(Vec<i64>),
    FloatListLit(Vec<f64>),
    /// Unary builtins: Len/Sum/Max/Min over lists; Sqrt/Exp/Log/Abs numeric;
    /// Range(n) builds 0..n.
    Builtin(Builtin, Box<Expr>),
    /// Binary builtins: Index(list, pos).
    Builtin2(Builtin, Box<Expr>, Box<Expr>),
    /// Expression-list constructor: [e1, e2, ...]. Elements may be any expr;
    /// typechecker enforces uniform element type. Distinct from ListLit/
    /// FloatListLit (pure literals) for backward compat.
    ListCons(Vec<Expr>),
    /// Vault dependency call: Path.name(arg, ...). Validated against the
    /// gen's declared `use` deps; executed from the vault at sieve time.
    Call(String, Vec<Expr>),
    Fold {
        var: String,
        acc: String,
        list: Box<Expr>,
        init: Box<Expr>,
        body: Box<Expr>,
    },
}

/// A parsed candidate: `fn @name(%p: T, ...) -> T { expr }`.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub name: String,
    pub params: Vec<(String, Ty)>,
    pub ret: Ty,
    pub body: Expr,
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Int(i64),
    Float(f64),
    PIdent(String),
    /// `@name` lexed as one token — bare words are keyword-only otherwise.
    AtName(String),
    /// Dotted vault-call path `Stats.mean` (only lexed when '(' follows).
    CallPath(String),
    Word(&'static str),
    Sym(&'static str),
}

struct Lexed {
    tok: Tok,
    offset: usize,
}

/// Parse error with byte offset into the candidate text (S1 reason payload).
#[derive(Debug, Clone)]
pub struct ParseError {
    pub offset: usize,
    pub message: String,
}

fn err(offset: usize, message: impl Into<String>) -> ParseError {
    ParseError {
        offset,
        message: message.into(),
    }
}

// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------

/// Tokenize candidate text. `%name` lexes as one identifier; keywords and
/// multi-char operators are recognized before single-char symbols.
fn lex(src: &str) -> Result<Vec<Lexed>, ParseError> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // `%name` is an identifier; a lone `%` is the modulo operator.
        if c == b'%' && i + 1 < b.len() && (b[i + 1].is_ascii_alphabetic() || b[i + 1] == b'_') {
            let start = i;
            i += 2;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            out.push(Lexed {
                tok: Tok::PIdent(src[start + 1..i].to_string()),
                offset: start,
            });
            continue;
        }
        // `@name` is the function-name token; consumed atomically.
        if c == b'@' && i + 1 < b.len() && (b[i + 1].is_ascii_alphabetic() || b[i + 1] == b'_') {
            let start = i;
            i += 2;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            out.push(Lexed {
                tok: Tok::AtName(src[start + 1..i].to_string()),
                offset: start,
            });
            continue;
        }
        if c.is_ascii_digit()
            || (c == b'.' && i + 1 < b.len() && b[i + 1].is_ascii_digit())
        {
            let start = i;
            let mut is_float = false;
            while i < b.len()
                && (b[i].is_ascii_digit()
                    || b[i] == b'.'
                    || b[i] == b'e'
                    || b[i] == b'E'
                    || ((b[i] == b'+' || b[i] == b'-') && matches!(exponent_sign(&src[start..i]), true)))
            {
                if b[i] == b'.' || b[i] == b'e' || b[i] == b'E' {
                    is_float = true;
                }
                i += 1;
            }
            let text = &src[start..i];
            if !is_float {
                let v: i64 = text
                    .parse()
                    .map_err(|_| err(start, "integer literal overflow"))?;
                out.push(Lexed {
                    tok: Tok::Int(v),
                    offset: start,
                });
            } else {
                let v: f64 = text
                    .parse()
                    .map_err(|_| err(start, format!("bad float literal `{}`", text)))?;
                out.push(Lexed {
                    tok: Tok::Float(v),
                    offset: start,
                });
            }
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            let mut j = i;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                j += 1;
            }
            // Builtin/keyword words never become vault calls, even before '('.
            let is_keyword = matches!(
                &src[start..j],
                "len" | "sum" | "max" | "min" | "sqrt" | "exp" | "log"
                    | "abs" | "fold" | "let" | "if" | "else" | "true"
                    | "false" | "in" | "from" | "Int" | "F64" | "Bool"
                    | "List" | "fn" | "index" | "range"
            );
            if !is_keyword {
                // Speculative dotted-path scan: ident ('.' ident)* then ws+'('
                // makes it a vault CALL path.
                i = j;
                let mut is_call = false;
                let mut probe = j;
                loop {
                    while probe < b.len() && b[probe].is_ascii_whitespace() {
                        probe += 1;
                    }
                    if probe < b.len() && b[probe] == b'.' {
                        let mut q = probe + 1;
                        if q < b.len()
                            && (b[q].is_ascii_alphabetic() || b[q] == b'_')
                        {
                            while q < b.len()
                                && (b[q].is_ascii_alphanumeric() || b[q] == b'_')
                            {
                                q += 1;
                            }
                            probe = q;
                            i = q;
                            continue;
                        }
                        break; // '.' not followed by ident: not a call path
                    }
                    if probe < b.len() && b[probe] == b'(' {
                        is_call = true;
                    }
                    break;
                }
                if is_call {
                    out.push(Lexed {
                        tok: Tok::CallPath(src[start..i].to_string()),
                        offset: start,
                    });
                    continue;
                }
            }
            i = j;
            let word = &src[start..i];
            let kw: Option<&'static str> = match word {
                "fn" => Some("fn"),
                "let" => Some("let"),
                "if" => Some("if"),
                "else" => Some("else"),
                "true" => Some("true"),
                "false" => Some("false"),
                "len" => Some("len"),
                "index" => Some("index"),
                "range" => Some("range"),
                "sum" => Some("sum"),
                "max" => Some("max"),
                "min" => Some("min"),
                "sqrt" => Some("sqrt"),
                "exp" => Some("exp"),
                "log" => Some("log"),
                "abs" => Some("abs"),
                "fold" => Some("fold"),
                "in" => Some("in"),
                "from" => Some("from"),
                "Int" => Some("Int"),
                "F64" => Some("F64"),
                "Bool" => Some("Bool"),
                "List" => Some("List"),
                _ => None,
            };
            match kw {
                // Keywords are matched case-sensitively; anything else is a hard error
                // so typos like "Fold" die at S1 instead of confusing later stages.
                None => return Err(err(start, format!("unknown word `{}`", word))),
                Some(w) => out.push(Lexed {
                    tok: Tok::Word(w),
                    offset: start,
                }),
            }
            continue;
        }
        let two = if i + 1 < b.len() { &src[i..i + 2] } else { "" };
        // Static literals keep Tok::Sym borrow-free of `src`.
        let sym2: Option<&'static str> = match two {
            "==" => Some("=="),
            "!=" => Some("!="),
            "<=" => Some("<="),
            ">=" => Some(">="),
            "&&" => Some("&&"),
            "||" => Some("||"),
            "->" => Some("->"),
            _ => None,
        };
        if let Some(s) = sym2 {
            out.push(Lexed {
                tok: Tok::Sym(s),
                offset: i,
            });
            i += 2;
            continue;
        }
        let sym1 = match c {
            b'(' => "(",
            b')' => ")",
            b'{' => "{",
            b'}' => "}",
            b'[' => "[",
            b']' => "]",
            b',' => ",",
            b';' => ";",
            b':' => ":",
            b'@' => "@",
            b'<' => "<",
            b'>' => ">",
            b'=' => "=",
            b'+' => "+",
            b'-' => "-",
            b'*' => "*",
            b'/' => "/",
            b'%' => "%",
            b'!' => "!",
            _ => return Err(err(i, format!("unexpected character `{}`", c as char))),
        };
        out.push(Lexed {
            tok: Tok::Sym(sym1),
            offset: i,
        });
        i += 1;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Parser (Pratt tiers)
// ---------------------------------------------------------------------------

struct Parser {
    toks: Vec<Lexed>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|l| &l.tok)
    }

    fn offset(&self) -> usize {
        self.toks
            .get(self.pos)
            .or_else(|| self.toks.last())
            .map(|l| l.offset)
            .unwrap_or(0)
    }

    fn eat_sym(&mut self, s: &str) -> Result<(), ParseError> {
        match self.peek() {
            Some(Tok::Sym(x)) if *x == s => {
                self.pos += 1;
                Ok(())
            }
            _ => Err(err(self.offset(), format!("expected `{}`", s))),
        }
    }

    fn eat_word(&mut self, w: &str) -> Result<(), ParseError> {
        match self.peek() {
            Some(Tok::Word(x)) if *x == w => {
                self.pos += 1;
                Ok(())
            }
            _ => Err(err(self.offset(), format!("expected `{}`", w))),
        }
    }

    fn eat_pident(&mut self) -> Result<String, ParseError> {
        match self.peek().cloned() {
            Some(Tok::PIdent(n)) => {
                self.pos += 1;
                Ok(n)
            }
            _ => Err(err(self.offset(), "expected %identifier".to_string())),
        }
    }

    fn parse_type(&mut self) -> Result<Ty, ParseError> {
        match self.peek() {
            Some(Tok::Word("Int")) => {
                self.pos += 1;
                Ok(Ty::Int)
            }
            Some(Tok::Word("F64")) => {
                self.pos += 1;
                Ok(Ty::F64)
            }
            Some(Tok::Word("Bool")) => {
                self.pos += 1;
                Ok(Ty::Bool)
            }
            Some(Tok::Word("List")) => {
                self.pos += 1;
                self.eat_sym("<")?;
                match self.peek() {
                    Some(Tok::Word("Int")) => {
                        self.pos += 1;
                        self.eat_sym(">")?;
                        Ok(Ty::ListInt)
                    }
                    Some(Tok::Word("F64")) => {
                        self.pos += 1;
                        self.eat_sym(">")?;
                        Ok(Ty::ListF64)
                    }
                    _ => Err(err(self.offset(), "List element must be Int or F64")),
                }
            }
            _ => Err(err(self.offset(), "expected type (`Int`, `Bool`, `List<Int>`)")),
        }
    }

    /// Entry: full candidate `fn @name(params) -> T { body }`.
    fn parse_candidate(&mut self) -> Result<Candidate, ParseError> {
        self.eat_word("fn")?;
        let name = match self.peek().cloned() {
            Some(Tok::AtName(n)) => {
                self.pos += 1;
                n
            }
            _ => return Err(err(self.offset(), "expected @function_name")),
        };
        self.eat_sym("(")?;
        let mut params = Vec::new();
        if !matches!(self.peek(), Some(Tok::Sym(")"))) {
            loop {
                let p = self.eat_pident()?;
                self.eat_sym(":")?;
                let t = self.parse_type()?;
                params.push((p, t));
                match self.peek() {
                    Some(Tok::Sym(",")) => {
                        self.pos += 1;
                    }
                    _ => break,
                }
            }
        }
        self.eat_sym(")")?;
        self.eat_sym("->")?;
        let ret = self.parse_type()?;
        self.eat_sym("{")?;
        let body = self.parse_expr()?;
        self.eat_sym("}")?;
        if self.pos != self.toks.len() {
            return Err(err(self.offset(), "trailing tokens after candidate body"));
        }
        Ok(Candidate {
            name,
            params,
            ret,
            body,
        })
    }

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        if matches!(self.peek(), Some(Tok::Word("let"))) {
            return self.parse_let();
        }
        if matches!(self.peek(), Some(Tok::Word("if"))) {
            return self.parse_if();
        }
        self.parse_or()
    }

    fn parse_let(&mut self) -> Result<Expr, ParseError> {
        self.eat_word("let")?;
        let name = self.eat_pident()?;
        self.eat_sym("=")?;
        let value = self.parse_expr()?;
        self.eat_sym(";")?;
        let body = self.parse_expr()?;
        Ok(Expr::Let(name, Box::new(value), Box::new(body)))
    }

    fn parse_if(&mut self) -> Result<Expr, ParseError> {
        self.eat_word("if")?;
        let cond = self.parse_expr()?;
        self.eat_sym("{")?;
        let then = self.parse_expr()?;
        self.eat_sym("}")?;
        self.eat_word("else")?;
        self.eat_sym("{")?;
        let alt = self.parse_expr()?;
        self.eat_sym("}")?;
        Ok(Expr::If(Box::new(cond), Box::new(then), Box::new(alt)))
    }

    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_and()?;
        while matches!(self.peek(), Some(Tok::Sym("||"))) {
            self.pos += 1;
            let rhs = self.parse_and()?;
            lhs = Expr::BinOp(BinOp::Or, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_cmp()?;
        while matches!(self.peek(), Some(Tok::Sym("&&"))) {
            self.pos += 1;
            let rhs = self.parse_cmp()?;
            lhs = Expr::BinOp(BinOp::And, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_cmp(&mut self) -> Result<Expr, ParseError> {
        let lhs = self.parse_add()?;
        let op = match self.peek() {
            Some(Tok::Sym("==")) => Some(BinOp::Eq),
            Some(Tok::Sym("!=")) => Some(BinOp::Ne),
            Some(Tok::Sym("<")) => Some(BinOp::Lt),
            Some(Tok::Sym("<=")) => Some(BinOp::Le),
            Some(Tok::Sym(">")) => Some(BinOp::Gt),
            Some(Tok::Sym(">=")) => Some(BinOp::Ge),
            _ => None,
        };
        match op {
            None => Ok(lhs),
            Some(op) => {
                self.pos += 1;
                let rhs = self.parse_add()?;
                Ok(Expr::BinOp(op, Box::new(lhs), Box::new(rhs)))
            }
        }
    }

    fn parse_add(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_mul()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Sym("+")) => BinOp::Add,
                Some(Tok::Sym("-")) => BinOp::Sub,
                _ => break,
            };
            self.pos += 1;
            let rhs = self.parse_mul()?;
            lhs = Expr::BinOp(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_mul(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Sym("*")) => BinOp::Mul,
                Some(Tok::Sym("/")) => BinOp::Div,
                Some(Tok::Sym("%")) => BinOp::Mod,
                _ => break,
            };
            self.pos += 1;
            let rhs = self.parse_unary()?;
            lhs = Expr::BinOp(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        if matches!(self.peek(), Some(Tok::Sym("-"))) {
            self.pos += 1;
            let e = self.parse_unary()?;
            return Ok(Expr::UnOp(UnOp::Neg, Box::new(e)));
        }
        if matches!(self.peek(), Some(Tok::Sym("!"))) {
            self.pos += 1;
            let e = self.parse_unary()?;
            return Ok(Expr::UnOp(UnOp::Not, Box::new(e)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        match self.peek().cloned() {
            Some(Tok::Int(v)) => {
                self.pos += 1;
                Ok(Expr::IntLit(v))
            }
            Some(Tok::Float(v)) => {
                self.pos += 1;
                Ok(Expr::FloatLit(v))
            }
            Some(Tok::Word("true")) => {
                self.pos += 1;
                Ok(Expr::BoolLit(true))
            }
            Some(Tok::Word("false")) => {
                self.pos += 1;
                Ok(Expr::BoolLit(false))
            }
            Some(Tok::PIdent(n)) => {
                self.pos += 1;
                Ok(Expr::Var(n))
            }
            Some(Tok::Sym("[")) => {
                self.pos += 1;
                let mut elems: Vec<Expr> = Vec::new();
                if !matches!(self.peek(), Some(Tok::Sym("]"))) {
                    loop {
                        elems.push(self.parse_expr()?);
                        match self.peek() {
                            Some(Tok::Sym(",")) => { self.pos += 1; }
                            _ => break,
                        }
                    }
                }
                self.eat_sym("]")?;
                // Backward-compat: pure-int lists stay ListLit,
                // mixed int+float literal lists become FloatListLit,
                // anything with expressions becomes ListCons.
                let all_int = elems.iter().all(|e| matches!(e, Expr::IntLit(_)));
                let all_num = elems.iter().all(|e| matches!(e, Expr::IntLit(_) | Expr::FloatLit(_)));
                let any_f = elems.iter().any(|e| matches!(e, Expr::FloatLit(_)));
                if all_int {
                    let items: Vec<i64> = elems.iter().filter_map(|e| {
                        if let Expr::IntLit(v) = e { Some(*v) } else { None }
                    }).collect();
                    Ok(Expr::ListLit(items))
                } else if all_num && any_f {
                    let items: Vec<f64> = elems.iter().filter_map(|e| match e {
                        Expr::FloatLit(v) => Some(*v),
                        Expr::IntLit(v) => Some(*v as f64),
                        _ => None,
                    }).collect();
                    Ok(Expr::FloatListLit(items))
                } else {
                    Ok(Expr::ListCons(elems))
                }
            }

            Some(Tok::Sym("(")) => {
                self.pos += 1;
                let e = self.parse_expr()?;
                self.eat_sym(")")?;
                Ok(e)
            }
            Some(Tok::CallPath(p)) => {
                self.pos += 1;
                self.eat_sym("(")?;
                let mut args = Vec::new();
                if !matches!(self.peek(), Some(Tok::Sym(")"))) {
                    loop {
                        args.push(self.parse_expr()?);
                        match self.peek() {
                            Some(Tok::Sym(",")) => {
                                self.pos += 1;
                            }
                            _ => break,
                        }
                    }
                }
                self.eat_sym(")")?;
                Ok(Expr::Call(p.clone(), args))
            }
            Some(Tok::Word(w @ ("len" | "index" | "range" | "sum" | "max" | "min" | "sqrt" | "exp" | "log" | "abs"))) => {
                self.pos += 1;
                self.eat_sym("(")?;
                let e = self.parse_expr()?;
                // index takes a second argument (the position).
                let second = if w == "index" {
                    self.eat_sym(",")?;
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                self.eat_sym(")")?;
                let op = match w {
                    "len" => Builtin::Len,
                    "index" => Builtin::Index,
                    "range" => Builtin::Range,
                    "sum" => Builtin::Sum,
                    "max" => Builtin::Max,
                    "min" => Builtin::Min,
                    "sqrt" => Builtin::Sqrt,
                    "exp" => Builtin::Exp,
                    "log" => Builtin::Log,
                    _ => Builtin::Abs,
                };
                match second {
                    Some(idx) => Ok(Expr::Builtin2(op, Box::new(e), Box::new(idx))),
                    None => Ok(Expr::Builtin(op, Box::new(e))),
                }
            }
            Some(Tok::Word("fold")) => self.parse_fold(),
            other => {
                let _ = other;
                Err(err(
                    self.offset(),
                    "expected literal, %var, list, len, fold, if, or let",
                ))
            }
        }
    }

    /// `fold %v in <list-expr>, %acc from <init-expr> { <body-expr> }`
    fn parse_fold(&mut self) -> Result<Expr, ParseError> {
        self.eat_word("fold")?;
        let var = self.eat_pident()?;
        self.eat_word("in")?;
        let list = self.parse_expr()?;
        self.eat_sym(",")?;
        let acc = self.eat_pident()?;
        self.eat_word("from")?;
        let init = self.parse_expr()?;
        self.eat_sym("{")?;
        let body = self.parse_expr()?;
        self.eat_sym("}")?;
        Ok(Expr::Fold {
            var,
            acc,
            list: Box::new(list),
            init: Box::new(init),
            body: Box::new(body),
        })
    }
}

/// Parse a standalone expression (used for gen invariants).
pub fn parse_expr_str(src: &str) -> Result<Expr, ParseError> {
    let toks = lex(src)?;
    if toks.is_empty() {
        return Err(err(0, "empty expression"));
    }
    let mut p = Parser { toks, pos: 0 };
    let e = p.parse_expr()?;
    if p.pos != p.toks.len() {
        return Err(err(p.offset(), "trailing tokens after expression"));
    }
    Ok(e)
}

/// Parse a full candidate from source text (stage S1).
pub fn parse(src: &str) -> Result<Candidate, ParseError> {
    let toks = lex(src)?;
    if toks.is_empty() {
        return Err(err(0, "empty candidate"));
    }
    let mut p = Parser { toks, pos: 0 };
    p.parse_candidate()
}

// ---------------------------------------------------------------------------
// GBNF grammar — server-side mirror of the Rust parser above.
// Any change here must be applied to the parser in the same commit.
// ---------------------------------------------------------------------------

// Prefill strategy: the PROMPT ends with a literal `fn @`, so the grammar
// takes over at the function NAME — no prose is ever reachable. The Rust
// parser still expects the full `fn @name(...)` form; forge reattaches the
// prefix when extracting (see forge::extract_candidate).
// Prefill strategy: the PROMPT ends with a literal `fn @`, so the grammar
// takes over at the function NAME — no prose is ever reachable. The Rust
// parser still expects the full `fn @name(...)` form; forge reattaches the
// prefix when extracting (see forge::extract_candidate).
pub const GRAMMAR: &str = r#"
root        ::= name ws "(" ws params ws ")" ws "->" ws type ws "{" ws e ws "}" ws
name        ::= [a-z_] [a-zA-Z0-9_]*
params      ::= param (ws "," ws param)*
param       ::= pid ws ":" ws type
type        ::= "Int" | "F64" | "Bool" | "List" "<" ("Int"| "F64") ">"
e           ::= letx | ifx | orx
letx        ::= "let" ws pid ws "=" ws e ws ";" ws e
ifx         ::= "if" ws e ws "{" ws e ws "}" ws "else" ws "{" ws e ws "}"
orx         ::= andx (ws "||" ws andx)*
andx        ::= cmpx (ws "&&" ws cmpx)*
cmpx        ::= addx (ws cmpeq ws addx)?
cmpeq       ::= "==" | "!=" | "<=" | ">=" | "<" | ">"
addx        ::= mulx (ws addsym ws mulx)*
addsym      ::= "+" | "-"
mulx        ::= unx (ws mulsym ws unx)*
mulsym      ::= "*" | "/" | "%"
unx         ::= "-" unx | "!" unx | prim
callx       ::= cpath ws "(" ws callargs ws ")"
cpath       ::= ident ("." ident)*
callargs    ::= e (ws "," ws e)*
unop1       ::= "len" | "sum" | "max" | "min" | "sqrt" | "exp" | "log" | "abs" | "range"
binop1      ::= "index"
prim        ::= int | float | "true" | "false" | pid | listlit | unop1 ws "(" ws e ws ")" | binop1 ws "(" ws e ws "," ws e ws ")" | callx | "fold" ws pid ws "in" ws e ws "," ws pid ws "from" ws e ws "{" ws e ws "}" | "(" ws e ws ")"
pid         ::= "%" [a-zA-Z_] [a-zA-Z0-9_]*
listlit     ::= "[" ws "]" | "[" ws int (ws "," ws int)* ws "]"
int         ::= "-"? [0-9]+
float       ::= "-"? [0-9]+ "." [0-9]+ ("e" ("+"|"-")? [0-9]+)? | "-"? [0-9]+ "e" ("+"|"-")? [0-9]+
ident       ::= [a-zA-Z_] [a-zA-Z0-9_]*
ws          ::= [ \t\n]*
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_fold_sum() {
        let src = "fn @total(%items: List<Int>) -> Int { fold %x in %items, %acc from 0 { %acc + %x } }";
        let c = parse(src).expect("parses");
        assert_eq!(c.name, "total");
        assert_eq!(c.params, vec![("items".to_string(), Ty::ListInt)]);
        assert_eq!(c.ret, Ty::Int);
        assert!(matches!(c.body, Expr::Fold { .. }));
    }

    #[test]
    fn test_precedence_mul_over_add() {
        let c = parse("fn @f() -> Int { 2 + 3 * 4 }").expect("parses");
        match c.body {
            Expr::BinOp(BinOp::Add, _, rhs) => assert!(matches!(*rhs, Expr::BinOp(BinOp::Mul, _, _))),
            other => panic!("wrong shape: {:?}", other),
        }
    }

    #[test]
    fn test_cmp_binds_looser_than_add() {
        let c = parse("fn @f() -> Bool { 1 + 2 == 3 }").expect("parses");
        assert!(matches!(c.body, Expr::BinOp(BinOp::Eq, _, _)));
    }

    #[test]
    fn test_if_else_and_let() {
        let src = "fn @g(%n: Int) -> Int { let %m = %n * 2; if %m > 10 { %m } else { 0 - %m } }";
        let c = parse(src).expect("parses");
        assert!(matches!(c.body, Expr::Let(_, _, _)));
    }

    #[test]
    fn test_list_literal_and_len() {
        let c = parse("fn @h() -> Int { len([7, 8, 9]) }").expect("parses");
        assert!(matches!(
            c.body,
            Expr::Builtin(Builtin::Len, _)
        ));
    }

    #[test]
    fn test_math_builtins_parse() {
        let c = parse("fn @s(%x: F64) -> F64 { sqrt(%x) + exp(%x) + log(%x) + abs(%x) }")
            .expect("parses");
        assert!(matches!(c.body, Expr::BinOp(_, _, _)));
    }

    #[test]
    fn test_reduction_builtins_parse() {
        let c = parse("fn @r(%xs: List<Int>) -> Int { sum(%xs) + max(%xs) + min(%xs) }")
            .expect("parses");
        assert!(matches!(c.body, Expr::BinOp(_, _, _)));
    }

    #[test]
    fn test_reject_unknown_word() {
        let e = parse("fn @f() -> Int { Foo }").expect_err("must reject");
        assert!(e.message.contains("unknown word"));
    }

    #[test]
    fn test_reject_trailing_tokens() {
        assert!(parse("fn @f() -> Int { 1 } 1 }").is_err());
    }

    #[test]
    fn test_reject_empty_body_marker() {
        assert!(parse("").is_err());
    }

    #[test]
    fn test_grammar_mentions_all_constructs() {
        for kw in ["fold", "len", "let", "if", "else", "List"] {
            assert!(GRAMMAR.contains(kw), "grammar missing {}", kw);
        }
    }
}

/// True when the text scanned so far ends inside an exponent marker, allowing
/// a sign character next in the numeric scanner.
fn exponent_sign(scanned: &str) -> bool {
    matches!(scanned.as_bytes().last(), Some(b'e') | Some(b'E'))
}
