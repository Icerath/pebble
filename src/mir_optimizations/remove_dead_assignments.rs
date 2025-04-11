use super::utils::{blocks, blocks_mut};
use crate::mir::{BodyId, Local, Mir, Statement};

pub fn optimize(mir: &mut Mir, body_id: BodyId) {
    let body = &mut mir.bodies[body_id];

    let mut access_counts = index_vec::index_vec![0; body.locals.index()];
    for param in 0..body.params {
        access_counts[param] += 1;
    }

    for block in blocks(body) {
        let mut incr = |local: Local| access_counts[local] += 1;
        for statement in &block.statements {
            let Statement::Assign { place, rvalue } = statement;
            rvalue.with_locals(&mut incr);
            place.projections.iter().for_each(|proj| proj.with_locals(&mut incr));
        }
        block.terminator.with_locals(incr);
    }

    for block in blocks_mut(body) {
        block.statements.retain(|statement| {
            let Statement::Assign { place, rvalue } = statement;
            if access_counts[place.local] > 0
                || rvalue.side_effect()
                || !place.projections.is_empty()
            {
                return true;
            }
            false
        });
    }
}
