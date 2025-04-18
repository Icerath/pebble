mod intrinsics;

use std::mem;

use arcstr::ArcStr;
use index_vec::IndexVec;

use crate::{
    HashMap,
    hir::{self, ArraySeg, ExprId, ExprKind, Hir, Lit},
    mir::{
        self, BinaryOp, Block, BlockId, Body, BodyId, Constant, Local, Mir, Operand, Place,
        Projection, RValue, Statement, Terminator, UnaryOp,
    },
    symbol::Symbol,
    ty::{StructId, Ty, TyKind},
};

pub fn lower(hir: &Hir) -> Mir {
    let mut mir = Mir::default();
    let root_body = mir.bodies.push(Body::new(None, 0).with_auto(true));
    let bodies = vec![BodyInfo::new(root_body)];

    let mut lowering = Lowering {
        hir,
        mir,
        bodies,
        struct_display_bodies: IndexVec::default(),
        strings: HashMap::default(),
    };
    for &expr in &hir.root {
        lowering.lower(expr);
    }
    // TODO: Instead produce an error for any non-body expr in the global scope (probably before type analysis?)
    assert!(lowering.mir.bodies.first().unwrap().blocks.is_empty());
    lowering.mir
}

struct Lowering<'hir, 'tcx> {
    hir: &'hir Hir<'tcx>,
    mir: Mir,
    bodies: Vec<BodyInfo>,
    struct_display_bodies: IndexVec<StructId, Option<BodyId>>,
    strings: HashMap<Symbol, ArcStr>,
}

macro_rules! str {
    ($s: literal) => {
        Constant::Str(arcstr::literal!($s)).into()
    };
    ($self:expr, $s: ident) => {
        Constant::Str($self.strings.entry($s).or_insert($s.as_str().into()).clone()).into()
    };
}

struct BodyInfo {
    body: BodyId,
    functions: HashMap<Symbol, BodyId>,
    stmts: Vec<Statement>,
    breaks: Vec<BlockId>,
    scopes: Vec<Scope>,
}

impl BodyInfo {
    pub fn scope(&mut self) -> &mut Scope {
        self.scopes.last_mut().unwrap()
    }
}

#[derive(Debug, Default)]
struct Scope {
    variables: HashMap<Symbol, Local>,
}

impl BodyInfo {
    pub fn new(body: BodyId) -> Self {
        Self {
            body,
            functions: HashMap::default(),
            scopes: vec![Scope::default()],
            stmts: vec![],
            breaks: vec![],
        }
    }
}

impl Lowering<'_, '_> {
    fn body_ref(&self) -> &Body {
        &self.mir.bodies[self.current().body]
    }
    fn body_mut(&mut self) -> &mut Body {
        let body = self.current().body;
        &mut self.mir.bodies[body]
    }

    fn current(&self) -> &BodyInfo {
        self.bodies.last().unwrap()
    }
    fn current_mut(&mut self) -> &mut BodyInfo {
        self.bodies.last_mut().unwrap()
    }

    fn finish_with(&mut self, terminator: Terminator) -> BlockId {
        let prev_block = Block { statements: mem::take(&mut self.current_mut().stmts), terminator };
        self.body_mut().blocks.push(prev_block)
    }

    fn current_block(&self) -> BlockId {
        self.body_ref().blocks.next_idx()
    }

    fn finish_next(&mut self) {
        let next_block = self.current_block() + 1;
        self.finish_with(Terminator::Goto(next_block));
    }

    fn new_local(&mut self) -> Local {
        self.body_mut().new_local()
    }

    fn lower(&mut self, id: ExprId) -> Operand {
        let rvalue = self.lower_rvalue(id);
        self.process(rvalue, self.hir.exprs[id].ty)
    }

    fn lower_local(&mut self, id: ExprId) -> Local {
        let rvalue = self.lower_rvalue(id);
        self.process_to_local(rvalue)
    }

    fn process_to_local(&mut self, rvalue: impl Into<RValue>) -> Local {
        match rvalue.into() {
            RValue::Use(Operand::Place(Place { local, projections })) if projections.is_empty() => {
                local
            }
            rvalue => self.assign_new(rvalue),
        }
    }

    fn process_to_place(&mut self, rvalue: impl Into<RValue>) -> Place {
        match rvalue.into() {
            RValue::Use(Operand::Place(place)) => place,
            rvalue => Place::local(self.assign_new(rvalue)),
        }
    }

    fn ref_of(&mut self, rvalue: RValue) -> Operand {
        match rvalue {
            RValue::Use(Operand::Place(place)) => Operand::Ref(place),
            rvalue => {
                let local = self.assign_new(rvalue);
                Operand::Ref(local.into())
            }
        }
    }

    fn process(&mut self, rvalue: RValue, ty: Ty) -> Operand {
        match rvalue {
            RValue::Use(operand) => operand,
            rvalue if ty.is_unit() => {
                let _ = self.assign_new(rvalue);
                Operand::UNIT
            }
            rvalue => Operand::Place(self.assign_new(rvalue).into()),
        }
    }

    fn assign(&mut self, place: impl Into<Place>, rvalue: impl Into<RValue>) {
        let rvalue = rvalue.into();
        let place = place.into();
        self.current_mut().stmts.push(Statement::Assign { place, rvalue });
    }

    #[must_use]
    fn assign_new(&mut self, rvalue: impl Into<RValue>) -> Local {
        let local = self.new_local();
        self.assign(local, rvalue);
        local
    }

    #[expect(clippy::too_many_lines)]
    fn lower_rvalue(&mut self, id: ExprId) -> RValue {
        let expr = &self.hir.exprs[id];
        let is_unit = expr.ty.is_unit();

        match expr.kind {
            ExprKind::Unreachable => {
                let _ = self.finish_with(Terminator::Unreachable);
                RValue::UNIT
            }
            ExprKind::Abort => {
                let _ = self.finish_with(Terminator::Abort);
                RValue::UNIT
            }
            ExprKind::Field { expr, field } => {
                let local = self.lower_local(expr);
                RValue::Use(Operand::Place(Place {
                    local,
                    projections: vec![Projection::Field(field.try_into().unwrap())],
                }))
            }
            ExprKind::StructInit => {
                let body = self.current_mut().body;
                let nparams = self.mir.bodies[body].params;
                let local =
                    self.assign_new(Constant::UninitStruct { size: nparams.try_into().unwrap() });
                for param in (0..nparams).map(Local::from) {
                    let field = Projection::Field(param.raw().into());
                    self.assign(Place { local, projections: vec![field] }, RValue::local(param));
                }
                RValue::local(local)
            }
            ExprKind::PrintStr(str) => {
                RValue::UnaryExpr { op: UnaryOp::Println, operand: str!(self, str) }
            }
            ExprKind::Literal(ref lit) => self.lit_rvalue(lit),
            ExprKind::Unary { op, expr } => 'outer: {
                if let hir::UnaryOp::Ref = op {
                    break 'outer RValue::Use(self.ref_expr(expr));
                } else if let hir::UnaryOp::Deref = op {
                    let rvalue = self.lower_rvalue(expr);
                    break 'outer RValue::Use(self.deref_operand(rvalue));
                }
                let operand = self.lower(expr);
                let op = match op {
                    hir::UnaryOp::Not => mir::UnaryOp::BoolNot,
                    hir::UnaryOp::Neg => mir::UnaryOp::IntNeg,
                    _ => unreachable!(),
                };
                RValue::UnaryExpr { op, operand }
            }
            ExprKind::FnDecl(ref decl) => {
                let hir::FnDecl { ident, ref params, ref body, .. } = **decl;

                assert!(self.current_mut().stmts.is_empty(), "TODO");
                let body_id = self.mir.bodies.push(Body::new(Some(ident), params.len()));
                self.current_mut().functions.insert(ident, body_id);
                self.bodies.push(BodyInfo::new(body_id));
                if self.bodies.len() == 2 && ident == "main" {
                    self.mir.main_body = Some(body_id);
                }

                if self.bodies.len() == 2 && self.try_instrinsic(ident) {
                    let current = self.current_mut().body;
                    self.mir.bodies[current].auto = true;
                } else {
                    for (i, param) in params.iter().enumerate() {
                        self.current_mut().scope().variables.insert(param.ident, Local::from(i));
                    }
                    let mut last = Operand::UNIT;
                    for &expr in body {
                        last = self.lower(expr);
                    }
                    self.finish_with(Terminator::Return(last));
                }
                self.bodies.pop().unwrap();
                RValue::Use(Operand::UNIT)
            }
            ExprKind::Let { ident, expr } => {
                let rvalue = self.lower_rvalue(expr);
                let local = self.assign_new(rvalue);
                self.current_mut().scope().variables.insert(ident, local);
                RValue::Use(Operand::UNIT)
            }
            ExprKind::Return(expr) => {
                let place = self.lower(expr);
                self.finish_with(Terminator::Return(place));
                RValue::UNIT
            }
            ExprKind::Loop(ref block) => {
                self.finish_next();
                let loop_block = self.current_block();

                let prev_loop = mem::take(&mut self.current_mut().breaks);
                for &expr in block {
                    self.lower(expr);
                }
                let breaks = mem::replace(&mut self.current_mut().breaks, prev_loop);

                let after_loop = self.finish_with(Terminator::Goto(loop_block)) + 1;

                for block in breaks {
                    self.body_mut().blocks[block].terminator.complete(after_loop);
                }
                RValue::Use(Operand::UNIT)
            }
            ExprKind::If { ref arms, ref els } => {
                let mut jump_to_ends = Vec::with_capacity(arms.len());
                let out_local = self.new_local();
                for arm in arms {
                    let condition = self.lower(arm.condition);
                    let to_fix = self.finish_with(Terminator::Branch {
                        condition,
                        fals: BlockId::PLACEHOLDER,
                        tru: self.current_block() + 1,
                    });
                    let block_out = self.block_expr(&arm.body);
                    if is_unit {
                        self.process(block_out, expr.ty);
                    } else {
                        self.assign(out_local, block_out);
                    }
                    jump_to_ends.push(self.finish_with(Terminator::Goto(BlockId::PLACEHOLDER)));
                    let current_block = self.current_block();
                    self.body_mut().blocks[to_fix].terminator.complete(current_block);
                }
                let els_out = self.block_expr(els);
                if is_unit {
                    self.process(els_out, expr.ty);
                } else {
                    self.assign(out_local, els_out);
                }

                self.finish_next();
                let current = self.current_block();
                for block in jump_to_ends {
                    self.body_mut().blocks[block].terminator.complete(current);
                }
                if is_unit {
                    RValue::Use(Operand::Constant(Constant::Unit))
                } else {
                    RValue::local(out_local)
                }
            }
            ExprKind::Assignment { lhs, expr } => {
                let rvalue = self.lower_rvalue(expr);
                let place = self.lower_place(lhs);
                self.assign(place, rvalue);
                RValue::Use(Operand::Constant(Constant::Unit))
            }
            ExprKind::Binary { lhs, op, rhs } => self.binary_op(lhs, op, rhs),
            ExprKind::Ident(ident) => match self.load_ident(ident) {
                RValue::Use(operand) => RValue::Use(operand),
                rvalue => rvalue,
            },
            ExprKind::FnCall { function, ref args } => {
                let function = self.lower(function);
                let args = args.iter().map(|arg| self.lower(*arg)).collect();

                match self.try_call_intrinsic(function, args) {
                    Ok(rvalue) | Err(rvalue) => rvalue,
                }
            }
            ExprKind::Break => {
                let block = self.finish_with(Terminator::Goto(BlockId::PLACEHOLDER));
                self.current_mut().breaks.push(block);
                RValue::UNIT
            }
            ExprKind::Index { expr, index } => {
                let op = if self.hir.exprs[expr].ty.is_str() {
                    if self.hir.exprs[index].ty.is_range() {
                        mir::BinaryOp::StrIndexSlice
                    } else {
                        mir::BinaryOp::StrIndex
                    }
                } else if self.hir.exprs[index].ty.is_range() {
                    mir::BinaryOp::ArrayIndexRange
                } else {
                    return self.index_array(expr, index);
                };
                let lhs = self.lower(expr);
                let rhs = self.lower(index);
                RValue::BinaryExpr { lhs, op, rhs }
            }
            ExprKind::Block(ref exprs) => self.block_expr(exprs),
        }
    }

    fn binary_op(&mut self, lhs: ExprId, op: hir::BinaryOp, rhs: ExprId) -> RValue {
        let lhs_ty = self.hir.exprs[lhs].ty;
        let rhs_ty = self.hir.exprs[rhs].ty;
        if let hir::BinaryOp::And | hir::BinaryOp::Or = op {
            return self.logical_op(op, lhs, rhs);
        }

        let lhs = self.lower_rvalue(lhs);
        let rhs = self.lower_rvalue(rhs);

        let (lhs, lhs_ty) = self.fully_deref(lhs, lhs_ty);
        let (rhs, rhs_ty) = self.fully_deref(rhs, rhs_ty);

        let op = match (lhs_ty, op) {
            (TyKind::Int, op) => match op {
                hir::BinaryOp::Add => mir::BinaryOp::IntAdd,
                hir::BinaryOp::Sub => mir::BinaryOp::IntSub,
                hir::BinaryOp::Mul => mir::BinaryOp::IntMul,
                hir::BinaryOp::Div => mir::BinaryOp::IntDiv,
                hir::BinaryOp::Mod => mir::BinaryOp::IntMod,
                hir::BinaryOp::Less => mir::BinaryOp::IntLess,
                hir::BinaryOp::Greater => mir::BinaryOp::IntGreater,
                hir::BinaryOp::LessEq => mir::BinaryOp::IntLessEq,
                hir::BinaryOp::GreaterEq => mir::BinaryOp::IntGreaterEq,
                hir::BinaryOp::Eq => mir::BinaryOp::IntEq,
                hir::BinaryOp::Neq => mir::BinaryOp::IntNeq,
                hir::BinaryOp::Range => mir::BinaryOp::IntRange,
                hir::BinaryOp::RangeInclusive => mir::BinaryOp::IntRangeInclusive,
                _ => unreachable!(),
            },
            (TyKind::Char, op) => match op {
                hir::BinaryOp::Eq => mir::BinaryOp::CharEq,
                hir::BinaryOp::Neq => mir::BinaryOp::CharNeq,
                _ => unreachable!("char - {op:?}"),
            },
            (TyKind::Str, op) => match op {
                hir::BinaryOp::Eq => mir::BinaryOp::StrEq,
                hir::BinaryOp::Neq => mir::BinaryOp::StrNeq,
                hir::BinaryOp::Add => mir::BinaryOp::StrAdd,
                _ => unreachable!("str - {op:?}"),
            },
            (ty, op) => unreachable!("{ty} - {op:?}"),
        };
        let lhs = self.process(lhs, lhs_ty);
        let rhs = self.process(rhs, rhs_ty);
        RValue::BinaryExpr { lhs, op, rhs }
    }

    fn logical_op(&mut self, op: hir::BinaryOp, lhs: ExprId, rhs: ExprId) -> RValue {
        let lhs_ty = self.hir.exprs[lhs].ty;
        let rhs_ty = self.hir.exprs[rhs].ty;

        let output = self.new_local();

        let lhs = self.lower_rvalue(lhs);
        let (lhs, _) = self.fully_deref(lhs, lhs_ty);
        self.assign(output, lhs.clone());

        let next = self.current_block() + 1;
        let condition = Operand::local(output);
        let terminator = match op {
            hir::BinaryOp::And => {
                Terminator::Branch { condition, fals: BlockId::PLACEHOLDER, tru: next }
            }
            hir::BinaryOp::Or => {
                Terminator::Branch { condition, fals: next, tru: BlockId::PLACEHOLDER }
            }
            _ => unreachable!(),
        };
        let to_fix = self.finish_with(terminator);

        let rhs = self.lower_rvalue(rhs);
        let (rhs, _) = self.fully_deref(rhs, rhs_ty);
        self.assign(output, rhs);
        self.finish_next();

        let current_block = self.current_block();

        self.body_mut().blocks[to_fix].terminator.complete(current_block);
        RValue::local(output)
    }

    fn fully_deref<'tcx>(&mut self, mut rvalue: RValue, mut ty: Ty<'tcx>) -> (RValue, Ty<'tcx>) {
        while let TyKind::Ref(of) = ty {
            rvalue = self.deref_operand(rvalue).into();
            ty = of;
        }
        (rvalue, ty)
    }

    fn index_array(&mut self, expr: ExprId, index: ExprId) -> RValue {
        let expr_ty = self.hir.exprs[expr].ty;

        let expr = self.lower_rvalue(expr);
        let expr = self.array_index_derefs(expr, expr_ty);
        let mut place = self.process_to_place(expr);
        let projection = match self.hir.exprs[index].kind {
            ExprKind::Literal(Lit::Int(int)) if u32::try_from(int).is_ok() => {
                Projection::ConstantIndex(int.try_into().unwrap())
            }
            _ => Projection::Index(self.lower_local(index)),
        };
        place.projections.push(projection);
        RValue::Use(Operand::Place(place))
    }

    fn array_index_derefs(&mut self, mut rvalue: RValue, mut ty: Ty) -> RValue {
        loop {
            match ty {
                TyKind::Array(_) => return rvalue,
                TyKind::Ref(of) => {
                    ty = of;
                    rvalue = RValue::Use(self.deref_operand(rvalue));
                }
                _ => unreachable!(),
            }
        }
    }

    fn read_ident(&self, ident: Symbol) -> Local {
        *self.current().scopes.iter().rev().find_map(|scope| scope.variables.get(&ident)).unwrap()
    }

    fn lower_place(&mut self, expr: hir::ExprId) -> Place {
        let mut projections = vec![];
        let local = self.lower_place_inner(expr, &mut projections);
        Place { local, projections }
    }

    fn lower_place_inner(&mut self, expr: hir::ExprId, proj: &mut Vec<Projection>) -> Local {
        match self.hir.exprs[expr].kind {
            ExprKind::Ident(ident) => self.read_ident(ident),
            ExprKind::Index { expr, index } => {
                let index_local = self.lower_local(index);
                let local = self.lower_place_inner(expr, proj);
                proj.push(Projection::Index(index_local));
                local
            }
            ExprKind::Unary { op: hir::UnaryOp::Deref, expr } => {
                let local = self.lower_place_inner(expr, proj);
                proj.push(Projection::Deref);
                local
            }
            ExprKind::Unary { op: hir::UnaryOp::Ref, expr } => {
                let rvalue = self.ref_expr(expr);
                self.process_to_local(rvalue)
            }
            ExprKind::Field { expr, field } => {
                let field = field.try_into().unwrap();
                let local = self.lower_place_inner(expr, proj);
                proj.push(Projection::Field(field));
                local
            }
            _ => {
                let expr = self.lower_rvalue(expr);
                self.process_to_local(expr)
            }
        }
    }

    fn ref_expr(&mut self, expr: hir::ExprId) -> Operand {
        let mut place = self.lower_place(expr);
        if place.projections.last() == Some(&Projection::Deref) {
            place.projections.pop();
            Operand::Place(place)
        } else {
            Operand::Ref(place)
        }
    }

    fn block_expr(&mut self, exprs: &[ExprId]) -> RValue {
        self.current_mut().scopes.push(Scope::default());
        let mut rvalue = None;
        for (i, &expr) in exprs.iter().enumerate() {
            if i == exprs.len() - 1 {
                rvalue = Some(self.lower_rvalue(expr));
            } else {
                self.lower(expr);
            }
        }
        self.current_mut().scopes.pop().unwrap();
        rvalue.unwrap_or(RValue::Use(Operand::UNIT))
    }

    fn load_ident(&self, ident: Symbol) -> RValue {
        if let Some(place) =
            self.current().scopes.iter().rev().find_map(|scope| scope.variables.get(&ident))
        {
            return RValue::Use(Operand::local(*place));
        }
        let Some(location) = self.bodies.iter().rev().find_map(|body| body.functions.get(&ident))
        else {
            panic!("{ident}");
        };
        RValue::Use(Operand::Constant(Constant::Func(*location)))
    }

    fn lit_rvalue(&mut self, lit: &Lit) -> RValue {
        match *lit {
            Lit::Unit => RValue::Use(Operand::Constant(Constant::Unit)),
            Lit::Bool(bool) => RValue::Use(Operand::Constant(Constant::Bool(bool))),
            Lit::Int(int) => RValue::Use(Operand::Constant(Constant::Int(int))),
            Lit::Char(char) => RValue::Use(Operand::Constant(Constant::Char(char))),
            Lit::String(str) => str!(self, str),
            Lit::Array { ref segments } => self.lower_array_lit(segments),
            Lit::FStr { ref segments } => self.lower_fstrings(segments),
        }
    }

    fn lower_array_lit(&mut self, segments: &[ArraySeg]) -> RValue {
        if segments.is_empty() {
            return RValue::Use(Operand::Constant(Constant::EmptyArray { cap: 0 }));
        }

        let mut mir_segments = Vec::with_capacity(segments.len());
        for hir::ArraySeg { expr, repeated } in segments {
            let elem = self.lower(*expr);
            let repeated = repeated.map(|repeat| self.lower(repeat));
            mir_segments.push((elem, repeated));
        }
        RValue::BuildArray(mir_segments)
    }

    fn lower_fstrings(&mut self, segments: &[ExprId]) -> RValue {
        if let [single] = *segments {
            return self.format_expr(single);
        }

        let mut mir_segments = vec![];
        for &segment in segments {
            let seg_rvalue = self.format_expr(segment);
            mir_segments.push(self.process(seg_rvalue, &TyKind::Str));
        }
        RValue::StrJoin(mir_segments)
    }

    fn format_expr(&mut self, id: ExprId) -> RValue {
        let expr = &self.hir.exprs[id];
        let rvalue = self.lower_rvalue(id);
        self.format_rvalue(rvalue, expr.ty)
    }

    fn format_rvalue(&mut self, rvalue: RValue, ty: Ty) -> RValue {
        let (rvalue, ty) = self.fully_deref(rvalue, ty);
        if ty.is_str() {
            return rvalue;
        }
        let operand = self.process(rvalue, ty);
        match ty {
            TyKind::Infer(_) | TyKind::Str => unreachable!(),
            TyKind::Never => str!("!"),
            TyKind::Unit => str!("()"),
            TyKind::Bool => RValue::UnaryExpr { op: UnaryOp::BoolToStr, operand },
            TyKind::Int => RValue::UnaryExpr { op: UnaryOp::IntToStr, operand },
            TyKind::Char => RValue::UnaryExpr { op: UnaryOp::CharToStr, operand },
            TyKind::Struct { id, symbols, fields } => {
                self.format_struct(*id, symbols, fields, operand)
            }
            _ => todo!("{}.to_string()", ty),
        }
    }

    fn deref_operand(&mut self, rvalue: RValue) -> Operand {
        match rvalue {
            RValue::Use(Operand::Place(mut place)) => {
                place.projections.push(Projection::Deref);
                Operand::Place(place)
            }
            RValue::Use(Operand::Ref(place)) => Operand::Place(place),
            _ => {
                let local = self.assign_new(rvalue);
                Operand::Place(Place { local, projections: vec![Projection::Deref] })
            }
        }
    }

    fn format_struct(
        &mut self,
        id: StructId,
        symbols: &[Symbol],
        fields: &[Ty],
        val: Operand,
    ) -> RValue {
        // TODO: This should pass the struct by ref
        let body = self.generate_struct_func(id, symbols, fields);
        let ref_struct = self.ref_of(RValue::Use(val));
        RValue::Call {
            function: Operand::Constant(Constant::Func(body)),
            args: [ref_struct].into(),
        }
    }

    fn generate_struct_func(&mut self, id: StructId, symbols: &[Symbol], fields: &[Ty]) -> BodyId {
        _ = symbols;
        let previous = self.bodies.pop().unwrap(); // TODO: We should pop till further up
        let body_id = self.mir.bodies.push(Body::new(None, 1).with_auto(false));
        self.bodies.push(BodyInfo::new(body_id));
        let local = Local::from(0);

        // segments + seperators + open/close brackets
        let num_parts = fields.len() + fields.len().saturating_sub(1) + 2;

        let strings = self.assign_new(Constant::EmptyArray { cap: num_parts });

        self.process(
            RValue::BinaryExpr {
                lhs: Operand::Ref(strings.into()),
                op: BinaryOp::ArrayPush,
                rhs: str!("("),
            },
            &TyKind::Unit,
        );

        for (i, ty) in (0u32..).zip(fields) {
            if i != 0 {
                self.process(
                    RValue::BinaryExpr {
                        lhs: Operand::Ref(strings.into()),
                        op: BinaryOp::ArrayPush,
                        rhs: str!(", "),
                    },
                    &TyKind::Unit,
                );
            }

            let projections = vec![Projection::Deref, Projection::Field(i as _)];
            let field = RValue::Use(Operand::Place(Place { local, projections }));
            let field_str = self.format_rvalue(field, ty);
            let rhs = self.process(field_str, &TyKind::Str);
            self.process(
                RValue::BinaryExpr {
                    lhs: Operand::Ref(strings.into()),
                    op: BinaryOp::ArrayPush,
                    rhs,
                },
                &TyKind::Unit,
            );
        }

        self.process(
            RValue::BinaryExpr {
                lhs: Operand::Ref(strings.into()),
                op: BinaryOp::ArrayPush,
                rhs: str!(")"),
            },
            &TyKind::Unit,
        );

        let out = self.assign_new(RValue::UnaryExpr {
            op: UnaryOp::StrJoin,
            operand: Operand::local(strings),
        });
        self.finish_with(Terminator::Return(Operand::local(out)));

        if self.struct_display_bodies.len() <= id {
            self.struct_display_bodies.resize(id.index() + 1, None);
        }
        self.struct_display_bodies[id] = Some(body_id);

        self.bodies.pop();
        self.bodies.push(previous);
        body_id
    }
}
