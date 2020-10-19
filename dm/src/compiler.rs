extern crate dreammaker as dm;

use dm::ast::*;
use dm::objtree::ProcRef;

use dm::ast::PathOp;
use dm::objtree::NavigatePathResult::ProcPath;

use crate::vm::vm as vmhook;
use vmhook::Opcode::*;

pub struct Compiler<'a> {
	proc: &'a ProcRef<'a>,

	bytecode: Vec<u8>,
	next_free_register: usize,
}

impl<'a> Compiler<'a> {
	fn new(proc: &'a ProcRef<'a>) -> Self {
		Self {
			proc,
			bytecode: Vec::new(),
			next_free_register: 0,
		}
	}

	fn get_free_register(&mut self) -> usize {
		let ret = self.next_free_register;
		self.next_free_register += 1;
		ret
	}

	pub fn emit<U: Into<u8> + Copy>(&mut self, bytes: &[U]) {
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

	fn visit_statement(&mut self, statement: &'a Statement) -> Result<usize, String> {
		return match statement {
			Statement::Expr(expr) => self.visit_expression_statement(expr),
			Statement::Return(Some(expr)) => {
				let return_reg = self.visit_expression(expr)?;
				self.emit(&[RETURN as u8, return_reg as u8]);
				Ok(0)
			}
			_ => Err("fuck".to_owned()),
		};
	}

	fn visit_expression(&mut self, expr: &'a Expression) -> Result<usize, String> {
		self.visit_expression_impl(expr, false)
	}

	fn visit_expression_statement(&mut self, expr: &'a Expression) -> Result<usize, String> {
		self.visit_expression_impl(expr, true)
	}

	pub fn visit_expression_impl(
		&mut self,
		expr: &'a Expression,
		is_statement: bool,
	) -> Result<usize, String> {
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
					_ => panic!("Binop not implemented"),
				};

				let result_reg = self.get_free_register();
				self.emit(&[
					oper as u8,
					left_reg as u8,
					right_reg as u8,
					result_reg as u8,
				]);
				return Ok(result_reg);
			}
			_ => return Err(format!("Unimplemented expression: {:?}", expr)),
		}
	}

	pub fn visit_term(
		&mut self,
		term: &'a Term,
		follow: &'a Vec<Spanned<Follow>>,
		is_statement: bool,
	) -> Result<usize, String> {
		match term {
			Term::Int(number) => {
				let reg = self.get_free_register();

				let mut instr = vec![LOAD_IMMEDIATE as u8, reg as u8, 0x2A];
				instr.extend((*number as f32).to_le_bytes().iter());

				self.emit(&instr.as_slice());
				Ok(reg)
			}
			Term::Float(number) => {
				let reg = self.next_free_register;
				self.next_free_register += 1;

				let mut instr = vec![LOAD_IMMEDIATE as u8, reg as u8, 0x2A];
				instr.extend(number.to_le_bytes().iter());

				self.emit(&instr.as_slice());
				Ok(reg)
			}
			_ => return Err("wtf".to_owned()),
		}
	}
}

pub fn whatever() -> String {
	let context = dm::Context::default();
	let env = dm::detect_environment_default()
		.expect("error detecting .dme")
		.expect("no .dme found");
	let pp = dm::preprocessor::Preprocessor::new(&context, env).expect("i/o error opening .dme");
	let indents = dm::indents::IndentProcessor::new(&context, pp);
	let mut parser = dm::parser::Parser::new(&context, indents);
	parser.enable_procs();
	let ot = parser.parse_object_tree();

	let procpath = "/proc/do_sum";

	let mut has_proc = false;
	let mut ass = procpath
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
		return format!("{:?}", c.compile());
	}

	"yeet".to_owned()
}
