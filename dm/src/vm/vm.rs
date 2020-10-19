extern crate byteorder;
use crate::proc;
use byteorder::{LittleEndian, ReadBytesExt};
use std::collections::HashMap;
use std::io::Cursor;
#[derive(Debug, PartialEq)]
#[repr(u8)]
pub enum Opcode {
	HALT,
	LOAD_IMMEDIATE,
	LOAD_ARGUMENT,
	ADD,
	SUB,
	MUL,
	DIV,
	PUSH,
	CALL,
	RETURN,
	INVALID,
}

impl From<u8> for Opcode {
	fn from(b: u8) -> Self {
		if b < Opcode::INVALID as u8 {
			unsafe { std::mem::transmute(b) }
		} else {
			Opcode::INVALID
		}
	}
}

impl From<Opcode> for u8 {
	fn from(opcode: Opcode) -> Self {
		opcode as u8
	}
}

type VType = u32;
type VValue = u32;
#[derive(Clone, Copy, Debug)]
pub struct Register {
	pub tag: VType,
	pub value: VValue,
}

impl Register {
	pub fn new(tag: u32, value: u32) -> Self {
		Self { tag, value }
	}
}

impl PartialEq for Register {
	fn eq(&self, other: &Register) -> bool {
		self.tag == other.tag && self.value == other.value
	}
}

impl Default for Register {
	fn default() -> Self {
		Self { tag: 0, value: 0 }
	}
}

const NUM_REGISTERS: usize = 16;
#[derive(Debug)]
pub struct Process {
	pub registers: [Register; NUM_REGISTERS],
	cursor: Cursor<Vec<u8>>,
	args: Vec<Register>,
	pid: u32,
	return_register_id: usize,
	call_arg_stack: Vec<Register>,
}

#[derive(Debug)]
pub struct VM {
	bytecodes: HashMap<u32, Vec<u8>>,
	programs: HashMap<u32, Process>,
	current_pid: u32,
}

enum MathOp {
	Add,
	Sub,
	Mul,
	Div,
}

impl VM {
	pub fn new() -> Self {
		Self {
			bytecodes: HashMap::new(),
			programs: HashMap::new(),
			current_pid: 0,
		}
	}

	pub fn run_program(&mut self, id: u32, args: Vec<Register>) -> Register {
		if self.bytecodes.contains_key(&id) {
			let mut prog = Process::new(self.current_pid, self.bytecodes[&id].clone(), args);
			prog.execute(self);
			prog.get_return_value()
		} else {
			proc::get_proc_by_id(id).unwrap()
		}
	}

	pub fn add_program(&mut self, id: u32, bytecode: Vec<u8>) {
		self.bytecodes.insert(id, bytecode);
	}
}

impl Process {
	pub fn new(pid: u32, bytecode: Vec<u8>, args: Vec<Register>) -> Self {
		Self {
			registers: [Register::default(); NUM_REGISTERS],
			cursor: Cursor::new(bytecode),
			args,
			pid,
			return_register_id: 0,
			call_arg_stack: Vec::new(),
		}
	}

	fn next_opcode(&mut self) -> Opcode {
		Opcode::from(self.next_byte())
	}

	fn next_byte(&mut self) -> u8 {
		self.cursor.read_u8().unwrap()
	}

	fn read_register(&mut self) -> usize {
		self.next_byte() as usize
	}

	pub fn get_return_value(&mut self) -> Register {
		self.registers[self.return_register_id].clone()
	}

	fn read_type(&mut self) -> VType {
		self.next_byte() as VType
	}

	fn read_value(&mut self) -> VValue {
		self.cursor.read_u32::<LittleEndian>().unwrap() as VValue
	}

	fn do_math_op(&mut self, op: MathOp) {
		let lefti = self.read_register();
		let righti = self.read_register();
		let desti = self.read_register();

		let left = f32::from_bits(self.registers[lefti].value);
		let right = f32::from_bits(self.registers[righti].value);

		let result = (match op {
			MathOp::Add => left + right,
			MathOp::Sub => left - right,
			MathOp::Mul => left * right,
			MathOp::Div => left / right,
		})
		.to_bits();

		let dest = &mut self.registers[desti];
		dest.tag = 0x2A;
		dest.value = result;
	}

	pub fn execute_one(&mut self, vm: &mut VM) -> Result<(), ()> {
		let op = self.next_opcode();
		match op {
			Opcode::LOAD_IMMEDIATE => {
				let reg_idx = self.read_register();
				let typ = self.read_type();
				let val = self.read_value();

				let reg = &mut self.registers[reg_idx];
				reg.tag = typ;
				reg.value = val;
			}
			Opcode::LOAD_ARGUMENT => {
				let arg_index = self.read_register();
				let dest_index = self.read_register();

				let arg = &self.args[arg_index];
				let dest = &mut self.registers[dest_index];

				dest.tag = arg.tag;
				dest.value = arg.value;
			}
			Opcode::ADD => self.do_math_op(MathOp::Add),
			Opcode::SUB => self.do_math_op(MathOp::Sub),
			Opcode::MUL => self.do_math_op(MathOp::Mul),
			Opcode::DIV => self.do_math_op(MathOp::Div),
			Opcode::PUSH => {
				let arg_idx = self.read_register();
				self.call_arg_stack.push(self.registers[arg_idx].clone());
			}
			Opcode::CALL => {
				let args = self.call_arg_stack.clone();
				self.call_arg_stack.clear();

				let proc_id = self.read_value() as u32;
				let result_register = self.read_register();

				let result = vm.run_program(proc_id, args);
				let r = &mut self.registers[result_register];
				r.tag = result.tag;
				r.value = result.value;
			}
			Opcode::RETURN => {
				self.return_register_id = self.read_register();
			}
			_ => return Err(()),
		}
		Ok(())
	}

	pub fn execute(&mut self, vm: &mut VM) -> Result<(), ()> {
		loop {
			self.execute_one(vm)?
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_create_process() {
		let test_process = Process::new(0, vec![], vec![]);
		assert_eq!(test_process.registers[0], Register::default());
		assert_eq!(test_process.cursor.position(), 0);
		assert_eq!(test_process.cursor.get_ref().len(), 0);
	}

	#[test]
	fn test_load_program() {
		let test_process = Process::new(
			0,
			vec![
				Opcode::LOAD_IMMEDIATE as u8,
				0,
				0x2A,
				0x3F,
				0x80,
				0x00,
				0x00,
				Opcode::HALT as u8,
			],
			vec![],
		);
		assert_eq!(test_process.cursor.get_ref().len(), 8);
	}

	#[test]
	fn test_execute_one() {
		let mut vm = VM::new();
		let mut test_process = Process::new(
			0,
			vec![
				Opcode::LOAD_IMMEDIATE as u8,
				0,
				0x2A,
				0x00,
				0x00,
				0x80,
				0x3F,
				Opcode::HALT as u8,
			],
			vec![],
		);
		assert!(test_process.execute_one(&mut vm).is_ok());
		assert_eq!(test_process.cursor.position(), 7);

		let first_register = &test_process.registers[0];
		assert_eq!(first_register.tag, 0x2A);
		assert_eq!(f32::from_bits(first_register.value), 1.0);
	}

	#[test]
	fn test_add() {
		let mut vm = VM::new();
		let mut test_process = Process::new(
			0,
			vec![
				Opcode::LOAD_IMMEDIATE as u8,
				0,
				0x2A,
				0x00,
				0x00,
				0x80,
				0x3F,
				Opcode::LOAD_IMMEDIATE as u8,
				1,
				0x2A,
				0x00,
				0x00,
				0x80,
				0x3F,
				Opcode::ADD as u8,
				0,
				1,
				2,
				Opcode::HALT as u8,
			],
			vec![],
		);
		assert!(test_process.execute_one(&mut vm).is_ok());
		assert!(test_process.execute_one(&mut vm).is_ok());
		assert!(test_process.execute_one(&mut vm).is_ok());

		let result_register = &test_process.registers[2];
		assert_eq!(result_register.tag, 0x2A);
		assert_eq!(f32::from_bits(result_register.value), 2.0);

		println!("{:#?}", test_process);
	}
}
