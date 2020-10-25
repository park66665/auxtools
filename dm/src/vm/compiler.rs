extern crate dreammaker as dm;

use dm::ast::*;
use dm::objtree::ProcRef;

use dm::ast::PathOp;
use dm::objtree::NavigatePathResult::ProcPath;

use crate::vm::vm as vmhook;
use vmhook::Opcode::*;

use std::cell::RefCell;
use std::sync::Arc;

use std::collections::HashMap;

use crate::value;

extern crate byteorder;

trait RegisterId {
	fn to_id(&self) -> u8;
}

struct TempRegister {
	free_regs: Arc<RefCell<Vec<usize>>>,
	id: usize,
}

impl Drop for TempRegister {
	fn drop(&mut self) {
		self.free_regs.borrow_mut().push(self.id);
	}
}

impl RegisterId for TempRegister {
	fn to_id(&self) -> u8 {
		self.id as u8
	}
}

struct Register {
	id: usize,
}

impl RegisterId for Register {
	fn to_id(&self) -> u8 {
		self.id as u8
	}
}

impl From<usize> for Register {
	fn from(id: usize) -> Self {
		Self { id }
	}
}

pub struct Compiler<'a> {
	proc: &'a ProcRef<'a>,

	bytecode: Vec<u8>,
	next_free_register: usize,
	free_registers: Arc<RefCell<Vec<usize>>>,
	locals: HashMap<String, Register>,
	args: HashMap<String, Register>,
}

impl<'a> Compiler<'a> {
	fn new(proc: &'a ProcRef<'a>) -> Self {
		let args = proc
			.get()
			.parameters
			.iter()
			.enumerate()
			.map(|(i, p)| (p.name.clone(), i.into()))
			.collect();
		Self {
			proc,
			bytecode: Vec::new(),
			next_free_register: 0,
			free_registers: Arc::new(RefCell::new(Vec::new())),
			locals: HashMap::new(),
			args,
		}
	}

	fn get_free_register(&mut self) -> TempRegister {
		let mut free_regs = self.free_registers.borrow_mut();
		let id;
		if free_regs.len() > 0 {
			id = free_regs.swap_remove(0);
		} else {
			id = self.next_free_register;
			self.next_free_register += 1;
		}
		return TempRegister {
			free_regs: self.free_registers.clone(),
			id,
		};
	}

	fn emit<U: Into<u8> + Copy>(&mut self, bytes: &[U]) {
		for byte in bytes {
			self.bytecode.push((*byte).into())
		}
	}

	pub fn visit_block(&mut self, block: &'a [Spanned<Statement>]) -> Result<(), String> {
		for stmt in block.iter() {
			self.visit_statement(&stmt.elem)?;
		}
		Ok(())
	}

	fn compile(&mut self) -> Result<Vec<u8>, String> {
		if let dm::objtree::Code::Present(ref code) = self.proc.code {
			self.visit_block(code)?;
		}
		Ok(self.bytecode.clone())
	}

	fn visit_statement(&mut self, statement: &'a Statement) -> Result<(), String> {
		return match statement {
			Statement::Expr(expr) => self.visit_expression_statement(expr),
			Statement::Return(Some(expr)) => {
				let return_reg = self.visit_expression(expr)?;
				self.emit(&[RETURN as u8, return_reg.to_id()]);
				Ok(())
			}
			Statement::Var(var) => self.visit_var(var),
			Statement::If { arms, else_arm } => self.visit_if(arms, else_arm),
			_ => Err(format!("Unsupported statement: {:#?}", statement)),
		};
	}

	fn visit_expression(&mut self, expr: &'a Expression) -> Result<TempRegister, String> {
		self.visit_expression_impl(expr, false)
	}

	fn visit_expression_statement(&mut self, expr: &'a Expression) -> Result<(), String> {
		if let Err(e) = self.visit_expression_impl(expr, true) {
			return Err(e);
		}
		Ok(())
	}

	fn visit_expression_impl(
		&mut self,
		expr: &'a Expression,
		is_statement: bool,
	) -> Result<TempRegister, String> {
		match expr {
			Expression::Base {
				unary,
				term,
				follow,
			} => self.visit_term(&term.elem, &follow, is_statement),
			Expression::BinaryOp { op, lhs, rhs } => {
				let left_reg = self.visit_expression(lhs)?;
				let right_reg = self.visit_expression(rhs)?;
				let oper = match op {
					BinaryOp::Add => ADD,
					BinaryOp::Sub => SUB,
					BinaryOp::Mul => MUL,
					BinaryOp::Div => DIV,

					BinaryOp::Less => LESS_THAN,
					BinaryOp::LessEq => LESS_OR_EQUAL,
					BinaryOp::Eq => EQUAL,
					BinaryOp::GreaterEq => GREATER_OR_EQUAL,
					BinaryOp::Greater => GREATER_THAN,
					_ => panic!("Binop not implemented"),
				};

				let result_reg = self.get_free_register();
				self.emit(&[
					oper as u8,
					left_reg.to_id(),
					right_reg.to_id(),
					result_reg.to_id(),
				]);
				return Ok(result_reg);
			}
			_ => return Err(format!("Unimplemented expression: {:#?}", expr)),
		}
	}

	fn visit_term(
		&mut self,
		term: &'a Term,
		follows: &'a Vec<Spanned<Follow>>,
		is_statement: bool,
	) -> Result<TempRegister, String> {
		match term {
			Term::Int(number) => {
				let reg = self.get_free_register();

				let mut instr = vec![LOAD_IMMEDIATE as u8, reg.to_id(), 0x2A];
				instr.extend((*number as f32).to_le_bytes().iter());

				self.emit(&instr.as_slice());
				Ok(reg)
			}
			Term::Float(number) => {
				let reg = self.get_free_register();

				let mut instr = vec![LOAD_IMMEDIATE as u8, reg.to_id(), 0x2A];
				instr.extend(number.to_le_bytes().iter());

				self.emit(&instr.as_slice());
				Ok(reg)
			}
			Term::Ident(name) => {
				let thing = if let Some(reg) = self.args.get(name) {
					let reg_id = reg.to_id();
					let target = self.get_free_register();
					self.emit(&[LOAD_ARGUMENT as u8, reg_id, target.to_id()]);
					target
				} else if let Some(reg) = self.locals.get(name) {
					let reg_id = reg.to_id();
					let target = self.get_free_register();
					self.emit(&[LOAD_LOCAL as u8, reg_id, target.to_id()]);
					target
				} else {
					return Err(format!("Unknown identifier: {}", name));
				};
				for follow in follows {
					let follow = &follow.elem;
					match follow {
						Follow::Field(_kind, name) => {
							let string_id =
								unsafe { value::Value::from_string(name).value.data.id } as u16;

							let mut bytes = vec![GET_FIELD as u8, thing.to_id()];
							bytes.extend(&string_id.to_le_bytes());
							bytes.push(thing.to_id());

							self.emit(&bytes.as_slice())
						}
						_ => return Err(format!("Unimplemented follow: {:#?}", follow)),
					}
				}
				Ok(thing)
			}
			Term::Expr(e) => self.visit_expression(e),
			_ => return Err(format!("Unimplemented term: {:#?}", term)),
		}
	}

	fn visit_if(
		&mut self,
		arms: &'a Vec<(Spanned<Expression>, Vec<Spanned<Statement>>)>,
		else_arm: &'a Option<Vec<Spanned<Statement>>>,
	) -> Result<(), String> {
		let mut patch_after_else: Vec<usize> = Vec::with_capacity(arms.len());
		for &(ref condition, ref block) in arms.iter() {
			let check_reg = self.visit_expression(&condition.elem)?;
			self.emit(&[JUMP_FALSE as u8, check_reg.to_id(), 0x00, 0x00, 0x00, 0x00]);
			let jump_location = self.bytecode.len() - 4;
			self.visit_block(block)?;
			if else_arm.is_some() {
				self.emit(&[JUMP as u8, 0x00, 0x00, 0x00, 0x00]);
				patch_after_else.push(self.bytecode.len() - 4);
			}
			let target = self.bytecode.len().to_le_bytes();
			for i in 0..4 {
				self.bytecode[jump_location + i] = target[i];
			}
		}
		if let Some(else_arm) = else_arm {
			self.visit_block(else_arm)?;
			let target = self.bytecode.len().to_le_bytes();
			for patch in patch_after_else {
				for i in 0..4 {
					self.bytecode[patch + i] = target[i];
				}
			}
		}
		Ok(())
	}

	fn visit_var(&mut self, var: &'a VarStatement) -> Result<(), String> {
		let local_id = self.locals.len();
		self.locals.insert(var.name.clone(), local_id.into());
		if let Some(ref expr) = var.value.as_ref() {
			let src_reg = self.visit_expression(expr)?;
			self.emit(&[STORE_LOCAL as u8, src_reg.to_id(), local_id as u8])
		}
		Ok(())
	}
}

pub fn compile<S: AsRef<str>>(procpath: S) -> String {
	let context = dm::Context::default();
	let env = dm::detect_environment_default()
		.expect("error detecting .dme")
		.expect("no .dme found");
	let pp = dm::preprocessor::Preprocessor::new(&context, env).expect("i/o error opening .dme");
	let indents = dm::indents::IndentProcessor::new(&context, pp);
	let mut parser = dm::parser::Parser::new(&context, indents);
	parser.enable_procs();
	let ot = parser.parse_object_tree();

	let mut has_proc = false;
	let mut ass = procpath
		.as_ref()
		.split("/")
		.skip(1)
		.map(|part| {
			if part == "proc" || part == "verb" {
				has_proc = true
			}
			(PathOp::Slash, part)
		})
		.collect::<Vec<(PathOp, &str)>>();
	if !has_proc {
		ass.insert(ass.len() - 1, (PathOp::Slash, "proc"));
	}

	let proc = match ot.root().navigate_path(&ass) {
		Some(ProcPath(xd, _kind)) => xd,
		_ => panic!("Proc not found"),
	};

	if let dm::objtree::Code::Present(ref code) = proc.code {
		let mut c = Compiler::new(&proc);
		return format!("{:#?}", c.compile());
	}

	"yeet".to_owned()
}
