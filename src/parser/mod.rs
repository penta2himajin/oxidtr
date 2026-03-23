pub mod ast;
pub mod lexer;

use ast::*;
use lexer::{Lexer, Token};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    UnexpectedToken { expected: String, found: String, pos: usize },
    UnexpectedEof { expected: String },
    InvalidSyntax { message: String, pos: usize },
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::UnexpectedToken { expected, found, pos } => {
                write!(f, "pos {pos}: expected {expected}, found {found}")
            }
            ParseError::UnexpectedEof { expected } => {
                write!(f, "unexpected end of input, expected {expected}")
            }
            ParseError::InvalidSyntax { message, pos } => {
                write!(f, "pos {pos}: {message}")
            }
        }
    }
}

impl std::error::Error for ParseError {}

struct Parser<'a> {
    lexer: Lexer<'a>,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Parser {
            lexer: Lexer::new(input),
        }
    }

    fn next(&mut self) -> Token {
        self.lexer.next_token()
    }

    fn peek(&mut self) -> Token {
        self.lexer.peek()
    }

    fn expect(&mut self, expected: &Token) -> Result<Token, ParseError> {
        let tok = self.next();
        if std::mem::discriminant(&tok) == std::mem::discriminant(expected) {
            Ok(tok)
        } else {
            Err(ParseError::UnexpectedToken {
                expected: format!("{expected:?}"),
                found: format!("{tok:?}"),
                pos: self.lexer.pos(),
            })
        }
    }

    fn expect_ident(&mut self) -> Result<String, ParseError> {
        match self.next() {
            Token::Ident(s) => Ok(s),
            other => Err(ParseError::UnexpectedToken {
                expected: "identifier".to_string(),
                found: format!("{other:?}"),
                pos: self.lexer.pos(),
            }),
        }
    }

    fn parse_model(&mut self) -> Result<AlloyModel, ParseError> {
        let mut model = AlloyModel {
            sigs: Vec::new(),
            facts: Vec::new(),
            preds: Vec::new(),
            funs: Vec::new(),
            asserts: Vec::new(),
        };

        loop {
            match self.peek() {
                Token::Eof => break,
                Token::Sig => {
                    model.sigs.push(self.parse_sig(false)?);
                }
                Token::Abstract => {
                    self.next();
                    self.expect(&Token::Sig)?;
                    model.sigs.push(self.parse_sig_body(true, SigMultiplicity::Default)?);
                }
                Token::One => {
                    self.next();
                    match self.peek() {
                        Token::Sig => {
                            self.next();
                            model.sigs.push(self.parse_sig_body(false, SigMultiplicity::One)?);
                        }
                        _ => {
                            return Err(ParseError::InvalidSyntax {
                                message: "'one' must be followed by 'sig'".to_string(),
                                pos: self.lexer.pos(),
                            });
                        }
                    }
                }
                Token::Some_ => {
                    // Peek ahead: `some sig` is a sig decl; otherwise it's an expression (handled elsewhere)
                    self.next();
                    match self.peek() {
                        Token::Sig => {
                            self.next();
                            model.sigs.push(self.parse_sig_body(false, SigMultiplicity::Some)?);
                        }
                        _ => {
                            return Err(ParseError::InvalidSyntax {
                                message: "'some' at top level must be followed by 'sig'".to_string(),
                                pos: self.lexer.pos(),
                            });
                        }
                    }
                }
                Token::Lone => {
                    // Peek ahead: `lone sig` is a sig decl; otherwise it's a field multiplicity
                    self.next();
                    match self.peek() {
                        Token::Sig => {
                            self.next();
                            model.sigs.push(self.parse_sig_body(false, SigMultiplicity::Lone)?);
                        }
                        _ => {
                            return Err(ParseError::InvalidSyntax {
                                message: "'lone' at top level must be followed by 'sig'".to_string(),
                                pos: self.lexer.pos(),
                            });
                        }
                    }
                }
                Token::Fact => {
                    model.facts.push(self.parse_fact()?);
                }
                Token::Pred => {
                    model.preds.push(self.parse_pred()?);
                }
                Token::Fun => {
                    model.funs.push(self.parse_fun()?);
                }
                Token::Assert => {
                    model.asserts.push(self.parse_assert()?);
                }
                Token::Check | Token::Run => {
                    self.skip_command();
                }
                _ => {
                    let tok = self.next();
                    return Err(ParseError::InvalidSyntax {
                        message: format!("unexpected top-level token: {tok:?}"),
                        pos: self.lexer.pos(),
                    });
                }
            }
        }

        Ok(model)
    }

    fn parse_sig(&mut self, is_abstract: bool) -> Result<SigDecl, ParseError> {
        self.expect(&Token::Sig)?;
        self.parse_sig_body(is_abstract, SigMultiplicity::Default)
    }

    fn parse_sig_body(&mut self, is_abstract: bool, multiplicity: SigMultiplicity) -> Result<SigDecl, ParseError> {
        let name = self.expect_ident()?;

        let parent = if self.peek() == Token::Extends {
            self.next();
            Some(self.expect_ident()?)
        } else {
            None
        };

        self.expect(&Token::LBrace)?;

        let mut fields = Vec::new();
        while self.peek() != Token::RBrace {
            fields.push(self.parse_field()?);
            if self.peek() == Token::Comma {
                self.next();
            }
        }

        self.expect(&Token::RBrace)?;

        Ok(SigDecl {
            name,
            is_abstract,
            multiplicity,
            parent,
            fields,
        })
    }

    fn parse_field(&mut self) -> Result<FieldDecl, ParseError> {
        let name = self.expect_ident()?;
        self.expect(&Token::Colon)?;
        let mult = self.parse_multiplicity()?;
        let target = self.expect_ident()?;
        // Check for `->` (product/map type): `field: A -> B`
        let value_type = if self.peek() == Token::Arrow {
            self.next(); // consume ->
            Some(self.expect_ident()?)
        } else {
            None
        };
        Ok(FieldDecl { name, mult, target, value_type })
    }

    fn parse_multiplicity(&mut self) -> Result<Multiplicity, ParseError> {
        match self.peek() {
            Token::One => { self.next(); Ok(Multiplicity::One) }
            Token::Lone => { self.next(); Ok(Multiplicity::Lone) }
            Token::Set => { self.next(); Ok(Multiplicity::Set) }
            Token::Seq => { self.next(); Ok(Multiplicity::Seq) }
            _ => Ok(Multiplicity::One),
        }
    }

    fn parse_fact(&mut self) -> Result<FactDecl, ParseError> {
        self.expect(&Token::Fact)?;

        let name = if let Token::Ident(_) = self.peek() {
            Some(self.expect_ident()?)
        } else {
            None
        };

        self.expect(&Token::LBrace)?;
        let body = self.parse_expr()?;
        self.expect(&Token::RBrace)?;

        Ok(FactDecl { name, body })
    }

    fn parse_pred(&mut self) -> Result<PredDecl, ParseError> {
        self.expect(&Token::Pred)?;
        let name = self.expect_ident()?;

        let mut params = Vec::new();
        if self.peek() == Token::LBracket {
            self.next();
            while self.peek() != Token::RBracket {
                params.push(self.parse_param()?);
                if self.peek() == Token::Comma {
                    self.next();
                }
            }
            self.expect(&Token::RBracket)?;
        }

        self.expect(&Token::LBrace)?;
        let mut body = Vec::new();
        while self.peek() != Token::RBrace {
            body.push(self.parse_expr()?);
        }
        self.expect(&Token::RBrace)?;

        Ok(PredDecl { name, params, body })
    }

    fn parse_fun(&mut self) -> Result<FunDecl, ParseError> {
        self.expect(&Token::Fun)?;
        let name = self.expect_ident()?;

        let mut params = Vec::new();
        if self.peek() == Token::LBracket {
            self.next();
            while self.peek() != Token::RBracket {
                params.push(self.parse_param()?);
                if self.peek() == Token::Comma {
                    self.next();
                }
            }
            self.expect(&Token::RBracket)?;
        }

        // Return type: `: mult Type`
        self.expect(&Token::Colon)?;
        let return_mult = self.parse_multiplicity()?;
        let return_type = self.expect_ident()?;

        self.expect(&Token::LBrace)?;
        let body = self.parse_expr()?;
        self.expect(&Token::RBrace)?;

        Ok(FunDecl { name, params, return_mult, return_type, body })
    }

    fn parse_param(&mut self) -> Result<ParamDecl, ParseError> {
        let name = self.expect_ident()?;
        self.expect(&Token::Colon)?;
        let mult = self.parse_multiplicity()?;
        let type_name = self.expect_ident()?;
        Ok(ParamDecl { name, mult, type_name })
    }

    fn parse_assert(&mut self) -> Result<AssertDecl, ParseError> {
        self.expect(&Token::Assert)?;
        let name = self.expect_ident()?;
        self.expect(&Token::LBrace)?;
        let body = self.parse_expr()?;
        self.expect(&Token::RBrace)?;
        Ok(AssertDecl { name, body })
    }

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_logic_or()
    }

    fn parse_logic_or(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_logic_and()?;
        while self.peek() == Token::Or {
            self.next();
            let right = self.parse_logic_and()?;
            left = Expr::BinaryLogic {
                op: LogicOp::Or,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_logic_and(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_logic_implies()?;
        while self.peek() == Token::And {
            self.next();
            let right = self.parse_logic_implies()?;
            left = Expr::BinaryLogic {
                op: LogicOp::And,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_logic_implies(&mut self) -> Result<Expr, ParseError> {
        let left = self.parse_not()?;
        if self.peek() == Token::Implies {
            self.next();
            let right = self.parse_not()?;
            return Ok(Expr::BinaryLogic {
                op: LogicOp::Implies,
                left: Box::new(left),
                right: Box::new(right),
            });
        }
        if self.peek() == Token::Iff {
            self.next();
            let right = self.parse_not()?;
            return Ok(Expr::BinaryLogic {
                op: LogicOp::Iff,
                left: Box::new(left),
                right: Box::new(right),
            });
        }
        Ok(left)
    }

    /// `not` binds less tightly than comparison in Alloy.
    /// `not a = b` means `not(a = b)`.
    fn parse_not(&mut self) -> Result<Expr, ParseError> {
        if self.peek() == Token::Not {
            self.next();
            let inner = self.parse_not()?;
            return Ok(Expr::Not(Box::new(inner)));
        }
        self.parse_comparison()
    }

    fn parse_comparison(&mut self) -> Result<Expr, ParseError> {
        let left = self.parse_set_op()?;
        let op = match self.peek() {
            Token::In    => Some(CompareOp::In),
            Token::Eq    => Some(CompareOp::Eq),
            Token::NotEq => Some(CompareOp::NotEq),
            Token::Lt    => Some(CompareOp::Lt),
            Token::Gt    => Some(CompareOp::Gt),
            Token::Lte   => Some(CompareOp::Lte),
            Token::Gte   => Some(CompareOp::Gte),
            _ => None,
        };
        if let Some(op) = op {
            self.next();
            let right = self.parse_set_op()?;
            Ok(Expr::Comparison {
                op,
                left: Box::new(left),
                right: Box::new(right),
            })
        } else {
            Ok(left)
        }
    }

    /// Set operations (+, &, -, ->) bind tighter than comparison but looser than unary/field access.
    fn parse_set_op(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_unary()?;
        loop {
            match self.peek() {
                Token::Plus => {
                    self.next();
                    let right = self.parse_unary()?;
                    left = Expr::SetOp {
                        op: SetOpKind::Union,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                Token::Ampersand => {
                    self.next();
                    let right = self.parse_unary()?;
                    left = Expr::SetOp {
                        op: SetOpKind::Intersection,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                Token::Minus => {
                    self.next();
                    let right = self.parse_unary()?;
                    left = Expr::SetOp {
                        op: SetOpKind::Difference,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                Token::Arrow => {
                    self.next();
                    let right = self.parse_unary()?;
                    left = Expr::Product {
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        match self.peek() {
            Token::Hash => {
                self.next();
                let inner = self.parse_field_access()?;
                Ok(Expr::Cardinality(Box::new(inner)))
            }
            Token::All => {
                self.parse_quantifier()
            }
            Token::Some_ | Token::No => {
                // Try quantifier first (some/no x: S | body), fall back to formula (some/no expr)
                let saved = self.lexer.pos();
                match self.parse_quantifier() {
                    Ok(expr) => Ok(expr),
                    Err(_) => {
                        self.lexer.set_pos(saved);
                        self.parse_mult_formula()
                    }
                }
            }
            _ => self.parse_field_access(),
        }
    }

    /// Parse `some expr` or `no expr` as a multiplicity formula.
    fn parse_mult_formula(&mut self) -> Result<Expr, ParseError> {
        let kind = match self.next() {
            Token::Some_ => QuantKind::Some,
            Token::No => QuantKind::No,
            _ => unreachable!(),
        };
        let inner = self.parse_field_access()?;
        Ok(Expr::MultFormula {
            kind,
            expr: Box::new(inner),
        })
    }

    fn parse_quantifier(&mut self) -> Result<Expr, ParseError> {
        let kind = match self.next() {
            Token::All => QuantKind::All,
            Token::Some_ => QuantKind::Some,
            Token::No => QuantKind::No,
            _ => unreachable!(),
        };

        let bindings = self.parse_quant_bindings()?;
        self.expect(&Token::Pipe)?;
        let body = self.parse_expr()?;

        Ok(Expr::Quantifier {
            kind,
            bindings,
            body: Box::new(body),
        })
    }

    /// Parse one or more quantifier bindings separated by `,` after the domain.
    /// Each binding: [disj] var1, var2, ... : domain
    /// Multiple bindings: `x: S, y: T` (comma after domain, before next var list)
    fn parse_quant_bindings(&mut self) -> Result<Vec<QuantBinding>, ParseError> {
        let mut bindings = Vec::new();

        loop {
            // Check for optional `disj` keyword
            let disj = if self.peek() == Token::Disj {
                self.next();
                true
            } else {
                false
            };

            // Parse first variable name
            let first_var = self.expect_ident()?;
            let mut vars = vec![first_var];

            // Parse additional comma-separated variables before the colon
            // We need to distinguish `x, y: S` (multi-var) from `x: S, y: T` (multi-binding)
            // Strategy: collect identifiers separated by commas until we see a colon
            while self.peek() == Token::Comma {
                // Save position to backtrack if this comma separates bindings, not vars
                let saved_pos = self.lexer.pos();
                self.next(); // consume comma

                // Check if next is `disj` or an ident followed by `:` or `,`
                // If next token is Pipe, it's the end — backtrack
                let next = self.peek();
                match &next {
                    Token::Disj => {
                        // This comma separates bindings: `x: S, disj y, z: T`
                        // But we haven't parsed the colon+domain yet for current binding
                        // So backtrack the comma, break, parse colon+domain
                        self.lexer.set_pos(saved_pos);
                        break;
                    }
                    Token::Ident(_) => {
                        // Could be `y: T` (new binding) or `y, z` (more vars)
                        // Peek ahead: if ident is followed by Colon, it could be either.
                        // If ident followed by Comma or Colon, need further lookahead.
                        // Save position after comma, read the ident, check what follows
                        let saved_after_comma = self.lexer.pos();
                        let ident = self.expect_ident()?;
                        let after_ident = self.peek();
                        match after_ident {
                            Token::Comma => {
                                // `ident,` — this is another var in the same binding
                                vars.push(ident);
                            }
                            Token::Colon => {
                                // `ident:` — could be: more vars then colon (if this is the last var),
                                // OR a new binding. We need to check: did we already have a colon
                                // for the current binding? No — we haven't parsed one yet.
                                // So this is actually `var1, var2: Domain` pattern.
                                vars.push(ident);
                            }
                            Token::Pipe => {
                                // `ident|` — this ident is a var, no colon yet
                                // This would be a parse error (missing colon)
                                vars.push(ident);
                            }
                            _ => {
                                // Unexpected — could be start of a new binding after domain
                                // Backtrack: restore to saved_pos (before comma)
                                self.lexer.set_pos(saved_after_comma);
                                // Put ident back — actually we can't easily do that.
                                // Let's use a different approach: backtrack fully
                                self.lexer.set_pos(saved_pos);
                                break;
                            }
                        }
                    }
                    _ => {
                        // Not an ident after comma — backtrack
                        self.lexer.set_pos(saved_pos);
                        break;
                    }
                }
            }

            // Now parse `: domain`
            self.expect(&Token::Colon)?;
            let domain = self.parse_field_access()?;

            bindings.push(QuantBinding { vars, domain, disj });

            // Check if there are more bindings: `, ident` where ident is followed by `:` eventually
            // vs `|` which ends bindings
            if self.peek() == Token::Comma {
                // Peek further: is this `, disj ...` or `, ident ...`?
                let saved = self.lexer.pos();
                self.next(); // consume comma
                let next = self.peek();
                match next {
                    Token::Disj | Token::Ident(_) => {
                        // More bindings — continue the loop
                        // (the comma has been consumed)
                    }
                    _ => {
                        // Not a binding, backtrack
                        self.lexer.set_pos(saved);
                        break;
                    }
                }
            } else {
                break;
            }
        }

        Ok(bindings)
    }

    fn parse_field_access(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_primary()?;
        while self.peek() == Token::Dot {
            self.next();
            if self.peek() == Token::Caret {
                self.next(); // consume ^
                let field = self.expect_ident()?;
                expr = Expr::FieldAccess {
                    base: Box::new(expr),
                    field: field.clone(),
                };
                expr = Expr::TransitiveClosure(Box::new(expr));
            } else {
                let field = self.expect_ident()?;
                expr = Expr::FieldAccess {
                    base: Box::new(expr),
                    field,
                };
            }
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        match self.peek() {
            Token::Ident(_) => {
                let name = self.expect_ident()?;
                Ok(Expr::VarRef(name))
            }
            Token::Int(_) => {
                match self.next() {
                    Token::Int(n) => Ok(Expr::IntLiteral(n)),
                    _ => unreachable!(),
                }
            }
            Token::LParen => {
                self.next();
                let expr = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                Ok(expr)
            }
            _ => {
                let tok = self.next();
                Err(ParseError::InvalidSyntax {
                    message: format!("expected expression, found {tok:?}"),
                    pos: self.lexer.pos(),
                })
            }
        }
    }

    fn skip_command(&mut self) {
        self.next(); // consume check/run
        loop {
            match self.peek() {
                Token::Eof | Token::Sig | Token::Abstract | Token::One
                | Token::Some_ | Token::Lone
                | Token::Fact | Token::Pred | Token::Fun | Token::Assert
                | Token::Check | Token::Run => break,
                _ => { self.next(); }
            }
        }
    }
}

pub fn parse(input: &str) -> Result<AlloyModel, ParseError> {
    let mut parser = Parser::new(input);
    parser.parse_model()
}
