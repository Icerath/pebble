mod display;

use index_vec::IndexVec;
use thin_vec::ThinVec;

use crate::{define_id, symbol::Symbol};

define_id!(pub BodyId);
define_id!(pub BlockId = u16);
define_id!(pub Local = u16);

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Place {
    pub local: Local,
    pub projections: Vec<Projection>,
}

impl Place {
    pub fn local(local: Local) -> Self {
        Self { local, projections: vec![] }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Projection {
    Deref,
    Field(u32),
    Index(Local),
}

impl BlockId {
    pub const PLACEHOLDER: Self = Self { _raw: u16::MAX };
}

#[derive(Default, Debug)]
pub struct Mir {
    pub bodies: IndexVec<BodyId, Body>,
    pub main_body: Option<BodyId>,
    pub num_intrinsics: usize,
}

#[derive(Debug)]
pub struct Body {
    pub blocks: IndexVec<BlockId, Block>,
    pub locals: Local,
}

impl Body {
    pub fn new(num_params: usize) -> Self {
        Self { blocks: IndexVec::default(), locals: num_params.into() }
    }
    pub fn new_local(&mut self) -> Local {
        self.locals += 1;
        self.locals - 1
    }
}
#[derive(Debug)]
pub struct Block {
    pub statements: Vec<Statement>,
    pub terminator: Terminator,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Terminator {
    Goto(BlockId),
    Branch { condition: Operand, fals: BlockId, tru: BlockId },
    Return(Operand),
    Abort,
}

impl Terminator {
    pub fn mentions_place(&self, place: &Place) -> bool {
        match self {
            Self::Abort | Self::Goto(..) => false,
            Self::Branch { condition, .. } => condition.mentions_place(place),
            Self::Return(operand) => operand.mentions_place(place),
        }
    }
    pub fn with_jumps(&self, mut f: impl FnMut(BlockId)) {
        match *self {
            Self::Abort | Self::Return(..) => {}
            Self::Goto(jump) => f(jump),
            Self::Branch { fals, tru, .. } => {
                f(fals);
                f(tru);
            }
        }
    }
    pub fn with_jumps_mut(&mut self, mut f: impl FnMut(&mut BlockId)) {
        match self {
            Self::Abort | Self::Return(..) => {}
            Self::Goto(jump) => f(jump),
            Self::Branch { fals, tru, .. } => {
                f(fals);
                f(tru);
            }
        }
    }
}

#[derive(Debug)]
pub enum Statement {
    Assign { place: Place, rvalue: RValue },
}

impl Statement {
    pub fn assign(local: Local, rvalue: RValue) -> Self {
        Self::Assign { place: Place::local(local), rvalue }
    }
}

#[must_use]
#[derive(Debug)]
pub enum RValue {
    Extend { array: Local, value: Operand, repeat: Operand },
    Use(Operand),
    BinaryExpr { lhs: Operand, op: BinaryOp, rhs: Operand },
    UnaryExpr { op: UnaryOp, operand: Operand },
    Call { function: Operand, args: ThinVec<Operand> },
}

impl RValue {
    pub fn local(local: Local) -> Self {
        Self::Use(Operand::local(local))
    }
    pub fn is_unreachable(&self) -> bool {
        matches!(self, Self::Use(Operand::Unreachable))
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Operand {
    Constant(Constant),
    Ref(Place),
    Place(Place),
    Unreachable,
}

impl Operand {
    pub const UNIT: Self = Self::Constant(Constant::Unit);

    pub fn local(local: Local) -> Self {
        Self::Place(Place::local(local))
    }

    // returns an operand to read to nth argument, used in intrinsics
    pub fn arg(nth: usize) -> Self {
        Self::local(nth.into())
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Constant {
    Unit,
    EmptyArray,
    Bool(bool),
    Int(i64),
    Char(char),
    Str(Symbol),
    Func(BodyId),
    StructInit,
}

#[expect(dead_code)]
#[derive(Debug, PartialEq, PartialOrd, Clone, Copy)]
pub enum BinaryOp {
    IntAdd,
    IntSub,
    IntMul,
    IntDiv,
    IntMod,
    IntLess,
    IntGreater,
    IntLessEq,
    IntGreaterEq,
    IntEq,
    IntNeq,
    IntRange,
    IntRangeInclusive,

    CharEq,
    CharNeq,

    StrEq,
    StrNeq,
    StrFind,
    StrRFind,
    StrIndex,
    StrIndexSlice,

    ArrayIndexRange,
}

#[derive(Debug, Clone, Copy)]
pub enum UnaryOp {
    BoolNot,

    IntToStr,
    IntNeg,

    Chr,
    PrintChar,

    StrLen,
    StrPrint,

    Deref,
}

impl Statement {
    pub fn rvalue(&self) -> &RValue {
        match self {
            Self::Assign { rvalue, .. } => rvalue,
        }
    }
}

impl RValue {
    pub fn mentions_place(&self, place: &Place) -> bool {
        match self {
            Self::BinaryExpr { lhs, rhs, .. } => {
                lhs.mentions_place(place) || rhs.mentions_place(place)
            }
            Self::Call { function, args } => {
                function.mentions_place(place) || args.iter().any(|arg| arg.mentions_place(place))
            }
            Self::Use(operand) | Self::UnaryExpr { operand, .. } => operand.mentions_place(place),
            Self::Extend { array, value, repeat } => {
                Place::local(*array) == *place
                    || value.mentions_place(place)
                    || repeat.mentions_place(place)
            }
        }
    }
}

impl Operand {
    pub fn mentions_place(&self, target: &Place) -> bool {
        match self {
            Self::Place(place) => target == place,
            _ => false,
        }
    }
}
