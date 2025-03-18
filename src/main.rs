#[cfg(test)]
mod tests;

mod ast;
mod ast_analysis;
mod ast_lowering;
mod compile;
mod hir;
mod hir_lowering;
mod mir;
mod mir_interpreter;
mod parse;
mod span;
mod symbol;
mod ty;

fn main() {
    match compile::compile_and_dump(include_str!("../examples/brainfuck.pebble")) {
        Ok(()) => {}
        Err(err) => eprintln!("{err:?}"),
    }
}
