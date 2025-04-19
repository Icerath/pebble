mod expr;
mod lex;
mod token;

use std::path::Path;

use lex::Lexer;
use miette::{Error, Result};
use thin_vec::{ThinVec, thin_vec};
use token::{Token, TokenKind};

use crate::{
    ast::{
        ArraySeg, Ast, BinOpKind, BinaryOp, Block, BlockId, Expr, ExprId, ExprKind, FnDecl, IfStmt,
        Impl, Lit, Param, Trait, Ty, TyKind, TypeId,
    },
    errors,
    span::Span,
    symbol::Symbol,
};

pub fn parse(src: &str, path: Option<&Path>) -> Result<Ast> {
    let lexer = Lexer::new(src);
    let mut ast = Ast::default();
    let mut stream = Stream { lexer, ast: &mut ast, path };
    let mut top_level = vec![];
    while let Some(next) = stream.lexer.clone().next() {
        if next?.kind == TokenKind::Semicolon {
            _ = stream.lexer.next();
            continue;
        }
        top_level.push(stream.parse()?);
    }
    ast.top_level = top_level;
    Ok(ast)
}

struct Stream<'src, 'path> {
    lexer: Lexer<'src>,
    ast: &'src mut Ast,
    path: Option<&'path Path>,
}

impl Stream<'_, '_> {
    fn next(&mut self) -> Result<Token> {
        if let Some(result) = self.lexer.next() {
            return result;
        }
        Err(self.handle_eof())
    }
    fn clone(&mut self) -> Stream {
        Stream { lexer: self.lexer.clone(), ast: self.ast, path: self.path }
    }
    fn peek(&mut self) -> Result<Token> {
        self.clone().next()
    }
    #[inline(never)]
    #[cold]
    fn handle_eof(&self) -> miette::Error {
        errors::error(
            "unexpected EOF",
            self.path,
            self.lexer.src(),
            [(self.lexer.span_eof(), "EOF")],
        )
    }
    fn expect(&mut self, kind: TokenKind) -> Result<Token> {
        let token = self.next()?;
        if token.kind != kind {
            return Err(errors::error(
                &format!("expected `{}`, found: `{}`", kind.repr(), token.kind.repr()),
                self.path,
                self.lexer.src(),
                [(self.lexer.span(), "here")],
            ));
        }
        Ok(token)
    }
    fn any(&mut self, toks: &[TokenKind]) -> Result<Token> {
        let token = self.next()?;
        if toks.contains(&token.kind) {
            return Ok(token);
        }
        Err(self.any_failed(token, toks))
    }
    #[inline(never)]
    #[cold]
    fn any_failed(&self, found: Token, toks: &[TokenKind]) -> Error {
        errors::error(
            &format!(
                "expected one of {}, found `{}`",
                toks.iter()
                    .map(|kind| format!("`{}`", kind.repr()))
                    .collect::<Vec<_>>()
                    .join(" or "),
                found.kind.repr()
            ),
            self.path,
            self.lexer.src(),
            [(self.lexer.span(), "here")],
        )
    }

    fn expect_ident(&mut self) -> Result<Symbol> {
        let token = self.expect(TokenKind::Ident)?;
        Ok(Symbol::from(&self.lexer.src()[token.span]))
    }

    fn ident_spanned(&mut self) -> Result<(Symbol, Span)> {
        let token = self.expect(TokenKind::Ident)?;
        Ok((Symbol::from(&self.lexer.src()[token.span]), token.span))
    }

    fn parse<T: Parse>(&mut self) -> Result<T> {
        T::parse(self)
    }
    fn parse_separated<T: Parse>(&mut self, sep: TokenKind, term: TokenKind) -> Result<ThinVec<T>> {
        let mut args = thin_vec![];
        loop {
            if self.peek()?.kind == term {
                _ = self.next();
                break;
            }
            let expr = self.parse()?;
            args.push(expr);
            match self.next()? {
                tok if tok.kind == term => break,
                tok if tok.kind == sep => {}
                found => return Err(self.any_failed(found, &[sep, term])),
            }
        }
        Ok(args)
    }
}

trait Parse: Sized {
    fn parse(stream: &mut Stream) -> Result<Self>;
}

impl Parse for Symbol {
    fn parse(stream: &mut Stream) -> Result<Self> {
        stream.expect_ident()
    }
}

impl Parse for Block {
    fn parse(stream: &mut Stream) -> Result<Self> {
        let start = stream.lexer.current_pos() - 1; // ugly hack to include lbrace in span.
        let mut stmts = thin_vec![];
        let mut is_expr = false;

        loop {
            match stream.peek()?.kind {
                TokenKind::RBrace => {
                    _ = stream.next();
                    break;
                }
                TokenKind::Semicolon => {
                    is_expr = false;
                    _ = stream.next();
                }
                _ => {
                    is_expr = true;
                    stmts.push(stream.parse()?);
                }
            }
        }

        let span = Span::from(start..stream.lexer.current_pos());
        Ok(Self { stmts, is_expr, span })
    }
}

impl Parse for BlockId {
    fn parse(stream: &mut Stream) -> Result<Self> {
        Block::parse(stream).map(|block| stream.ast.blocks.push(block))
    }
}

impl Parse for TypeId {
    fn parse(stream: &mut Stream) -> Result<Self> {
        Ty::parse(stream).map(|block| stream.ast.types.push(block))
    }
}

impl Parse for Ty {
    fn parse(stream: &mut Stream) -> Result<Self> {
        let any = stream.any(&[
            TokenKind::Fn,
            TokenKind::Ident,
            TokenKind::LBracket,
            TokenKind::LParen,
            TokenKind::Not,
            TokenKind::Ampersand,
        ])?;
        let start = any.span.start();
        let kind = match any.kind {
            TokenKind::Fn => {
                stream.expect(TokenKind::LParen)?;
                let params = stream.parse_separated(TokenKind::Comma, TokenKind::RParen)?;
                let ret = if stream.peek()?.kind == TokenKind::ThinArrow {
                    _ = stream.next();
                    Some(stream.parse()?)
                } else {
                    None
                };
                TyKind::Func { params, ret }
            }
            TokenKind::Not => TyKind::Never,
            TokenKind::Ident => TyKind::Name(Symbol::from(&stream.lexer.src()[any.span])),
            TokenKind::LBracket => {
                let of = stream.parse()?;
                stream.expect(TokenKind::RBracket)?;
                TyKind::Array(of)
            }
            TokenKind::LParen => {
                stream.expect(TokenKind::RParen)?;
                TyKind::Unit
            }
            TokenKind::Ampersand => TyKind::Ref(stream.parse()?),
            _ => unreachable!(),
        };
        let end = stream.lexer.current_pos();
        Ok(Ty { kind, span: Span::from(start..end) })
    }
}

impl Parse for Impl {
    fn parse(stream: &mut Stream) -> Result<Self> {
        let trait_ = stream.expect_ident()?;
        stream.expect(TokenKind::For)?;
        let ty = stream.parse()?;
        stream.expect(TokenKind::LBrace)?;
        let methods = parse_trait_methods(stream)?;
        Ok(Self { trait_, ty, methods })
    }
}

impl Parse for Trait {
    fn parse(stream: &mut Stream) -> Result<Self> {
        let ident = stream.expect_ident()?;
        stream.expect(TokenKind::LBrace)?;
        let methods = parse_trait_methods(stream)?;
        Ok(Self { ident, methods })
    }
}

fn parse_trait_methods(stream: &mut Stream) -> Result<ThinVec<FnDecl>> {
    let mut methods = ThinVec::<FnDecl>::new();

    loop {
        let next = stream.any(&[TokenKind::Fn, TokenKind::RBrace])?;
        match next.kind {
            TokenKind::Fn => methods.push(stream.parse()?),
            TokenKind::RBrace => break Ok(methods),
            _ => unreachable!(),
        }
    }
}

impl Parse for FnDecl {
    fn parse(stream: &mut Stream) -> Result<Self> {
        let ident = stream.expect_ident()?;
        let peek = stream.clone().any(&[TokenKind::Less, TokenKind::LParen])?;
        let mut generics = ThinVec::new();
        if peek.kind == TokenKind::Less {
            _ = stream.next();
            generics = stream.parse_separated(TokenKind::Comma, TokenKind::Greater)?;
        }

        stream.expect(TokenKind::LParen)?;
        let params = stream.parse_separated(TokenKind::Comma, TokenKind::RParen)?;

        let mut chosen =
            stream.any(&[TokenKind::LBrace, TokenKind::ThinArrow, TokenKind::Semicolon])?;
        let mut ret = None;
        if chosen.kind == TokenKind::ThinArrow {
            ret = Some(stream.parse()?);
            chosen = stream.any(&[TokenKind::Semicolon, TokenKind::LBrace])?;
        }
        let block = if chosen.kind == TokenKind::Semicolon { None } else { Some(stream.parse()?) };
        Ok(Self { ident, generics, params, ret, block })
    }
}

fn parse_struct(stream: &mut Stream) -> Result<Expr> {
    let (ident, span) = stream.ident_spanned()?;
    stream.expect(TokenKind::LParen)?;
    let fields = stream.parse_separated(TokenKind::Comma, TokenKind::RParen)?;

    Ok((ExprKind::Struct { ident, fields, span }).todo_span())
}

fn parse_let(stream: &mut Stream, let_tok: Token) -> Result<Expr> {
    let ident = stream.expect_ident()?;
    let tok = stream.any(&[TokenKind::Colon, TokenKind::Eq])?;
    let mut ty = None;
    if tok.kind == TokenKind::Colon {
        ty = Some(stream.parse()?);
        stream.expect(TokenKind::Eq)?;
    }
    let expr = stream.parse()?;
    let span = Span::new(
        let_tok.span.start() as _..stream.lexer.current_pos() as _,
        let_tok.span.source(),
    );
    Ok((ExprKind::Let { ident, ty, expr }).with_span(span))
}

fn parse_while(stream: &mut Stream) -> Result<Expr> {
    let condition = stream.parse()?;
    stream.expect(TokenKind::LBrace)?;
    let block = stream.parse()?;
    Ok((ExprKind::While { condition, block }).todo_span())
}

fn parse_for(stream: &mut Stream) -> Result<Expr> {
    let ident = stream.expect_ident()?;
    stream.expect(TokenKind::In)?;
    let iter = stream.parse()?;
    stream.expect(TokenKind::LBrace)?;
    let body = stream.parse()?;
    Ok((ExprKind::For { ident, iter, body }).todo_span())
}

fn parse_ifchain(stream: &mut Stream, if_tok: Token) -> Result<Expr> {
    let mut arms = thin_vec![];
    let els = loop {
        let condition = stream.parse()?;
        stream.expect(TokenKind::LBrace)?;
        let body = stream.parse()?;
        arms.push(IfStmt { condition, body });
        if stream.peek()?.kind != TokenKind::Else {
            break None;
        }
        _ = stream.next();
        if stream.peek()?.kind == TokenKind::If {
            _ = stream.next();
        } else {
            stream.expect(TokenKind::LBrace)?;
            break Some(stream.parse()?);
        }
    };
    let end = stream.lexer.current_pos() as usize;
    let span = Span::new(if_tok.span.start() as usize..end, if_tok.span.source());
    Ok((ExprKind::If { arms, els }).with_span(span))
}

impl Parse for ArraySeg {
    fn parse(stream: &mut Stream) -> Result<Self> {
        let expr = stream.parse()?;
        let repeated = if stream.peek()?.kind == TokenKind::Semicolon {
            _ = stream.next();
            Some(stream.parse()?)
        } else {
            None
        };
        Ok(Self { expr, repeated })
    }
}

impl Parse for Param {
    fn parse(stream: &mut Stream) -> Result<Self> {
        let ident = stream.expect_ident()?;
        stream.expect(TokenKind::Colon)?;
        let ty = stream.parse()?;
        Ok(Self { ident, ty })
    }
}

impl TryFrom<Token> for BinaryOp {
    type Error = ();
    fn try_from(token: Token) -> Result<Self, Self::Error> {
        let kind = BinOpKind::try_from(token.kind)?;
        Ok(Self { kind, span: token.span })
    }
}

impl TryFrom<TokenKind> for BinOpKind {
    type Error = ();
    fn try_from(kind: TokenKind) -> Result<Self, Self::Error> {
        Ok(match kind {
            TokenKind::Eq => Self::Assign,
            TokenKind::PlusEq => Self::AddAssign,
            TokenKind::MinusEq => Self::SubAssign,
            TokenKind::MulEq => Self::MulAssign,
            TokenKind::DivEq => Self::DivAssign,
            TokenKind::ModEq => Self::ModAssign,

            TokenKind::Plus => Self::Add,
            TokenKind::Minus => Self::Sub,
            TokenKind::Star => Self::Mul,
            TokenKind::Slash => Self::Div,
            TokenKind::Percent => Self::Mod,

            TokenKind::EqEq => Self::Eq,
            TokenKind::Neq => Self::Neq,
            TokenKind::Greater => Self::Greater,
            TokenKind::Less => Self::Less,
            TokenKind::GreaterEq => Self::GreaterEq,
            TokenKind::LessEq => Self::LessEq,

            TokenKind::DotDot => Self::Range,
            TokenKind::DotDotEq => Self::RangeInclusive,

            TokenKind::And => Self::And,
            TokenKind::Or => Self::Or,
            _ => return Err(()),
        })
    }
}

fn parse_atom_with(stream: &mut Stream, tok: Token) -> Result<ExprId> {
    macro_rules! lit {
        ($lit: expr, $span: expr) => {
            Ok(ExprKind::Lit($lit).with_span($span))
        };
        ($lit: expr) => {
            Ok(ExprKind::Lit($lit).with_span(tok.span))
        };
    }
    macro_rules! all {
        () => {
            Span::from(tok.span.start()..stream.lexer.current_pos())
        };
    }

    let expr = match tok.kind {
        TokenKind::Unreachable => Ok(ExprKind::Unreachable.with_span(tok.span)),
        TokenKind::LBrace => Ok(ExprKind::Block(stream.parse()?).with_span(all!())),
        TokenKind::Break => Ok(ExprKind::Break.todo_span()),
        TokenKind::Assert => {
            let expr: ExprId = stream.parse()?;
            Ok(ExprKind::Assert(expr).with_span(stream.ast.exprs[expr].span))
        }
        TokenKind::Return => {
            if (stream.lexer.clone().next().transpose()?).is_none_or(|tok| tok.kind.is_terminator())
            {
                Ok(ExprKind::Return(None).with_span(tok.span))
            } else {
                let expr = stream.parse()?;
                let span = tok.span.start()..((&stream.ast.exprs[expr] as &Expr).span.end());
                Ok(ExprKind::Return(Some(expr)).with_span(span))
            }
        }
        TokenKind::Impl => Ok(ExprKind::Impl(stream.parse()?).todo_span()),
        TokenKind::Trait => Ok(ExprKind::Trait(stream.parse()?).todo_span()),
        TokenKind::Fn => Ok(ExprKind::FnDecl(stream.parse()?).todo_span()),
        TokenKind::Struct => parse_struct(stream),
        TokenKind::Let => parse_let(stream, tok),
        TokenKind::While => parse_while(stream),
        TokenKind::For => parse_for(stream),
        TokenKind::If => parse_ifchain(stream, tok),
        TokenKind::True => lit!(Lit::Bool(true)),
        TokenKind::False => lit!(Lit::Bool(false)),
        TokenKind::Int => lit!(Lit::Int(stream.lexer.src()[tok.span].parse::<i64>().unwrap())),
        TokenKind::Str => parse_string(stream, tok.span),
        TokenKind::Char => {
            // TODO: Escaping
            let str = &stream.lexer.src()[tok.span.shrink(1)];
            lit!(Lit::Char(str.chars().next().unwrap()))
        }
        TokenKind::Ident => {
            Ok(ExprKind::Ident(stream.lexer.src()[tok.span].into()).with_span(tok.span))
        }
        found => {
            return Err(errors::error(
                &format!("expected `expression`, found {found:?}"),
                stream.path,
                stream.lexer.src(),
                [(stream.lexer.span(), "here")],
            ));
        }
    };
    Ok(stream.ast.exprs.push(expr?))
}

fn parse_string(stream: &mut Stream, outer_span: Span) -> Result<Expr> {
    // FIXME: Bring a cross.
    let span = outer_span.shrink(1); // remove double quotes.
    let raw = &stream.lexer.src()[span];
    let lexer_offset = stream.lexer.offset();
    stream.lexer.set_offset(span.start() as usize);
    let mut current_start = span.start() as usize;
    let mut current = String::new();
    let mut segments = thin_vec![];

    let mut chars = raw.char_indices();

    let mut escaped = false;
    while let Some((_, char)) = chars.next() {
        match char {
            '$' if !escaped && chars.clone().next().is_some_and(|c| c.1 == '{') => {
                let char_pos = chars.next().unwrap().0 + span.start() as usize;
                if !current.is_empty() {
                    let current_span = Span::from(current_start..char_pos);
                    let expr =
                        ExprKind::Lit(Lit::Str(current.as_str().into())).with_span(current_span);
                    segments.push(stream.ast.exprs.push(expr));
                    current.clear();
                }

                stream.lexer.bump(char_pos - current_start + 1);
                let offset = stream.lexer.offset();
                segments.push(stream.parse()?);
                let diff = stream.lexer.offset() - offset;

                chars = chars.as_str()[diff..].char_indices();
                let next = chars.next().unwrap();
                assert_eq!(next.1, '}');
                current_start = next.0 + span.start() as usize;
            }
            '/' if !escaped => escaped = true,
            _ if escaped => panic!(),
            _ => {
                escaped = false;
                current.push(char);
            }
        }
    }
    if segments.is_empty() {
        stream.lexer.set_offset(lexer_offset);
        return Ok(ExprKind::Lit(Lit::Str(current.into())).with_span(outer_span));
    }
    if !current.is_empty() {
        let current_span = Span::from(current_start..(current_start + raw.len()));
        let expr = ExprKind::Lit(Lit::Str(current.into())).with_span(current_span);
        segments.push(stream.ast.exprs.push(expr));
    }
    stream.lexer.set_offset(lexer_offset);
    Ok(ExprKind::Lit(Lit::FStr(segments)).with_span(outer_span))
}
