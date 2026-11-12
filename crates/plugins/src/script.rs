//! TypeScript is transpiled before the single-module QuickJS compile check.
use anyhow::{Result, ensure};
use musicx_core::protocol::tr;
use oxc::{
    allocator::Allocator,
    codegen::Codegen,
    parser::Parser,
    semantic::SemanticBuilder,
    span::SourceType,
    transformer::{TransformOptions, Transformer},
};
use std::path::Path;

pub(crate) fn javascript(name: &str, source: &str) -> Result<String> {
    if !name.ends_with(".ts") {
        return Ok(source.to_owned());
    }
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::ts()).parse();
    ensure!(
        parsed.diagnostics.is_empty(),
        tr!("Invalid TypeScript plugin syntax")
    );
    let mut program = parsed.program;
    let semantic = SemanticBuilder::new().with_enum_eval(true).build(&program);
    ensure!(
        semantic.diagnostics.is_empty(),
        tr!("Invalid TypeScript plugin declaration")
    );
    let result = Transformer::new(&allocator, Path::new(name), &TransformOptions::default())
        .build_with_scoping(semantic.semantic.into_scoping(), &mut program);
    ensure!(
        result.diagnostics.is_empty(),
        tr!("Could not transpile TypeScript plugin")
    );
    Ok(Codegen::new().build(&program).code)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strips_types_without_running_or_changing_string_contents() {
        let js = javascript("index.ts", "interface Input { id: string }; export function stream({id}: Input): {url:string} { return {url:'https://example.com/'+id}; }").unwrap();
        assert!(js.contains("function stream"));
        assert!(!js.contains("interface Input"));
        crate::music_sources::check_single_file_script(&js).unwrap();
        assert!(javascript("bad.ts", "export const value: = ;").is_err());
    }
}
