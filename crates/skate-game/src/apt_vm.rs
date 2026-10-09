//! APT's compact ActionScript instruction stream. All execution is bounded;
//! native callbacks are supplied by the HUD owner, never by extracted scripts.
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Value {
    #[default]
    Undefined,
    Number(f64),
    Bool(bool),
    Text(String),
    Object(usize),
}
impl Value {
    pub fn number(&self) -> f64 {
        match self {
            Self::Number(n) => *n,
            Self::Bool(v) => {
                if *v {
                    1.0
                } else {
                    0.0
                }
            }
            Self::Text(s) => s.parse().unwrap_or(f64::NAN),
            _ => f64::NAN,
        }
    }
    pub fn truth(&self) -> bool {
        match self {
            Self::Undefined => false,
            Self::Number(n) => *n != 0.0 && !n.is_nan(),
            Self::Bool(v) => *v,
            Self::Text(s) => !s.is_empty(),
            Self::Object(_) => true,
        }
    }
    pub fn text(&self) -> String {
        match self {
            Self::Undefined => "undefined".into(),
            Self::Number(n) => n.to_string(),
            Self::Bool(v) => v.to_string(),
            Self::Text(s) => s.clone(),
            Self::Object(_) => "[object Object]".into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Constant {
    pub kind: u32,
    pub value: serde_json::Value,
}
impl Constant {
    fn value(&self) -> Result<Value, String> {
        Ok(match self.kind {
            1 => Value::Text(
                self.value
                    .as_str()
                    .ok_or("Invalid APT string constant")?
                    .into(),
            ),
            5 => Value::Bool(self.value.as_u64().ok_or("Invalid APT bool constant")? != 0),
            6 | 7 => Value::Number(self.value.as_f64().ok_or("Invalid APT number constant")?),
            3 => Value::Undefined,
            _ => return Err(format!("Unresolved APT constant type {}", self.kind)),
        })
    }
}
#[derive(Clone, Debug, Deserialize)]
pub struct Parameter {
    pub register: usize,
    pub name: String,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Instruction {
    pub offset: u32,
    pub opcode: u8,
    #[serde(default)]
    pub next: u32,
    #[serde(default)]
    pub operand: serde_json::Value,
    pub target: Option<u32>,
    #[serde(default)]
    pub values: Vec<Constant>,
    #[serde(default)]
    pub body: Vec<Instruction>,
    #[serde(default)]
    pub flags: u32,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    #[serde(default)]
    pub name: String,
}

#[derive(Clone, Debug)]
pub enum ObjectKind {
    Plain,
    Function(usize),
    Native(String),
}
#[derive(Clone, Debug)]
pub struct Object {
    pub kind: ObjectKind,
    pub fields: BTreeMap<String, Value>,
    pub prototype: Option<usize>,
}
#[derive(Clone, Debug)]
struct Function {
    code: Instruction,
    constants: Vec<Value>,
}

struct Scope {
    this: usize,
    local_definitions: bool,
    locals: BTreeMap<String, Value>,
}

pub trait Host {
    fn property_changed(&mut self, _vm: &mut Vm, _object: usize, _key: &str) -> Result<(), String> {
        Ok(())
    }
    /// ActionScript `trace` (opcode 0x26). Retail prints "AptTrace: %s" to the
    /// debug output (handler 82E6D958); hosts may log it, the default drops it.
    fn trace(&mut self, _message: &str) {}
    fn call(
        &mut self,
        vm: &mut Vm,
        object: usize,
        method: &str,
        arguments: Vec<Value>,
    ) -> Result<Value, String>;
}
#[derive(Default)]
pub struct Vm {
    pub objects: Vec<Object>,
    free_objects: Vec<usize>,
    functions: Vec<Function>,
    pub global: usize,
    remaining: usize,
    depth: usize,
    /// State of the script `random` generator (opcode 0x30). Seeded, so a run
    /// is deterministic (replays, multiplayer); see `seed_random`.
    random_state: u64,
}

/// Retail ToInteger (82E5F2A8): int as is, float clamped to the i32 range
/// and truncated (NaN gives i32::MIN like `fctiwz`), bool 0 / 1, strings via
/// strtol base 16 when longer than 2 chars and starting "0x", else atoi;
/// undefined 0, any other value (objects) 1.
pub fn to_integer(value: &Value) -> i32 {
    match value {
        Value::Undefined => 0,
        Value::Bool(v) => *v as i32,
        Value::Object(_) => 1,
        Value::Number(n) => {
            if n.is_nan() {
                i32::MIN
            } else {
                n.clamp(i32::MIN as f64, i32::MAX as f64) as i32
            }
        }
        Value::Text(s) => {
            let b = s.as_bytes();
            if b.len() > 2 && b[0] == b'0' && b[1] == b'x' {
                c_parse(&s[2..], 16)
            } else {
                c_parse(s, 10)
            }
        }
    }
}
/// C `strtol` / `atoi` subset: leading spaces, optional sign, digits until the
/// first non-digit, saturating instead of undefined overflow.
fn c_parse(s: &str, radix: u32) -> i32 {
    let s = s.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let (negative, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let mut value: i64 = 0;
    for c in digits.chars() {
        let Some(d) = c.to_digit(radix) else { break };
        value = (value * radix as i64 + d as i64).min(1 << 32);
    }
    let value = if negative { -value } else { value };
    value.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}
/// Retail ToNumber action (0x4A, handler 82E702F0): numbers stay; values the
/// retail NaN test (82E67400) rejects (undefined, objects, empty or
/// non-numeric strings) become NaN; a string without '.' becomes an integer
/// via ToInteger, one with '.' a float; bools become 0 / 1.
pub fn to_number(value: &Value) -> Value {
    match value {
        Value::Number(n) => Value::Number(*n),
        Value::Bool(v) => Value::Number(*v as i32 as f64),
        Value::Undefined | Value::Object(_) => Value::Number(f64::NAN),
        Value::Text(s) => {
            let t = s.trim();
            let b = t.as_bytes();
            let hex = b.len() > 2 && b[0] == b'0' && b[1] == b'x';
            let numeric = if hex {
                t[2..].chars().all(|c| c.is_ascii_hexdigit())
            } else {
                !t.is_empty() && t.parse::<f64>().is_ok_and(|n| n.is_finite())
            };
            if !numeric {
                Value::Number(f64::NAN)
            } else if hex || !t.contains('.') {
                Value::Number(to_integer(&Value::Text(t.into())) as f64)
            } else {
                Value::Number(t.parse::<f32>().map_or(f64::NAN, |n| n as f64))
            }
        }
    }
}
impl Vm {
    fn variable(&self, scope: &Scope, key: &str) -> Value {
        if let Some(value) = scope.locals.get(key) {
            return value.clone();
        }
        let value = self.get(scope.this, key);
        if value != Value::Undefined {
            return value;
        }
        self.get(self.global, key)
    }
    pub fn new() -> Self {
        let mut vm = Self::default();
        vm.global = vm.object(ObjectKind::Plain);
        vm
    }
    pub fn object(&mut self, kind: ObjectKind) -> usize {
        if let Some(id) = self.free_objects.pop() {
            self.objects[id] = Object {
                kind,
                fields: BTreeMap::new(),
                prototype: None,
            };
            return id;
        }
        let id = self.objects.len();
        self.objects.push(Object {
            kind,
            fields: BTreeMap::new(),
            prototype: None,
        });
        id
    }
    /// Collect only between updates. Host-owned movie handles are explicit
    /// roots; script references and prototypes preserve retired clips when
    /// they remain reachable. Stable handles never move during collection.
    pub fn collect(&mut self, host_roots: impl IntoIterator<Item = usize>) -> Result<(), String> {
        if self.depth != 0 {
            return Err("Cannot collect during an APT call".into());
        }
        let mut marked = vec![false; self.objects.len()];
        let mut pending: Vec<_> = host_roots.into_iter().collect();
        pending.push(self.global);
        while let Some(id) = pending.pop() {
            let seen = marked.get_mut(id).ok_or("Invalid APT collection root")?;
            if *seen {
                continue;
            }
            *seen = true;
            let object = &self.objects[id];
            if let Some(id) = object.prototype {
                pending.push(id);
            }
            for value in object.fields.values() {
                if let Value::Object(id) = value {
                    pending.push(*id);
                }
            }
        }
        self.free_objects.clear();
        for (id, live) in marked.into_iter().enumerate() {
            if !live {
                self.objects[id] = Object {
                    kind: ObjectKind::Plain,
                    fields: BTreeMap::new(),
                    prototype: None,
                };
                self.free_objects.push(id);
            }
        }
        Ok(())
    }
    pub fn set(
        &mut self,
        object: usize,
        key: impl Into<String>,
        value: Value,
    ) -> Result<(), String> {
        self.objects
            .get_mut(object)
            .ok_or("Invalid APT object handle")?
            .fields
            .insert(key.into(), value);
        Ok(())
    }
    pub fn get(&self, object: usize, key: &str) -> Value {
        let mut current = Some(object);
        for _ in 0..64 {
            let Some(o) = current.and_then(|i| self.objects.get(i)) else {
                break;
            };
            if let Some(v) = o.fields.get(key) {
                return v.clone();
            }
            current = o.prototype;
        }
        Value::Undefined
    }
    /// Seed for the script `random` generator. Hosts that replay or share a
    /// session seed it from session data; the default seed is fixed.
    pub fn seed_random(&mut self, seed: u64) {
        self.random_state = seed;
    }
    /// Next value of the deterministic generator (xorshift64*). Retail uses
    /// its own runtime generator (82E82528, Mersenne-Twister style) whose seed
    /// is not reproducible, so only the `% n` contract is retail's.
    fn next_random(&mut self) -> u32 {
        if self.random_state == 0 {
            self.random_state = 0x9E37_79B9_7F4A_7C15;
        }
        let mut x = self.random_state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.random_state = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as u32
    }
    pub fn begin_update(&mut self) {
        self.remaining = 100_000;
    }
    pub fn run(&mut self, code: &[Instruction], host: &mut impl Host) -> Result<Value, String> {
        self.begin_update();
        self.run_on(self.global, code, host)
    }
    pub fn run_on(
        &mut self,
        object: usize,
        code: &[Instruction],
        host: &mut impl Host,
    ) -> Result<Value, String> {
        self.execute(
            code,
            &mut vec![Value::Undefined; 256],
            &mut Vec::new(),
            &mut Scope {
                this: object,
                local_definitions: false,
                locals: BTreeMap::new(),
            },
            host,
        )
    }
    pub fn call_method(
        &mut self,
        object: usize,
        method: &str,
        args: Vec<Value>,
        host: &mut impl Host,
    ) -> Result<Value, String> {
        if let Value::Object(f) = self.get(object, method) {
            if let Some(Object {
                kind: ObjectKind::Function(index),
                ..
            }) = self.objects.get(f)
            {
                return self.invoke(*index, object, args, host);
            }
        }
        host.call(self, object, method, args)
    }
    /// Use the same prototype, argument and execution limits as NewObject.
    /// The caller owns the update boundary; construction never replenishes it.
    pub fn construct(
        &mut self,
        class: usize,
        args: Vec<Value>,
        host: &mut impl Host,
    ) -> Result<usize, String> {
        if args.len() > 256 {
            return Err("APT argument limit".into());
        }
        let kind = self
            .objects
            .get(class)
            .ok_or("Invalid APT class")?
            .kind
            .clone();
        if self.remaining == 0 || self.objects.len() > 4096 {
            return Err("APT execution/object budget exceeded".into());
        }
        let id = self.object(ObjectKind::Plain);
        if let Value::Object(proto) = self.get(class, "prototype") {
            self.objects[id].prototype = Some(proto);
        }
        if let ObjectKind::Function(f) = kind {
            self.invoke(f, id, args, host)?;
        }
        Ok(id)
    }
    fn invoke(
        &mut self,
        index: usize,
        this: usize,
        args: Vec<Value>,
        host: &mut impl Host,
    ) -> Result<Value, String> {
        if self.depth >= 32 {
            return Err("APT call-depth limit".into());
        }
        let f = self
            .functions
            .get(index)
            .ok_or("Invalid APT function")?
            .clone();
        let mut regs = vec![Value::Undefined; 256];
        let mut reg = 1;
        let mut scope = Scope {
            this,
            local_definitions: true,
            locals: BTreeMap::new(),
        };
        if f.code.flags & 2 == 0 {
            scope.locals.insert("this".into(), Value::Object(this));
        }
        let root = match self.get(this, "_root") {
            Value::Undefined => Value::Object(self.global),
            value => value,
        };
        let parent = self.get(this, "_parent");
        let arguments = if f.code.flags & 4 != 0 || f.code.flags & 8 == 0 {
            let object = self.object(ObjectKind::Plain);
            for (index, value) in args.iter().enumerate() {
                self.set(object, index.to_string(), value.clone())?;
            }
            self.set(object, "length", Value::Number(args.len() as f64))?;
            Value::Object(object)
        } else {
            Value::Undefined
        };
        if f.code.flags & 8 == 0 {
            scope.locals.insert("arguments".into(), arguments.clone());
        }
        // DefineFunction2 preload flags (82E47688). Native integer registers
        // are scoped to a call, unlike persistent display-object properties.
        for (flag, value) in [
            (1, Value::Object(this)),
            (4, arguments),
            (16, Value::Undefined),
            (64, root),
            (128, parent),
            (256, Value::Object(self.global)),
        ] {
            if f.code.flags & flag != 0 {
                regs[reg] = value;
                reg += 1;
            }
        }
        for (i, p) in f.code.parameters.iter().enumerate() {
            if p.register >= regs.len() {
                return Err("APT parameter register outside bank".into());
            }
            if p.register != 0 {
                regs[p.register] = args.get(i).cloned().unwrap_or_default();
            } else {
                scope
                    .locals
                    .insert(p.name.clone(), args.get(i).cloned().unwrap_or_default());
            }
        }
        self.depth += 1;
        let result = self.execute(
            &f.code.body,
            &mut regs,
            &mut f.constants.clone(),
            &mut scope,
            host,
        );
        self.depth -= 1;
        result
    }
    fn execute(
        &mut self,
        code: &[Instruction],
        regs: &mut Vec<Value>,
        constants: &mut Vec<Value>,
        scope: &mut Scope,
        host: &mut impl Host,
    ) -> Result<Value, String> {
        let mut stack = Vec::<Value>::new();
        let mut pc = 0;
        fn pop(s: &mut Vec<Value>) -> Result<Value, String> {
            s.pop().ok_or("APT stack underflow".into())
        }
        while let Some(i) = code.get(pc) {
            if self.remaining == 0 || self.objects.len() > 4096 {
                return Err("APT execution/object budget exceeded".into());
            }
            self.remaining -= 1;
            pc += 1;
            let op = i.opcode;
            let operand = i.operand.as_u64().unwrap_or(0) as usize;
            let constant = |k: usize| {
                constants
                    .get(k)
                    .cloned()
                    .ok_or_else(|| format!("APT constant index {k} at {:x}", i.offset))
            };
            match op {
                0 => break,
                0x70 => stack.push(Value::Object(scope.this)),
                0xa1 => stack.push(Value::Text(
                    i.operand
                        .as_str()
                        .ok_or("APT string operand missing")?
                        .into(),
                )),
                0xa4 => stack.push(self.variable(
                    scope,
                    i.operand.as_str().ok_or("APT variable operand missing")?,
                )),
                0x06 | 0x07 => {
                    host.call(
                        self,
                        scope.this,
                        if op == 6 { "play" } else { "stop" },
                        vec![],
                    )?;
                }
                0x17 => {
                    pop(&mut stack)?;
                }
                0x12 => {
                    let a = pop(&mut stack)?;
                    stack.push(Value::Bool(!a.truth()));
                }
                0x4c => stack.push(
                    stack
                        .last()
                        .cloned()
                        .ok_or("APT duplicate on empty stack")?,
                ),
                0x59 => stack.push(Value::Number(0.0)),
                0x5a => stack.push(Value::Number(1.0)),
                0x71 => stack.push(Value::Object(self.global)),
                0x73 => stack.push(Value::Bool(true)),
                0x74 => stack.push(Value::Bool(false)),
                0x75 | 0x76 => stack.push(Value::Undefined),
                0xb5 => stack.push(Value::Number((operand as u8 as i8) as f64)),
                0xb6 => stack.push(Value::Number((operand as u16 as i16) as f64)),
                0xb7 => stack.push(Value::Number((operand as u32 as i32) as f64)),
                0xb4 => stack.push(Value::Number(f32::from_bits(operand as u32) as f64)),
                0xb9 => stack.push(
                    regs.get(operand)
                        .cloned()
                        .ok_or("APT register outside bank")?,
                ),
                0x87 => {
                    *regs.get_mut(operand).ok_or("APT register outside bank")? = stack
                        .last()
                        .cloned()
                        .ok_or("APT register store on empty stack")?;
                }
                0x88 => {
                    *constants = i
                        .values
                        .iter()
                        .map(Constant::value)
                        .collect::<Result<_, _>>()?;
                }
                0x96 => {
                    for c in &i.values {
                        stack.push(c.value()?);
                    }
                }
                0xa2 | 0xa3 => stack.push(constant(operand)?),
                0xae => stack.push(self.variable(scope, &constant(operand)?.text())),
                // EA trace (82E6D958): pop one value, print it.
                0x26 => {
                    let v = pop(&mut stack)?;
                    host.trace(&v.text());
                }
                // EA random (82E6DDD0): replace the top with rand % n, n by
                // ToInteger and divided unsigned (divwu); retail traps on
                // n == 0, untrusted scripts get 0 instead.
                0x30 => {
                    let n = to_integer(&pop(&mut stack)?) as u32;
                    let r = self.next_random();
                    let value = if n == 0 { 0 } else { r % n };
                    stack.push(Value::Number(value as i32 as f64));
                }
                // EA toNumber (82E702F0): replace the top with its number.
                0x4a => {
                    let v = pop(&mut stack)?;
                    stack.push(to_number(&v));
                }
                // initArray (82E6E9C0) / initObject (82E6EB00): count by
                // ToInteger, <= 0 gives an empty object. Array element i is
                // the i-th value from the top (length = count); object pairs
                // are (value on top, name below), set from the top pair down,
                // so the deepest duplicate name wins like retail.
                0x42 | 0x43 => {
                    let n = to_integer(&pop(&mut stack)?).max(0) as usize;
                    let width = if op == 0x43 { 2 } else { 1 };
                    if n > 256 {
                        return Err("APT initializer limit".into());
                    }
                    if n * width > stack.len() {
                        return Err("APT stack underflow".into());
                    }
                    let id = self.object(ObjectKind::Plain);
                    for j in 0..n {
                        let v = pop(&mut stack)?;
                        if op == 0x42 {
                            self.set(id, j.to_string(), v)?;
                        } else {
                            let k = pop(&mut stack)?.text();
                            self.set(id, k, v)?;
                        }
                    }
                    if op == 0x42 {
                        self.set(id, "length", Value::Number(n as f64))?;
                    }
                    stack.push(Value::Object(id));
                }
                0x4e | 0xa5 | 0xaf => {
                    // 0xa5 (82E73F50) pushes its string operand, then runs the
                    // getMember handler 82E705C0 (0x4e).
                    let name = if op == 0xaf {
                        constant(operand)?
                    } else if op == 0xa5 {
                        Value::Text(
                            i.operand
                                .as_str()
                                .ok_or("APT string operand missing")?
                                .into(),
                        )
                    } else {
                        pop(&mut stack)?
                    }
                    .text();
                    let obj = pop(&mut stack)?;
                    stack.push(if let Value::Object(id) = obj {
                        self.get(id, &name)
                    } else {
                        Value::Undefined
                    });
                }
                0x4f => {
                    let v = pop(&mut stack)?;
                    let k = pop(&mut stack)?.text();
                    let o = pop(&mut stack)?;
                    if let Value::Object(id) = o {
                        self.set(id, &k, v)?;
                        host.property_changed(self, id, &k)?;
                    }
                }
                0x1c => {
                    let k = pop(&mut stack)?.text();
                    stack.push(self.variable(scope, &k));
                }
                0x1d | 0x3c | 0xa6 => {
                    // 0xa6 (82E74028) pushes its string operand, then runs the
                    // setVariable handler 82E6D248 (0x1d).
                    let v = if op == 0xa6 {
                        Value::Text(
                            i.operand
                                .as_str()
                                .ok_or("APT string operand missing")?
                                .into(),
                        )
                    } else {
                        pop(&mut stack)?
                    };
                    let k = pop(&mut stack)?.text();
                    if (op == 0x3c && scope.local_definitions) || scope.locals.contains_key(&k) {
                        scope.locals.insert(k, v);
                    } else {
                        self.set(scope.this, &k, v)?;
                        host.property_changed(self, scope.this, &k)?;
                    }
                }
                0x3a => {
                    let k = pop(&mut stack)?.text();
                    let o = pop(&mut stack)?;
                    let existed = if let Value::Object(id) = o {
                        self.objects[id].fields.remove(&k).is_some()
                    } else {
                        false
                    };
                    stack.push(Value::Bool(existed));
                }
                0x47 | 0x0b | 0x0c | 0x0d | 0x48 | 0x49 | 0x66 | 0x67 => {
                    let b = pop(&mut stack)?;
                    let a = pop(&mut stack)?;
                    stack.push(match op {
                        0x47 if matches!(a, Value::Text(_)) || matches!(b, Value::Text(_)) => {
                            Value::Text(a.text() + &b.text())
                        }
                        0x47 => Value::Number(a.number() + b.number()),
                        0x0b => Value::Number(a.number() - b.number()),
                        0x0c => Value::Number(a.number() * b.number()),
                        0x0d => Value::Number(a.number() / b.number()),
                        0x48 => Value::Bool(a.number() < b.number()),
                        0x67 => Value::Bool(a.number() > b.number()),
                        0x66 => Value::Bool(a == b),
                        _ => Value::Bool(a == b || a.number() == b.number()),
                    });
                }
                0x50 | 0x51 => {
                    let a = pop(&mut stack)?.number();
                    stack.push(Value::Number(a + if op == 0x50 { 1.0 } else { -1.0 }));
                }
                0x99 | 0x9d | 0xb8 => {
                    let jump = op == 0x99 || {
                        let v = pop(&mut stack)?.truth();
                        if op == 0xb8 { !v } else { v }
                    };
                    if jump {
                        let target = i.target.ok_or("APT branch lacks target")?;
                        if target == code.last().map_or(0, |x| x.next) {
                            break;
                        }
                        pc = code
                            .iter()
                            .position(|x| x.offset == target)
                            .ok_or_else(|| format!("APT invalid jump target {target:x}"))?;
                    }
                }
                0x8e | 0x9b => {
                    let index = self.functions.len();
                    self.functions.push(Function {
                        code: i.clone(),
                        constants: constants.clone(),
                    });
                    let object = self.object(ObjectKind::Function(index));
                    let proto = self.object(ObjectKind::Plain);
                    self.set(object, "prototype", Value::Object(proto))?;
                    if i.name.is_empty() {
                        stack.push(Value::Object(object));
                    } else {
                        self.set(self.global, &i.name, Value::Object(object))?;
                    }
                }
                0x69 => {
                    let super_class = pop(&mut stack)?;
                    let sub_class = pop(&mut stack)?;
                    if let (Value::Object(a), Value::Object(b)) = (sub_class, super_class) {
                        if let (Value::Object(ap), Value::Object(bp)) =
                            (self.get(a, "prototype"), self.get(b, "prototype"))
                        {
                            self.objects[ap].prototype = Some(bp);
                        }
                    }
                }
                0x40 => {
                    let name = pop(&mut stack)?.text();
                    let n = pop(&mut stack)?.number() as usize;
                    if n > 256 {
                        return Err("APT argument limit".into());
                    }
                    let args = (0..n)
                        .map(|_| pop(&mut stack))
                        .collect::<Result<Vec<_>, _>>()?;
                    let id = if name == "Array" {
                        let id = self.object(ObjectKind::Plain);
                        for (j, v) in args.into_iter().enumerate() {
                            self.set(id, j.to_string(), v)?;
                        }
                        self.set(id, "length", Value::Number(n as f64))?;
                        id
                    } else if let Value::Object(class) = self.get(self.global, &name) {
                        self.construct(class, args, host)?
                    } else {
                        return Err(format!("APT constructor absent: {name}"));
                    };
                    stack.push(Value::Object(id));
                }
                0x52 | 0xb0 | 0xb1 | 0xb2 | 0xb3 | 0x3d | 0x5d => {
                    let method = if matches!(op, 0xb0 | 0xb1 | 0xb2 | 0xb3) {
                        constant(operand)?
                    } else {
                        pop(&mut stack)?
                    };
                    let object = if matches!(op, 0x3d | 0xb0 | 0xb1) {
                        Value::Object(self.global)
                    } else {
                        pop(&mut stack)?
                    };
                    let count = pop(&mut stack)?.number() as usize;
                    if count > 256 {
                        return Err("APT argument limit".into());
                    }
                    let args = (0..count)
                        .map(|_| pop(&mut stack))
                        .collect::<Result<Vec<_>, _>>()?;
                    let value = if let Value::Object(id) = object {
                        self.call_method(id, &method.text(), args, host)?
                    } else {
                        Value::Undefined
                    };
                    if matches!(op, 0xb1 | 0xb3) {
                        return Ok(value);
                    }
                    if !matches!(op, 0xb0 | 0xb2 | 0x5d) {
                        stack.push(value);
                    }
                }
                0x3e => return Ok(stack.pop().unwrap_or_default()),
                _ => return Err(format!("Unsupported APT opcode {op:02x} at {:x}", i.offset)),
            }
            if stack.len() > 1024 {
                return Err("APT stack limit".into());
            }
        }
        Ok(Value::Undefined)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Stub(Vec<String>);
    impl Host for Stub {
        fn trace(&mut self, message: &str) {
            self.0.push(message.into());
        }
        fn call(&mut self, _: &mut Vm, _: usize, _: &str, _: Vec<Value>) -> Result<Value, String> {
            Ok(Value::Undefined)
        }
    }
    fn op(opcode: u8) -> Instruction {
        ins(opcode, serde_json::Value::Null)
    }
    fn ins(opcode: u8, operand: serde_json::Value) -> Instruction {
        Instruction {
            offset: 0,
            opcode,
            next: 0,
            operand,
            target: None,
            values: vec![],
            body: vec![],
            flags: 0,
            parameters: vec![],
            name: String::new(),
        }
    }
    fn text(s: &str) -> Instruction {
        ins(0xa1, serde_json::json!(s))
    }
    fn int(n: i32) -> Instruction {
        ins(0xb7, serde_json::json!(n as u32))
    }
    fn run(code: Vec<Instruction>) -> (Vm, Stub, Result<Value, String>) {
        let mut vm = Vm::new();
        let mut host = Stub(vec![]);
        let mut code = code;
        code.push(op(0x3e));
        let r = vm.run(&code, &mut host);
        (vm, host, r)
    }

    #[test]
    fn trace_pops_and_reports() {
        let (_, host, r) = run(vec![int(7), text("hi"), op(0x26)]);
        assert_eq!(r, Ok(Value::Number(7.0)));
        assert_eq!(host.0, vec!["hi".to_string()]);
    }
    #[test]
    fn random_is_in_range_deterministic_and_safe() {
        let draw = |seed| {
            let mut vm = Vm::new();
            vm.seed_random(seed);
            let code = [int(10), op(0x30), op(0x3e)];
            (0..50)
                .map(|_| vm.run(&code, &mut Stub(vec![])).unwrap().number())
                .collect::<Vec<_>>()
        };
        let a = draw(5);
        assert!(
            a.iter()
                .all(|v| (0.0..10.0).contains(v) && v.fract() == 0.0)
        );
        assert_eq!(a, draw(5));
        assert_ne!(a, draw(6));
        // Retail traps on 0; untrusted scripts get 0.
        assert_eq!(run(vec![int(0), op(0x30)]).2, Ok(Value::Number(0.0)));
        assert!(run(vec![op(0x30)]).2.is_err());
    }
    #[test]
    fn to_number_follows_retail() {
        let n = |v: Value| to_number(&v).number();
        assert_eq!(n(Value::Text("42".into())), 42.0);
        assert_eq!(n(Value::Text("0x1F".into())), 31.0);
        assert_eq!(n(Value::Text("2.5".into())), 2.5);
        assert_eq!(n(Value::Bool(true)), 1.0);
        assert!(n(Value::Text("abc".into())).is_nan());
        assert!(n(Value::Text(String::new())).is_nan());
        assert!(n(Value::Undefined).is_nan());
        let (_, _, r) = run(vec![text("12"), op(0x4a)]);
        assert_eq!(r, Ok(Value::Number(12.0)));
        assert_eq!(to_integer(&Value::Number(1e20)), i32::MAX);
        assert_eq!(to_integer(&Value::Text(" -9x".into())), -9);
    }
    #[test]
    fn init_array_orders_from_top() {
        let (vm, _, r) = run(vec![text("c"), text("b"), text("a"), int(3), op(0x42)]);
        let Ok(Value::Object(id)) = r else {
            panic!("{r:?}")
        };
        assert_eq!(vm.get(id, "0"), Value::Text("a".into()));
        assert_eq!(vm.get(id, "2"), Value::Text("c".into()));
        assert_eq!(vm.get(id, "length"), Value::Number(3.0));
        assert!(run(vec![int(2), op(0x42)]).2.is_err());
        assert!(run(vec![int(100_000), op(0x42)]).2.is_err());
        let (vm, _, r) = run(vec![int(-4), op(0x42)]);
        let Ok(Value::Object(id)) = r else { panic!() };
        assert_eq!(vm.get(id, "length"), Value::Number(0.0));
    }
    #[test]
    fn init_object_pairs_and_duplicates() {
        // { x: 1, y: 2, x: 3 } pushed in source order: name, value pairs.
        let (vm, _, r) = run(vec![
            text("x"),
            int(1),
            text("y"),
            int(2),
            text("x"),
            int(3),
            int(3),
            op(0x43),
        ]);
        let Ok(Value::Object(id)) = r else {
            panic!("{r:?}")
        };
        assert_eq!(vm.get(id, "y"), Value::Number(2.0));
        // Retail sets from the top pair down, so the first-written pair wins.
        assert_eq!(vm.get(id, "x"), Value::Number(1.0));
        assert!(run(vec![int(1), int(1), op(0x43)]).2.is_err());
    }
    #[test]
    fn get_string_member() {
        let (_, _, r) = run(vec![
            text("k"),
            int(9),
            int(1),
            op(0x43),
            ins(0xa5, serde_json::json!("k")),
        ]);
        assert_eq!(r, Ok(Value::Number(9.0)));
        let (_, _, r) = run(vec![int(4), ins(0xa5, serde_json::json!("k"))]);
        assert_eq!(r, Ok(Value::Undefined));
        assert!(run(vec![ins(0xa5, serde_json::Value::Null)]).2.is_err());
    }

    /// Runs every action stream of the decoded retail front-end movies (local
    /// data from `apt_actions_json.py`, never committed). Skips when absent.
    #[test]
    fn retail_menu_movies_use_no_unknown_opcode() {
        let dir = std::env::var("SKATE3_FE_ACTIONS").unwrap_or_else(|_| {
            crate::apt_imports::main_checkout()
                .join(".local/research/fe-menus/actions")
                .to_string_lossy()
                .into_owned()
        });
        let Ok(entries) = std::fs::read_dir(&dir) else {
            eprintln!("skipped: no decoded menu movies in {dir}");
            return;
        };
        fn walk(code: &[Instruction], out: &mut std::collections::BTreeSet<u8>) {
            for i in code {
                out.insert(i.opcode);
                walk(&i.body, out);
            }
        }
        let mut movies = 0;
        for entry in entries.flatten() {
            let json: serde_json::Value =
                serde_json::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap();
            let streams: BTreeMap<String, Vec<Instruction>> =
                serde_json::from_value(json["actions"].clone()).unwrap();
            movies += 1;
            let mut ops = std::collections::BTreeSet::new();
            for (name, code) in &streams {
                walk(code, &mut ops);
                let mut vm = Vm::new();
                let this = vm.object(ObjectKind::Plain);
                vm.begin_update();
                if let Err(e) = vm.run_on(this, code, &mut Stub(vec![])) {
                    assert!(
                        !e.contains("Unsupported APT opcode"),
                        "{:?} {name}: {e}",
                        entry.path()
                    );
                }
            }
            for op in ops {
                let mut code = vec![int(0); 8];
                code.push(ins(op, serde_json::json!(0)));
                let mut vm = Vm::new();
                vm.begin_update();
                if let Err(e) = vm.run(&code, &mut Stub(vec![])) {
                    assert!(
                        !e.contains("Unsupported APT opcode"),
                        "{:?}: {e}",
                        entry.path()
                    );
                }
            }
            for label in json["labels"].as_array().into_iter().flatten() {
                assert!(label.as_str().is_some_and(|s| !s.is_empty()));
            }
        }
        eprintln!("checked {movies} decoded menu movies");
    }
}

#[cfg(test)]
#[path = "apt_vm_tests.rs"]
mod construction_tests;
