use std::fmt;

use super::{Constant, Mir, Operand, RValue, Statement, Terminator, UnaryOp};

impl fmt::Display for Mir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (id, body) in self.bodies.iter_enumerated() {
            writeln!(f, "fn _{id:?}() {{")?;
            for (id, block) in body.blocks.iter_enumerated() {
                writeln!(f, "{}bb{id:?}: {{", Indent(1))?;
                for statement in &block.statements {
                    write!(f, "{}", Indent(2))?;
                    match statement {
                        Statement::Assign { place, rvalue } => {
                            write!(f, "_{place:?} = ")?;
                            match rvalue {
                                RValue::Abort => write!(f, "abort"),
                                RValue::BinaryExpr { lhs, op, rhs } => {
                                    write!(f, "{op:?}({lhs}, {rhs})")
                                }

                                RValue::Use(arg) => write!(f, "{arg}"),
                                RValue::Call { function, args } => {
                                    write!(f, "call {function}")?;
                                    write!(f, "(")?;
                                    for (i, arg) in args.iter().enumerate() {
                                        write!(f, "{}{arg}", if i == 0 { "" } else { ", " })?;
                                    }
                                    write!(f, ")")
                                }
                                RValue::Index { indexee, index } => {
                                    write!(f, "Index({indexee}, {index})")
                                }
                                RValue::UnaryExpr { op, operand } => match op {
                                    UnaryOp::Neg => write!(f, "neg {operand}"),
                                    UnaryOp::Not => write!(f, "not {operand}"),
                                },
                            }?;
                        }
                    }
                    writeln!(f)?;
                }
                write!(f, "{}", Indent(2))?;
                match &block.terminator {
                    Terminator::Goto(to) => write!(f, "goto bb{to:?}"),
                    Terminator::Branch { condition, fals, tru } => {
                        write!(f, "branch {condition}[false: bb{fals:?}, true: bb{tru:?}]")
                    }
                    Terminator::Return(arg) => write!(f, "return {arg}"),
                }?;
                writeln!(f, "\n{}}}", Indent(1))?;
            }
            writeln!(f, "}}")?;
        }
        Ok(())
    }
}

struct Indent(u8);

impl fmt::Display for Indent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for _ in 0..self.0 {
            write!(f, "    ")?;
        }
        Ok(())
    }
}

impl fmt::Display for Operand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Place(place) => write!(f, "_{place:?}"),
            Self::Constant(constant) => write!(f, "const {constant}"),
        }
    }
}

impl fmt::Display for Constant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unit => write!(f, "()"),
            Self::Bool(bool) => write!(f, "{bool}"),
            Self::Int(int) => write!(f, "{int}"),
            Self::Char(char) => write!(f, "{char:?}"),
            Self::Str(str) => write!(f, "{str:?}"),
            Self::Func(id) => write!(f, "function {id:?}"),
        }
    }
}
