#![allow(clippy::identity_op)]

use std::collections::{hash_map, HashMap};
use std::hash::{BuildHasherDefault, Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use nohash_hasher::NoHashHasher;

use crate::util;
use crate::bits;
use crate::dbg_println;
use crate::util::U128_SIZE;

use std::collections::hash_map::DefaultHasher;

// Types
// -----

// A native HVM term
#[derive(Clone, Debug, PartialEq)]
pub enum Term {
  Var { name: u128 },
  Dup { nam0: u128, nam1: u128, expr: Box<Term>, body: Box<Term> },
  Lam { name: u128, body: Box<Term> },
  App { func: Box<Term>, argm: Box<Term> },
  Ctr { name: u128, args: Vec<Term> },
  Fun { name: u128, args: Vec<Term> },
  Num { numb: u128 },
  Op2 { oper: u128, val0: Box<Term>, val1: Box<Term> },
}

// A native HVM 60-bit machine integer operation
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Oper {
  Add, Sub, Mul, Div,
  Mod, And, Or,  Xor,
  Shl, Shr, Ltn, Lte,
  Eql, Gte, Gtn, Neq,
}

// A
//[(Term,Term)]

// A u64 HashMap
pub type Map<A> = HashMap<u64, A, BuildHasherDefault<NoHashHasher<u64>>>;

// A left-hand side variable in a rewrite rule (equation)
#[derive(Clone, Debug, PartialEq)]
pub struct Var {
  pub name : u128,         // this variable's name
  pub param: u128,         // in what parameter is this variable located?
  pub field: Option<u128>, // in what field is this variable located? (if any)
  pub erase: bool,         // should this variable be collected (because it is unused)?
}

// A rewrite rule (equation)
pub type Rule = (Term, Term);

// A function (vector of rules)
pub type Func = Vec<(Term, Term)>;

// A compiled rewrite rule
#[derive(Clone, Debug, PartialEq)]
pub struct CompRule {
  pub cond: Vec<Lnk>,          // left-hand side matching conditions
  pub vars: Vec<Var>,          // left-hand side variable locations
  pub eras: Vec<(u128, u128)>, // must-clear locations (argument number and arity)
  pub body: Term,              // right-hand side body of rule
}

// A compiled function
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CompFunc {
  func: Func,           // the original function
  arity: u128,          // number of arguments
  redux: Vec<u128>,     // index of strict arguments
  rules: Vec<CompRule>, // vector of rules
}

// A file is a map of `FuncID -> Function`
#[derive(Clone, Debug)]
pub struct File {
  pub funcs: Map<Arc<CompFunc>>,
}

// A map of `FuncID -> Arity`
#[derive(Clone, Debug)]
pub struct Arit {
  pub arits: Map<u128>,
}

// A map of `FuncID -> Lnk`, pointing to a function's state
#[derive(Clone, Debug)]
pub struct Disk {
  pub links: Map<Lnk>,
}

// Can point to a node, a variable, or hold an unboxed value
pub type Lnk = u128;

// A global statement that alters the state of the blockchain
pub enum Statement {
  Fun { name: u128, args: Vec<u128>, func: Vec<(Term, Term)>, init: Term },
  Ctr { name: u128, args: Vec<u128>, },
  Run { expr: Term },
}

// A mergeable vector of u128 values
#[derive(Debug, Clone)]
pub struct Blob {
  data: Vec<u128>,
  used: Vec<usize>,
}

// HVM's memory state (nodes, functions, metadata, statistics)
#[derive(Debug)]
pub struct Heap {
  pub uuid: u128, // unique identifier
  pub blob: Blob, // memory block holding HVM nodes
  pub disk: Disk, // points to stored function states
  pub file: File, // function codes
  pub arit: Arit, // function arities
  pub tick: u128, // time counter
  pub funs: u128, // total function count
  pub dups: u128, // total dups count
  pub rwts: u128, // total graph rewrites
  pub mana: u128, // total mana cost
  pub size: i128, // total used memory (in 64-bit words)
  pub next: u128, // memory index that *may* be empty
}

// A serialized Heap
pub struct SerializedHeap {
  pub uuid: u128,
  pub blob: Vec<u128>,
  pub disk: Vec<u128>,
  pub file: Vec<u128>,
  pub arit: Vec<u128>,
  pub nums: Vec<u128>,
}

// A list of past heap states, for block-reorg rollback
// FIXME: this should be replaced by a much simpler index array
#[derive(Debug)]
pub enum Rollback {
  Cons {
    keep: u64,
    head: u64,
    tail: Arc<Rollback>,
  },
  Nil
}

// The current and past states
pub struct Runtime {
  heap: Vec<Heap>,      // heap objects
  draw: u64,            // drawing heap index
  curr: u64,            // current heap index
  nuls: Vec<u64>,       // reuse heap indices
  back: Arc<Rollback>,  // past states
}

pub fn heaps_invariant(rt: &Runtime) -> (bool, Vec<u8>, Vec<u64>) {
  let mut seen = vec![0u8; 10];
  let mut heaps = vec![0u64; 0];
  let mut push = |id: u64| {
    let idx = id as usize;
    seen[idx] += 1;
    heaps.push(id);
  };
  push(rt.draw);
  push(rt.curr);
  for nul in &rt.nuls {
    push(*nul);
  }
  {
    let mut back = &*rt.back;
    while let Rollback::Cons { keep, head, tail } = back {
      push(*head);
      back = &*tail;
    }
  }
  let failed = seen.iter().all(|c| *c == 1);
  (failed, seen, heaps)
}

// Constants
// ---------

const U128_PER_KB: u128 = (1024 / U128_SIZE) as u128;
const U128_PER_MB: u128 = U128_PER_KB << 10;
const U128_PER_GB: u128 = U128_PER_MB << 10;

const HEAP_SIZE: u128 = 32 * U128_PER_MB;

pub const MAX_ARITY: u128 = 16;
pub const MAX_FUNCS: u128 = 1 << 24; // TODO: increase to 2^30 once arity is moved out

pub const VARS_SIZE: usize = 1 << 18; // maximum variables per rule

pub const VAL: u128 = 1 << 0;
pub const EXT: u128 = 1 << 60;
pub const TAG: u128 = 1 << 120;

pub const VAL_MASK: u128 = EXT - 1;
pub const EXT_MASK: u128 = (TAG - 1)   ^ VAL_MASK;
pub const TAG_MASK: u128 = (u128::MAX) ^ EXT_MASK;
pub const NUM_MASK: u128 = EXT_MASK | VAL_MASK;

pub const DP0: u128 = 0x0;
pub const DP1: u128 = 0x1;
pub const VAR: u128 = 0x2;
pub const ARG: u128 = 0x3;
pub const ERA: u128 = 0x4;
pub const LAM: u128 = 0x5;
pub const APP: u128 = 0x6;
pub const SUP: u128 = 0x7;
pub const CTR: u128 = 0x8;
pub const FUN: u128 = 0x9;
pub const OP2: u128 = 0xA;
pub const NUM: u128 = 0xB;

pub const ADD: u128 = 0x0;
pub const SUB: u128 = 0x1;
pub const MUL: u128 = 0x2;
pub const DIV: u128 = 0x3;
pub const MOD: u128 = 0x4;
pub const AND: u128 = 0x5;
pub const OR : u128 = 0x6;
pub const XOR: u128 = 0x7;
pub const SHL: u128 = 0x8;
pub const SHR: u128 = 0x9;
pub const LTN: u128 = 0xA;
pub const LTE: u128 = 0xB;
pub const EQL: u128 = 0xC;
pub const GTE: u128 = 0xD;
pub const GTN: u128 = 0xE;
pub const NEQ: u128 = 0xF;

pub const VAR_NONE  : u128 = 0x3FFFF;
pub const U128_NONE : u128 = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF;
pub const I128_NONE : i128 = -0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF;

// (IO r:Type) : Type
//   (IO.done retr:r)                                       : (IO r)
//   (IO.laod               cont:(∀? (IO r))) : (IO r)
//   (IO.save expr:?        cont:(∀? (IO r))) : (IO r)
//   (IO.call expr:? args:? cont:(∀? (IO r))) : (IO r)
//   (IO.from               cont:(∀? (IO r))) : (IO r)
const IO_DONE : u128 = 0x13640a33ca9;
const IO_TAKE : u128 = 0x13640e25be9;
const IO_SAVE : u128 = 0x13640de5ea9;
const IO_CALL : u128 = 0x136409e5c30;
const IO_FROM : u128 = 0x13640ab6cf1;

const fn GET_ARITY(fid: u128) -> Option<u128> {
  match fid {
    IO_DONE => Some(1),
    IO_TAKE => Some(1),
    IO_SAVE => Some(2),
    IO_CALL => Some(3),
    IO_FROM => Some(1),
    _       => None,
  }
}

// Maximum mana that can be spent in a block
pub const BLOCK_MANA_LIMIT : u128 = 42_000_000_000;

// Maximum state growth per block, in bits
pub const BLOCK_BITS_LIMIT : i128 = 1024; // 1024 bits per sec = about 4 GB per year

// Mana Table
// ----------

// |---------|---------------------------------|----------|
// | Opcode  | Effect                          | Mana     |
// |---------|---------------------------------|----------|
// | APP-LAM | applies a lambda                | 10       |
// | APP-SUP | applies a superposition         | 20       |
// | OP2-NUM | operates on a number            | 10       |
// | OP2-SUP | operates on a superposition     | 20       |
// | FUN-CTR | pattern-matches a constructor   | 10 + M   |
// | FUN-SUP | pattern-matches a superposition | 10 + A*5 |
// | DUP-LAM | clones a lambda                 | 20       |
// | DUP-NUM | clones a number                 | 10       |
// | DUP-CTR | clones a constructor            | 10 + A*5 |
// | DUP-SUP | clones a superposition          | 20       |
// | DUP-SUP | undoes a superposition          | 10       |
// | DUP-ERA | clones an erasure               | 10       |
// |------------------------------------------------------|
// | * A is the constructor or function arity             |
// | * M is the alloc count of the right-hand side        |
// |------------------------------------------------------|


fn AppLamMana() -> u128 {
  return 10;
}

fn AppSupMana() -> u128 {
  return 20; // 19?
}

fn Op2NumMana() -> u128 {
  return 10;
}

fn Op2SupMana() -> u128 {
  return 20; // 19?
}

fn FunCtrMana(body: &Term) -> u128 {
  return 10 + count_allocs(body) * 5;
}

fn FunSupMana(arity: u128) -> u128 {
  return 10 + arity * 5;
}

fn DupLamMana() -> u128 {
  return 20; // 19?
}

fn DupNumMana() -> u128 {
  return 10;
}

fn DupCtrMana(arity: u128) -> u128 {
  return 10 + arity * 5;
}

fn DupDupMana() -> u128 {
  return 20;
}

fn DupSupMana() -> u128 {
  return 10;
}

fn DupEraMana() -> u128 {
  return 10;
}

fn count_allocs(body: &Term) -> u128 {
  match body {
    Term::Var { name } => {
      0
    }
    Term::Dup { nam0, nam1, expr, body } => {
      let expr = count_allocs(expr);
      let body = count_allocs(body);
      3 + expr + body
    }
    Term::Lam { name, body } => {
      let body = count_allocs(body);
      2 + body
    }
    Term::App { func, argm } => {
      let func = count_allocs(func);
      let argm = count_allocs(argm);
      2 + func + argm
    }
    Term::Fun { name, args } => {
      let size = args.len() as u128;
      let mut count = 0;
      for (i, arg) in args.iter().enumerate() {
        count += count_allocs(arg);
      }
      size + count
    }
    Term::Ctr { name, args } => {
      let size = args.len() as u128;
      let mut count = 0;
      for (i, arg) in args.iter().enumerate() {
        count += count_allocs(arg);
      }
      size + count
    }
    Term::Num { numb } => {
      0
    }
    Term::Op2 { oper, val0, val1 } => {
      let val0 = count_allocs(val0);
      let val1 = count_allocs(val1);
      2 + val0 + val1
    }
  }
}

const GENESIS : &str = "
$(Tuple0)
$(Tuple1 x0)
$(Tuple2 x0 x1)
$(Tuple3 x0 x1 x2)
$(Tuple4 x0 x1 x2 x3)
$(Tuple5 x0 x1 x2 x3 x4)
$(Tuple6 x0 x1 x2 x3 x4 x5)
$(Tuple7 x0 x1 x2 x3 x4 x5 x6)
$(Tuple8 x0 x1 x2 x3 x4 x5 x6 x7)

!(IO.load cont) {
  !(IO.load cont) = $(IO.take @x &{x0 x1} = x; $(IO.save x0 @~ (cont x1)))
} = #0

$(Inc)
$(Get)
!(Count action) {
  !(Count $(Inc)) = $(IO.take @x $(IO.save (+ x #1) @~ $(IO.done #0)))
  !(Count $(Get)) = !(IO.load @x $(IO.done x))
} = #0
";

// Utils
// -----

fn init_map<A>() -> Map<A> {
  HashMap::with_hasher(BuildHasherDefault::default())
}

// Rollback
// --------

fn absorb_u128(a: u128, b: u128, overwrite: bool) -> u128 {
  if b == U128_NONE { a } else if overwrite || a == U128_NONE { b } else { a }
}

fn absorb_i128(a: i128, b: i128, overwrite: bool) -> i128 {
  if b == I128_NONE { a } else if overwrite || a == I128_NONE { b } else { a }
}

impl Heap {
  fn write(&mut self, idx: usize, val: u128) {
    return self.blob.write(idx, val);
  }
  fn read(&self, idx: usize) -> u128 {
    return self.blob.read(idx);
  }
  fn write_disk(&mut self, fid: u128, val: Lnk) {
    return self.disk.write(fid, val);
  }
  fn read_disk(&self, fid: u128) -> Option<Lnk> {
    return self.disk.read(fid);
  }
  fn write_file(&mut self, fid: u128, fun: Arc<CompFunc>) {
    return self.file.write(fid, fun);
  }
  fn read_file(&self, fid: u128) -> Option<Arc<CompFunc>> {
    return self.file.read(fid);
  }
  fn write_arit(&mut self, fid: u128, val: u128) {
    return self.arit.write(fid, val);
  }
  fn read_arit(&self, fid: u128) -> Option<u128> {
    return self.arit.read(fid);
  }
  fn set_tick(&mut self, tick: u128) {
    self.tick = tick;
  }
  fn get_tick(&self) -> u128 {
    return self.tick;
  }
  fn set_funs(&mut self, funs: u128) {
    self.funs = funs;
  }
  fn get_funs(&self) -> u128 {
    return self.funs;
  }
  fn set_dups(&mut self, dups: u128) {
    self.dups = dups;
  }
  fn get_dups(&self) -> u128 {
    return self.dups;
  }
  fn set_rwts(&mut self, rwts: u128) {
    self.rwts = rwts;
  }
  fn get_rwts(&self) -> u128 {
    return self.rwts;
  }
  fn set_mana(&mut self, mana: u128) {
    self.mana = mana;
  }
  fn get_mana(&self) -> u128 {
    return self.mana;
  }
  fn set_size(&mut self, size: i128) {
    self.size = size;
  }
  fn get_size(&self) -> i128 {
    return self.size;
  }
  fn set_next(&mut self, next: u128) {
    self.next = next;
  }
  fn get_next(&self) -> u128 {
    return self.next;
  }
  fn absorb(&mut self, other: &mut Self, overwrite: bool) {
    self.blob.absorb(&mut other.blob, overwrite);
    self.disk.absorb(&mut other.disk, overwrite);
    self.file.absorb(&mut other.file, overwrite);
    self.arit.absorb(&mut other.arit, overwrite);
    self.tick = absorb_u128(self.tick, other.tick, overwrite);
    self.funs = absorb_u128(self.funs, other.funs, overwrite);
    self.dups = absorb_u128(self.dups, other.dups, overwrite);
    self.rwts = absorb_u128(self.rwts, other.rwts, overwrite);
    self.mana = absorb_u128(self.mana, other.mana, overwrite);
    self.size = absorb_i128(self.size, other.size, overwrite);
    self.next = absorb_u128(self.next, other.next, overwrite);
  }
  fn clear(&mut self) {
    self.uuid = fastrand::u128(..);
    self.blob.clear();
    self.disk.clear();
    self.file.clear();
    self.arit.clear();
    self.tick = U128_NONE;
    self.funs = U128_NONE;
    self.dups = U128_NONE;
    self.rwts = U128_NONE;
    self.mana = U128_NONE;
    self.size = I128_NONE;
    self.next = U128_NONE;
  }
  fn serialize(&self) -> SerializedHeap {
    // Serializes Blob
    let mut blob_buff : Vec<u128> = vec![];
    for used_index in &self.blob.used {
      blob_buff.push(*used_index as u128);
      blob_buff.push(self.blob.data[*used_index]);
    }
    // Serializes Disk
    let mut disk_buff : Vec<u128> = vec![];
    for (fnid, lnk) in &self.disk.links {
      disk_buff.push(*fnid as u128);
      disk_buff.push(*lnk as u128);
    }
    // Serializes File
    let mut file_buff : Vec<u128> = vec![];
    for (fnid, func) in &self.file.funcs {
      let mut func_buff = util::u8s_to_u128s(&mut bits::serialized_func(&func.func).to_bytes());
      file_buff.push(*fnid as u128);
      file_buff.push(func_buff.len() as u128);
      file_buff.append(&mut func_buff);
    }
    // Serializes Arit
    let mut arit_buff : Vec<u128> = vec![];
    for (fnid, arit) in &self.arit.arits {
      arit_buff.push(*fnid as u128);
      arit_buff.push(*arit);
    }
    // Serializes Nums
    let mut nums_buff : Vec<u128> = vec![];
    nums_buff.push(self.tick);
    nums_buff.push(self.funs);
    nums_buff.push(self.dups);
    nums_buff.push(self.rwts);
    nums_buff.push(self.mana);
    nums_buff.push(self.size as u128);
    nums_buff.push(self.next);
    // Returns the serialized heap
    return SerializedHeap {
      uuid: self.uuid,
      blob: blob_buff,
      disk: disk_buff,
      file: file_buff,
      arit: arit_buff,
      nums: nums_buff,
    };
  }
  fn deserialize(&mut self, serial: &SerializedHeap) {
    // Deserializes Blob
    let mut i = 0;
    while i < serial.blob.len() {
      let idx = serial.blob[i + 0];
      let val = serial.blob[i + 1];
      self.write(idx as usize, val);
      i += 2;
    }
    // Deserializes Disk
    let mut i = 0;
    while i < serial.disk.len() {
      let fnid = serial.disk[i + 0];
      let lnk  = serial.disk[i + 1];
      self.write_disk(fnid, lnk);
      i += 2;
    }
    // Deserializes File
    let mut i = 0;
    while i < serial.file.len() {
      let fnid = serial.file[i * 2 + 0];
      let size = serial.file[i * 2 + 1];
      let buff = &serial.file[i * 2 + 2 .. i * 2 + 2 + size as usize];
      let func = build_func(&bits::deserialized_func(&bit_vec::BitVec::from_bytes(&util::u128s_to_u8s(&buff))),false).unwrap();
      self.write_file(fnid, Arc::new(func));
      i += 1;
    }
    // Deserializes Arit
    for i in 0 .. serial.arit.len() / 2 {
      let fnid = serial.file[i * 2 + 0];
      let arit = serial.file[i * 2 + 1];
      self.write_arit(fnid, arit);
    }
  }
  fn heap_path(&self) -> PathBuf {
    dirs::home_dir().unwrap().join(".kindelia").join("state").join("heap")
  }
  fn save_buffers(&self) -> std::io::Result<()> {
    self.append_buffers(self.uuid)
  }
  fn append_buffers(&self, uuid: u128) -> std::io::Result<()> {
    fn save_buffer(path: &PathBuf, uuid: u128, name: &str, buffer: &[u128]) -> std::io::Result<()> {
      use std::io::Write;
      let file_path = path.join(format!("{}.{}.bin", uuid, name));
      //format!("{}/{}.{}.bin", path, uuid, name);
      //println!("saving: {:?}", file_path);
      let mut file = std::fs::OpenOptions::new().append(true).create(true).open(file_path)?;
      file.write_all(&util::u128s_to_u8s(buffer))?;
      return Ok(());
    }
    let path = &self.heap_path();
    let serial = self.serialize();
    std::fs::create_dir_all(path)?;
    save_buffer(path, uuid, "blob", &serial.blob)?;
    save_buffer(path, uuid, "disk", &serial.disk)?;
    save_buffer(path, uuid, "file", &serial.file)?;
    save_buffer(path, uuid, "arit", &serial.arit)?;
    save_buffer(path, uuid, "nums", &serial.nums)?;
    return Ok(());
  }
  fn load_buffers(&mut self, uuid: u128) -> std::io::Result<()> {
    fn load_buffer(path: &PathBuf, uuid: u128, name: &str) -> std::io::Result<Vec<u128>> {
      let file_path = path.join(format!("{}.{}.bin", uuid, name));
      std::fs::read(file_path).map(|x| util::u8s_to_u128s(&x))
    }
    let path = &self.heap_path();
    self.deserialize(&SerializedHeap {
      uuid: uuid,
      blob: load_buffer(path, uuid, "blob")?,
      disk: load_buffer(path, uuid, "disk")?,
      file: load_buffer(path, uuid, "file")?,
      arit: load_buffer(path, uuid, "arit")?,
      nums: load_buffer(path, uuid, "nums")?,
    });
    return Ok(());
  }
  fn delete_buffers(&mut self) {
    // TODO
  }
}

pub fn init_heap() -> Heap {
  Heap {
    uuid: fastrand::u128(..),
    blob: init_heap_data(U128_NONE),
    disk: Disk { links: init_map() },
    file: File { funcs: init_map() },
    arit: Arit { arits: init_map() },
    tick: U128_NONE,
    funs: U128_NONE,
    dups: U128_NONE,
    rwts: U128_NONE,
    mana: U128_NONE,
    size: I128_NONE,
    next: U128_NONE,
  }
}

pub fn init_heap_data(zero: u128) -> Blob {
  return Blob {
    data: vec![zero; HEAP_SIZE as usize],
    used: vec![],
  };
}

impl Blob {
  fn write(&mut self, idx: usize, val: u128) {
    unsafe {
      let got = self.data.get_unchecked_mut(idx);
      if *got == U128_NONE {
        self.used.push(idx);
      }
      *got = val;
    }
  }
  fn read(&self, idx: usize) -> u128 {
    unsafe {
      return *self.data.get_unchecked(idx);
    }
  }
  fn clear(&mut self) {
    for idx in &self.used {
      unsafe {
        let val = self.data.get_unchecked_mut(*idx);
        *val = U128_NONE;
      }
    }
    self.used.clear();
  }
  fn absorb(&mut self, other: &mut Self, overwrite: bool) {
    for idx in &other.used {
      unsafe {
        let other_val = other.data.get_unchecked_mut(*idx);
        let self_val = self.data.get_unchecked_mut(*idx);
        if overwrite || *self_val == U128_NONE {
          self.write(*idx, *other_val);
        }
      }
    }
    other.clear();
  }
}

fn show_buff(vec: &[u128]) -> String {
  let mut result = String::new();
  for x in vec {
    if *x == U128_NONE {
      result.push_str("_ ");
    } else {
      result.push_str(&format!("{:x} ", *x));
    }
  }
  return result;
}

impl Disk {
  fn write(&mut self, fid: u128, val: Lnk) {
    self.links.insert(fid as u64, val);
  }
  fn read(&self, fid: u128) -> Option<Lnk> {
    self.links.get(&(fid as u64)).map(|x| *x)
  }
  fn clear(&mut self) {
    self.links.clear();
  }
  fn absorb(&mut self, other: &mut Self, overwrite: bool) {
    for (fid, func) in other.links.drain() {
      if overwrite || !self.links.contains_key(&fid) {
        self.write(fid as u128, func);
      }
    }
  }
}

impl File {
  fn write(&mut self, fid: u128, val: Arc<CompFunc>) {
    self.funcs.entry(fid as u64).or_insert(val);
  }
  fn read(&self, fid: u128) -> Option<Arc<CompFunc>> {
    return self.funcs.get(&(fid as u64)).map(|x| x.clone());
  }
  fn clear(&mut self) {
    self.funcs.clear();
  }
  fn absorb(&mut self, other: &mut Self, overwrite: bool) {
    for (fid, func) in other.funcs.drain() {
      if overwrite || !self.funcs.contains_key(&fid) {
        self.write(fid as u128, func.clone());
      }
    }
  }
}

impl Arit {
  fn write(&mut self, fid: u128, val: u128) {
    self.arits.entry(fid as u64).or_insert(val);
  }
  fn read(&self, fid: u128) -> Option<u128> {
    return self.arits.get(&(fid as u64)).map(|x| *x);
  }
  fn clear(&mut self) {
    self.arits.clear();
  }
  fn absorb(&mut self, other: &mut Self, overwrite: bool) {
    for (fid, arit) in other.arits.drain() {
      if overwrite || !self.arits.contains_key(&fid) {
        self.arits.insert(fid, arit);
      }
    }
  }
}

pub fn init_runtime() -> Runtime {
  let mut heap = Vec::new();
  for i in 0 .. 10 {
    heap.push(init_heap());
  }
  let mut rt = Runtime {
    heap,
    draw: 0,
    curr: 1,
    nuls: vec![2, 3, 4, 5, 6, 7, 8, 9],
    back: Arc::new(Rollback::Nil),
  };
  rt.run_statements_from_code(GENESIS);
  rt.snapshot();
  return rt;
}

impl Runtime {

  // API
  // ---

  pub fn define_function(&mut self, fid: u128, func: CompFunc) {
    self.get_heap_mut(self.draw).write_arit(fid, func.arity);
    self.get_heap_mut(self.draw).write_file(fid, Arc::new(func));
  }

  pub fn define_constructor(&mut self, cid: u128, arity: u128) {
    self.get_heap_mut(self.draw).write_arit(cid, arity);
  }

  // pub fn define_function_from_code(&mut self, name: &str, code: &str) {
  //   self.define_function(name_to_u128(name), read_func(code).1);
  // }

  pub fn create_term(&mut self, term: &Term, loc: u128, vars_data: &mut Map<u128>) -> Lnk {
    return create_term(self, term, loc, vars_data);
  }

  pub fn alloc_term(&mut self, term: &Term) -> u128 {
    let loc = alloc(self, 1);
    let lnk = create_term(self, term, loc, &mut init_map());
    self.write(loc as usize, lnk);
    return loc;
  }

  pub fn alloc_term_from_code(&mut self, code: &str) -> u128 {
    self.alloc_term(&read_term(code).1)
  }

  pub fn collect(&mut self, term: Lnk, mana: u128) -> Option<()> {
    collect(self, term, mana)
  }

  pub fn collect_at(&mut self, loc: u128, mana: u128) -> Option<()> {
    collect(self, self.read(loc as usize), mana)
  }

  //fn run_io_term(&mut self, subject: u128, caller: u128, term: &Term) -> Option<Lnk> {
    //let main = self.alloc_term(term);
    //let done = self.run_io(subject, caller, main);
    //return done;
  //}

  //fn run_io_from_code(&mut self, code: &str) -> Option<Lnk> {
    //return self.run_io_term(0, 0, &read_term(code).1);
  //}

  pub fn run_statements(&mut self, statements: &[Statement]) {
    for statement in statements {
      self.run_statement(statement);
    }
  }

  pub fn run_statements_from_code(&mut self, code: &str) {
    return self.run_statements(&read_statements(code).1);
  }

  pub fn compute_at(&mut self, loc: u128, mana: u128) -> Option<Lnk> {
    compute_at(self, loc, mana)
  }

  pub fn compute(&mut self, lnk: Lnk, mana: u128) -> Option<Lnk> {
    let host = alloc_lnk(self, lnk);
    let done = self.compute_at(host, mana)?;
    clear(self, host, 1);
    return Some(done);
  }

  pub fn show_term(&self, lnk: Lnk) -> String {
    return show_term(self, lnk);
  }

  pub fn show_term_at(&self, loc: u128) -> String {
    return show_term(self, self.read(loc as usize));
  }

  // Heaps
  // -----

  pub fn get_heap(&self, index: u64) -> &Heap {
    return &self.heap[index as usize];
  }

  pub fn get_heap_mut(&mut self, index: u64) -> &mut Heap {
    return &mut self.heap[index as usize];
  }

  // Copies the contents of the absorbed heap into the absorber heap
  fn absorb_heap(&mut self, absorber: u64, absorbed: u64, overwrite: bool) {
    // FIXME: can we satisfy the borrow checker without using unsafe pointers?
    unsafe {
      let a_arr = &mut self.heap as *mut Vec<Heap>;
      let a_ref = &mut *(&mut (*a_arr)[absorber as usize] as *mut Heap);
      let b_ref = &mut *(&mut (*a_arr)[absorbed as usize] as *mut Heap);
      a_ref.absorb(b_ref, overwrite);
    }
  }

  fn clear_heap(&mut self, index: u64) {
    self.heap[index as usize].clear();
  }

  fn undo(&mut self) {
    self.clear_heap(self.draw);
  }

  fn draw(&mut self) {
    self.absorb_heap(self.curr, self.draw, true);
    self.clear_heap(self.draw);
  }

  // IO
  // --

  pub fn run_io(&mut self, subject: u128, caller: u128, host: u128, mana: u128) -> Option<Lnk> {
    let term = reduce(self, host, mana)?;
    // eprintln!("-- {}", show_term(self, term));
    match get_tag(term) {
      CTR => {
        match get_ext(term) {
          IO_DONE => {
            let retr = ask_arg(self, term, 0);
            clear(self, host, 1);
            clear(self, get_loc(term, 0), 1);
            return Some(retr);
          }
          IO_TAKE => {
            //println!("- IO_TAKE subject is {} {}", u128_to_name(subject), subject);
            let cont = ask_arg(self, term, 0);
            if let Some(state) = self.read_disk(subject) {
              if state != 0 {
                self.write_disk(subject, 0);
                let cont = alloc_app(self, cont, state);
                let done = self.run_io(subject, subject, cont, mana);
                clear(self, host, 1);
                clear(self, get_loc(term, 0), 1);
                return done;
              }
            }
            clear(self, host, 1);
            clear(self, get_loc(term, 0), 1);
            return None;
          }
          IO_SAVE => {
            //println!("- IO_SAVE subject is {} {}", u128_to_name(subject), subject);
            let expr = ask_arg(self, term, 0);
            let save = self.compute(expr, mana)?;
            self.write_disk(subject, save);
            let cont = ask_arg(self, term, 1);
            let cont = alloc_app(self, cont, Num(0));
            let done = self.run_io(subject, subject, cont, mana);
            clear(self, host, 1);
            clear(self, get_loc(term, 0), 2);
            return done;
          }
          IO_CALL => {
            let fnid = ask_arg(self, term, 0);
            let tupl = ask_arg(self, term, 1);
            let cont = ask_arg(self, term, 2);
            // Builds the argument vector
            let arit = self.get_arity(get_ext(tupl));
            let mut args = Vec::new();
            for i in 0 .. arit {
              args.push(ask_arg(self, tupl, i));
            }
            // Calls called function IO, changing the subject
            // TODO: this should not alloc a Fun as it's limited to 60-bit names
            let ioxp = alloc_fun(self, get_num(fnid), &args);
            let retr = self.run_io(get_num(fnid), subject, ioxp, mana)?;
            // Calls the continuation with the value returned
            let cont = alloc_app(self, cont, retr);
            let done = self.run_io(subject, caller, cont, mana);
            // Clears memory
            clear(self, host, 1);
            clear(self, get_loc(tupl, 0), arit);
            clear(self, get_loc(term, 0), 3);
            return done;
          }
          IO_FROM => {
            let cont = ask_arg(self, term, 0);
            let cont = alloc_app(self, cont, Num(caller));
            let done = self.run_io(subject, caller, cont, mana);
            clear(self, host, 1);
            clear(self, get_loc(term, 0), 1);
            return done;
          }
          _ => {
            //self.collect(term, mana)?;
            return None;
          }
        }
      }
      _ => {
        return None;
      }
    }
  }

  pub fn run_statement(&mut self, statement: &Statement) {
    match statement {
      Statement::Fun { name, args, func, init } => {
        // println!("- fun {} {}", u128_to_name(*name), args.len());
        if let Some(func) = build_func(func, true) {
          self.set_arity(*name, args.len() as u128);
          self.define_function(*name, func);
          let state = self.create_term(init, 0, &mut init_map());
          self.write_disk(*name, state);
          self.draw();
        }
      }
      Statement::Ctr { name, args } => {
        // println!("- ctr {} {}", u128_to_name(*name), args.len());
        self.set_arity(*name, args.len() as u128);
        self.draw();
      }
      Statement::Run { expr } => {
        let mana_ini = self.get_mana(); 
        let mana_lim = self.get_mana_limit(); // max mana we can reach on this statement
        let size_ini = self.get_size();
        let size_lim = self.get_size_limit(); // max size we can reach on this statement
        let host = self.alloc_term(expr);
        // eprintln!("  => run term: {}", show_term(self, host)); // ?? why this is showing dups?
        if let Some(done) = self.run_io(0, 0, host, mana_lim) {
          if let Some(done) = self.compute(done, mana_lim) {
            let done_code = self.show_term(done);
            if let Some(()) = self.collect(done, mana_lim) {
              let size_end = self.get_size();
              let mana_dif = self.get_mana() - mana_ini;
              let size_dif = size_end - size_ini;
              // dbg!(size_end, size_dif, size_lim);
              if size_end <= size_lim {
                // println!("- run {} ({} mana, {} size)", done_code, mana_dif, size_dif);
                self.draw();
              } else {
                println!("- run fail: exceeded size limit {}/{}", size_end, size_lim);
                self.undo();
              }
              return;
            }
          }
        }
        println!("- run fail");
        self.undo();
      }
    }
  }

  // Maximum mana = 42m * block_number
  pub fn get_mana_limit(&self) -> u128 {
    (self.get_tick() + 1) * BLOCK_MANA_LIMIT
  }

  // Maximum size =  * block_number
  pub fn get_size_limit(&self) -> i128 {
    (self.get_tick() as i128 + 1) * (BLOCK_BITS_LIMIT / 128)
  }

  // Rollback
  // --------

  // Advances the heap time counter, saving past states for rollback.
  pub fn tick(&mut self) {
    self.set_tick(self.get_tick() + 1);
    self.draw();
    self.snapshot();
  }

  fn snapshot(&mut self) {
    //println!("tick self.curr={}", self.curr);
    let (included, absorber, deleted, rollback) = rollback_push(self.curr, self.back.clone());
    //println!("- tick self.curr={}, included={:?} absorber={:?} deleted={:?} rollback={}", self.curr, included, absorber, deleted, view_rollback(&self.back));
    self.back = rollback;
    if included {
      
      self.heap[self.curr as usize].save_buffers().expect("Error saving buffers."); // TODO: persistence-WIP
      if let Some(deleted) = deleted {
        if let Some(absorber) = absorber {
          self.absorb_heap(absorber, deleted, false);
          //deleted.append_buffers(absorber.uuid); // TODO: persistence-WIP
          //deleted.delete_buffers(); // TODO: persistence-WIP
        }
        self.clear_heap(deleted);
        self.curr = deleted;
      } else if let Some(empty) = self.nuls.pop() {
        self.curr = empty;
      } else {
        println!("- {} {} {:?} {}", self.draw, self.curr, self.nuls, view_rollback(&self.back));
        panic!("Not enough heaps.");
      }
    }
  }
  
  // Rolls back to the earliest state before or equal `tick`
  pub fn rollback(&mut self, tick: u128) {
    // If target tick is older than current tick
    if tick < self.get_tick() {
      println!("- rolling back from {} to {}", tick, self.get_tick());
      self.clear_heap(self.curr);
      self.nuls.push(self.curr);
      // Removes heaps until the runtime's tick is larger than, or equal to, the target tick
      while tick < self.get_tick() {
        if let Rollback::Cons { keep, head, tail } = &*self.back.clone() {
          self.clear_heap(*head);
          self.nuls.push(*head);
          self.back = tail.clone();
        }
      }
      // Moves the most recent valid heap to `self.curr`
      if let Rollback::Cons { keep, head, tail } = &*self.back.clone() {
        self.back = tail.clone();
        self.curr = *head;
      } else {
        self.back = Arc::new(Rollback::Nil);
        self.curr = self.nuls.pop().expect("No heap available!");
      }
    }
  }

  // Heap writers and readers
  // ------------------------

  // Attempts to read data from the latest heap.
  // If not present, looks for it on past states.
  pub fn get_with<A: std::cmp::PartialEq>(&self, zero: A, none: A, get: impl Fn(&Heap) -> A) -> A {
    let got = get(&self.get_heap(self.draw));
    if none != got {
      return got;
    }
    let got = get(&self.get_heap(self.curr));
    if none != got {
      return got;
    }
    let mut back = &self.back;
    loop {
      match &**back {
        Rollback::Cons { keep, head, tail } => {
          let val = get(self.get_heap(*head));
          if val != none {
            return val;
          }
          back = &*tail;
        }
        Rollback::Nil => {
          return zero;
        }
      }
    }
  }

  pub fn write(&mut self, idx: usize, val: u128) {
    return self.get_heap_mut(self.draw).write(idx, val);
  }

  pub fn read(&self, idx: usize) -> u128 {
    return self.get_with(0, U128_NONE, |heap| heap.read(idx));
  }

  pub fn write_disk(&mut self, fid: u128, val: Lnk) {
    return self.get_heap_mut(self.draw).write_disk(fid, val);
  }

  pub fn read_disk(&mut self, fid: u128) -> Option<Lnk> {
    return self.get_with(Some(0), None, |heap| heap.read_disk(fid));
  }

  pub fn get_arity(&self, fid: u128) -> u128 {
    if let Some(arity) = GET_ARITY(fid) {
      return arity;
    } else if let Some(arity) = self.get_with(None, None, |heap| heap.read_arit(fid)) {
      return arity;
    } else {
      return 0;
    }
  }

  pub fn set_arity(&mut self, fid: u128, arity: u128) {
    self.get_heap_mut(self.draw).write_arit(fid, arity);
  }

  pub fn get_func(&self, fid: u128) -> Option<Arc<CompFunc>> {
    let got = self.get_heap(self.draw).read_file(fid);
    if let Some(func) = got {
      return Some(func);
    }
    let got = self.get_heap(self.curr).read_file(fid);
    if let Some(func) = got {
      return Some(func);
    }
    let mut back = &self.back;
    loop {
      match &**back {
        Rollback::Cons { keep, head, tail } => {
          let got = self.get_heap(*head).file.read(fid);
          if let Some(func) = got {
            return Some(func);
          }
          back = &*tail;
        }
        Rollback::Nil => {
          return None;
        }
      }
    }
  }

  pub fn get_dups(&self) -> u128 {
    return self.get_with(0, U128_NONE, |heap| heap.get_dups());
  }

  pub fn set_rwts(&mut self, rwts: u128) {
    self.get_heap_mut(self.draw).set_rwts(rwts);
  }

  pub fn get_rwts(&self) -> u128 {
    return self.get_with(0, U128_NONE, |heap| heap.rwts);
  }

  pub fn set_mana(&mut self, mana: u128) {
    self.get_heap_mut(self.draw).set_mana(mana);
  }

  pub fn get_mana(&self) -> u128 {
    return self.get_with(0, U128_NONE, |heap| heap.mana);
  }

  pub fn set_tick(&mut self, tick: u128) {
    self.get_heap_mut(self.draw).set_tick(tick);
  }

  pub fn get_tick(&self) -> u128 {
    return self.get_with(0, U128_NONE, |heap| heap.tick);
  }

  pub fn set_size(&mut self, size: i128) {
    self.get_heap_mut(self.draw).size = size;
  }

  pub fn get_size(&self) -> i128 {
    return self.get_with(0, I128_NONE, |heap| heap.size);
  }

  pub fn set_next(&mut self, next: u128) {
    self.get_heap_mut(self.draw).next = next;
  }

  pub fn get_next(&self) -> u128 {
    return self.get_with(0, U128_NONE, |heap| heap.next);
  }

  pub fn fresh_dups(&mut self) -> u128 {
    let dups = self.get_heap(self.draw).get_dups();
    self.get_heap_mut(self.draw).set_dups(dups + 1);
    return dups & 0x3FFFFFFF;
  }

}

// Attempts to include a heap state on the list of past heap states. It only keeps at most
// `log_16(tick)` heaps in memory, rejecting heaps that it doesn't need to store. It returns:
// - included : Bool             = true if the heap was included, false if it was rejected
// - absorber : Option<Box<u64>> = the index of the dropped heap absorber (if any)
// - deleted  : Option<Box<u64>> = the index of the dropped heap (if any)
// - rollback : Rollback         = the updated rollback object
pub fn rollback_push(elem: u64, back: Arc<Rollback>) -> (bool, Option<u64>, Option<u64>, Arc<Rollback>) {
  match &*back {
    Rollback::Nil => {
      let rollback = Arc::new(Rollback::Cons { keep: 0, head: elem, tail: Arc::new(Rollback::Nil) });
      return (true, None, None, rollback);
    }
    Rollback::Cons { keep, head, tail } => {
      if *keep == 0xF {
        let (included, absorber, deleted, tail) = rollback_push(*head, tail.clone());
        let absorber = if !included { Some(elem) } else { absorber };
        let rollback = Arc::new(Rollback::Cons { keep: 0, head: elem, tail });
        return (true, absorber, deleted, rollback);
      } else {
        let rollback = Arc::new(Rollback::Cons { keep: keep + 1, head: *head, tail: tail.clone() });
        return (false, None, Some(elem), rollback);
      }
    }
  }
}

pub fn view_rollback(back: &Arc<Rollback>) -> String {
  match &**back {
    Rollback::Nil => {
      return String::new();
    }
    Rollback::Cons { keep, head, tail } => {
      return format!("[{:x} {}] {}", keep, head, view_rollback(tail));
    }
  }
}


// Constructors
// ------------

pub fn Var(pos: u128) -> Lnk {
  (VAR * TAG) | pos
}

pub fn Dp0(col: u128, pos: u128) -> Lnk {
  (DP0 * TAG) | (col * EXT) | pos
}

pub fn Dp1(col: u128, pos: u128) -> Lnk {
  (DP1 * TAG) | (col * EXT) | pos
}

pub fn Arg(pos: u128) -> Lnk {
  (ARG * TAG) | pos
}

pub fn Era() -> Lnk {
  ERA * TAG
}

pub fn Lam(pos: u128) -> Lnk {
  (LAM * TAG) | pos
}

pub fn App(pos: u128) -> Lnk {
  (APP * TAG) | pos
}

pub fn Par(col: u128, pos: u128) -> Lnk {
  (SUP * TAG) | (col * EXT) | pos
}

pub fn Op2(ope: u128, pos: u128) -> Lnk {
  (OP2 * TAG) | (ope * EXT) | pos
}

pub fn Num(val: u128) -> Lnk {
  debug_assert!((!NUM_MASK & val) == 0, "Num overflow: `{}`.", val);
  (NUM * TAG) | val
}

pub fn Ctr(fun: u128, pos: u128) -> Lnk {
  (CTR * TAG) | (fun * EXT) | pos
}

pub fn Fun(fun: u128, pos: u128) -> Lnk {
  debug_assert!(fun < 1<<60,
    "Directly calling function with too long name: `{}`.", u128_to_name(fun));
  (FUN * TAG) | (fun * EXT) | pos
}

// Getters
// -------

pub fn get_tag(lnk: Lnk) -> u128 {
  lnk / TAG
}

pub fn get_ext(lnk: Lnk) -> u128 {
  (lnk / EXT) & 0xFFFFFFFFFFFFFFF
}

pub fn get_val(lnk: Lnk) -> u128 {
  lnk & 0xFFFFFFFFFFFFFFF
}

pub fn get_num(lnk: Lnk) -> u128 {
  lnk & 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF
}

//pub fn get_ari(lnk: Lnk) -> u128 {
  //(lnk / ARI) & 0xF
//}

pub fn get_loc(lnk: Lnk, arg: u128) -> u128 {
  get_val(lnk) + arg
}

// Memory
// ------

pub fn ask_lnk(rt: &Runtime, loc: u128) -> Lnk {
  rt.read(loc as usize)
  //unsafe { *rt.heap.get_unchecked(loc as usize) }
}

pub fn ask_arg(rt: &Runtime, term: Lnk, arg: u128) -> Lnk {
  ask_lnk(rt, get_loc(term, arg))
}

pub fn link(rt: &mut Runtime, loc: u128, lnk: Lnk) -> Lnk {
  rt.write(loc as usize, lnk);
  if get_tag(lnk) <= VAR {
    let pos = get_loc(lnk, get_tag(lnk) & 0x01);
    rt.write(pos as usize, Arg(loc));
  }
  lnk
}

pub fn alloc(rt: &mut Runtime, size: u128) -> u128 {
  if size == 0 {
    return 0;
  } else {
    loop {
      let index = rt.get_next();
      if index <= HEAP_SIZE - size {
        let mut empty = true;
        for i in 0 .. size {
          if rt.read((index + i) as usize) != 0 {
            empty = false;
            break;
          }
        }
        if empty {
          rt.set_next(rt.get_next() + size);
          rt.set_size(rt.get_size() + size as i128);
          return index;
        }
      }
      rt.set_next((fastrand::u64(..) % HEAP_SIZE as u64) as u128);
    }
  }
}

pub fn clear(rt: &mut Runtime, loc: u128, size: u128) {
  //println!("- clear {} {}", loc, size);
  for i in 0 .. size {
    if rt.read((loc + i) as usize) == 0 {
      eprintln!("- clear again {}", loc);
      panic!("clear happened twice");
    }
    rt.write((loc + i) as usize, 0);
  }
  rt.set_size(rt.get_size() - size as i128);
  //rt.free[size as usize].push(loc);
}

pub fn collect(rt: &mut Runtime, term: Lnk, mana: u128) -> Option<()> {
  let mut stack : Vec<Lnk> = Vec::new();
  let mut next = term;
  let mut dups : Vec<u128> = Vec::new();
  loop {
    let term = next;
    match get_tag(term) {
      DP0 => {
        link(rt, get_loc(term, 0), Era());
        dups.push(term);
      }
      DP1 => {
        link(rt, get_loc(term, 1), Era());
        dups.push(term);
      }
      VAR => {
        link(rt, get_loc(term, 0), Era());
      }
      LAM => {
        if get_tag(ask_arg(rt, term, 0)) != ERA {
          link(rt, get_loc(ask_arg(rt, term, 0), 0), Era());
        }
        next = ask_arg(rt, term, 1);
        clear(rt, get_loc(term, 0), 2);
        continue;
      }
      APP => {
        stack.push(ask_arg(rt, term, 0));
        next = ask_arg(rt, term, 1);
        clear(rt, get_loc(term, 0), 2);
        continue;
      }
      SUP => {
        stack.push(ask_arg(rt, term, 0));
        next = ask_arg(rt, term, 1);
        clear(rt, get_loc(term, 0), 2);
        continue;
      }
      OP2 => {
        stack.push(ask_arg(rt, term, 0));
        next = ask_arg(rt, term, 1);
        clear(rt, get_loc(term, 0), 2);
        continue;
      }
      NUM => {}
      CTR | FUN => {
        let arity = rt.get_arity(get_ext(term));
        for i in 0 .. arity {
          if i < arity - 1 {
            stack.push(ask_arg(rt, term, i));
          } else {
            next = ask_arg(rt, term, i);
          }
        }
        clear(rt, get_loc(term, 0), arity);
        if arity > 0 {
          continue;
        }
      }
      _ => {}
    }
    if let Some(got) = stack.pop() {
      next = got;
    } else {
      break;
    }
  }
  for dup in dups {
    let fst = ask_arg(rt, dup, 0);
    let snd = ask_arg(rt, dup, 1);
    if get_tag(fst) == ERA && get_tag(snd) == ERA {
      collect(rt, ask_arg(rt, dup, 2), mana);
      clear(rt, get_loc(dup, 0), 3);
    }
  }
  return Some(());
}

// Term
// ----

// Writes a Term represented as a Rust enum on the Runtime's rt.
pub fn create_term(rt: &mut Runtime, term: &Term, loc: u128, vars_data: &mut Map<u128>) -> Lnk {
  fn bind(rt: &mut Runtime, loc: u128, name: u128, lnk: Lnk, vars_data: &mut Map<u128>) {
    //println!("~~ bind {} {}", u128_to_name(name), show_lnk(lnk));
    if name == VAR_NONE {
      link(rt, loc, Era());
    } else {
      let got = vars_data.get(&(name as u64)).map(|x| *x);
      match got {
        Some(got) => {
          vars_data.remove(&(name as u64));
          link(rt, got, lnk);
        }
        None => {
          vars_data.insert(name as u64, lnk);
          link(rt, loc, Era());
        }
      }
    }
  }
  match term {
    Term::Var { name } => {
      //println!("~~ var {} {}", u128_to_name(*name), vars_data.len());
      let got = vars_data.get(&(*name as u64)).map(|x| *x);
      match got {
        Some(got) => {
          vars_data.remove(&(*name as u64));
          return got;
        }
        None => {
          vars_data.insert(*name as u64, loc);
          return Num(0);
        }
      }
    }
    Term::Dup { nam0, nam1, expr, body } => {
      let node = alloc(rt, 3);
      let dupk = rt.get_dups();
      bind(rt, node + 0, *nam0, Dp0(dupk, node), vars_data);
      bind(rt, node + 1, *nam1, Dp1(dupk, node), vars_data);
      let expr = create_term(rt, expr, node + 2, vars_data);
      link(rt, node + 2, expr);
      let body = create_term(rt, body, loc, vars_data);
      body
    }
    Term::Lam { name, body } => {
      let node = alloc(rt, 2);
      bind(rt, node + 0, *name, Var(node), vars_data);
      let body = create_term(rt, body, node + 1, vars_data);
      link(rt, node + 1, body);
      Lam(node)
    }
    Term::App { func, argm } => {
      let node = alloc(rt, 2);
      let func = create_term(rt, func, node + 0, vars_data);
      link(rt, node + 0, func);
      let argm = create_term(rt, argm, node + 1, vars_data);
      link(rt, node + 1, argm);
      App(node)
    }
    Term::Fun { name, args } => {
      let size = args.len() as u128;
      let node = alloc(rt, size);
      for (i, arg) in args.iter().enumerate() {
        let arg_lnk = create_term(rt, arg, node + i as u128, vars_data);
        link(rt, node + i as u128, arg_lnk);
      }
      Fun(*name, node)
    }
    Term::Ctr { name, args } => {
      let size = args.len() as u128;
      let node = alloc(rt, size);
      for (i, arg) in args.iter().enumerate() {
        let arg_lnk = create_term(rt, arg, node + i as u128, vars_data);
        link(rt, node + i as u128, arg_lnk);
      }
      Ctr(*name, node)
    }
    Term::Num { numb } => {
      // TODO: assert numb size
      Num(*numb as u128)
    }
    Term::Op2 { oper, val0, val1 } => {
      let node = alloc(rt, 2);
      let val0 = create_term(rt, val0, node + 0, vars_data);
      link(rt, node + 0, val0);
      let val1 = create_term(rt, val1, node + 1, vars_data);
      link(rt, node + 1, val1);
      Op2(*oper, node)
    }
  }
}

// Given a vector of rules (lhs/rhs pairs), builds the Func object
pub fn build_func(func: &Vec<Rule>, debug: bool) -> Option<CompFunc> {
  // If there are no rules, return none
  if func.len() == 0 {
    if debug {
      println!("- failed to build function: no rules");
    }
    return None;
  }

  // Find the function arity
  let arity;
  if let Term::Fun { args, .. } = &func[0].0 {
    arity = args.len() as u128;
  } else {
    if debug {
      println!("- failed to build function: no arity");
    }
    return None;
  }

  // The resulting vector
  let mut comp_rules = Vec::new();

  // A vector with the indices that are strict
  let mut strict = vec![false; arity as usize];

  // For each rule (lhs/rhs pair)
  for rule in 0 .. func.len() {
    let comp_rule = &func[rule];

    let mut cond = Vec::new();
    let mut vars = Vec::new();
    let mut eras = Vec::new();

    // If the lhs is a Fun
    if let Term::Fun { ref name, ref args } = comp_rule.0 {

      // If there is an arity mismatch, return None
      if args.len() as u128 != arity {
        if debug {
          println!("  - failed to build function: arity mismatch on rule {}", rule);
        }
        return None;
      }

      // For each lhs argument
      for i in 0 .. args.len() as u128 {
        
        match &args[i as usize] {
          // If it is a constructor...
          Term::Ctr { name: arg_name, args: arg_args } => {
            strict[i as usize] = true;
            cond.push(Ctr(*arg_name, 0)); // adds its matching condition
            eras.push((i, arg_args.len() as u128)); // marks its index and arity for freeing
            // For each of its fields...
            for j in 0 .. arg_args.len() as u128 {
              // If it is a variable...
              if let Term::Var { name } = arg_args[j as usize] {
                vars.push(Var { name, param: i, field: Some(j), erase: name == VAR_NONE }); // add its location
              // Otherwise..
              } else {
                if debug {
                  println!("  - failed to build function: nested match on rule {}, argument {}", rule, i);
                }
                return None; // return none, because we don't allow nested matches
              }
            }
          }
          // If it is a number...
          Term::Num { numb: arg_numb } => {
            strict[i as usize] = true;
            cond.push(Num(*arg_numb as u128)); // adds its matching condition
          }
          // If it is a variable...
          Term::Var { name: arg_name } => {
            vars.push(Var { name: *arg_name, param: i, field: None, erase: *arg_name == VAR_NONE }); // add its location
            cond.push(0); // it has no matching condition
          }
          _ => {
            if debug {
              println!("  - failed to build function: unsupported match on rule {}, argument {}", rule, i);
            }
            return None;
          }
        }
      }

    // If lhs isn't a Ctr, return None
    } else {
      if debug {
        println!("  - failed to build function: left-hand side isn't a constructor, on rule {}", rule);
      }
      return None;
    }

    // Creates the rhs body
    let body = comp_rule.1.clone();

    // Adds the rule to the result vector
    comp_rules.push(CompRule { cond, vars, eras, body });
  }

  // Builds the redux object, with the index of strict arguments
  let mut redux = Vec::new();
  for i in 0 .. strict.len() {
    if strict[i] {
      redux.push(i as u128);
    }
  }

  return Some(CompFunc { func: func.clone(), arity, redux, rules: comp_rules });
}

pub fn create_app(rt: &mut Runtime, func: Lnk, argm: Lnk) -> Lnk {
  let node = alloc(rt, 2);
  link(rt, node + 0, func);
  link(rt, node + 1, argm);
  App(node)
}

pub fn create_fun(rt: &mut Runtime, fun: u128, args: &[Lnk]) -> Lnk {
  let node = alloc(rt, args.len() as u128);
  for i in 0 .. args.len() {
    link(rt, node + i as u128, args[i]);
  }
  Fun(fun, node)
}

pub fn alloc_lnk(rt: &mut Runtime, term: Lnk) -> u128 {
  let loc = alloc(rt, 1);
  link(rt, loc, term);
  return loc;
}

pub fn alloc_app(rt: &mut Runtime, func: Lnk, argm: Lnk) -> u128 {
  let app = create_app(rt, func, argm);
  return alloc_lnk(rt, app);
}

pub fn alloc_fun(rt: &mut Runtime, fun: u128, args: &[Lnk]) -> u128 {
  let fun = create_fun(rt, fun, args);
  return alloc_lnk(rt, fun);
}

// Reduction
// ---------

pub fn subst(rt: &mut Runtime, lnk: Lnk, val: Lnk, mana: u128) -> Option<()> {
  if get_tag(lnk) != ERA {
    link(rt, get_loc(lnk, 0), val);
  } else {
    collect(rt, val, mana)?;
  }
  return Some(());
}

pub fn reduce(rt: &mut Runtime, root: u128, mana: u128) -> Option<Lnk> {
  let mut vars_data: Map<u128> = init_map();

  let mut stack: Vec<u128> = Vec::new();

  let mut init = 1;
  let mut host = root;

  let mut func_val : Option<CompFunc>;
  let mut func_ref : Option<&mut CompFunc>;

  loop {
    let term = ask_lnk(rt, host);

    if rt.get_mana() > mana {
      return None;
    }

    //if debug || true {
      //println!("------------------------");
      //println!("{}", show_term(rt, ask_lnk(rt, 0)));
    //}

    if init == 1 {
      match get_tag(term) {
        APP => {
          stack.push(host);
          init = 1;
          host = get_loc(term, 0);
          continue;
        }
        DP0 | DP1 => {
          stack.push(host);
          host = get_loc(term, 2);
          continue;
        }
        OP2 => {
          stack.push(host);
          stack.push(get_loc(term, 1) | 0x80000000);
          host = get_loc(term, 0);
          continue;
        }
        FUN => {
          let fun = get_ext(term);
          let ari = rt.get_arity(fun);
          if let Some(func) = &rt.get_func(fun) {
            if ari == func.arity {
              if func.redux.len() == 0 {
                init = 0;
              } else {
                stack.push(host);
                for (i, redux) in func.redux.iter().enumerate() {
                  if i < func.redux.len() - 1 {
                    stack.push(get_loc(term, *redux) | 0x80000000);
                  } else {
                    host = get_loc(term, *redux);
                  }
                }
              }
              continue;
            }
          }
        }
        _ => {}
      }
    } else {
      match get_tag(term) {
        APP => {
          let arg0 = ask_arg(rt, term, 0);
          // (@x(body) a)
          // ------------ APP-LAM
          // x <- a
          // body
          if get_tag(arg0) == LAM {
            //println!("app-lam");
            rt.set_mana(rt.get_mana() + AppLamMana());
            rt.set_rwts(rt.get_rwts() + 1);
            subst(rt, ask_arg(rt, arg0, 0), ask_arg(rt, term, 1), mana);
            let _done = link(rt, host, ask_arg(rt, arg0, 1));
            clear(rt, get_loc(term, 0), 2);
            clear(rt, get_loc(arg0, 0), 2);
            init = 1;
            continue;
          }
          // ({a b} c)
          // ----------------- APP-SUP
          // dup x0 x1 = c
          // {(a x0) (b x1)}
          if get_tag(arg0) == SUP {
            //println!("app-sup");
            rt.set_mana(rt.get_mana() + AppSupMana());
            rt.set_rwts(rt.get_rwts() + 1);
            let app0 = get_loc(term, 0);
            let app1 = get_loc(arg0, 0);
            let let0 = alloc(rt, 3);
            let par0 = alloc(rt, 2);
            link(rt, let0 + 2, ask_arg(rt, term, 1));
            link(rt, app0 + 1, Dp0(get_ext(arg0), let0));
            link(rt, app0 + 0, ask_arg(rt, arg0, 0));
            link(rt, app1 + 0, ask_arg(rt, arg0, 1));
            link(rt, app1 + 1, Dp1(get_ext(arg0), let0));
            link(rt, par0 + 0, App(app0));
            link(rt, par0 + 1, App(app1));
            let done = Par(get_ext(arg0), par0);
            link(rt, host, done);
          }
        }
        DP0 | DP1 => {
          let arg0 = ask_arg(rt, term, 2);
          // dup r s = @x(f)
          // --------------- DUP-LAM
          // dup f0 f1 = f
          // r <- @x0(f0)
          // s <- @x1(f1)
          // x <- {x0 x1}
          if get_tag(arg0) == LAM {
            //println!("dup-lam");
            rt.set_mana(rt.get_mana() + DupLamMana());
            rt.set_rwts(rt.get_rwts() + 1);
            let let0 = get_loc(term, 0);
            let par0 = get_loc(arg0, 0);
            let lam0 = alloc(rt, 2);
            let lam1 = alloc(rt, 2);
            link(rt, let0 + 2, ask_arg(rt, arg0, 1));
            link(rt, par0 + 1, Var(lam1));
            let arg0_arg_0 = ask_arg(rt, arg0, 0);
            link(rt, par0 + 0, Var(lam0));
            subst(rt, arg0_arg_0, Par(get_ext(term), par0), mana);
            let term_arg_0 = ask_arg(rt, term, 0);
            link(rt, lam0 + 1, Dp0(get_ext(term), let0));
            subst(rt, term_arg_0, Lam(lam0), mana);
            let term_arg_1 = ask_arg(rt, term, 1);
            link(rt, lam1 + 1, Dp1(get_ext(term), let0));
            subst(rt, term_arg_1, Lam(lam1), mana);
            let done = Lam(if get_tag(term) == DP0 { lam0 } else { lam1 });
            link(rt, host, done);
            init = 1;
            continue;
          // dup x y = {a b}
          // --------------- DUP-SUP (equal)
          // x <- a
          // y <- b
          } else if get_tag(arg0) == SUP {
            if get_ext(term) == get_ext(arg0) {
              //println!("dup-sup");
              rt.set_mana(rt.get_mana() + DupSupMana());
              rt.set_rwts(rt.get_rwts() + 1);
              subst(rt, ask_arg(rt, term, 0), ask_arg(rt, arg0, 0), mana);
              subst(rt, ask_arg(rt, term, 1), ask_arg(rt, arg0, 1), mana);
              let _done = link(rt, host, ask_arg(rt, arg0, if get_tag(term) == DP0 { 0 } else { 1 }));
              clear(rt, get_loc(term, 0), 3);
              clear(rt, get_loc(arg0, 0), 2);
              init = 1;
              continue;
            // dup x y = {a b}
            // ----------------- DUP-SUP (different)
            // x <- {xA xB}
            // y <- {yA yB}
            // dup xA yA = a
            // dup xB yB = b
            } else {
              //println!("dup-sup");
              rt.set_mana(rt.get_mana() + DupDupMana());
              rt.set_rwts(rt.get_rwts() + 1);
              let par0 = alloc(rt, 2);
              let let0 = get_loc(term, 0);
              let par1 = get_loc(arg0, 0);
              let let1 = alloc(rt, 3);
              link(rt, let0 + 2, ask_arg(rt, arg0, 0));
              link(rt, let1 + 2, ask_arg(rt, arg0, 1));
              let term_arg_0 = ask_arg(rt, term, 0);
              let term_arg_1 = ask_arg(rt, term, 1);
              link(rt, par1 + 0, Dp1(get_ext(term), let0));
              link(rt, par1 + 1, Dp1(get_ext(term), let1));
              link(rt, par0 + 0, Dp0(get_ext(term), let0));
              link(rt, par0 + 1, Dp0(get_ext(term), let1));
              subst(rt, term_arg_0, Par(get_ext(arg0), par0), mana);
              subst(rt, term_arg_1, Par(get_ext(arg0), par1), mana);
              let done = Par(get_ext(arg0), if get_tag(term) == DP0 { par0 } else { par1 });
              link(rt, host, done);
            }
          // dup x y = N
          // ----------- DUP-NUM
          // x <- N
          // y <- N
          // ~
          } else if get_tag(arg0) == NUM {
            //println!("dup-num");
            rt.set_mana(rt.get_mana() + DupNumMana());
            rt.set_rwts(rt.get_rwts() + 1);
            subst(rt, ask_arg(rt, term, 0), arg0, mana);
            subst(rt, ask_arg(rt, term, 1), arg0, mana);
            clear(rt, get_loc(term, 0), 3);
            let _done = arg0;
            link(rt, host, arg0);
          // dup x y = (K a b c ...)
          // ----------------------- DUP-CTR
          // dup a0 a1 = a
          // dup b0 b1 = b
          // dup c0 c1 = c
          // ...
          // x <- (K a0 b0 c0 ...)
          // y <- (K a1 b1 c1 ...)
          } else if get_tag(arg0) == CTR {
            //println!("dup-ctr");
            let func = get_ext(arg0);
            let arit = rt.get_arity(func);
            rt.set_mana(rt.get_mana() + DupCtrMana(arit));
            rt.set_rwts(rt.get_rwts() + 1);
            if arit == 0 {
              subst(rt, ask_arg(rt, term, 0), Ctr(func, 0), mana);
              subst(rt, ask_arg(rt, term, 1), Ctr(func, 0), mana);
              clear(rt, get_loc(term, 0), 3);
              let _done = link(rt, host, Ctr(func, 0));
            } else {
              let ctr0 = get_loc(arg0, 0);
              let ctr1 = alloc(rt, arit);
              for i in 0..arit - 1 {
                let leti = alloc(rt, 3);
                link(rt, leti + 2, ask_arg(rt, arg0, i));
                link(rt, ctr0 + i, Dp0(get_ext(term), leti));
                link(rt, ctr1 + i, Dp1(get_ext(term), leti));
              }
              let leti = get_loc(term, 0);
              link(rt, leti + 2, ask_arg(rt, arg0, arit - 1));
              let term_arg_0 = ask_arg(rt, term, 0);
              link(rt, ctr0 + arit - 1, Dp0(get_ext(term), leti));
              subst(rt, term_arg_0, Ctr(func, ctr0), mana);
              let term_arg_1 = ask_arg(rt, term, 1);
              link(rt, ctr1 + arit - 1, Dp1(get_ext(term), leti));
              subst(rt, term_arg_1, Ctr(func, ctr1), mana);
              let done = Ctr(func, if get_tag(term) == DP0 { ctr0 } else { ctr1 });
              link(rt, host, done);
            }
          // dup x y = *
          // ----------- DUP-ERA
          // x <- *
          // y <- *
          } else if get_tag(arg0) == ERA {
            //println!("dup-era");
            rt.set_mana(rt.get_mana() + DupEraMana());
            rt.set_rwts(rt.get_rwts() + 1);
            subst(rt, ask_arg(rt, term, 0), Era(), mana);
            subst(rt, ask_arg(rt, term, 1), Era(), mana);
            link(rt, host, Era());
            clear(rt, get_loc(term, 0), 3);
            init = 1;
            continue;
          }
        }
        OP2 => {
          let arg0 = ask_arg(rt, term, 0);
          let arg1 = ask_arg(rt, term, 1);
          // (+ a b)
          // --------- OP2-NUM
          // add(a, b)
          if get_tag(arg0) == NUM && get_tag(arg1) == NUM {
            //println!("op2-num");
            rt.set_mana(rt.get_mana() + Op2NumMana());
            rt.set_rwts(rt.get_rwts() + 1);
            let a = get_num(arg0);
            let b = get_num(arg1);
            let c = match get_ext(term) {
              ADD => (a + b)  & 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF,
              SUB => (a - b)  & 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF,
              MUL => (a * b)  & 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF,
              DIV => (a / b)  & 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF,
              MOD => (a % b)  & 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF,
              AND => (a & b)  & 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF,
              OR  => (a | b)  & 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF,
              XOR => (a ^ b)  & 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF,
              SHL => (a << b) & 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF,
              SHR => (a >> b) & 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF,
              LTN => u128::from(a <  b),
              LTE => u128::from(a <= b),
              EQL => u128::from(a == b),
              GTE => u128::from(a >= b),
              GTN => u128::from(a >  b),
              NEQ => u128::from(a != b),
              _   => 0,
            };
            let done = Num(c);
            clear(rt, get_loc(term, 0), 2);
            link(rt, host, done);
          // (+ {a0 a1} b)
          // --------------------- OP2-SUP-0
          // let b0 b1 = b
          // {(+ a0 b0) (+ a1 b1)}
          } else if get_tag(arg0) == SUP {
            //println!("op2-sup-0");
            rt.set_mana(rt.get_mana() + Op2SupMana());
            rt.set_rwts(rt.get_rwts() + 1);
            let op20 = get_loc(term, 0);
            let op21 = get_loc(arg0, 0);
            let let0 = alloc(rt, 3);
            let par0 = alloc(rt, 2);
            link(rt, let0 + 2, arg1);
            link(rt, op20 + 1, Dp0(get_ext(arg0), let0));
            link(rt, op20 + 0, ask_arg(rt, arg0, 0));
            link(rt, op21 + 0, ask_arg(rt, arg0, 1));
            link(rt, op21 + 1, Dp1(get_ext(arg0), let0));
            link(rt, par0 + 0, Op2(get_ext(term), op20));
            link(rt, par0 + 1, Op2(get_ext(term), op21));
            let done = Par(get_ext(arg0), par0);
            link(rt, host, done);
          // (+ a {b0 b1})
          // --------------- OP2-SUP-1
          // dup a0 a1 = a
          // {(+ a0 b0) (+ a1 b1)}
          } else if get_tag(arg1) == SUP {
            //println!("op2-sup-1");
            rt.set_mana(rt.get_mana() + Op2SupMana());
            rt.set_rwts(rt.get_rwts() + 1);
            let op20 = get_loc(term, 0);
            let op21 = get_loc(arg1, 0);
            let let0 = alloc(rt, 3);
            let par0 = alloc(rt, 2);
            link(rt, let0 + 2, arg0);
            link(rt, op20 + 0, Dp0(get_ext(arg1), let0));
            link(rt, op20 + 1, ask_arg(rt, arg1, 0));
            link(rt, op21 + 1, ask_arg(rt, arg1, 1));
            link(rt, op21 + 0, Dp1(get_ext(arg1), let0));
            link(rt, par0 + 0, Op2(get_ext(term), op20));
            link(rt, par0 + 1, Op2(get_ext(term), op21));
            let done = Par(get_ext(arg1), par0);
            link(rt, host, done);
          }
        }
        FUN => {

          fn call_function(rt: &mut Runtime, func: Arc<CompFunc>, host: u128, term: Lnk, mana: u128, vars_data: &mut Map<u128>) -> Option<bool> {
            // For each argument, if it is a redex and a SUP, apply the cal_par rule
            for idx in &func.redux {
              // (F {a0 a1} b c ...)
              // ------------------- FUN-SUP
              // dup b0 b1 = b
              // dup c0 c1 = c
              // ...
              // {(F a0 b0 c0 ...) (F a1 b1 c1 ...)}
              if get_tag(ask_arg(rt, term, *idx)) == SUP {
                //println!("fun-sup");
                let funx = get_ext(term);
                let arit = rt.get_arity(funx);
                rt.set_mana(rt.get_mana() + FunSupMana(arit));
                rt.set_rwts(rt.get_rwts() + 1);
                let argn = ask_arg(rt, term, *idx);
                let fun0 = get_loc(term, 0);
                let fun1 = alloc(rt, arit);
                let par0 = get_loc(argn, 0);
                for i in 0..arit {
                  if i != *idx {
                    let leti = alloc(rt, 3);
                    let argi = ask_arg(rt, term, i);
                    link(rt, fun0 + i, Dp0(get_ext(argn), leti));
                    link(rt, fun1 + i, Dp1(get_ext(argn), leti));
                    link(rt, leti + 2, argi);
                  } else {
                    link(rt, fun0 + i, ask_arg(rt, argn, 0));
                    link(rt, fun1 + i, ask_arg(rt, argn, 1));
                  }
                }
                link(rt, par0 + 0, Fun(funx, fun0));
                link(rt, par0 + 1, Fun(funx, fun1));
                let done = Par(get_ext(argn), par0);
                link(rt, host, done);
                return Some(true);
              }
            }
            // For each rule condition vector
            for rule in &func.rules {
              // Check if the rule matches
              let mut matched = true;
              //println!("- matching rule");
              // Tests each rule condition (ex: `get_tag(args[0]) == SUCC`)
              for i in 0 .. rule.cond.len() as u128 {
                let cond = rule.cond[i as usize];
                match get_tag(cond) {
                  NUM => {
                    //println!("Didn't match because of NUM. i={} {} {}", i, get_val(ask_arg(rt, term, i)), get_val(cond));
                    let same_tag = get_tag(ask_arg(rt, term, i)) == NUM;
                    let same_val = get_val(ask_arg(rt, term, i)) == get_val(cond);
                    matched = matched && same_tag && same_val;
                  }
                  CTR => {
                    //println!("Didn't match because of CTR. i={} {} {}", i, get_tag(ask_arg(rt, term, i)), get_val(cond));
                    let same_tag = get_tag(ask_arg(rt, term, i)) == CTR;
                    let same_ext = get_ext(ask_arg(rt, term, i)) == get_ext(cond);
                    matched = matched && same_tag && same_ext;
                  }
                  _ => {}
                }
              }
              // (user-defined)
              // -------------- FUN-CTR
              // (user-defined)
              // If all conditions are satisfied, the rule matched, so we must apply it
              if matched {
                //println!("fun-ctr");
                //println!("- matched");
                // Increments the gas count
                rt.set_mana(rt.get_mana() + FunCtrMana(&rule.body));
                rt.set_rwts(rt.get_rwts() + 1);
                // Gathers matched variables
                //let mut vars = vec![None; 16]; // FIXME: pre-alloc statically
                for (i, rule_var) in rule.vars.iter().enumerate() {
                  let mut var = term;
                  var = ask_arg(rt, var, rule_var.param);
                  if let Some(field) = rule_var.field {
                    var = ask_arg(rt, var, field);
                  }
                  //eprintln!("~~ set {} {}", u128_to_name(rule_var.name), show_lnk(var));
                  if !rule_var.erase {
                    vars_data.insert(rule_var.name as u64, var);
                  } else {
                    // Collects unused argument
                    collect(rt, var, mana)?;
                  }
                }
                // Builds the right-hand side term (ex: `(Succ (Add a b))`)
                //println!("-- vars: {:?}", vars);
                let done = create_term(rt, &rule.body, host, vars_data);
                // Links the host location to it
                link(rt, host, done);
                // Clears the matched ctrs (the `(Succ ...)` and the `(Add ...)` ctrs)
                for (eras_index, eras_arity) in &rule.eras {
                  clear(rt, get_loc(ask_arg(rt, term, *eras_index), 0), *eras_arity);
                }
                clear(rt, get_loc(term, 0), func.arity);
                // // Collects unused variables (none in this example)
                // for i in 0 .. rule.vars.len() {
                //   if rule.vars[i].erase {
                //     if let Some(var) = vars_data.get(&(i as u64)) {
                //       collect(rt, *var, mana)?;
                //     }
                //   }
                // }
                return Some(true);
              }
            }
            // ?? clear vars_data ?
            return Some(false);
          }

          let fun = get_ext(term);
          if let Some(func) = rt.get_func(fun) {
            if call_function(rt, func, host, term, mana, &mut vars_data)? {
              init = 1;
              continue;
            }
          }

        }
        _ => {}
      }
    }

    if let Some(item) = stack.pop() {
      init = item >> 31;
      host = item & 0x7FFFFFFF;
      continue;
    }

    break;
  }

  // FIXME: remove this when Runtime is split (see above)
  //rt.get_heap_mut(self.curr).file = file;

  return Some(ask_lnk(rt, root));
}

pub fn set_bit(bits: &mut [u128], bit: u128) {
  bits[bit as usize >> 6] |= 1 << (bit & 0x3f);
}

pub fn get_bit(bits: &[u128], bit: u128) -> bool {
  (((bits[bit as usize >> 6] >> (bit & 0x3f)) as u8) & 1) == 1
}

// Evaluates redexes recursively. This is used to save space before storing a term, since,
// otherwise, chunks would grow indefinitely due to lazy evaluation. It does not reduce the term to
// normal form, though, since it stops on whnfs. If it did, then storing a state wouldn't be O(1),
// since it would require passing over the entire state.
pub fn compute_at(rt: &mut Runtime, host: u128, mana: u128) -> Option<Lnk> {
  let term = ask_lnk(rt, host);
  let norm = reduce(rt, host, mana)?;
  if term != norm {
    match get_tag(norm) {
      LAM => {
        let loc_1 = get_loc(norm, 1);
        let lnk_1 = compute_at(rt, loc_1, mana)?;
        link(rt, loc_1, lnk_1);
      }
      APP => {
        let loc_0 = get_loc(norm, 0);
        let lnk_0 = compute_at(rt, loc_0, mana)?;
        link(rt, loc_0, lnk_0);
        let loc_1 = get_loc(norm, 1);
        let lnk_1 = compute_at(rt, loc_1, mana)?;
        link(rt, loc_1, lnk_1);
      }
      SUP => {
        let loc_0 = get_loc(norm, 0);
        let lnk_0 = compute_at(rt, loc_0, mana)?;
        link(rt, loc_0, lnk_0);
        let loc_1 = get_loc(norm, 1);
        let lnk_1 = compute_at(rt, loc_1, mana)?;
        link(rt, loc_1, lnk_1);
      }
      DP0 => {
        let loc_2 = get_loc(norm, 2);
        let lnk_2 = compute_at(rt, loc_2, mana)?;
        link(rt, loc_2, lnk_2);
      }
      DP1 => {
        let loc_2 = get_loc(norm, 2);
        let lnk_2 = compute_at(rt, loc_2, mana)?;
        link(rt, loc_2, lnk_2);
      }
      CTR | FUN => {
        for i in 0 .. rt.get_arity(get_ext(norm)) {
          let loc_i = get_loc(norm, i);
          let lnk_i = compute_at(rt, loc_i, mana)?;
          link(rt, loc_i, lnk_i);
        }
      }
      _ => {}
    };
    return Some(norm);
  } else {
    return Some(term);
  }
}

// Debug
// -----

pub fn show_lnk(x: Lnk) -> String {
  if x == 0 {
    String::from("~")
  } else {
    let tag = get_tag(x);
    let ext = get_ext(x);
    let val = get_val(x);
    let tgs = match tag {
      DP0 => "DP0",
      DP1 => "DP1",
      VAR => "VAR",
      ARG => "ARG",
      ERA => "ERA",
      LAM => "LAM",
      APP => "APP",
      SUP => "SUP",
      CTR => "CTR",
      FUN => "FUN",
      OP2 => "OP2",
      NUM => "NUM",
      _   => "?",
    };
    format!("{}:{}:{:x}", tgs, u128_to_name(ext), val)
  }
}

pub fn show_rt(rt: &Runtime) -> String {
  let mut s: String = String::new();
  for i in 0..32 {
    // pushes to the string
    s.push_str(&format!("{:x} | ", i));
    s.push_str(&show_lnk(rt.read(i)));
    s.push('\n');
  }
  s
}

pub fn show_term(rt: &Runtime, term: Lnk) -> String {
  let mut lets: HashMap<u128, u128> = HashMap::new();
  let mut kinds: HashMap<u128, u128> = HashMap::new();
  let mut names: HashMap<u128, String> = HashMap::new();
  let mut count: u128 = 0;
  fn find_lets(
    rt: &Runtime,
    term: Lnk,
    lets: &mut HashMap<u128, u128>,
    kinds: &mut HashMap<u128, u128>,
    names: &mut HashMap<u128, String>,
    count: &mut u128,
  ) {
    match get_tag(term) {
      LAM => {
        names.insert(get_loc(term, 0), format!("{}", count));
        *count += 1;
        find_lets(rt, ask_arg(rt, term, 1), lets, kinds, names, count);
      }
      APP => {
        find_lets(rt, ask_arg(rt, term, 0), lets, kinds, names, count);
        find_lets(rt, ask_arg(rt, term, 1), lets, kinds, names, count);
      }
      SUP => {
        find_lets(rt, ask_arg(rt, term, 0), lets, kinds, names, count);
        find_lets(rt, ask_arg(rt, term, 1), lets, kinds, names, count);
      }
      DP0 => {
        if let hash_map::Entry::Vacant(e) = lets.entry(get_loc(term, 0)) {
          names.insert(get_loc(term, 0), format!("{}", count));
          *count += 1;
          kinds.insert(get_loc(term, 0), get_ext(term));
          e.insert(get_loc(term, 0));
          find_lets(rt, ask_arg(rt, term, 2), lets, kinds, names, count);
        }
      }
      DP1 => {
        if let hash_map::Entry::Vacant(e) = lets.entry(get_loc(term, 0)) {
          names.insert(get_loc(term, 0), format!("{}", count));
          *count += 1;
          kinds.insert(get_loc(term, 0), get_ext(term));
          e.insert(get_loc(term, 0));
          find_lets(rt, ask_arg(rt, term, 2), lets, kinds, names, count);
        }
      }
      OP2 => {
        find_lets(rt, ask_arg(rt, term, 0), lets, kinds, names, count);
        find_lets(rt, ask_arg(rt, term, 1), lets, kinds, names, count);
      }
      CTR | FUN => {
        let arity = rt.get_arity(get_ext(term));
        for i in 0 .. arity {
          find_lets(rt, ask_arg(rt, term, i), lets, kinds, names, count);
        }
      }
      _ => {}
    }
  }
  fn go(rt: &Runtime, term: Lnk, names: &HashMap<u128, String>) -> String {
    let done = match get_tag(term) {
      DP0 => {
        format!("a{}", names.get(&get_loc(term, 0)).unwrap_or(&String::from("?a")))
      }
      DP1 => {
        format!("b{}", names.get(&get_loc(term, 0)).unwrap_or(&String::from("?b")))
      }
      VAR => {
        format!("x{}", names.get(&get_loc(term, 0)).unwrap_or(&String::from("?c")))
      }
      LAM => {
        let name = format!("x{}", names.get(&get_loc(term, 0)).unwrap_or(&String::from("?")));
        format!("@{} {}", name, go(rt, ask_arg(rt, term, 1), names))
      }
      APP => {
        let func = go(rt, ask_arg(rt, term, 0), names);
        let argm = go(rt, ask_arg(rt, term, 1), names);
        format!("({} {})", func, argm)
      }
      SUP => {
        //let kind = get_ext(term);
        let func = go(rt, ask_arg(rt, term, 0), names);
        let argm = go(rt, ask_arg(rt, term, 1), names);
        format!("{{{} {}}}", func, argm)
      }
      OP2 => {
        let oper = get_ext(term);
        let val0 = go(rt, ask_arg(rt, term, 0), names);
        let val1 = go(rt, ask_arg(rt, term, 1), names);
        let symb = match oper {
          ADD => "+",
          SUB => "-",
          MUL => "*",
          DIV => "/",
          MOD => "%",
          AND => "&",
          OR  => "|",
          XOR => "^",
          SHL => "<<",
          SHR => ">>",
          LTN => "<",
          LTE => "<=",
          EQL => "=",
          GTE => ">=",
          GTN => ">",
          NEQ => "!=",
          _   => "?",
        };
        format!("({} {} {})", symb, val0, val1)
      }
      NUM => {
        let numb = get_num(term);
        // If it has 26-30 bits, pretty-print as a name
        //if numb > 0x3FFFFFF && numb <= 0x3FFFFFFF {
          //return format!("@{}", view_name(numb));
        //} else {
          return format!("#{}", numb);
        //}
      }
      CTR => {
        let func = get_ext(term);
        let arit = rt.get_arity(func);
        //println!("  - arity is: {} {}", u128_to_name(func), arit);
        let args: Vec<String> = (0..arit).map(|i| go(rt, ask_arg(rt, term, i), names)).collect();
        format!("$({}{})", u128_to_name(func), args.iter().map(|x| format!(" {}", x)).collect::<String>())
      }
      FUN => {
        let func = get_ext(term);
        let arit = rt.get_arity(func);
        //println!("  - arity is: {} {}", u128_to_name(func), arit);
        let args: Vec<String> = (0..arit).map(|i| go(rt, ask_arg(rt, term, i), names)).collect();
        format!("!({}{})", u128_to_name(func), args.iter().map(|x| format!(" {}", x)).collect::<String>())
      }
      ERA => {
        "*".to_string()
      }
      _ => format!("?g({})", get_tag(term)),
    };
    return done;
  }
  find_lets(rt, term, &mut lets, &mut kinds, &mut names, &mut count);
  let mut text = go(rt, term, &names);
  for (_key, pos) in lets {
    // todo: reverse
    let what = String::from("?h");
    //let kind = kinds.get(&key).unwrap_or(&0);
    let name = names.get(&pos).unwrap_or(&what);
    let nam0 = if ask_lnk(rt, pos + 0) == Era() { String::from("*") } else { format!("a{}", name) };
    let nam1 = if ask_lnk(rt, pos + 1) == Era() { String::from("*") } else { format!("b{}", name) };
    text.push_str(&format!(" | &{{{} {}}} = {};", nam0, nam1, go(rt, ask_lnk(rt, pos + 2), &names)));
  }
  text
}

// Parsing
// -------

fn head(code: &str) -> char {
  return code.chars().take(1).last().unwrap_or('\0');
}

fn tail(code: &str) -> &str {
  if code.len() > 0 {
    return &code[head(code).len_utf8()..];
  } else {
    return "";
  }
}

fn skip(code: &str) -> &str {
  let mut code = code;
  loop {
    if head(code) == ' ' || head(code) == '\n' {
      while head(code) == ' ' || head(code) == '\n' {
        code = tail(code);
      }
      continue;
    }
    if head(code) == '/' && head(tail(code)) == '/' {
      while head(code) != '\n' && head(code) != '\0' {
        code = tail(code);
      }
      continue;
    }
    break;
  }
  return code;
}

fn hash(name: &str) -> u128 {
  let mut hasher = hash_map::DefaultHasher::new();
  name.hash(&mut hasher);
  hasher.finish() as u128
}

fn is_name_char(chr: char) -> bool {
  return chr == '_' || chr == '.'
      || chr >= 'a' && chr <= 'z'
      || chr >= 'A' && chr <= 'Z'
      || chr >= '0' && chr <= '9';
}

fn read_char(code: &str, chr: char) -> (&str, ()) {
  let code = skip(code);
  if head(code) == chr {
    return (tail(code), ());
  } else {
    panic!("Expected '{}', found '{}'. Context:\n\x1b[2m{}\x1b[0m", chr, head(code), code.chars().take(256).collect::<String>());
  }
}

fn read_numb(code: &str) -> (&str, u128) {
  let code = skip(code);
  let mut numb = 0;
  let mut code = code;
  while head(code) >= '0' && head(code) <= '9' {
    numb = numb * 10 + head(code) as u128 - 0x30;
    code = tail(code);
  }
  return (code, numb);
}

fn read_name(code: &str) -> (&str, u128) {
  let code = skip(code);
  let mut name = String::new();
  if head(code) == '~' {
    return (tail(code), VAR_NONE);
  } else {
    let mut code = code;
    while is_name_char(head(code)) {
      name.push(head(code));
      code = tail(code);
    }
    if name.is_empty() {
      panic!("Expected identifier, found `{}`.", head(code));
    }
    // TODO: check identifier size and propagate error
    return (code, name_to_u128(&name));
  }
}

/// Converts a name to a number, using the following table:
/// ```
/// '.'       =>  0
/// '0' - '9' =>  1 to 10
/// 'A' - 'Z' => 11 to 36
/// 'a' - 'z' => 37 to 62
/// '_'       => 63
/// ```
pub fn name_to_u128(name: &str) -> u128 {
  let mut num: u128 = 0;
  for (i, chr) in name.chars().enumerate() {
    debug_assert!(i < 20, "Name too big: `{}`.", name);
    if chr == '.' {
      num = num * 64 + 0;
    } else if chr >= '0' && chr <= '9' {
      num = num * 64 + 1 + chr as u128 - '0' as u128;
    } else if chr >= 'A' && chr <= 'Z' {
      num = num * 64 + 11 + chr as u128 - 'A' as u128;
    } else if chr >= 'a' && chr <= 'z' {
      num = num * 64 + 37 + chr as u128 - 'a' as u128;
    } else if chr == '_' {
      num = num * 64 + 63;
    }
  }
  return num;
}

/// Inverse of `name_to_u128`
pub fn u128_to_name(num: u128) -> String {
  let mut name = String::new();
  let mut num = num;
  while num > 0 {
    let chr = (num % 64) as u8;
    let chr =
        match chr {
            0         => '.',
            1  ..= 10 => (chr -  1 + b'0') as char,
            11 ..= 36 => (chr - 11 + b'A') as char,
            37 ..= 62 => (chr - 37 + b'a') as char,
            63        => '_',
            64 ..     => panic!("impossible character value")
        };
    name.push(chr);
    num = num / 64;
  }
  name.chars().rev().collect()
}

fn read_until<A>(code: &str, stop: char, read: fn(&str) -> (&str, A)) -> (&str, Vec<A>) {
  let mut elems = Vec::new();
  let mut code = code;
  while code.len() > 0 && head(skip(code)) != stop {
    let (new_code, elem) = read(code);
    code = new_code;
    elems.push(elem);
  }
  code = tail(skip(code));
  return (code, elems);
}

pub fn read_term(code: &str) -> (&str, Term) {
  let code = skip(code);
  match head(code) {
    '@' => {
      let code         = tail(code);
      let (code, name) = read_name(code);
      let (code, body) = read_term(code);
      return (code, Term::Lam { name, body: Box::new(body) });
    },
    '&' => {
      let code         = tail(code);
      let (code, skip) = read_char(code, '{');
      let (code, nam0) = read_name(code);
      let (code, nam1) = read_name(code);
      let (code, skip) = read_char(code, '}');
      let (code, skip) = read_char(code, '=');
      let (code, expr) = read_term(code);
      let (code, skip) = read_char(code, ';');
      let (code, body) = read_term(code);
      return (code, Term::Dup { nam0, nam1, expr: Box::new(expr), body: Box::new(body) });
    },
    '(' => {
      let code = tail(code);
      let (code, oper) = read_oper(code);
      if let Some(oper) = oper {
        let code = tail(code);
        let (code, val0) = read_term(code);
        let (code, val1) = read_term(code);
        let (code, skip) = read_char(code, ')');
        return (code, Term::Op2 { oper: oper, val0: Box::new(val0), val1: Box::new(val1) });
      } else {
        let (code, func) = read_term(code);
        let (code, argm) = read_term(code);
        let (code, skip) = read_char(code, ')');
        return (code, Term::App { func: Box::new(func), argm: Box::new(argm) });
      }
    },
    '$' => {
      let code = tail(code);
      let (code, skip) = read_char(code, '(');
      let (code, name) = read_name(code);
      let (code, args) = read_until(code, ')', read_term);
      return (code, Term::Ctr { name, args });
    },
    '!' => {
      let code = tail(code);
      let (code, skip) = read_char(code, '(');
      let (code, name) = read_name(code);
      let (code, args) = read_until(code, ')', read_term);
      // TODO: check function name size _on direct calling_, and propagate error
      return (code, Term::Fun { name, args });
    },
    '#' => {
      let code = tail(code);
      let (code, numb) = read_numb(code);
      return (code, Term::Num { numb });
    },
    '\'' => {
      let code = tail(code);
      let (code, numb) = read_name(code);
      let (code, skip) = read_char(code, '\'');
      return (code, Term::Num { numb });
    },
    _ => {
      let (code, name) = read_name(code);
      return (code, Term::Var { name });
    }
  }
}

fn read_oper(code: &str) -> (&str, Option<u128>) {
  let code = skip(code);
  match head(code) {
    '+' => (tail(code), Some(ADD)),
    '-' => (tail(code), Some(SUB)),
    '*' => (tail(code), Some(MUL)),
    '/' => (tail(code), Some(DIV)),
    '%' => (tail(code), Some(MOD)),
    '&' => (tail(code), Some(AND)),
    '|' => (tail(code), Some(OR)),
    '^' => (tail(code), Some(XOR)),
    '<' => {
      let code = tail(code);
      if head(code) == '=' { 
        (tail(code), Some(LTE))
      } else if head(code) == '<' {
        (tail(code), Some(SHL))
      } else {
        (code, Some(LTN))
      }
    },
    '>' => {
      let code = tail(code);
      if head(code) == '=' { 
        (tail(code), Some(GTE))
      } else if head(code) == '>' {
        (tail(code), Some(SHR))
      } else {
        (code, Some(GTN))
      }
    },
    '=' => {
      if head(tail(code)) == '=' {
        (tail(tail(code)), Some(EQL))
      } else {
        (code, None)
      }
    },
    '!' => {
      if head(tail(code)) == '=' {
        (tail(tail(code)), Some(NEQ))
      } else {
        (code, None)
      }
    },
    _ => (code, None)
  }
}

fn read_rule(code: &str) -> (&str, (Term,Term)) {
  let (code, lhs) = read_term(code);
  let (code, ())  = read_char(code, '=');
  let (code, rhs) = read_term(code);
  return (code, (lhs, rhs));
}

fn read_rules(code: &str) -> (&str, Vec<(Term,Term)>) {
  let (code, rules) = read_until(code, '\0', read_rule);
  return (code, rules);
}

fn read_func(code: &str) -> (&str, CompFunc) {
  let (code, rules) = read_until(code, '\0', read_rule);
  if let Some(func) = build_func(&rules, false) {
    return (code, func);
  } else {
    panic!("Couldn't parse function.");
  }
}

fn read_statement(code: &str) -> (&str, Statement) {
  let code = skip(code);
  match head(code) {
    '!' => {
      let code = tail(code);
      let (code, skip) = read_char(code, '(');
      let (code, name) = read_name(code);
      let (code, args) = read_until(code, ')', read_name);
      let (code, skip) = read_char(code, '{');
      let (code, func) = read_until(code, '}', read_rule);
      let (code, skip) = read_char(code, '=');
      let (code, init) = read_term(code);
      return (code, Statement::Fun { name, args, func, init });
    }
    '$' => {
      let code = tail(code);
      let (code, skip) = read_char(code, '(');
      let (code, name) = read_name(code);
      let (code, args) = read_until(code, ')', read_name);
      return (code, Statement::Ctr { name, args });
    }
    '{' => {
      let code = tail(code);
      let (code, expr) = read_term(code);
      let (code, skip) = read_char(code, '}');
      return (code, Statement::Run { expr });
    }
    _ => {
      panic!("Couldn't parse statement.");
    }
  }
}

pub fn read_statements(code: &str) -> (&str, Vec<Statement>) {
  let (code, statements) = read_until(code, '\0', read_statement);
  //for statement in &statements {
    //println!("... statement {}", view_statement(statement));
  //}
  return (code, statements);
}

// View
// ----

pub fn view_name(name: u128) -> String {
  if name == VAR_NONE {
    return "~".to_string();
  } else {
    return u128_to_name(name);
  }
}

pub fn view_term(term: &Term) -> String {
  match term {
    Term::Var { name } => {
      return view_name(*name);
    }
    Term::Dup { nam0, nam1, expr, body } => {
      let nam0 = view_name(*nam0);
      let nam1 = view_name(*nam1);
      let expr = view_term(expr);
      let body = view_term(body);
      return format!("&{{{} {}}} = {}; {}", nam0, nam1, expr, body);
    }
    Term::Lam { name, body } => {
      let name = view_name(*name);
      let body = view_term(body);
      return format!("@{} {}", name, body);
    }
    Term::App { func, argm } => {
      let func = view_term(func);
      let argm = view_term(argm);
      return format!("({} {})", func, argm);
    }
    Term::Ctr { name, args } => {
      let name = view_name(*name);
      let args = args.iter().map(|x| format!(" {}", view_term(x))).collect::<Vec<String>>().join("");
      return format!("$({}{})", name, args);
    }
    Term::Fun { name, args } => {
      let name = view_name(*name);
      let args = args.iter().map(|x| format!(" {}", view_term(x))).collect::<Vec<String>>().join("");
      return format!("!({}{})", name, args);
    }
    Term::Num { numb } => {
      // If it has 26-30 bits, pretty-print as a name
      //if *numb > 0x3FFFFFF && *numb <= 0x3FFFFFFF {
        //return format!("@{}", view_name(*numb));
      //} else {
        return format!("#{}", numb);
      //}
    }
    Term::Op2 { oper, val0, val1 } => {
      let oper = view_oper(oper);
      let val0 = view_term(val0);
      let val1 = view_term(val1);
      return format!("({} {} {})", oper, val0, val1);
    }
  }
}

pub fn view_oper(oper: &u128) -> String {
  match oper {
     0 => "+".to_string(),
     1 => "-".to_string(),
     2 => "*".to_string(),
     3 => "/".to_string(),
     4 => "%".to_string(),
     5 => "&".to_string(),
     6 => "|".to_string(),
     7 => "^".to_string(),
     8 => "<<".to_string(),
     9 => ">>".to_string(),
    10 => "<=".to_string(),
    11 => "<".to_string(),
    12 => "==".to_string(),
    13 => ">=".to_string(),
    14 => ">".to_string(),
    15 => "!=".to_string(),
     _ => "?".to_string(),
  }
}

pub fn view_statement(statement: &Statement) -> String {
  match statement {
    Statement::Fun { name, args, func, init } => {
      let name = u128_to_name(*name);
      let func = func.iter().map(|x| format!("  {} = {}", view_term(&x.0), view_term(&x.1))).collect::<Vec<String>>().join("\n");
      let args = args.iter().map(|x| u128_to_name(*x)).collect::<Vec<String>>().join(" ");
      let init = view_term(init);
      return format!("!({} {}) {{\n{}\n}} = {}", name, args, func, init);
    }
    Statement::Ctr { name, args } => {
      // correct:
      let name = u128_to_name(*name);
      let args = args.iter().map(|x| u128_to_name(*x)).collect::<Vec<String>>().join(" ");
      return format!("$({} {})", name, args);
    }
    Statement::Run { expr } => {
      let expr = view_term(expr);
      return format!("{{\n  {}\n}}", expr);
    }
  }
}

pub fn view_statements(statements: &[Statement]) -> String {
  let mut result = String::new();
  for statement in statements {
    result.push_str(&view_statement(statement));
    result.push_str("\n");
  }
  return result;
}

// Tests
// -----

// Serializes, deserializes and evaluates statements
pub fn test_statements(statements: &[Statement]) {
  //println!("[Serialization]");
  let str_0 = view_statements(statements);
  let str_1 = view_statements(&crate::bits::deserialized_statements(&crate::bits::serialized_statements(&statements)));
  //println!("[Deserialization] {}", if str_0 == str_1 { "(ok)" } else { "(error: not equal)" });
  //println!("{}", str_0);
  //println!("---------------");
  //println!("{}", str_1);

  println!("[Evaluation] {}", if str_0 == str_1 { "" } else { "(note: serialization error, please report)" });
  let mut rt = init_runtime();
  let init = Instant::now();
  rt.run_statements(&statements);
  println!();

  println!("[Stats]");
  println!("- cost: {} mana ({} rewrites)", rt.get_mana(), rt.get_rwts());
  println!("- size: {} words", rt.get_size());
  println!("- time: {} ms", init.elapsed().as_millis());

}

pub fn test_statements_from_code(code: &str) {
  test_statements(&read_statements(code).1);
}

pub fn test(file: &str) {
  test_statements_from_code(&std::fs::read_to_string(file).expect("file not found"));
}

fn test_heap_checksum(fn_names: &[&str], rt: &mut Runtime) -> u64 {
  let fn_ids = fn_names.iter().map(|x| name_to_u128(x)).collect::<Vec<u128>>();
  let mut hasher = DefaultHasher::new();
  for fn_id in fn_ids {
    let term_lnk = rt.read_disk(fn_id);
    if let Some(term_lnk) = term_lnk {
      let term = rt.show_term(term_lnk);
      term.hash(&mut hasher);
    }
  }
  let res = hasher.finish();
  res
}

pub fn test_runtime_rollback() {
  let pre_code = "
    $(Succ p)
    $(Zero)
    !(Add n) {
      !(Add n) = $(Succ n)
    } = #0
    
    !(Sub n) {
      !(Sub $(Succ p)) = p
      !(Sub $(Zero)) = $(Zero)
    } = #0
    
    !(Store action) {
      !(Store $(Add)) =
        $(IO.take @l 
        $(IO.save !(Add l) @~
        $(IO.done #0)))
      !(Store $(Sub)) =
        $(IO.take @l 
        $(IO.save !(Sub l) @~
        $(IO.done #0)))
      !(Store $(Get)) = !(IO.load @l $(IO.done l))
    } = $(Zero)
  ";
  let code = "
    {
      $(IO.call 'Count' $(Tuple1 $(Inc #1)) @~
      $(IO.call 'Count' $(Tuple1 $(Get)) @x
      $(IO.done x)))
    }
    
    {
     $(IO.call 'Store' $(Tuple1 $(Add)) @~
     $(IO.call 'Store' $(Tuple1 $(Get)) @x
     $(IO.done x)))
    }
  ";
  let fn_names = ["Count", "IO.load", "Store", "Sub", "Add"];
  let mut rt = init_runtime();
  let init_tick = rt.get_tick();
  
  
  let total_tick = 50;
  rt.run_statements_from_code(pre_code);
  for i in 0..total_tick {
    rt.run_statements_from_code(code);
    rt.tick();
    println!("{}", test_heap_checksum(&fn_names, &mut rt));
  }
  println!("{}", test_heap_checksum(&fn_names, &mut rt));
  rt.rollback(init_tick + 49);
  println!("{}", test_heap_checksum(&fn_names, &mut rt));

  // rt.rollback(init_tick + 8);
  // test_heap_checksum(&fn_names, &mut rt);
}
