#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Ident(String),
    Unary(String, Box<Expr>),
    Binary(String, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    Apply(Box<Expr>, Vec<Expr>),
    Lambda(Vec<Param>, Option<String>, Box<Expr>),
    MethodCall(Box<Expr>, String, Vec<Expr>),
    Field(Box<Expr>, String),
    StructLiteral(String, Vec<(String, Expr)>),
    Index(Box<Expr>, Box<Expr>),
    Array(Vec<Expr>),
    Walrus(String, Box<Expr>),
    If(Box<Expr>, Vec<Stmt>, Vec<Stmt>),
    While(Box<Expr>, Vec<Stmt>),
    Match(Box<Expr>, Vec<(Pattern, Vec<Stmt>)>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Wildcard,
    Bind(String),
    Literal(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    VarDecl {
        name: String,
        ty: Option<String>,
        init: Expr,
    },
    Assign {
        name: String,
        expr: Expr,
    },
    IndexAssign {
        name: String,
        index: Expr,
        expr: Expr,
    },
    FieldAssign {
        name: String,
        field: String,
        expr: Expr,
    },
    Walrus {
        name: String,
        expr: Expr,
    },
    Ret(Option<Expr>),
    Expr(Expr),
    If {
        cond: Expr,
        then_body: Vec<Stmt>,
        else_body: Vec<Stmt>,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FuncDef {
    pub name: String,
    pub params: Vec<Param>,
    pub ret_ty: Option<String>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

#[derive(Clone)]
struct Line {
    indent: usize,
    text: String,
    number: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LexTokenKind {
    Int,
    Float,
    String,
    Ident,
    Op,
}

#[derive(Clone, Debug)]
struct LexToken {
    kind: LexTokenKind,
    text: String,
}

fn strip_comment(line: &str) -> String {
    let mut quoted = false;
    let mut escaped = false;
    let chars = line.chars();
    let mut out = String::new();
    for ch in chars {
        if escaped {
            out.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' && quoted {
            out.push(ch);
            escaped = true;
            continue;
        }
        if ch == '"' {
            quoted = !quoted;
            out.push(ch);
            continue;
        }
        if ch == '#' && !quoted {
            break;
        }
        out.push(ch);
    }
    out.trim_end().to_string()
}

fn expression_tokens(source: &str) -> Vec<LexToken> {
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
        } else if chars[i].is_ascii_digit() {
            let start = i;
            i += 1;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let kind = if i + 1 < chars.len() && chars[i] == '.' && chars[i + 1].is_ascii_digit() {
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
                LexTokenKind::Float
            } else {
                LexTokenKind::Int
            };
            tokens.push(LexToken {
                kind,
                text: chars[start..i].iter().collect(),
            });
        } else if chars[i] == '"' {
            i += 1;
            let mut value = String::new();
            let mut escaped = false;
            while i < chars.len() {
                let ch = chars[i];
                i += 1;
                if escaped {
                    value.push(ch);
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    break;
                } else {
                    value.push(ch);
                }
            }
            tokens.push(LexToken {
                kind: LexTokenKind::String,
                text: value,
            });
        } else if chars[i].is_ascii_alphabetic() || chars[i] == '_' {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            tokens.push(LexToken {
                kind: LexTokenKind::Ident,
                text: chars[start..i].iter().collect(),
            });
        } else {
            let two: String = if i + 1 < chars.len() {
                chars[i..i + 2].iter().collect()
            } else {
                String::new()
            };
            if ["==", "!=", "<=", ">=", ":=", "->", "=>"]
                .iter()
                .any(|op| *op == two)
            {
                tokens.push(LexToken {
                    kind: LexTokenKind::Op,
                    text: two,
                });
                i += 2;
            } else {
                let text = chars[i].to_string();
                tokens.push(LexToken {
                    kind: LexTokenKind::Op,
                    text,
                });
                i += 1;
            }
        }
    }
    tokens
}

struct ExprParser {
    tokens: Vec<LexToken>,
    pos: usize,
}

impl ExprParser {
    fn peek(&self) -> Option<&LexToken> {
        self.tokens.get(self.pos)
    }
    fn take(&mut self) -> Option<LexToken> {
        let token = self.tokens.get(self.pos).cloned();
        if token.is_some() {
            self.pos += 1;
        }
        token
    }

    fn parse(source: &str) -> Result<Expr, String> {
        let mut parser = ExprParser {
            tokens: expression_tokens(source),
            pos: 0,
        };
        let expr = parser.parse_or()?;
        if parser.peek().is_some() {
            return Err(format!(
                "unexpected expression token `{}`",
                parser.peek().unwrap().text
            ));
        }
        Ok(expr)
    }

    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_and()?;
        while self.is_keyword("or") {
            self.take();
            left = Expr::Binary("or".into(), Box::new(left), Box::new(self.parse_and()?));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_compare()?;
        while self.is_keyword("and") {
            self.take();
            left = Expr::Binary(
                "and".into(),
                Box::new(left),
                Box::new(self.parse_compare()?),
            );
        }
        Ok(left)
    }

    fn parse_compare(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_add()?;
        loop {
            let is_operator = self
                .peek()
                .is_some_and(|token| token.kind == LexTokenKind::Op);
            let op = self.peek().map(|token| token.text.as_str());
            if (is_operator && matches!(op, Some("<" | "<=" | ">" | ">=" | "==" | "!=")))
                || self.is_keyword("is")
            {
                let op = self.take().unwrap().text;
                left = Expr::Binary(op, Box::new(left), Box::new(self.parse_add()?));
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_add(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_mul()?;
        loop {
            let op = self.peek().map(|t| t.text.as_str());
            if self
                .peek()
                .is_some_and(|token| token.kind == LexTokenKind::Op)
                && matches!(op, Some("+") | Some("-"))
            {
                let op = self.take().unwrap().text;
                left = Expr::Binary(op, Box::new(left), Box::new(self.parse_mul()?));
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_mul(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;
        loop {
            let op = self.peek().map(|t| t.text.as_str());
            if self
                .peek()
                .is_some_and(|token| token.kind == LexTokenKind::Op)
                && matches!(op, Some("*") | Some("/") | Some("%"))
            {
                let op = self.take().unwrap().text;
                left = Expr::Binary(op, Box::new(left), Box::new(self.parse_unary()?));
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        if self.is_keyword("not") {
            self.take();
            return Ok(Expr::Unary("not".into(), Box::new(self.parse_unary()?)));
        }
        if self
            .peek()
            .is_some_and(|token| token.kind == LexTokenKind::Op && token.text == "-")
        {
            self.take();
            return Ok(Expr::Unary("-".into(), Box::new(self.parse_unary()?)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        let token = self
            .take()
            .ok_or_else(|| "missing expression".to_string())?;
        let mut expr = match token.kind {
            LexTokenKind::Int => Expr::Int(
                token
                    .text
                    .parse()
                    .map_err(|_| "invalid integer".to_string())?,
            ),
            LexTokenKind::Float => Expr::Float(
                token
                    .text
                    .parse()
                    .map_err(|_| "invalid float".to_string())?,
            ),
            LexTokenKind::String => Expr::String(token.text),
            LexTokenKind::Ident if token.text == "fn" => self.parse_lambda()?,
            LexTokenKind::Ident => match token.text.as_str() {
                "true" => Expr::Bool(true),
                "false" => Expr::Bool(false),
                name => Expr::Ident(name.into()),
            },
            LexTokenKind::Op => {
                if token.text == "(" {
                    let expr = self.parse_or()?;
                    if self.take().map(|t| t.text).as_deref() != Some(")") {
                        return Err("missing )".into());
                    }
                    expr
                } else if token.text == "[" {
                    let mut elements = Vec::new();
                    if self.peek().map(|t| t.text.as_str()) != Some("]") {
                        loop {
                            elements.push(self.parse_or()?);
                            if self.peek().map(|t| t.text.as_str()) == Some(",") {
                                self.take();
                            } else {
                                break;
                            }
                        }
                    }
                    if self.take().map(|t| t.text).as_deref() != Some("]") {
                        return Err("missing ]".into());
                    }
                    Expr::Array(elements)
                } else {
                    return Err(format!("unexpected operator `{}`", token.text));
                }
            }
        };

        loop {
            if self.peek().map(|t| t.text.as_str()) == Some("(") {
                self.take();
                let args = self.parse_arguments(")")?;
                expr = match expr {
                    Expr::Ident(name) => Expr::Call(name, args),
                    callee => Expr::Apply(Box::new(callee), args),
                };
            } else if self.peek().map(|t| t.text.as_str()) == Some(".") {
                self.take();
                let member = self
                    .take()
                    .filter(|token| token.kind == LexTokenKind::Ident)
                    .ok_or("expected field or method name after `.`")?
                    .text;
                if self.peek().map(|t| t.text.as_str()) == Some("(") {
                    self.take();
                    expr = Expr::MethodCall(Box::new(expr), member, self.parse_arguments(")")?);
                } else {
                    expr = Expr::Field(Box::new(expr), member);
                }
            } else if self.peek().map(|t| t.text.as_str()) == Some("{") {
                let name = match expr {
                    Expr::Ident(name) => name,
                    _ => return Err("struct literal name must be an identifier".into()),
                };
                self.take();
                let mut fields = Vec::new();
                if self.peek().map(|t| t.text.as_str()) != Some("}") {
                    loop {
                        let field = self
                            .take()
                            .filter(|token| token.kind == LexTokenKind::Ident)
                            .ok_or("expected field name in struct literal")?
                            .text;
                        if self.take().map(|token| token.text).as_deref() != Some(":") {
                            return Err("expected `:` after struct field name".into());
                        }
                        if fields.iter().any(|(name, _)| name == &field) {
                            return Err(format!("duplicate struct field `{field}`"));
                        }
                        fields.push((field, self.parse_or()?));
                        if self.peek().map(|t| t.text.as_str()) == Some(",") {
                            self.take();
                        } else {
                            break;
                        }
                    }
                }
                if self.take().map(|t| t.text).as_deref() != Some("}") {
                    return Err("missing }".into());
                }
                expr = Expr::StructLiteral(name, fields);
            } else if self.peek().map(|t| t.text.as_str()) == Some("[") {
                self.take();
                let index = self.parse_or()?;
                if self.take().map(|t| t.text).as_deref() != Some("]") {
                    return Err("missing ]".into());
                }
                expr = Expr::Index(Box::new(expr), Box::new(index));
            } else if self.peek().map(|t| t.text.as_str()) == Some(":=") {
                self.take();
                let value = self.parse_or()?;
                let name = match expr {
                    Expr::Ident(name) => name,
                    _ => return Err("walrus target must be a name".into()),
                };
                expr = Expr::Walrus(name, Box::new(value));
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_arguments(&mut self, closing: &str) -> Result<Vec<Expr>, String> {
        let mut args = Vec::new();
        if self.peek().map(|token| token.text.as_str()) != Some(closing) {
            loop {
                args.push(self.parse_or()?);
                if self.peek().map(|token| token.text.as_str()) == Some(",") {
                    self.take();
                } else {
                    break;
                }
            }
        }
        if self.take().map(|token| token.text).as_deref() != Some(closing) {
            return Err(format!("missing {closing}"));
        }
        Ok(args)
    }

    fn parse_lambda(&mut self) -> Result<Expr, String> {
        if self.take().map(|token| token.text).as_deref() != Some("(") {
            return Err("lambda parameters must start with `(`".into());
        }
        let mut params = Vec::new();
        if self.peek().map(|token| token.text.as_str()) != Some(")") {
            loop {
                let name = self
                    .take()
                    .filter(|token| token.kind == LexTokenKind::Ident)
                    .ok_or("expected lambda parameter name")?
                    .text;
                let ty = if self.peek().map(|token| token.text.as_str()) == Some(":") {
                    self.take();
                    Some(
                        self.take()
                            .filter(|token| token.kind == LexTokenKind::Ident)
                            .ok_or("expected lambda parameter type")?
                            .text,
                    )
                } else {
                    None
                };
                params.push(Param { name, ty });
                if self.peek().map(|token| token.text.as_str()) == Some(",") {
                    self.take();
                } else {
                    break;
                }
            }
        }
        if self.take().map(|token| token.text).as_deref() != Some(")") {
            return Err("missing ) in lambda parameters".into());
        }
        let return_type = if self.peek().map(|token| token.text.as_str()) == Some("->") {
            self.take();
            Some(
                self.take()
                    .filter(|token| token.kind == LexTokenKind::Ident)
                    .ok_or("expected lambda return type")?
                    .text,
            )
        } else {
            None
        };
        if self.take().map(|token| token.text).as_deref() != Some("=>") {
            return Err("expected `=>` before lambda body".into());
        }
        Ok(Expr::Lambda(
            params,
            return_type,
            Box::new(self.parse_or()?),
        ))
    }

    fn is_keyword(&self, keyword: &str) -> bool {
        self.peek()
            .map(|t| t.kind == LexTokenKind::Ident && t.text == keyword)
            .unwrap_or(false)
    }
}

fn parse_expr(source: &str) -> Result<Expr, String> {
    ExprParser::parse(source)
}

pub fn parse_source(source: &str) -> Result<Vec<FuncDef>, ParseError> {
    let mut lines = Vec::new();
    for (index, raw) in source.lines().enumerate() {
        let text = strip_comment(raw);
        if text.trim().is_empty() {
            continue;
        }
        let indent = raw.len() - raw.trim_start().len();
        if text.trim_start().starts_with("import ") {
            if indent != 0 {
                return Err(ParseError {
                    line: index + 1,
                    message: "imports are only allowed at module scope".into(),
                });
            }
            parse_import_path(text.trim()).map_err(|message| ParseError {
                line: index + 1,
                message,
            })?;
            continue;
        }
        lines.push(Line {
            indent,
            text: text.trim().to_string(),
            number: index + 1,
        });
    }
    let mut index = 0;
    let mut funcs = Vec::new();
    while index < lines.len() {
        let line = &lines[index];
        if !line.text.starts_with("fn ") {
            return Err(ParseError {
                line: line.number,
                message: "top-level statement must be fn".into(),
            });
        }
        let (name, params, ret_ty) =
            parse_func_header(&line.text).map_err(|message| ParseError {
                line: line.number,
                message,
            })?;
        index += 1;
        let base_indent = line.indent;
        let mut body = Vec::new();
        while index < lines.len() {
            if lines[index].indent <= base_indent && lines[index].text.starts_with("fn ") {
                break;
            }
            if lines[index].indent <= base_indent && lines[index].text == "end" {
                index += 1;
                break;
            }
            let (stmt, next) = parse_stmt(&lines, index).map_err(|message| ParseError {
                line: lines[index].number,
                message,
            })?;
            body.push(stmt);
            index = next;
        }
        funcs.push(FuncDef {
            name,
            params,
            ret_ty,
            body,
        });
    }
    Ok(funcs)
}

pub fn parse_imports(source: &str) -> Result<Vec<String>, ParseError> {
    let mut imports = Vec::new();
    for (index, raw) in source.lines().enumerate() {
        let text = strip_comment(raw);
        if text.trim().is_empty() {
            continue;
        }
        let indent = raw.len() - raw.trim_start().len();
        if text.trim_start().starts_with("import ") {
            if indent != 0 {
                return Err(ParseError {
                    line: index + 1,
                    message: "imports are only allowed at module scope".into(),
                });
            }
            imports.push(
                parse_import_path(text.trim()).map_err(|message| ParseError {
                    line: index + 1,
                    message,
                })?,
            );
        }
    }
    Ok(imports)
}

fn parse_import_path(text: &str) -> Result<String, String> {
    let path = text
        .strip_prefix("import ")
        .ok_or("invalid import declaration")?
        .trim();
    let path = if let Some(path) = path
        .strip_prefix('"')
        .and_then(|path| path.strip_suffix('"'))
    {
        path
    } else {
        path
    };
    if path.is_empty() || path.chars().any(char::is_whitespace) {
        return Err("import expects one module path".into());
    }
    Ok(path.to_string())
}

fn parse_func_header(text: &str) -> Result<(String, Vec<Param>, Option<String>), String> {
    let rest = text.strip_prefix("fn ").ok_or("invalid function")?;
    let open = rest.find('(').ok_or("missing ( in function header")?;
    let close = rest.rfind(')').ok_or("missing ) in function header")?;
    let raw_name = rest[..open].trim();
    let name = if let Some(generic_start) = raw_name.find('<') {
        let generic_end = raw_name
            .strip_suffix('>')
            .map(str::len)
            .ok_or("generic parameter list must end with `>`")?;
        if generic_start == 0 || generic_start >= generic_end {
            return Err("invalid generic function name".into());
        }
        let generic_names = &raw_name[generic_start + 1..generic_end];
        if generic_names
            .split(',')
            .any(|param| !is_identifier(param.trim()))
        {
            return Err("generic parameters must be identifiers".into());
        }
        raw_name[..generic_start].trim()
    } else {
        raw_name
    };
    if name.is_empty() {
        return Err("missing function name".into());
    }
    let mut params = Vec::new();
    let param_text = &rest[open + 1..close];
    if !param_text.trim().is_empty() {
        for item in param_text.split(',') {
            let item = item.trim();
            let mut parts = item.splitn(2, ':');
            let pname = parts.next().unwrap().trim();
            let ty = parts
                .next()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty());
            params.push(Param {
                name: pname.into(),
                ty,
            });
        }
    }
    let tail = rest[close + 1..].trim();
    let ret_ty = tail
        .strip_prefix("->")
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    Ok((name.into(), params, ret_ty))
}

fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(ch) if ch.is_ascii_alphabetic() || ch == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn split_assignment(text: &str) -> Option<(String, Option<String>, String)> {
    let eq = text.find('=')?;
    if text.as_bytes().get(eq + 1) == Some(&b'=') {
        return None;
    }
    if eq > 0 && text.as_bytes()[eq - 1] == b':' {
        return None;
    }
    let lhs = text[..eq].trim();
    let rhs = text[eq + 1..].trim();
    if lhs.is_empty() {
        return None;
    }
    if let Some(colon) = lhs.find(':') {
        let name = lhs[..colon].trim();
        let ty = lhs[colon + 1..].trim();
        Some((name.into(), Some(ty.into()), rhs.into()))
    } else {
        Some((lhs.into(), None, rhs.into()))
    }
}

fn parse_if_statement(
    lines: &[Line],
    index: usize,
    condition: &str,
) -> Result<(Stmt, usize), String> {
    let (stmt, next) = parse_if_chain(lines, index, condition)?;
    Ok((stmt, next))
}

fn parse_if_expression(
    lines: &[Line],
    index: usize,
    condition: &str,
) -> Result<(Stmt, Expr, usize), String> {
    let (stmt, next) = parse_if_chain(lines, index, condition)?;
    let expr = match &stmt {
        Stmt::If {
            cond,
            then_body,
            else_body,
        } => Expr::If(Box::new(cond.clone()), then_body.clone(), else_body.clone()),
        _ => unreachable!(),
    };
    Ok((stmt, expr, next))
}

fn parse_if_chain(lines: &[Line], index: usize, condition: &str) -> Result<(Stmt, usize), String> {
    let indent = lines[index].indent;
    let cond = parse_expr(condition)?;
    let mut next = index + 1;
    let mut then_body = Vec::new();
    while next < lines.len() && lines[next].indent > indent {
        let (stmt, following) = parse_stmt(lines, next)?;
        then_body.push(stmt);
        next = following;
    }

    if next >= lines.len() || lines[next].indent > indent {
        return Err("if block is missing `end`".into());
    }

    let mut else_body = Vec::new();
    if lines[next].indent == indent && lines[next].text == "else" {
        next += 1;
        while next < lines.len() && lines[next].indent > indent {
            let (stmt, following) = parse_stmt(lines, next)?;
            else_body.push(stmt);
            next = following;
        }
    } else if lines[next].indent == indent
        && let Some(else_if_condition) = lines[next].text.strip_prefix("else if ")
    {
        let (else_if, following) = parse_if_chain(lines, next, else_if_condition)?;
        else_body.push(else_if);
        return Ok((
            Stmt::If {
                cond,
                then_body,
                else_body,
            },
            following,
        ));
    }

    if next >= lines.len() || lines[next].indent != indent || lines[next].text != "end" {
        return Err("if block is missing `end`".into());
    }
    Ok((
        Stmt::If {
            cond,
            then_body,
            else_body,
        },
        next + 1,
    ))
}

fn parse_match_expression(
    lines: &[Line],
    index: usize,
    value_source: &str,
) -> Result<(Expr, usize), String> {
    let indent = lines[index].indent;
    let value = parse_expr(value_source)?;
    let mut next = index + 1;
    let mut arms = Vec::new();
    while next < lines.len() && lines[next].indent > indent {
        let line = &lines[next];
        let pattern_source = line
            .text
            .strip_prefix("case ")
            .ok_or("expected `case` in match expression")?;
        let pattern_expr = parse_expr(pattern_source)?;
        let pattern = match pattern_expr {
            Expr::Ident(name) if name == "_" => Pattern::Wildcard,
            Expr::Ident(name) => Pattern::Bind(name),
            literal @ (Expr::Int(_) | Expr::Float(_) | Expr::String(_) | Expr::Bool(_)) => {
                Pattern::Literal(literal)
            }
            Expr::Unary(op, inner)
                if op == "-" && matches!(*inner, Expr::Int(_) | Expr::Float(_)) =>
            {
                Pattern::Literal(Expr::Unary(op, inner))
            }
            _ => return Err("match pattern must be a literal, name, or `_`".into()),
        };
        let case_indent = line.indent;
        next += 1;
        let mut body = Vec::new();
        while next < lines.len() && lines[next].indent > case_indent {
            let (statement, following) = parse_stmt(lines, next)?;
            body.push(statement);
            next = following;
        }
        arms.push((pattern, body));
    }
    if arms.is_empty() {
        return Err("match expression must contain at least one `case`".into());
    }
    if next >= lines.len() || lines[next].indent != indent || lines[next].text != "end" {
        return Err("match expression is missing `end`".into());
    }
    Ok((Expr::Match(Box::new(value), arms), next + 1))
}

fn parse_stmt(lines: &[Line], index: usize) -> Result<(Stmt, usize), String> {
    let line = &lines[index];
    let text = line.text.as_str();
    if text == "end" {
        return Err("unexpected `end`".into());
    }
    if text == "ret" {
        return Ok((Stmt::Ret(None), index + 1));
    }
    if let Some(rest) = text.strip_prefix("ret ") {
        return Ok((Stmt::Ret(Some(parse_expr(rest)?)), index + 1));
    }

    if let Some(rest) = text.strip_prefix("while ") {
        let cond = parse_expr(rest)?;
        let mut next = index + 1;
        let mut body = Vec::new();
        let mut closed = false;
        while next < lines.len() {
            if lines[next].text == "end" && lines[next].indent == line.indent {
                next += 1;
                closed = true;
                break;
            }
            if lines[next].indent <= line.indent {
                return Err("while block is missing `end`".into());
            }
            let (stmt, following) = parse_stmt(lines, next)?;
            body.push(stmt);
            next = following;
        }
        if !closed {
            return Err("while block is missing `end`".into());
        }
        return Ok((Stmt::While { cond, body }, next));
    }

    if let Some(rest) = text.strip_prefix("if ") {
        return parse_if_statement(lines, index, rest);
    }

    if let Some(rest) = text.strip_prefix("match ") {
        let (expr, next) = parse_match_expression(lines, index, rest)?;
        return Ok((Stmt::Expr(expr), next));
    }

    if let Some(assign) = split_assignment(text) {
        let (name, ty, rhs) = assign;
        if rhs.starts_with("if ") {
            let (_, expr, next) =
                parse_if_expression(lines, index, rhs.strip_prefix("if ").unwrap())?;
            if let Some(ty) = ty {
                return Ok((
                    Stmt::VarDecl {
                        name,
                        ty: Some(ty),
                        init: expr,
                    },
                    next,
                ));
            }
            return Ok((Stmt::Assign { name, expr }, next));
        }
        if rhs.starts_with("while ") {
            let cond = parse_expr(rhs.strip_prefix("while ").unwrap())?;
            let mut next = index + 1;
            let mut body = Vec::new();
            let mut closed = false;
            while next < lines.len() {
                if lines[next].text == "end" && lines[next].indent == line.indent {
                    next += 1;
                    closed = true;
                    break;
                }
                if lines[next].indent <= line.indent {
                    break;
                }
                let (stmt, following) = parse_stmt(lines, next)?;
                body.push(stmt);
                next = following;
            }
            if !closed {
                return Err("while block expression is missing `end`".into());
            }
            let expr = Expr::While(Box::new(cond), body);
            if let Some(ty) = ty {
                return Ok((
                    Stmt::VarDecl {
                        name,
                        ty: Some(ty),
                        init: expr,
                    },
                    next,
                ));
            }
            return Ok((Stmt::Assign { name, expr }, next));
        }
        if rhs.starts_with("match ") {
            let (expr, next) =
                parse_match_expression(lines, index, rhs.strip_prefix("match ").unwrap())?;
            if let Some(ty) = ty {
                return Ok((
                    Stmt::VarDecl {
                        name,
                        ty: Some(ty),
                        init: expr,
                    },
                    next,
                ));
            }
            return Ok((Stmt::Assign { name, expr }, next));
        }
        if let Some(open) = name.find('[')
            && let Some(close) = name.rfind(']')
            && close > open
        {
            let target = name[..open].trim();
            let target = match parse_expr(target)? {
                Expr::Ident(target) => target,
                _ => return Err("array assignment target must be an identifier".into()),
            };
            let index_expr = parse_expr(&name[open + 1..close])?;
            let value_expr = parse_expr(&rhs)?;
            return Ok((
                Stmt::IndexAssign {
                    name: target,
                    index: index_expr,
                    expr: value_expr,
                },
                index + 1,
            ));
        }
        if let Some((target, field)) = name.split_once('.') {
            if !is_identifier(target.trim()) || !is_identifier(field.trim()) {
                return Err("field assignment target must be `name.field`".into());
            }
            return Ok((
                Stmt::FieldAssign {
                    name: target.trim().into(),
                    field: field.trim().into(),
                    expr: parse_expr(&rhs)?,
                },
                index + 1,
            ));
        }
        let expr = parse_expr(&rhs)?;
        if let Some(ty) = ty {
            return Ok((
                Stmt::VarDecl {
                    name,
                    ty: Some(ty),
                    init: expr,
                },
                index + 1,
            ));
        }
        return Ok((Stmt::Assign { name, expr }, index + 1));
    }

    if text.contains(":=") {
        let mut parts = text.splitn(2, ":=");
        let name = parts.next().unwrap().trim();
        let rhs = parts.next().unwrap_or("").trim();
        return Ok((
            Stmt::Walrus {
                name: name.into(),
                expr: parse_expr(rhs)?,
            },
            index + 1,
        ));
    }

    Ok((Stmt::Expr(parse_expr(text)?), index + 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluator::{Evaluator, Value};

    #[test]
    fn parses_comments_types_functions_and_ret() {
        let funcs = parse_source(
            r#"
fn add(a:int, b:int) -> int
    c:int = a + b # comment
ret c
"#,
        )
        .unwrap();
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "add");
        assert_eq!(funcs[0].params.len(), 2);
    }

    #[test]
    fn parses_string_comparisons_combined_with_or() {
        parse_source(
            "fn is_add(operator:string) -> bool\nret operator is \"+\" or operator is \"-\"\n",
        )
        .unwrap();
    }

    #[test]
    fn parses_and_executes_erased_generic_functions() {
        let funcs = parse_source(
            r#"
fn identity<T>(value:T) -> T
ret value
fn main() -> int
ret identity(42)
"#,
        )
        .unwrap();
        let value = Evaluator::new(funcs).call("main", vec![]).unwrap();
        assert!(matches!(value, Value::Int(42)));
    }

    #[test]
    fn evaluates_struct_fields_and_methods() {
        let funcs = parse_source(
            r#"
fn Point.sum(self:Point) -> int
ret self.x + self.y
fn main() -> int
    point:Point = Point{x: 3, y: 4}
    point.x = 8
ret point.sum()
"#,
        )
        .unwrap();
        crate::typecheck::check_program(&funcs).unwrap();
        let value = Evaluator::new(funcs).call("main", vec![]).unwrap();
        assert!(matches!(value, Value::Int(12)));
    }

    #[test]
    fn evaluates_match_literal_and_binding_patterns() {
        let funcs = parse_source(
            r#"
fn classify(value:int) -> int
    result:int = match value
        case 0
            12
        case other
            other + 10
    end
ret result
"#,
        )
        .unwrap();
        crate::typecheck::check_program(&funcs).unwrap();
        let evaluator = Evaluator::new(funcs);
        assert!(matches!(
            evaluator.call("classify", vec![Value::Int(0)]).unwrap(),
            Value::Int(12)
        ));
        assert!(matches!(
            evaluator.call("classify", vec![Value::Int(3)]).unwrap(),
            Value::Int(13)
        ));
    }

    #[test]
    fn evaluates_lexically_captured_closures() {
        let funcs = parse_source(
            r#"
fn main() -> int
    base:int = 37
    add_base:fn = fn(value:int) -> int => value + base
ret add_base(5)
"#,
        )
        .unwrap();
        crate::typecheck::check_program(&funcs).unwrap();
        let value = Evaluator::new(funcs).call("main", vec![]).unwrap();
        assert!(matches!(value, Value::Int(42)));
    }

    #[test]
    fn runs_the_lume_written_lexer() {
        let functions = parse_source(include_str!("../../../lib/syntax/lexer.lume")).unwrap();
        crate::typecheck::check_program(&functions).unwrap();
        let value = Evaluator::new(functions)
            .call(
                "lex",
                vec![Value::String(
                    "fn answer() -> int # comment\nret 42\n".into(),
                )],
            )
            .unwrap();
        let Value::Array(tokens) = value else {
            panic!("lexer must return an array");
        };
        assert_eq!(tokens.len(), 10);
        assert!(matches!(
            &tokens[0],
            Value::Struct { fields, .. }
                if matches!(fields.get("text"), Some(Value::String(text)) if text == "fn")
                    && matches!(fields.get("start"), Some(Value::Int(0)))
        ));
        assert!(matches!(
            &tokens[1],
            Value::Struct { fields, .. }
                if matches!(fields.get("text"), Some(Value::String(text)) if text == "answer")
                    && matches!(fields.get("start"), Some(Value::Int(3)))
                    && matches!(fields.get("line"), Some(Value::Int(1)))
                    && matches!(fields.get("column"), Some(Value::Int(3)))
        ));
        assert!(matches!(
            &tokens[6],
            Value::Struct { fields, .. }
                if matches!(fields.get("kind"), Some(Value::String(kind)) if kind == "newline")
                    && matches!(fields.get("line"), Some(Value::Int(1)))
        ));
    }

    #[test]
    fn runs_the_lume_written_parser_on_integer_functions() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib/syntax/parser.lume");
        let functions = crate::modules::load_source_graph(path).unwrap();
        crate::typecheck::check_program(&functions).unwrap();
        let value = Evaluator::new(functions)
            .call(
                "parse",
                vec![Value::String(
                    "fn add(a:int, b:int) -> int\nret a + b\n\nfn main() -> int\nret add(19, 23)\n"
                        .into(),
                )],
            )
            .unwrap();
        let Value::Struct { fields, .. } = value else {
            panic!("parser must return an AST file");
        };
        assert!(matches!(fields.get("error"), Some(Value::String(error)) if error.is_empty()));
        assert!(
            matches!(fields.get("functions"), Some(Value::Array(functions)) if functions.len() == 2)
        );
    }

    #[test]
    fn self_hosted_parser_does_not_confuse_nested_end_with_function_end() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib/syntax/parser.lume");
        let functions = crate::modules::load_source_graph(path).unwrap();
        let value = Evaluator::new(functions)
            .call(
                "parse",
                vec![Value::String(
                    "fn main()\n    if true\n        if true\n            ret 1\n        end\n    end\nfn after()\nret 0\n"
                        .into(),
                )],
            )
            .unwrap();
        assert!(matches!(
            value,
            Value::Struct { fields, .. }
                if matches!(
                    fields.get("error"),
                    Some(Value::String(message))
                        if message.contains("straight-line function bodies")
                )
        ));
    }

    #[test]
    fn evaluates_sum_with_walrus_and_while() {
        let funcs = parse_source(
            r#"
fn sum_to(n:int) -> int
    s:int = 0
    i:int = 1
    while i <= n
        s := s + i
        i := i + 1
    end
ret s
"#,
        )
        .unwrap();
        let value = Evaluator::new(funcs)
            .call("sum_to", vec![Value::Int(10)])
            .unwrap();
        assert!(matches!(value, Value::Int(55)));
    }

    #[test]
    fn evaluates_if_block_expression() {
        let funcs = parse_source(
            r#"
fn max(a:int, b:int) -> int
    res:int = if a > b
        a
    else
        b
    end
ret res
"#,
        )
        .unwrap();
        let value = Evaluator::new(funcs)
            .call("max", vec![Value::Int(7), Value::Int(3)])
            .unwrap();
        assert!(matches!(value, Value::Int(7)));
    }

    #[test]
    fn evaluates_else_if_block_expression() {
        let funcs = parse_source(
            r#"
fn classify(n:int) -> int
    result:int = if n < 0
        -1
    else if n == 0
        0
    else
        1
    end
ret result
"#,
        )
        .unwrap();
        let evaluator = Evaluator::new(funcs);
        assert!(matches!(
            evaluator.call("classify", vec![Value::Int(-3)]).unwrap(),
            Value::Int(-1)
        ));
        assert!(matches!(
            evaluator.call("classify", vec![Value::Int(0)]).unwrap(),
            Value::Int(0)
        ));
        assert!(matches!(
            evaluator.call("classify", vec![Value::Int(3)]).unwrap(),
            Value::Int(1)
        ));
    }

    #[test]
    fn evaluates_while_block_expression() {
        let funcs = parse_source(
            r#"
fn count_to_three() -> int
    i:int = 0
    result:int = while i < 3
        i := i + 1
        i
    end
ret result
"#,
        )
        .unwrap();
        let value = Evaluator::new(funcs)
            .call("count_to_three", vec![])
            .unwrap();
        assert!(matches!(value, Value::Int(3)));
    }

    #[test]
    fn while_block_expression_without_iterations_returns_zero() {
        let funcs = parse_source(
            r#"
fn no_iterations() -> int
    result:int = while false
        9
    end
ret result
"#,
        )
        .unwrap();
        let value = Evaluator::new(funcs).call("no_iterations", vec![]).unwrap();
        assert!(matches!(value, Value::Int(0)));
    }

    #[test]
    fn evaluates_array_element_assignment() {
        let funcs = parse_source(
            r#"
fn update() -> int
    values:int = [10, 20, 30]
    values[1] = 42
ret values[1]
"#,
        )
        .unwrap();
        let value = Evaluator::new(funcs).call("update", vec![]).unwrap();
        assert!(matches!(value, Value::Int(42)));
    }

    #[test]
    fn array_element_assignment_reports_out_of_bounds() {
        let funcs = parse_source(
            r#"
fn update() -> int
    values:int = [10]
    values[1] = 42
ret 0
"#,
        )
        .unwrap();
        let error = Evaluator::new(funcs).call("update", vec![]).unwrap_err();
        assert!(error.contains("out of bounds"));
    }

    #[test]
    fn reports_missing_control_flow_end() {
        let error = parse_source(
            r#"
fn incomplete() -> int
    if true
        ret 1
"#,
        )
        .unwrap_err();
        assert!(error.message.contains("missing `end`"));
    }

    #[test]
    fn reports_unmatched_end() {
        let error = parse_source(
            r#"
fn unmatched()
    end
"#,
        )
        .unwrap_err();
        assert!(error.message.contains("unexpected `end`"));
    }

    #[test]
    fn evaluates_float_math_and_sleef_aliases() {
        let funcs = parse_source(
            r#"
fn math() -> float
    result:float = sin(0.0) + sqrt(9.0) + pow(2.0, 3.0)
    result = result + sleef_sin_u10(0.0)
ret result
"#,
        )
        .unwrap();
        let value = Evaluator::new(funcs).call("math", vec![]).unwrap();
        assert!(matches!(value, Value::Float(value) if (value - 11.0).abs() < 1e-12));
    }

    #[test]
    fn evaluates_log_builtin() {
        let funcs = parse_source(
            r#"
fn log_test() -> int
    log_at("info", "math initialized")
ret 1
"#,
        )
        .unwrap();
        let value = Evaluator::new(funcs).call("log_test", vec![]).unwrap();
        assert!(matches!(value, Value::Int(1)));
    }
}
