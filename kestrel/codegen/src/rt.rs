use cranelift_codegen::ir::{types, AbiParam, Signature, Type};
use cranelift_module::{DataId, FuncId, Linkage, Module};
use kestrel_kir::MathOp;

/// Runtime functions, declared as imports in a module.
pub(crate) struct RtFuncs {
    pub print_i64: FuncId,
    pub print_f32: FuncId,
    pub print_f64: FuncId,
    pub print_bool: FuncId,
    pub print_str: FuncId,
    pub print_space: FuncId,
    pub print_newline: FuncId,
    pub panic: FuncId,
    pub fmod_f32: FuncId,
    pub fmod_f64: FuncId,
    pub alloc: FuncId,
    pub dup: FuncId,
    pub list_grow: FuncId,
    pub list_remove: FuncId,
    pub list_insert: FuncId,
    pub list_clear: FuncId,
    pub list_compact: FuncId,
    pub current_arena: FuncId,
    pub panic_index: FuncId,
    pub arena_new: FuncId,
    pub arena_new_static: FuncId,
    pub arena_free: FuncId,
    pub arena_push: FuncId,
    pub arena_pop: FuncId,
    pub map_find: FuncId,
    pub map_insert: FuncId,
    pub map_remove: FuncId,
    pub map_dup: FuncId,
    pub str_eq: FuncId,
    pub par_for: FuncId,
    /// The function for each float operation the runtime does (see `MATH`).
    pub math: Vec<(MathOp, FuncId)>,
    pub rand_float: FuncId,
    pub rand_int: FuncId,
    pub rand_seed: FuncId,
    pub str_begin: FuncId,
    pub str_end: FuncId,
    pub gpu_drop: FuncId,
    /// The runtime's `kestrel_rt_call_line`, the line of the latest call into a built-in function.
    pub call_line: DataId,
    /// The runtime's `kestrel_rt_call_file`, the file of that call.
    pub call_file: DataId,
}

impl RtFuncs {
    pub fn declare<M: Module>(module: &mut M) -> Result<Self, String> {
        let cc = module.isa().default_call_conv();
        let ptr = module.target_config().pointer_type();
        let call_line = module.declare_data("kestrel_rt_call_line", Linkage::Import, true, false).map_err(|e| e.to_string())?;
        let call_file = module.declare_data("kestrel_rt_call_file", Linkage::Import, true, false).map_err(|e| e.to_string())?;
        let mut decl = |name: &str, params: &[Type], ret: Option<Type>| -> Result<FuncId, String> {
            let mut sig = Signature::new(cc);
            // The C ABI leaves an i8 argument's upper bits undefined unless the caller extends it.
            sig.params.extend(params.iter().map(|&p| {
                if p == types::I8 {
                    AbiParam::new(p).uext()
                } else {
                    AbiParam::new(p)
                }
            }));
            sig.returns.extend(ret.map(AbiParam::new));
            module.declare_function(name, Linkage::Import, &sig).map_err(|e| e.to_string())
        };
        use types::{F32, F64, I32, I64, I8};
        Ok(RtFuncs {
            print_i64: decl("kestrel_rt_print_i64", &[I64], None)?,
            print_f32: decl("kestrel_rt_print_f32", &[F32], None)?,
            print_f64: decl("kestrel_rt_print_f64", &[F64], None)?,
            print_bool: decl("kestrel_rt_print_bool", &[I8], None)?,
            print_str: decl("kestrel_rt_print_str", &[ptr, I64], None)?,
            print_space: decl("kestrel_rt_print_space", &[], None)?,
            print_newline: decl("kestrel_rt_print_newline", &[], None)?,
            panic: decl("kestrel_rt_panic", &[I32, I32, ptr], None)?,
            fmod_f32: decl("kestrel_rt_fmod_f32", &[F32, F32], Some(F32))?,
            fmod_f64: decl("kestrel_rt_fmod_f64", &[F64, F64], Some(F64))?,
            alloc: decl("kestrel_rt_alloc", &[I64, I64, I64, I32, ptr], Some(ptr))?,
            dup: decl("kestrel_rt_dup", &[ptr, I64, I64], Some(ptr))?,
            list_grow: decl("kestrel_rt_list_grow", &[ptr, I64, I64], None)?,
            list_remove: decl("kestrel_rt_list_remove", &[ptr, I64, I64, I32, ptr], None)?,
            list_insert: decl("kestrel_rt_list_insert", &[ptr, I64, I64, I64, I32, ptr], Some(ptr))?,
            list_clear: decl("kestrel_rt_list_clear", &[ptr, I64], None)?,
            list_compact: decl("kestrel_rt_list_compact", &[ptr, I64, I64, I64], None)?,
            current_arena: decl("kestrel_rt_current_arena", &[], Some(ptr))?,
            panic_index: decl("kestrel_rt_panic_index", &[I64, I64, I32, ptr], None)?,
            arena_new: decl("kestrel_rt_arena_new", &[], Some(ptr))?,
            arena_new_static: decl("kestrel_rt_arena_new_static", &[], Some(ptr))?,
            arena_free: decl("kestrel_rt_arena_free", &[ptr], None)?,
            arena_push: decl("kestrel_rt_arena_push", &[ptr], None)?,
            arena_pop: decl("kestrel_rt_arena_pop", &[], Some(ptr))?,
            map_find: decl("kestrel_rt_map_find", &[ptr, ptr, I32, I64], Some(ptr))?,
            map_insert: decl("kestrel_rt_map_insert", &[ptr, ptr, I32, I64, I64, I32, ptr], Some(ptr))?,
            map_remove: decl("kestrel_rt_map_remove", &[ptr, ptr, I32, I64], Some(I8))?,
            map_dup: decl("kestrel_rt_map_dup", &[ptr, I64, I64], None)?,
            str_eq: decl("kestrel_rt_str_eq", &[ptr, I64, ptr, I64], Some(I8))?,
            par_for: decl("kestrel_rt_par_for", &[I64, ptr, ptr], None)?,
            math: MATH.iter().map(|&(op, name, args)| Ok((op, decl(name, &[F64, F64][..args], Some(F64))?))).collect::<Result<_, String>>()?,
            rand_float: decl("kestrel_rt_rand_float", &[], Some(F64))?,
            rand_int: decl("kestrel_rt_rand_int", &[I64, I64], Some(I64))?,
            rand_seed: decl("kestrel_rt_rand_seed", &[I64], None)?,
            str_begin: decl("kestrel_rt_str_begin", &[], None)?,
            str_end: decl("kestrel_rt_str_end", &[ptr], None)?,
            gpu_drop: decl("kestrel_rt_gpu_drop", &[I64], None)?,
            call_line,
            call_file,
        })
    }
}

/// The float operations the runtime does: each one's function and how many `f64`s it takes.
pub(crate) const MATH: [(MathOp, &str, usize); 10] = [
    (MathOp::Sin, "kestrel_rt_sin", 1),
    (MathOp::Cos, "kestrel_rt_cos", 1),
    (MathOp::Tan, "kestrel_rt_tan", 1),
    (MathOp::Asin, "kestrel_rt_asin", 1),
    (MathOp::Acos, "kestrel_rt_acos", 1),
    (MathOp::Atan, "kestrel_rt_atan", 1),
    (MathOp::Atan2, "kestrel_rt_atan2", 2),
    (MathOp::Exp, "kestrel_rt_exp", 1),
    (MathOp::Log, "kestrel_rt_log", 1),
    (MathOp::Pow, "kestrel_rt_pow", 2),
];

/// Addresses of every runtime export, for registering with the JIT.
pub(crate) fn jit_symbols() -> Vec<(&'static str, *const u8)> {
    use kestrel_runtime::{arena, gpu, list, map, math, panic, par, print, start};
    vec![
        ("kestrel_rt_print_i64", print::kestrel_rt_print_i64 as *const u8),
        ("kestrel_rt_print_f32", print::kestrel_rt_print_f32 as *const u8),
        ("kestrel_rt_print_f64", print::kestrel_rt_print_f64 as *const u8),
        ("kestrel_rt_print_bool", print::kestrel_rt_print_bool as *const u8),
        ("kestrel_rt_print_str", print::kestrel_rt_print_str as *const u8),
        ("kestrel_rt_print_space", print::kestrel_rt_print_space as *const u8),
        ("kestrel_rt_print_newline", print::kestrel_rt_print_newline as *const u8),
        ("kestrel_rt_panic", panic::kestrel_rt_panic as *const u8),
        ("kestrel_rt_fmod_f32", panic::kestrel_rt_fmod_f32 as *const u8),
        ("kestrel_rt_fmod_f64", panic::kestrel_rt_fmod_f64 as *const u8),
        ("kestrel_rt_start", start::kestrel_rt_start as *const u8),
        ("kestrel_rt_alloc", arena::kestrel_rt_alloc as *const u8),
        ("kestrel_rt_dup", arena::kestrel_rt_dup as *const u8),
        ("kestrel_rt_list_grow", arena::kestrel_rt_list_grow as *const u8),
        ("kestrel_rt_list_remove", list::kestrel_rt_list_remove as *const u8),
        ("kestrel_rt_list_insert", list::kestrel_rt_list_insert as *const u8),
        ("kestrel_rt_list_clear", list::kestrel_rt_list_clear as *const u8),
        ("kestrel_rt_list_compact", list::kestrel_rt_list_compact as *const u8),
        ("kestrel_rt_current_arena", arena::kestrel_rt_current_arena as *const u8),
        ("kestrel_rt_panic_index", panic::kestrel_rt_panic_index as *const u8),
        ("kestrel_rt_arena_new", arena::kestrel_rt_arena_new as *const u8),
        ("kestrel_rt_arena_new_static", arena::kestrel_rt_arena_new_static as *const u8),
        ("kestrel_rt_arena_free", arena::kestrel_rt_arena_free as *const u8),
        ("kestrel_rt_arena_push", arena::kestrel_rt_arena_push as *const u8),
        ("kestrel_rt_arena_pop", arena::kestrel_rt_arena_pop as *const u8),
        ("kestrel_rt_map_find", map::kestrel_rt_map_find as *const u8),
        ("kestrel_rt_map_insert", map::kestrel_rt_map_insert as *const u8),
        ("kestrel_rt_map_remove", map::kestrel_rt_map_remove as *const u8),
        ("kestrel_rt_map_dup", map::kestrel_rt_map_dup as *const u8),
        ("kestrel_rt_str_eq", map::kestrel_rt_str_eq as *const u8),
        ("kestrel_rt_par_for", par::kestrel_rt_par_for as *const u8),
        ("kestrel_rt_sin", math::kestrel_rt_sin as *const u8),
        ("kestrel_rt_cos", math::kestrel_rt_cos as *const u8),
        ("kestrel_rt_tan", math::kestrel_rt_tan as *const u8),
        ("kestrel_rt_asin", math::kestrel_rt_asin as *const u8),
        ("kestrel_rt_acos", math::kestrel_rt_acos as *const u8),
        ("kestrel_rt_atan", math::kestrel_rt_atan as *const u8),
        ("kestrel_rt_atan2", math::kestrel_rt_atan2 as *const u8),
        ("kestrel_rt_exp", math::kestrel_rt_exp as *const u8),
        ("kestrel_rt_log", math::kestrel_rt_log as *const u8),
        ("kestrel_rt_pow", math::kestrel_rt_pow as *const u8),
        ("kestrel_rt_rand_float", math::kestrel_rt_rand_float as *const u8),
        ("kestrel_rt_rand_int", math::kestrel_rt_rand_int as *const u8),
        ("kestrel_rt_rand_seed", math::kestrel_rt_rand_seed as *const u8),
        ("kestrel_rt_str_begin", print::kestrel_rt_str_begin as *const u8),
        ("kestrel_rt_str_end", print::kestrel_rt_str_end as *const u8),
        ("kestrel_rt_gpu_drop", gpu::kestrel_rt_gpu_drop as *const u8),
        ("kestrel_rt_call_line", std::ptr::addr_of!(panic::kestrel_rt_call_line).cast::<u8>()),
        ("kestrel_rt_call_file", std::ptr::addr_of!(panic::kestrel_rt_call_file).cast::<u8>()),
    ]
}

#[cfg(test)]
mod tests {
    use cranelift_codegen::ir::ArgumentExtension;
    use cranelift_module::{default_libcall_names, Module};
    use cranelift_object::{ObjectBuilder, ObjectModule};

    use super::RtFuncs;
    use crate::isa;

    #[test]
    fn i8_runtime_arguments_are_zero_extended() {
        let builder = ObjectBuilder::new(isa::host_isa().unwrap(), "t", default_libcall_names()).unwrap();
        let mut module = ObjectModule::new(builder);
        let rt = RtFuncs::declare(&mut module).unwrap();
        let sig = &module.declarations().get_function_decl(rt.print_bool).signature;
        assert_eq!(sig.params[0].extension, ArgumentExtension::Uext);
        // Only the i8 parameter is extended.
        let sig = &module.declarations().get_function_decl(rt.print_i64).signature;
        assert_eq!(sig.params[0].extension, ArgumentExtension::None);
    }
}
