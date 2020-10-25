use super::proc::Proc;
use super::raw_types;
use super::value::Value;
use super::DMContext;
use crate::raw_types::values::IntoRawValue;
use crate::runtime::DMResult;
use detour::RawDetour;
use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Once;

use crate::vm::vm as vmhook;

#[doc(hidden)]
pub struct CompileTimeHook {
	pub proc_path: &'static str,
	pub hook: ProcHook,
}

impl CompileTimeHook {
	pub fn new(proc_path: &'static str, hook: ProcHook) -> Self {
		CompileTimeHook { proc_path, hook }
	}
}

inventory::collect!(CompileTimeHook);

extern "C" {

	static mut call_proc_by_id_original: *const c_void;

	fn call_proc_by_id_original_trampoline(
		usr: raw_types::values::Value,
		proc_type: u32,
		proc_id: raw_types::procs::ProcId,
		unk_0: u32,
		src: raw_types::values::Value,
		args: *mut raw_types::values::Value,
		args_countL: usize,
		unk_1: u32,
		unk_2: u32,
	) -> raw_types::values::Value;

	fn call_proc_by_id_hook_trampoline(
		usr: raw_types::values::Value,
		proc_type: u32,
		proc_id: raw_types::procs::ProcId,
		unk_0: u32,
		src: raw_types::values::Value,
		args: *mut raw_types::values::Value,
		args_countL: usize,
		unk_1: u32,
		unk_2: u32,
	) -> raw_types::values::Value;
}

pub enum HookFailure {
	NotInitialized,
	ProcNotFound,
	AlreadyHooked,
	UnknownFailure,
}

impl std::fmt::Debug for HookFailure {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::NotInitialized => write!(f, "Library not initialized"),
			Self::ProcNotFound => write!(f, "Proc not found"),
			Self::AlreadyHooked => write!(f, "Proc is already hooked"),
			Self::UnknownFailure => write!(f, "Unknown failure"),
		}
	}
}

pub fn init() -> Result<(), String> {
	unsafe {
		let hook = RawDetour::new(
			raw_types::funcs::call_proc_by_id_byond as *const (),
			call_proc_by_id_hook_trampoline as *const (),
		)
		.unwrap();

		hook.enable().unwrap();
		call_proc_by_id_original = std::mem::transmute(hook.trampoline());
		std::mem::forget(hook);
	}
	Ok(())
}

pub type ProcHook =
	for<'a, 'r> fn(&'a DMContext<'r>, &Value<'a>, &Value<'a>, &mut Vec<Value<'a>>) -> DMResult<'a>;

enum HookType {
	Rust(ProcHook),
	VM,
}
thread_local! {
	static PROC_HOOKS: RefCell<HashMap<raw_types::procs::ProcId, HookType>> = RefCell::new(HashMap::new());
	static HOOK_VM: RefCell<vmhook::VM> = RefCell::new(vmhook::VM::new());
}

static PROC_HOOKS_INIT: Once = Once::new();

fn hook_by_id(id: raw_types::procs::ProcId, hook: ProcHook) -> Result<(), HookFailure> {
	PROC_HOOKS_INIT.call_once(|| {
		if let Err(e) = init() {
			panic!(e);
		}
	});
	PROC_HOOKS.with(|h| {
		let mut map = h.borrow_mut();
		match map.entry(id) {
			Entry::Vacant(v) => {
				v.insert(HookType::Rust(hook));
				Ok(())
			}
			Entry::Occupied(_) => Err(HookFailure::AlreadyHooked),
		}
	})
}

pub fn hook_by_id_with_bytecode_dont_use_this(id: raw_types::procs::ProcId, hook: Vec<u8>) {
	PROC_HOOKS_INIT.call_once(|| {
		if let Err(e) = init() {
			panic!(e);
		}
	});
	PROC_HOOKS.with(|h| {
		let mut map = h.borrow_mut();
		match map.entry(id) {
			Entry::Vacant(v) => {
				v.insert(HookType::VM);
				HOOK_VM.with(|vm| {
					vm.borrow_mut().add_program(id.0, hook);
				});
				Ok(())
			}
			Entry::Occupied(_) => Err(HookFailure::AlreadyHooked),
		}
	});
}

pub fn hook<S: Into<String>>(name: S, hook: ProcHook) -> Result<(), HookFailure> {
	match super::proc::get_proc(name) {
		Some(p) => hook_by_id(p.id, hook),
		None => Err(HookFailure::ProcNotFound),
	}
}

impl Proc {
	#[allow(unused)]
	pub fn hook(&self, func: ProcHook) -> Result<(), HookFailure> {
		hook_by_id(self.id, func)
	}
}

#[no_mangle]
extern "C" fn call_proc_by_id_hook(
	usr_raw: raw_types::values::Value,
	proc_type: u32,
	proc_id: raw_types::procs::ProcId,
	unknown1: u32,
	src_raw: raw_types::values::Value,
	args_ptr: *mut raw_types::values::Value,
	num_args: usize,
	unknown2: u32,
	unknown3: u32,
) -> raw_types::values::Value {
	return PROC_HOOKS.with(|h| match h.borrow().get(&proc_id) {
		Some(hook) => {
			let ctx = unsafe { DMContext::new() };
			let src;
			let usr;
			let mut args: Vec<Value>;

			unsafe {
				src = Value::from_raw(src_raw);
				usr = Value::from_raw(usr_raw);

				// Taking ownership of args here
				args = std::slice::from_raw_parts(args_ptr, num_args)
					.iter()
					.map(|v| Value::from_raw_owned(*v))
					.collect();
			}

			let result = match hook {
				HookType::Rust(func) => func(&ctx, &src, &usr, &mut args),
				HookType::VM => {
					let register_args = args
						.iter()
						.map(|a| vmhook::Register {
							tag: a.value.tag as u32,
							value: unsafe { a.value.data.id },
						})
						.collect();

					HOOK_VM.with(|vm| {
						let ret = vm.borrow_mut().run_program(proc_id.0, register_args);
						Ok(unsafe {
							Value::from_raw(raw_types::values::Value {
								tag: std::mem::transmute(ret.tag as u8),
								data: std::mem::transmute(ret.value),
							})
						})
					})
				}
			};

			match result {
				Ok(r) => {
					let result_raw = unsafe { (&r).into_raw_value() };
					// Stealing our reference out of the Value
					std::mem::forget(r);
					result_raw
				}
				Err(e) => {
					// TODO: Some info about the hook would be useful (as the hook is never part of byond's stack, the runtime won't show it.)
					src.call("stack_trace", &[&Value::from_string(e.message.as_str())])
						.unwrap();
					unsafe { Value::null().into_raw_value() }
				}
			}
		}
		None => unsafe {
			call_proc_by_id_original_trampoline(
				usr_raw, proc_type, proc_id, unknown1, src_raw, args_ptr, num_args, unknown2,
				unknown3,
			)
		},
	});
}
