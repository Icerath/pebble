mod array;
mod value;

use std::io::{self, Write};

use array::Array;
use index_vec::{IndexSlice, IndexVec};
use value::{Allocation, Value};

use crate::mir::{
    BinaryOp, BlockId, BodyId, Constant, Local, Mir, Operand, Place, Projection, RValue, Statement,
    Terminator, UnaryOp,
};

type Places = IndexSlice<Local, [Allocation]>;

pub fn interpret(mir: &Mir) {
    let Some(main) = mir.main_body else { return };
    let mut interpreter = Interpreter { mir };
    interpreter.run(main, vec![]);
}

struct Interpreter<'mir> {
    mir: &'mir Mir,
}

impl Interpreter<'_> {
    fn run(&mut self, body_id: BodyId, args: Vec<Value>) -> Value {
        let body = &self.mir.bodies[body_id];
        let mut block_id = BlockId::from(0);
        let mut locals: IndexVec<Local, Allocation> =
            std::iter::repeat_with(|| Allocation::from(Value::Unit))
                .take(body.locals.index())
                .collect();
        for (i, arg) in args.into_iter().enumerate() {
            locals[i] = arg.into();
        }
        loop {
            let block = &body.blocks[block_id];
            for stmt in &block.statements {
                let Statement::Assign { place, rvalue } = stmt;
                let rvalue = self.rvalue(rvalue, &locals);
                let alloc = self.load_place(place, &locals);
                *alloc.borrow() = rvalue;
            }
            match block.terminator {
                #[cfg(test)]
                Terminator::Abort => std::panic::panic_any("assertion failed"),
                #[cfg(not(test))]
                Terminator::Abort => std::process::exit(1),
                Terminator::Goto(block) => block_id = block,
                Terminator::Branch { ref condition, fals, tru } => {
                    let condition = self.operand(condition, &locals).unwrap_bool();
                    block_id = if condition { tru } else { fals };
                }
                Terminator::Return(ref operand) => return self.operand(operand, &locals),
            }
        }
    }
    #[allow(clippy::too_many_lines)]
    fn rvalue(&mut self, rvalue: &RValue, locals: &Places) -> Value {
        match rvalue {
            RValue::Use(operand) => self.operand(operand, locals),
            RValue::Extend { array, value, repeat } => {
                let value = self.operand(value, locals);
                let repeat: usize = self.operand(repeat, locals).unwrap_int().try_into().unwrap();
                locals[*array].borrow().unwrap_array().extend(value, repeat);
                Value::Unit
            }
            RValue::Call { function, args } => {
                let call_body = self.operand(function, locals).unwrap_fn();
                let args = args.iter().map(|arg| self.operand(arg, locals)).collect();
                self.run(call_body, args)
            }
            RValue::BinaryExpr { lhs, op, rhs } => {
                let lhs = self.operand(lhs, locals);
                let rhs = self.operand(rhs, locals);
                match op {
                    BinaryOp::IntAdd => Value::Int(lhs.unwrap_int() + rhs.unwrap_int()),
                    BinaryOp::IntSub => Value::Int(lhs.unwrap_int() - rhs.unwrap_int()),
                    BinaryOp::IntMul => Value::Int(lhs.unwrap_int() * rhs.unwrap_int()),
                    BinaryOp::IntDiv => Value::Int(lhs.unwrap_int() / rhs.unwrap_int()),
                    BinaryOp::IntMod => Value::Int(lhs.unwrap_int() % rhs.unwrap_int()),
                    BinaryOp::IntLess => Value::Bool(lhs.unwrap_int() < rhs.unwrap_int()),
                    BinaryOp::IntGreater => Value::Bool(lhs.unwrap_int() > rhs.unwrap_int()),
                    BinaryOp::IntLessEq => Value::Bool(lhs.unwrap_int() <= rhs.unwrap_int()),
                    BinaryOp::IntGreaterEq => Value::Bool(lhs.unwrap_int() >= rhs.unwrap_int()),
                    BinaryOp::IntEq => Value::Bool(lhs.unwrap_int() == rhs.unwrap_int()),
                    BinaryOp::IntNeq => Value::Bool(lhs.unwrap_int() != rhs.unwrap_int()),
                    BinaryOp::IntRange => {
                        Value::Range(Box::new(lhs.unwrap_int()..rhs.unwrap_int()))
                    }
                    BinaryOp::IntRangeInclusive =>
                    {
                        #[expect(clippy::range_plus_one)]
                        Value::Range(Box::new(lhs.unwrap_int()..rhs.unwrap_int() + 1))
                    }

                    BinaryOp::CharEq => Value::Bool(lhs.unwrap_char() == rhs.unwrap_char()),
                    BinaryOp::CharNeq => Value::Bool(lhs.unwrap_char() != rhs.unwrap_char()),

                    BinaryOp::StrEq => Value::Bool(lhs.unwrap_str() == rhs.unwrap_str()),
                    BinaryOp::StrNeq => Value::Bool(lhs.unwrap_str() != rhs.unwrap_str()),
                    BinaryOp::StrIndex => {
                        Value::Char(lhs.unwrap_str().as_bytes()[rhs.unwrap_int_usize()] as char)
                    }
                    BinaryOp::StrIndexSlice => {
                        Value::Str(lhs.unwrap_str()[rhs.unwrap_range_usize()].into())
                    }
                    BinaryOp::StrFind => Value::Int(
                        lhs.unwrap_str()
                            .find(rhs.unwrap_str().as_str())
                            .unwrap()
                            .try_into()
                            .unwrap(),
                    ),
                    BinaryOp::StrRFind => Value::Int(
                        lhs.unwrap_str()
                            .rfind(rhs.unwrap_str().as_str())
                            .unwrap()
                            .try_into()
                            .unwrap(),
                    ),
                    BinaryOp::ArrayIndexRange => todo!(),
                }
            }
            RValue::UnaryExpr { op, operand } => {
                let operand = self.operand(operand, locals);
                match op {
                    UnaryOp::Deref => operand.unwrap_ref().clone_raw(),
                    UnaryOp::BoolNot => Value::Bool(!operand.unwrap_bool()),

                    UnaryOp::IntNeg => Value::Int(-operand.unwrap_int()),
                    UnaryOp::IntToStr => Value::Str(operand.unwrap_int().to_string().into()),
                    UnaryOp::Chr => {
                        Value::Char(u8::try_from(operand.unwrap_int()).unwrap() as char)
                    }

                    UnaryOp::PrintChar => {
                        let mut stdout = io::stdout().lock();
                        _ = write!(stdout, "{}", operand.unwrap_char());
                        _ = stdout.flush();
                        Value::Unit
                    }

                    UnaryOp::StrPrint => {
                        println!("{}", operand.unwrap_str());
                        Value::Unit
                    }
                    UnaryOp::StrLen => Value::Int(operand.unwrap_str().len().try_into().unwrap()),
                }
            }
        }
    }

    fn operand(&mut self, operand: &Operand, locals: &Places) -> Value {
        match *operand {
            Operand::Ref(ref place) => Value::Ref(self.load_place(place, locals)),
            Operand::Constant(ref constant) => match *constant {
                Constant::Unit => Value::Unit,
                Constant::EmptyArray => Value::Array(Array::default()),
                Constant::Bool(bool) => Value::Bool(bool),
                Constant::Int(int) => Value::Int(int),
                Constant::Char(char) => Value::Char(char),
                Constant::Str(str) => Value::Str(str.as_str().into()),
                Constant::Func(body) => Value::Fn(body),
                Constant::StructInit => Value::Struct(locals.iter().cloned().collect()),
            },
            Operand::Place(ref place) => self.load_place(place, locals).clone_raw(),
            Operand::Unreachable => unreachable!(),
        }
    }

    #[expect(clippy::unused_self)]
    fn load_place(&mut self, place: &Place, locals: &Places) -> Allocation {
        let mut alloc = locals[place.local].clone();
        for projection in &place.projections {
            alloc = match *projection {
                Projection::Deref => alloc.borrow().unwrap_ref().clone(),
                Projection::Field(field) => alloc.borrow().unwrap_struct()[field as usize].clone(),
                Projection::Index(index) => {
                    let index = locals[index].borrow().unwrap_int_usize();
                    alloc.borrow().unwrap_array().get(index).unwrap().clone()
                }
            };
        }
        alloc
    }
}
