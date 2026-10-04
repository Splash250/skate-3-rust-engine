//! Lua's C pattern matcher does not run the VM instruction hook. Reserve a
//! conservative bound on its search/backtracking work before entering C.
use mlua::{Function, Lua, MultiValue, Table, Value};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn cost(subject: usize, pattern: &[u8], plain: bool) -> usize {
    let n = subject.saturating_add(1);
    let mut work = n.saturating_mul(pattern.len().max(1));
    if plain {
        return work;
    }
    let mut i = 0;
    while i < pattern.len() {
        match pattern[i] {
            b'%' => {
                i += 1;
                if i < pattern.len() && (pattern[i] == b'b' || pattern[i].is_ascii_digit()) {
                    // Balanced scans and backreferences can compare the entire
                    // subject at each candidate match position.
                    work = work.saturating_mul(n);
                }
            }
            b'[' => {
                i += 1;
                if pattern.get(i) == Some(&b'^') {
                    i += 1;
                }
                if pattern.get(i) == Some(&b']') {
                    i += 1;
                }
                while i < pattern.len() && pattern[i] != b']' {
                    if pattern[i] == b'%' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'?' => work = work.saturating_mul(2),
            b'*' | b'+' | b'-' => {
                // A final repetition always succeeds once its atom matches;
                // there is no failing suffix to explore at every split point.
                // This keeps common token scans such as %w+ linear.
                if i + 1 != pattern.len() {
                    work = work.saturating_mul(n);
                }
            }
            _ => {}
        }
        i += 1;
    }
    work
}

fn reserve(budget: &AtomicUsize, work: usize) -> mlua::Result<()> {
    let units = work
        .div_ceil(crate::vm::LUA_INSTRUCTIONS_PER_BUDGET_UNIT)
        .max(1);
    let available = budget.load(Ordering::Relaxed);
    if units >= available {
        budget.store(0, Ordering::Relaxed);
        return Err(mlua::Error::RuntimeError(
            "resource instruction budget exhausted by native Lua pattern work".into(),
        ));
    }
    budget.fetch_sub(units, Ordering::Relaxed);
    Ok(())
}

fn arguments(lua: &Lua, args: &MultiValue, plain: bool) -> mlua::Result<usize> {
    // Preserve Lua's numeric-to-string coercion and let the original function
    // report ordinary argument errors; those do not enter the pattern matcher.
    let Some(subject) = args
        .front()
        .cloned()
        .map(|v| lua.coerce_string(v))
        .transpose()?
        .flatten()
    else {
        return Ok(1);
    };
    let Some(pattern) = args
        .get(1)
        .cloned()
        .map(|v| lua.coerce_string(v))
        .transpose()?
        .flatten()
    else {
        return Ok(1);
    };
    Ok(cost(subject.as_bytes().len(), &pattern.as_bytes(), plain))
}

pub(crate) fn install(lua: &Lua, budget: Arc<AtomicUsize>) -> mlua::Result<()> {
    let strings: Table = lua.globals().get("string")?;
    for name in ["find", "match", "gsub"] {
        let original: Function = strings.get(name)?;
        let counter = budget.clone();
        strings.set(
            name,
            lua.create_function(move |lua, args: MultiValue| {
                let plain = name == "find"
                    && args
                        .get(3)
                        .is_some_and(|v| !matches!(v, Value::Nil | Value::Boolean(false)));
                let mut work = arguments(lua, &args, plain)?;
                if name == "gsub" {
                    if let (Some(subject), Some(replacement)) = (args.front(), args.get(2)) {
                        if let (Some(subject), Some(replacement)) = (
                            lua.coerce_string(subject.clone())?,
                            lua.coerce_string(replacement.clone())?,
                        ) {
                            work = work.saturating_add(
                                subject
                                    .as_bytes()
                                    .len()
                                    .saturating_add(1)
                                    .saturating_mul(replacement.as_bytes().len()),
                            );
                        }
                    }
                }
                reserve(&counter, work)?;
                original.call::<MultiValue>(args)
            })?,
        )?;
    }
    let original: Function = strings.get("gmatch")?;
    strings.set(
        "gmatch",
        lua.create_function(move |lua, args: MultiValue| {
            let work = arguments(lua, &args, false)?;
            // The native iterator retains the source and pattern. Wrap every step,
            // including an iterator saved across callbacks with a fresh budget.
            let iterator = original.call::<Function>(args)?;
            let counter = budget.clone();
            lua.create_function(move |_, args: MultiValue| {
                reserve(&counter, work)?;
                iterator.call::<MultiValue>(args)
            })
        })?,
    )?;
    Ok(())
}
