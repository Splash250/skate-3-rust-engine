use super::*;

struct NoHostCalls;
impl Host for NoHostCalls {
    fn call(&mut self, _: &mut Vm, _: usize, _: &str, _: Vec<Value>) -> Result<Value, String> {
        Err("Unexpected native call".into())
    }
}

fn class(vm: &mut Vm) -> usize {
    let code = serde_json::from_value::<Vec<Instruction>>(serde_json::json!([{
        "offset": 0, "opcode": 155, "name": "Audit",
        "parameters": [{"register": 0, "name": "value"}],
        "body": [
            {"offset": 0, "opcode": 112},
            {"offset": 1, "opcode": 161, "operand": "received"},
            {"offset": 2, "opcode": 164, "operand": "value"},
            {"offset": 3, "opcode": 79}
        ]
    }]))
    .unwrap();
    vm.run(&code, &mut NoHostCalls).unwrap();
    let Value::Object(class) = vm.get(vm.global, "Audit") else {
        panic!("class was not published");
    };
    let Value::Object(prototype) = vm.get(class, "prototype") else {
        panic!("prototype was not published");
    };
    vm.set(prototype, "inherited", Value::Number(7.0)).unwrap();
    class
}

#[test]
fn direct_and_opcode_construction_share_arguments_and_prototypes() {
    let mut vm = Vm::new();
    let class = class(&mut vm);
    vm.begin_update();
    let direct = vm
        .construct(class, vec![Value::Number(42.0)], &mut NoHostCalls)
        .unwrap();
    let code = serde_json::from_value::<Vec<Instruction>>(serde_json::json!([
        {"offset": 0, "opcode": 183, "operand": 42},
        {"offset": 1, "opcode": 90},
        {"offset": 2, "opcode": 161, "operand": "Audit"},
        {"offset": 3, "opcode": 64},
        {"offset": 4, "opcode": 62}
    ]))
    .unwrap();
    let Value::Object(opcode) = vm.run(&code, &mut NoHostCalls).unwrap() else {
        panic!("NewObject did not return an instance");
    };
    assert_ne!(direct, opcode);
    for id in [direct, opcode] {
        assert_eq!(vm.get(id, "received"), Value::Number(42.0));
        assert_eq!(vm.get(id, "inherited"), Value::Number(7.0));
        assert_eq!(vm.objects[id].prototype, vm.objects[direct].prototype);
    }
}

#[test]
fn construction_preserves_execution_budget_and_rejects_invalid_inputs() {
    let mut vm = Vm::new();
    let class = class(&mut vm);
    let before = vm.objects.len();
    assert_eq!(
        vm.construct(class, vec![Value::Undefined; 257], &mut NoHostCalls),
        Err("APT argument limit".into())
    );
    assert_eq!(
        vm.construct(usize::MAX, vec![], &mut NoHostCalls),
        Err("Invalid APT class".into())
    );
    assert_eq!(vm.objects.len(), before);
    vm.remaining = 2;
    assert_eq!(
        vm.construct(class, vec![], &mut NoHostCalls),
        Err("APT execution/object budget exceeded".into())
    );
    assert_eq!(vm.remaining, 0);
    let after = vm.objects.len();
    assert_eq!(
        vm.construct(class, vec![], &mut NoHostCalls),
        Err("APT execution/object budget exceeded".into())
    );
    assert_eq!(
        vm.objects.len(),
        after,
        "exhaustion must not allocate again"
    );
    vm.begin_update();
    vm.objects.resize_with(4097, || Object {
        kind: ObjectKind::Plain,
        fields: BTreeMap::new(),
        prototype: None,
    });
    assert_eq!(
        vm.construct(class, vec![], &mut NoHostCalls),
        Err("APT execution/object budget exceeded".into())
    );
}
