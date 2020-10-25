extern crate byteorder;
use crate::proc;
use crate::raw_types;
use crate::raw_types::strings::StringId;
use crate::raw_types::values::Value;
use byteorder::{LittleEndian, ReadBytesExt};
use std::collections::HashMap;
use std::io::Cursor;

/// Each opcode is one byte. They may be followed by zero or more operands.
/// Operands that are more than 1 byte are stored in little-endian format.
///
/// #### Sizes:
/// - Register: 1 byte
/// - Type: 1 byte
/// - Immediate value: 4 bytes
///
/// #### Register types:
/// - General purpose: Temporarily hold the results of intermediate calculations.
/// - Argument: Store the arguments with which the proc was invoked.
/// - Local: Store local variables.
/// All registers have a type and a value field, mirroring [value::Value].
#[derive(Debug, PartialEq)]
#[repr(u8)]
pub enum Opcode {
	/// Stops the virtual machine.
	HALT,
	/// `[destination register, type, value]`\
	/// Loads an immediate into a register.
	LOAD_IMMEDIATE,
	/// `[argument register, destination register]`\
	/// Loads an argument into a register.
	LOAD_ARGUMENT,
	/// `[local register, destination register]`\
	/// Loads a local into a register.
	LOAD_LOCAL,
	/// `[source register, local register]`\
	/// Stores a value in a local register.
	STORE_LOCAL,
	GET_FIELD,
	SET_FIELD,
	/// `[left register, right register, result register]`\
	/// This and the next 3 opcodes perform mathematical operations on left and
	/// right registers and store the result in the result register.
	ADD,
	SUB,
	MUL,
	DIV,
	/// `[left register, right register, result register]`\
	/// This and the next 4 opcodes compare the left and
	/// right registers and store the result in the result register.
	LESS_THAN,
	LESS_OR_EQUAL,
	EQUAL,
	GREATER_OR_EQUAL,
	GREATER_THAN,
	/// `[immediate destination]`\
	/// Unconditionally jumps to the destination.
	JUMP,
	/// `[condition register, immediate destination]`\
	/// Jumps to the destination if the condition register's value is NOT equal to zero.
	JUMP_TRUE,
	/// `[condition register, immediate destination]`\
	/// Jumps to the destination if the condition register's value is equal to zero.
	JUMP_FALSE,
	/// `[source register]`\
	/// Pushes the source register's contents onto the argument stack, in order to be passed to a called function.
	PUSH,
	/// `[proc id register, result register]`\
	/// Calls a proc with the given proc id. Passes arguments specified with [Opcode::PUSH], saves the return value to the return register.
	CALL,
	/// `[return value register]`\
	/// Returns the value in the specified register to the caller.
	RETURN,
	/// Coming across an invalid opcode means we screwed something up and need to bail.
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

impl From<Register> for Value {
	fn from(register: Register) -> Self {
		unsafe { std::mem::transmute(register) }
	}
}

impl From<Value> for Register {
	fn from(v: Value) -> Self {
		unsafe { std::mem::transmute(v) }
	}
}

type VType = u32;
type VValue = u32;

/// Contains a type tag and a value. See [value::Value].
#[derive(Clone, Copy, Debug)]
pub struct Register {
	pub tag: VType,
	pub value: VValue,
}

impl Register {
	pub fn new(tag: u32, value: u32) -> Self {
		Self { tag, value }
	}

	pub fn assign(&mut self, other: &Self) {
		self.tag = other.tag;
		self.value = other.value;
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

impl From<&Register> for f32 {
	fn from(register: &Register) -> Self {
		f32::from_bits(register.value)
	}
}

const NUM_REGISTERS: usize = 16;

#[derive(Debug)]
pub struct Process {
	pub registers: [Register; NUM_REGISTERS],
	cursor: Cursor<Vec<u8>>,
	args: Vec<Register>,
	locals: [Register; NUM_REGISTERS],
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
			let value_args = args
				.iter()
				.map(|a| unsafe {
					crate::value::Value::new(
						std::mem::transmute(a.tag as u8),
						std::mem::transmute(a.value),
					)
				})
				.collect::<Vec<crate::value::Value>>();
			let fuck: Vec<_> = value_args.iter().map(|v| v).collect();
			let res = proc::get_proc_by_id(id)
				.unwrap()
				.call(fuck.as_slice())
				.unwrap();
			Register {
				tag: res.value.tag as u32,
				value: unsafe { res.value.data.id },
			}
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
			locals: [Register::default(); NUM_REGISTERS], //16 locals max for now
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

	fn read_short(&mut self) -> u16 {
		self.cursor.read_u16::<LittleEndian>().unwrap()
	}

	fn compare(&self, left: &Register, right: &Register, op: Opcode) -> bool {
		let left: f32 = left.into();
		let right: f32 = right.into();

		match op {
			Opcode::LESS_THAN => left < right,
			Opcode::LESS_OR_EQUAL => left <= right,
			Opcode::EQUAL => left == right,
			Opcode::GREATER_OR_EQUAL => left >= right,
			Opcode::GREATER_THAN => left > right,
			_ => unreachable!("Invalid opcode passed to compare"),
		}
	}

	fn do_math_op(&mut self, op: Opcode) {
		let lefti = self.read_register();
		let righti = self.read_register();
		let desti = self.read_register();

		let left = f32::from_bits(self.registers[lefti].value);
		let right = f32::from_bits(self.registers[righti].value);

		let result = (match op {
			Opcode::ADD => left + right,
			Opcode::SUB => left - right,
			Opcode::MUL => left * right,
			Opcode::DIV => left / right,
			_ => unreachable!("Invalid opcode passed to do_math_op"),
		})
		.to_bits();

		let dest = &mut self.registers[desti];
		dest.tag = 0x2A;
		dest.value = result;
	}

	pub fn execute_one(&mut self, vm: &mut VM) -> Result<(), ()> {
		use Opcode::*;
		let op = self.next_opcode();
		match op {
			LOAD_IMMEDIATE => {
				let reg_idx = self.read_register();
				let typ = self.read_type();
				let val = self.read_value();

				let reg = &mut self.registers[reg_idx];
				reg.tag = typ;
				reg.value = val;
			}
			LOAD_ARGUMENT => {
				let arg_index = self.read_register();
				let dest_index = self.read_register();

				let arg = &self.args[arg_index];
				let dest = &mut self.registers[dest_index];

				dest.assign(arg);
			}
			LOAD_LOCAL => {
				let local_index = self.read_register();
				let dest_index = self.read_register();

				let local = &self.locals[local_index];
				let dest = &mut self.registers[dest_index];

				dest.assign(local);
			}
			STORE_LOCAL => {
				let dest_index = self.read_register();
				let local_index = self.read_register();

				let local = &mut self.locals[local_index];
				let dest = &self.registers[dest_index];

				local.assign(dest);
			}
			GET_FIELD => {
				let source_index = self.read_register();
				let field_name = self.read_short();
				let destination_index = self.read_register();

				let source = self.registers[source_index].clone();
				let mut out = raw_types::values::Value {
					tag: raw_types::values::ValueTag::Null,
					data: raw_types::values::ValueData { id: 0 },
				};
				unsafe {
					crate::raw_types::funcs::get_variable(
						&mut out,
						source.into(),
						StringId(field_name as u32),
					);
				}
				self.registers[destination_index] = out.into();
			}
			ADD | SUB | MUL | DIV => self.do_math_op(op),
			LESS_THAN | LESS_OR_EQUAL | EQUAL | GREATER_OR_EQUAL | GREATER_THAN => {
				let left = self.read_register();
				let right = self.read_register();
				let result = self.read_register();

				let left = self.registers[left].clone();
				let right = self.registers[right].clone();

				let res = if self.compare(&left, &right, op) {
					f32::to_bits(1.0)
				} else {
					f32::to_bits(0.0)
				};

				let result = &mut self.registers[result];
				result.tag = 0x2A;
				result.value = res;
			}
			JUMP => {
				let dest = self.read_value();
				self.cursor.set_position(dest as u64);
			}
			JUMP_TRUE => {
				let reg = self.read_register();
				let dest = self.read_value();
				if self.registers[reg].value != 0 {
					self.cursor.set_position(dest as u64);
				}
			}
			JUMP_FALSE => {
				let reg = self.read_register();
				let dest = self.read_value();
				if self.registers[reg].value == 0 {
					self.cursor.set_position(dest as u64);
				}
			}
			PUSH => {
				let arg_idx = self.read_register();
				self.call_arg_stack.push(self.registers[arg_idx].clone());
			}
			CALL => {
				let args = self.call_arg_stack.clone();
				self.call_arg_stack.clear();

				let proc_id = self.read_value() as u32;
				let result_register = self.read_register();

				let result = vm.run_program(proc_id, args);
				let r = &mut self.registers[result_register];
				r.tag = result.tag;
				r.value = result.value;
			}
			RETURN => {
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
