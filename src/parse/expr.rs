use crate::ast::{BinOpKind, BinaryOp, Expr, ExprId, Lit, UnaryOp};

use super::{
    Stream, parse_atom_with,
    token::{Token, TokenKind},
};
use miette::Result;

pub fn parse_expr_inner(
    stream: &mut Stream,
    precedence: u8,
    allow_struct_init: bool,
) -> Result<ExprId> {
    const OPS: &[&[BinOpKind]] = &[
        &[
            BinOpKind::Assign,
            BinOpKind::AddAssign,
            BinOpKind::SubAssign,
            BinOpKind::MulAssign,
            BinOpKind::DivAssign,
            BinOpKind::ModAssign,
        ],
        // &[BinOpKind::Or],
        // &[BinOpKind::And],
        &[
            BinOpKind::Eq,
            BinOpKind::Neq,
            BinOpKind::Greater,
            BinOpKind::Less,
            BinOpKind::GreaterEq,
            BinOpKind::LessEq,
        ],
        &[BinOpKind::Range, BinOpKind::RangeInclusive],
        &[BinOpKind::Add, BinOpKind::Sub],
        &[BinOpKind::Mul, BinOpKind::Div, BinOpKind::Mod],
    ];

    let Some(&ops) = OPS.get(precedence as usize) else {
        return parse_leaf_expr(stream, allow_struct_init);
    };
    let mut root = parse_expr_inner(stream, precedence + 1, allow_struct_init)?;
    loop {
        let Some(token) = stream.lexer.clone().next().transpose()? else { break };
        let Ok(op) = BinaryOp::try_from(token) else { break };
        if !ops.contains(&op.kind) {
            break;
        }
        _ = stream.next();
        let expr = parse_expr_inner(stream, precedence + 1, allow_struct_init)?;
        root = stream.ast.exprs.push(Expr::Binary { lhs: root, op, rhs: expr });
    }
    Ok(root)
}

fn parse_leaf_expr(stream: &mut Stream, allow_struct_init: bool) -> Result<ExprId> {
    let mut expr = parse_unary_expr(stream, allow_struct_init)?;

    loop {
        let Some(token) = stream.lexer.clone().next().transpose()? else { break };
        match token.kind {
            TokenKind::LParen => {
                _ = stream.next();
                let args = stream.parse_separated(TokenKind::Comma, TokenKind::RParen)?;
                expr = stream.ast.exprs.push(Expr::FnCall { function: expr, args });
            }
            TokenKind::Dot => 'block: {
                _ = stream.next();
                let field = stream.expect_ident()?;
                if stream.peek()?.kind == TokenKind::LParen {
                    _ = stream.next();
                    expr = stream.ast.exprs.push(Expr::FieldAccess { expr, field });
                    break 'block;
                }
                let args = stream.parse_separated(TokenKind::Comma, TokenKind::RParen)?;
                expr = stream.ast.exprs.push(Expr::MethodCall { expr, method: field, args });
            }
            TokenKind::LBracket => {
                _ = stream.next();
                let index = stream.parse()?;
                stream.expect(TokenKind::RBracket)?;
                expr = stream.ast.exprs.push(Expr::Index { expr, index });
            }
            _ => break,
        }
    }
    if !allow_struct_init {
        return Ok(expr);
    }
    let Expr::Ident(ident) = stream.ast.exprs[expr] else {
        return Ok(expr);
    };
    let TokenKind::LBrace = stream.peek()?.kind else { return Ok(expr) };
    _ = stream.next();
    let args = stream.parse_separated(TokenKind::Comma, TokenKind::RBrace)?;
    Ok(stream.ast.exprs.push(Expr::StructInit { ident, args }))
}

fn parse_unary_expr(stream: &mut Stream, allow_struct_init: bool) -> Result<ExprId> {
    _ = allow_struct_init;
    let token = stream.next()?;
    let expr = match token.kind {
        kind @ (TokenKind::Minus | TokenKind::Not) => {
            let op = if kind == TokenKind::Minus { UnaryOp::Neg } else { UnaryOp::Not };
            let next = stream.next()?;
            Expr::Unary { op, expr: parse_paren_expr(stream, next)? }
        }
        TokenKind::LBracket => Expr::Lit(Lit::Array {
            segments: stream.parse_separated(TokenKind::Comma, TokenKind::RBracket)?,
        }),
        _ => return parse_paren_expr(stream, token),
    };
    Ok(stream.ast.exprs.push(expr))
}

fn parse_paren_expr(stream: &mut Stream, token: Token) -> Result<ExprId> {
    if token.kind == TokenKind::LParen {
        if stream.peek()?.kind == TokenKind::RParen {
            _ = stream.next();
            return Ok(stream.ast.exprs.push(Expr::Lit(Lit::Unit)));
        }
        let expr = stream.parse()?;
        stream.expect(TokenKind::RParen)?;
        return Ok(expr);
    }
    parse_atom_with(stream, token)
}
