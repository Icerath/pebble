mod generic_range;
mod interner;
mod kind;

use std::{cell::RefCell, collections::HashMap, hash::Hash};

pub use generic_range::GenericRange;
use index_vec::IndexVec;
pub use interner::TyInterner;
pub use kind::TyKind;
use thin_vec::ThinVec;

use crate::{define_id, symbol::Symbol};

pub type Ty<'tcx> = &'tcx TyKind<'tcx>;

define_id!(pub TyVid = u32);
define_id!(pub GenericId = u32);
define_id!(pub StructId = u32);

// TODO: We shouldn't actually need to keep track of a function's generics.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Function<'tcx> {
    pub params: ThinVec<Ty<'tcx>>,
    pub ret: Ty<'tcx>,
}

impl<'tcx> Function<'tcx> {
    pub fn caller(&self, tcx: &'tcx TyCtx<'tcx>) -> (Vec<Ty<'tcx>>, Ty<'tcx>) {
        let mut map = HashMap::new();
        self.generics(&mut |id| _ = map.entry(id).or_insert_with(|| tcx.new_vid()));
        let f = |id| map[&id];
        let params = self.params.iter().map(|param| param.replace_generics(tcx, f)).collect();
        let ret = self.ret.replace_generics(tcx, f);
        (params, ret)
    }
    pub fn generics(&self, f: &mut impl FnMut(GenericId)) {
        self.params.iter().for_each(|param| param.generics(f));
        self.ret.generics(f);
    }
}

pub struct TyCtx<'tcx> {
    inner: RefCell<TyCtxInner<'tcx>>,
    interner: &'tcx TyInterner,
}

impl<'tcx> TyCtx<'tcx> {
    pub fn new(interner: &'tcx TyInterner) -> Self {
        Self { inner: RefCell::default(), interner }
    }
    pub fn new_generics(&self, generics: &[Symbol]) -> GenericRange {
        let mut inner = self.inner.borrow_mut();
        let mut iter = generics.iter();
        let Some(start) = iter.next() else { return GenericRange::EMPTY };
        let start = inner.new_generic(*start);
        iter.for_each(|generic| _ = inner.new_generic(*generic));
        GenericRange { start, len: generics.len().try_into().unwrap() }
    }
    pub fn generic_symbol(&self, id: GenericId) -> Symbol {
        self.inner.borrow_mut().generic_names[id]
    }
    pub fn new_struct(&self, name: Symbol, fields: ThinVec<Ty<'tcx>>) -> Ty<'tcx> {
        self.intern(self.inner.borrow_mut().new_struct(name, fields))
    }
    pub fn intern(&self, kind: TyKind<'tcx>) -> Ty<'tcx> {
        self.interner.intern(kind)
    }
    pub fn new_vid(&self) -> TyVid {
        self.inner.borrow_mut().vid(self.interner)
    }
    pub fn new_infer(&self) -> Ty<'tcx> {
        self.interner.intern(TyKind::Infer(self.new_vid()))
    }
    pub fn infer_shallow(&self, ty: Ty<'tcx>) -> Ty<'tcx> {
        self.inner.borrow().infer_shallow(ty)
    }
    pub fn infer_deep(&self, ty: Ty<'tcx>) -> Ty<'tcx> {
        self.inner.borrow().infer_deep(ty, self.interner)
    }
    pub fn try_eq(&self, lhs: Ty<'tcx>, rhs: Ty<'tcx>) -> Result<(), [Ty<'tcx>; 2]> {
        self.inner.borrow_mut().try_eq(lhs, rhs)
    }
    pub fn try_subtype(&self, lhs: Ty<'tcx>, rhs: Ty<'tcx>) -> Result<(), [Ty<'tcx>; 2]> {
        self.inner.borrow_mut().subtype(lhs, rhs)
    }
}

#[derive(Default, Debug)]
struct TyCtxInner<'tcx> {
    subs: IndexVec<TyVid, Ty<'tcx>>,
    struct_names: IndexVec<StructId, Symbol>,
    generic_names: IndexVec<GenericId, Symbol>,
}

impl<'tcx> TyCtxInner<'tcx> {
    fn new_struct(&mut self, name: Symbol, fields: ThinVec<Ty<'tcx>>) -> TyKind<'tcx> {
        let id = self.struct_names.push(name);
        TyKind::Struct { id, fields }
    }

    fn new_generic(&mut self, symbol: Symbol) -> GenericId {
        self.generic_names.push(symbol)
    }

    fn vid(&mut self, intern: &'tcx TyInterner) -> TyVid {
        let id = self.subs.next_idx();
        self.subs.push(intern.intern(TyKind::Infer(id)))
    }

    fn infer_shallow(&self, ty: Ty<'tcx>) -> Ty<'tcx> {
        match *ty {
            TyKind::Infer(var) if self.subs[var] == ty => panic!("Failed to infer"),
            TyKind::Infer(var) => self.infer_shallow(self.subs[var]),
            _ => ty,
        }
    }

    fn infer_deep(&self, ty: Ty<'tcx>, intern: &'tcx TyInterner) -> Ty<'tcx> {
        match self.infer_shallow(ty) {
            TyKind::Array(of) => intern.intern(TyKind::Array(self.infer_deep(of, intern))),
            ty => ty,
        }
    }

    fn try_eq(&mut self, lhs: Ty<'tcx>, rhs: Ty<'tcx>) -> Result<(), [Ty<'tcx>; 2]> {
        match (lhs, rhs) {
            (TyKind::Infer(l), TyKind::Infer(r)) if l == r => Ok(()),
            (TyKind::Infer(var), _) => self.insertl(*var, rhs),
            (_, TyKind::Infer(var)) => self.insertr(lhs, *var),
            (TyKind::Array(lhs), TyKind::Array(rhs)) => self.try_eq(lhs, rhs),
            (TyKind::Function(lhs), TyKind::Function(rhs)) => {
                assert_eq!(lhs.params.len(), rhs.params.len());
                lhs.params.iter().zip(&rhs.params).try_for_each(|(l, r)| self.try_eq(l, r))?;
                self.try_eq(lhs.ret, rhs.ret)
            }
            (lhs, rhs) if lhs == rhs => Ok(()),
            (..) => Err([lhs, rhs]),
        }
    }

    /// Says that `lhs` must be a subtype of `rhs`.
    /// never is a subtype of everything.
    fn subtype(&mut self, lhs: Ty<'tcx>, rhs: Ty<'tcx>) -> Result<(), [Ty<'tcx>; 2]> {
        let Err([lhs, rhs]) = self.try_eq(lhs, rhs) else { return Ok(()) };
        if lhs.is_never() { Ok(()) } else { Err([lhs, rhs]) }
    }

    fn insertl(&mut self, var: TyVid, ty: Ty<'tcx>) -> Result<(), [Ty<'tcx>; 2]> {
        self.insert_inner(var, ty, true)
    }

    fn insertr(&mut self, ty: Ty<'tcx>, var: TyVid) -> Result<(), [Ty<'tcx>; 2]> {
        self.insert_inner(var, ty, false)
    }

    fn insert_inner(
        &mut self,
        var: TyVid,
        ty: Ty<'tcx>,
        is_left: bool,
    ) -> Result<(), [Ty<'tcx>; 2]> {
        if let Some(&sub) = self.subs.get(var) {
            if let TyKind::Infer(sub) = *sub {
                if sub == var {
                    self.subs[var] = ty;
                }
            }
            return if is_left { self.try_eq(sub, ty) } else { self.try_eq(ty, sub) };
        }
        assert!(!self.occurs_in(var, ty), "Infinite type: {var:?} - {ty:?}");
        self.subs[var] = ty;
        Ok(())
    }

    fn occurs_in(&self, this: TyVid, ty: Ty<'tcx>) -> bool {
        match *ty {
            TyKind::Infer(var) => {
                if let Some(&sub) = self.subs.get(var) {
                    if *sub != TyKind::Infer(var) {
                        return self.occurs_in(var, sub);
                    }
                }
                this == var
            }
            _ => false,
        }
    }
}
