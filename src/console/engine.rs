//! The JavaScript engine behind the shell. Knows nothing about gpui: it
//! takes code and returns what to show.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use mongodb::Client;
use mongodb::bson::Bson;
use mongodb::error::ErrorKind;
use rquickjs::{Context, Ctx, Exception, Function, Runtime, Value};
use rust_i18n::t;

use crate::mongo::RT;
use crate::query;
use crate::shell::to_shell;

const PRELUDE: &str = include_str!("prelude.js");
const MEMORY_LIMIT: usize = 256 * 1024 * 1024;

/// One thing for the console to show.
#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    /// An expression's value, as canonical extended JSON.
    Value(serde_json::Value),
    /// Text from `print`.
    Print(String),
    /// A note from the shell itself, like "switched to db shop".
    Info(String),
    Error(String),
}

pub struct Engine {
    // Kept alive for the context; dropped after it.
    context: Context,
    _runtime: Runtime,
    out: Rc<RefCell<Vec<Output>>>,
    stop: Arc<AtomicBool>,
}

/// Rewrites mongosh's non-JavaScript commands (`use x`, `show dbs`, `it`)
/// into calls; anything else passes through untouched.
pub fn preprocess(code: &str) -> String {
    let line = code.trim().trim_end_matches(';').trim();
    let words: Vec<&str> = line.split_whitespace().collect();
    let quote = |s: &str| serde_json::to_string(s).unwrap_or_default();
    match words[..] {
        _ if line.contains('\n') => code.to_string(),
        ["use", name] => format!("use({})", quote(name)),
        ["show", "dbs" | "databases"] => r#"show("dbs")"#.into(),
        ["show", "collections" | "tables"] => r#"show("collections")"#.into(),
        ["it"] => "it()".into(),
        _ => code.to_string(),
    }
}

/// `print`'s arguments (a JSON array of extended JSON) joined by spaces:
/// strings as they are, other values in shell syntax.
fn print_text(args: &str) -> String {
    let values: Vec<serde_json::Value> = serde_json::from_str(args).unwrap_or_default();
    values
        .iter()
        .map(|v| match v {
            serde_json::Value::String(s) => s.clone(),
            other => to_shell(other, ""),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// "Name: message" for a thrown error, or its string form.
fn describe<'js>(ctx: &Ctx<'js>, thrown: Value<'js>) -> String {
    if let Some(obj) = thrown.as_object() {
        let name: Option<String> = obj.get("name").ok();
        let message: Option<String> = obj.get("message").ok();
        if let Some(message) = message {
            return match name {
                Some(name) if name != "Error" => format!("{name}: {message}"),
                _ => message,
            };
        }
    }
    ctx.json_stringify(thrown)
        .ok()
        .flatten()
        .and_then(|s| s.to_string().ok())
        .unwrap_or_else(|| "error".into())
}

/// Text for a shell message key, in the current UI language.
fn message(key: &str, arg: &str) -> String {
    match key {
        "switched" => t!("console.switched", db = arg).into(),
        "more" => t!("console.more").into(),
        "no_cursor" => t!("console.no_cursor").into(),
        other => other.to_string(),
    }
}

/// Runs one database command for the JavaScript side; errors become JS exceptions.
fn run_command(ctx: &Ctx, client: Option<&Client>, db: &str, cmd: &str) -> rquickjs::Result<String> {
    let throw = |msg: String| Exception::throw_message(ctx, &msg);
    let client = client.ok_or_else(|| throw(t!("console.not_connected").into()))?;
    let value: serde_json::Value = serde_json::from_str(cmd).map_err(|e| throw(e.to_string()))?;
    let Ok(Bson::Document(doc)) = Bson::try_from(value) else { return Err(throw("a command must be a document".into())) };
    match RT.block_on(async { client.database(db).run_command(doc).await }) {
        Ok(reply) => Ok(Bson::Document(reply).into_canonical_extjson().to_string()),
        // The server's own message reads better than the driver's wrapper.
        Err(e) => Err(throw(match &*e.kind {
            ErrorKind::Command(c) => c.message.clone(),
            _ => e.to_string(),
        })),
    }
}

/// Test-only `__echo(value)`: sends a value to Rust as extended JSON and
/// back, exactly like a server reply, so conversions can be tested offline.
#[cfg(test)]
fn install_echo(ctx: &Ctx) -> rquickjs::Result<()> {
    use mongodb::bson::Bson;
    let raw = Function::new(ctx.clone(), |json: String| -> String {
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        Bson::try_from(value).unwrap().into_canonical_extjson().to_string()
    })?;
    ctx.globals().set("__echoRaw", raw)?;
    ctx.eval::<(), _>("globalThis.__echo = (v) => __fromEJSON(JSON.parse(__echoRaw(JSON.stringify(__toEJSON(v)))));")
}

impl Engine {
    pub fn new(client: Option<Client>, db: &str) -> Result<Self, String> {
        let client = client.clone();
        let runtime = Runtime::new().map_err(|e| e.to_string())?;
        runtime.set_memory_limit(MEMORY_LIMIT);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || flag.load(Ordering::Relaxed))));
        let context = Context::full(&runtime).map_err(|e| e.to_string())?;
        let out = Rc::new(RefCell::new(Vec::new()));
        context.with(|ctx| -> Result<(), String> {
            let globals = ctx.globals();
            let sink = out.clone();
            let print = Function::new(ctx.clone(), move |args: String| sink.borrow_mut().push(Output::Print(print_text(&args))))
                .map_err(|e| e.to_string())?;
            globals.set("__print", print).map_err(|e| e.to_string())?;
            let new_id = Function::new(ctx.clone(), || mongodb::bson::oid::ObjectId::new().to_hex()).map_err(|e| e.to_string())?;
            globals.set("__newObjectId", new_id).map_err(|e| e.to_string())?;
            // Dates as mongosh reads them: a bare date or a missing zone means UTC.
            let iso_date = Function::new(ctx.clone(), |ctx: Ctx, text: String| -> rquickjs::Result<f64> {
                query::parse_date(&text)
                    .map(|d| d.timestamp_millis() as f64)
                    .map_err(|e| rquickjs::Exception::throw_type(&ctx, &e))
            })
            .map_err(|e| e.to_string())?;
            globals.set("__isoDate", iso_date).map_err(|e| e.to_string())?;
            let sink = out.clone();
            let info = Function::new(ctx.clone(), move |key: String, arg: String| sink.borrow_mut().push(Output::Info(message(&key, &arg))))
                .map_err(|e| e.to_string())?;
            globals.set("__info", info).map_err(|e| e.to_string())?;
            let tr = Function::new(ctx.clone(), |key: String| message(&key, "")).map_err(|e| e.to_string())?;
            globals.set("__tr", tr).map_err(|e| e.to_string())?;
            let run = Function::new(ctx.clone(), move |ctx: Ctx, db: String, cmd: String| run_command(&ctx, client.as_ref(), &db, &cmd))
                .map_err(|e| e.to_string())?;
            globals.set("__runCommand", run).map_err(|e| e.to_string())?;
            globals.set("__db", db).map_err(|e| e.to_string())?;
            #[cfg(test)]
            install_echo(&ctx).map_err(|e| e.to_string())?;
            ctx.eval::<(), _>(PRELUDE).map_err(|_| describe(&ctx, ctx.catch()))
        })?;
        Ok(Self { context, _runtime: runtime, out, stop })
    }

    /// Setting this to true stops the running code. It interrupts JavaScript,
    /// not a server command already in flight.
    pub fn stop_handle(&self) -> Arc<AtomicBool> {
        self.stop.clone()
    }

    /// Runs `code` and returns everything it printed, then its value.
    pub fn eval(&mut self, code: &str) -> Vec<Output> {
        self.stop.store(false, Ordering::Relaxed);
        let code = preprocess(code);
        let result = self.context.with(|ctx| {
            let run = || -> rquickjs::Result<String> {
                let value: Value = ctx.eval(code.as_str())?;
                let display: Function = ctx.globals().get("__display")?;
                display.call((value,))
            };
            run().map_err(|_| describe(&ctx, ctx.catch()))
        });
        let mut out = std::mem::take(&mut *self.out.borrow_mut());
        match result {
            Ok(json) if json.is_empty() => {}
            Ok(json) => match serde_json::from_str::<serde_json::Value>(&json) {
                Ok(shown) => {
                    out.push(Output::Value(shown["v"].clone()));
                    if shown["more"] == true {
                        out.push(Output::Info(message("more", "")));
                    }
                }
                Err(e) => out.push(Output::Error(e.to_string())),
            },
            Err(e) => out.push(Output::Error(e)),
        }
        out
    }

    /// The database `db` refers to, for the prompt.
    pub fn db(&self) -> String {
        self.context.with(|ctx| ctx.globals().get::<_, String>("__db")).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn engine() -> Engine {
        Engine::new(None, "test").unwrap()
    }

    #[test]
    fn print_errors_and_values() {
        let mut e = engine();
        assert_eq!(e.eval("print('hi', 2)"), vec![Output::Print("hi 2".into())]);
        assert_eq!(e.eval("print({ a: 1, b: 'x' })"), vec![Output::Print("{\n  a: 1,\n  b: \"x\"\n}".into())]);
        assert_eq!(e.eval("throw new Error('boom')"), vec![Output::Error("boom".into())]);
        let syntax = e.eval("let = ;");
        assert!(matches!(&syntax[..], [Output::Error(m)] if m.starts_with("SyntaxError")), "{syntax:?}");
        assert_eq!(e.eval("undefined"), vec![]);
        assert_eq!(e.eval("'text'"), vec![Output::Value(json!("text"))]);
        assert_eq!(e.eval("({ a: [1, 1.5, true, null] })"), vec![Output::Value(json!({ "a": [{ "$numberInt": "1" }, { "$numberDouble": "1.5" }, true, null] }))]);
    }

    #[test]
    fn variables_persist_between_commands() {
        let mut e = engine();
        assert_eq!(e.eval("let x = 5"), vec![]);
        assert_eq!(e.eval("const f = (n) => n * x"), vec![]);
        assert_eq!(e.eval("f(2)"), vec![Output::Value(json!({ "$numberInt": "10" }))]);
    }

    fn value(e: &mut Engine, code: &str) -> serde_json::Value {
        match &e.eval(code)[..] {
            [Output::Value(v)] => v.clone(),
            other => panic!("{code}: {other:?}"),
        }
    }

    #[test]
    fn shell_types_become_extended_json() {
        let mut e = engine();
        let cases = [
            (r#"ObjectId("65a1b2c3d4e5f60718293a4b")"#, json!({ "$oid": "65a1b2c3d4e5f60718293a4b" })),
            (r#"ISODate("2024-01-02")"#, json!({ "$date": { "$numberLong": "1704153600000" } })),
            ("new Date(0)", json!({ "$date": { "$numberLong": "0" } })),
            (r#"NumberLong("9007199254740993")"#, json!({ "$numberLong": "9007199254740993" })),
            ("NumberLong(5)", json!({ "$numberLong": "5" })),
            ("NumberInt('7')", json!({ "$numberInt": "7" })),
            (r#"NumberDecimal("1.10")"#, json!({ "$numberDecimal": "1.10" })),
            (r#"UUID("123e4567-e89b-12d3-a456-426614174000")"#, json!({ "$binary": { "base64": "Ej5FZ+ibEtOkVkJmFBdAAA==", "subType": "04" } })),
            (r#"BinData(0, "AQID")"#, json!({ "$binary": { "base64": "AQID", "subType": "00" } })),
            ("/ab+c/i", json!({ "$regularExpression": { "pattern": "ab+c", "options": "i" } })),
            ("Timestamp(10, 2)", json!({ "$timestamp": { "t": 10, "i": 2 } })),
            ("MinKey()", json!({ "$minKey": 1 })),
            ("MaxKey()", json!({ "$maxKey": 1 })),
        ];
        for (code, expected) in cases {
            assert_eq!(value(&mut e, code), expected, "{code}");
        }
        let oid = value(&mut e, "ObjectId()");
        assert_eq!(oid["$oid"].as_str().map(str::len), Some(24), "a new ObjectId: {oid}");
    }

    #[test]
    fn extended_json_from_the_server_becomes_shell_types() {
        // __echo sends a value to Rust as extended JSON and back, like a server reply.
        let mut e = engine();
        e.eval("var v = __echo({ id: ObjectId('65a1b2c3d4e5f60718293a4b'), at: new Date(5), n: NumberLong(9), d: NumberDecimal('2.5'), i: 3, f: 1.5, re: /x/m, u: UUID('123e4567-e89b-12d3-a456-426614174000') })");
        assert_eq!(value(&mut e, "v.id instanceof ObjectId && v.id.toHexString()"), json!("65a1b2c3d4e5f60718293a4b"));
        assert_eq!(value(&mut e, "v.at instanceof Date && v.at.getTime()"), json!({ "$numberInt": "5" }));
        assert_eq!(value(&mut e, "v.n instanceof Long && v.n.toString()"), json!("9"));
        assert_eq!(value(&mut e, "v.d instanceof Decimal128 && v.d.toString()"), json!("2.5"));
        assert_eq!(value(&mut e, "[typeof v.i, typeof v.f, v.i + v.f]"), json!(["number", "number", { "$numberDouble": "4.5" }]));
        assert_eq!(value(&mut e, "v.re instanceof RegExp && v.re.flags"), json!("m"));
        assert_eq!(value(&mut e, "v.u.toString()"), json!("123e4567-e89b-12d3-a456-426614174000"));
        // And it all goes back unchanged.
        assert_eq!(
            value(&mut e, "v"),
            json!({
                "id": { "$oid": "65a1b2c3d4e5f60718293a4b" },
                "at": { "$date": { "$numberLong": "5" } },
                "n": { "$numberLong": "9" },
                "d": { "$numberDecimal": "2.5" },
                "i": { "$numberInt": "3" },
                "f": { "$numberDouble": "1.5" },
                "re": { "$regularExpression": { "pattern": "x", "options": "m" } },
                "u": { "$binary": { "base64": "Ej5FZ+ibEtOkVkJmFBdAAA==", "subType": "04" } },
            })
        );
    }

    #[test]
    fn shell_commands_become_calls() {
        assert_eq!(preprocess("use shop"), r#"use("shop")"#);
        assert_eq!(preprocess("  use  my-db ; "), r#"use("my-db")"#);
        assert_eq!(preprocess("show dbs"), r#"show("dbs")"#);
        assert_eq!(preprocess("show databases"), r#"show("dbs")"#);
        assert_eq!(preprocess("show collections"), r#"show("collections")"#);
        assert_eq!(preprocess("show tables"), r#"show("collections")"#);
        assert_eq!(preprocess("it"), "it()");
        // Ordinary code passes through, even when it mentions these words.
        for code in ["db.users.find({ use: 1 })", "const it = 5", "use", "show", "let show = 1; show", "db.x.find()\nuse shop"] {
            assert_eq!(preprocess(code), code);
        }
    }

    #[test]
    fn a_runaway_loop_can_be_stopped_and_the_engine_still_works() {
        let mut e = engine();
        let stop = e.stop_handle();
        let timer = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        let out = e.eval("while (true) {}");
        timer.join().unwrap();
        assert!(matches!(&out[..], [Output::Error(m)] if m.contains("interrupted")), "{out:?}");
        assert_eq!(e.eval("1"), vec![Output::Value(json!({ "$numberInt": "1" }))]);
    }

    #[test]
    fn evaluates_an_expression() {
        let mut engine = Engine::new(None, "test").unwrap();
        assert_eq!(engine.eval("1 + 2"), vec![Output::Value(json!({ "$numberInt": "3" }))]);
    }

    /// An engine connected to the live test server, or None to skip.
    fn live() -> Engine {
        let uri = std::env::var("HUMON_TEST_URI").expect("HUMON_TEST_URI");
        let client = crate::mongo::RT.block_on(crate::mongo::open(uri)).unwrap();
        Engine::new(Some(client), "shop").unwrap()
    }

    fn ints(values: &[i32]) -> serde_json::Value {
        json!(values.iter().map(|n| json!({ "$numberInt": n.to_string() })).collect::<Vec<_>>())
    }

    #[test]
    #[ignore]
    fn live_navigation() {
        let mut e = live();
        let dbs = e.eval("show dbs");
        assert!(dbs.iter().any(|o| matches!(o, Output::Print(l) if l.starts_with("shop"))), "{dbs:?}");
        assert_eq!(e.eval("use logs"), vec![Output::Info("switched to db logs".into())]);
        assert_eq!(e.db(), "logs");
        assert_eq!(value(&mut e, "db.getName()"), json!("logs"));
        let cols = e.eval("show collections");
        assert_eq!(cols, vec![Output::Print("events".into())]);
        e.eval("use shop");
        assert_eq!(value(&mut e, "db.getSiblingDB('logs').events.countDocuments()"), json!({ "$numberInt": "1" }));
    }

    #[test]
    #[ignore]
    fn live_reads() {
        let mut e = live();
        // Ages cycle 18..=67 over 137 users; city is Izmir when i % 3 == 0.
        assert_eq!(value(&mut e, "db.users.find({ age: { $gte: 60 } }).sort({ age: -1, name: 1 }).limit(3).toArray().map(u => u.age)"), ints(&[67, 67, 66]));
        assert_eq!(value(&mut e, "db.users.findOne({ name: 'user5' }).age"), json!({ "$numberInt": "23" }));
        assert_eq!(value(&mut e, "db.users.countDocuments({ 'address.city': 'Izmir' })"), json!({ "$numberInt": "46" }));
        assert_eq!(value(&mut e, "db.users.estimatedDocumentCount()"), json!({ "$numberInt": "137" }));
        assert_eq!(value(&mut e, "db.users.distinct('address.city').sort()"), json!(["Istanbul", "Izmir"]));
        assert_eq!(
            value(&mut e, "db.users.aggregate([{ $group: { _id: '$address.city', n: { $sum: 1 } } }, { $sort: { _id: 1 } }]).toArray()"),
            json!([{ "_id": "Istanbul", "n": { "$numberInt": "91" } }, { "_id": "Izmir", "n": { "$numberInt": "46" } }])
        );
        assert_eq!(
            e.eval("db.users.find({ age: 18 }).sort({ name: 1 }).forEach(u => print(u.name))"),
            vec![Output::Print("user0".into()), Output::Print("user100".into()), Output::Print("user50".into())]
        );
        let bad = e.eval("db.users.find({ $bad: 1 }).toArray()");
        assert!(matches!(&bad[..], [Output::Error(m)] if m.contains("unknown top level operator")), "{bad:?}");
    }

    #[test]
    #[ignore]
    fn live_cursor_pages_with_it() {
        let mut e = live();
        let first = e.eval("db.users.find({}, { _id: 0, name: 1 }).sort({ name: 1 })");
        let [Output::Value(docs), Output::Info(more)] = &first[..] else { panic!("{first:?}") };
        assert_eq!(docs.as_array().map(Vec::len), Some(20));
        assert_eq!(docs[0], json!({ "name": "user0" }));
        assert_eq!(more, "Type \"it\" for more");
        let second = e.eval("it");
        let [Output::Value(docs), Output::Info(_)] = &second[..] else { panic!("{second:?}") };
        assert_eq!(docs[0], json!({ "name": "user116" }), "continues where the first page stopped (checked against mongosh)");
        // A small result needs no "it".
        let small = e.eval("db.users.find({ age: 18 }, { _id: 0, name: 1 }).sort({ name: 1 })");
        assert_eq!(small, vec![Output::Value(json!([{ "name": "user0" }, { "name": "user100" }, { "name": "user50" }]))]);
    }

    #[test]
    #[ignore]
    fn live_writes() {
        let mut e = live();
        e.eval("db.humon_console_test.drop()");
        let inserted = value(&mut e, "db.humon_console_test.insertOne({ n: 1 })");
        assert_eq!(inserted["acknowledged"], json!(true));
        assert_eq!(inserted["insertedId"]["$oid"].as_str().map(str::len), Some(24), "{inserted}");
        assert_eq!(value(&mut e, "Object.keys(db.humon_console_test.insertMany([{ n: 2 }, { n: 3 }]).insertedIds).length"), json!({ "$numberInt": "2" }));
        let updated = value(&mut e, "db.humon_console_test.updateMany({ n: { $gte: 2 } }, { $set: { big: true } })");
        assert_eq!((updated["matchedCount"].clone(), updated["modifiedCount"].clone()), (json!({ "$numberInt": "2" }), json!({ "$numberInt": "2" })));
        assert_eq!(value(&mut e, "db.humon_console_test.deleteOne({ n: 1 }).deletedCount"), json!({ "$numberInt": "1" }));
        assert_eq!(value(&mut e, "db.humon_console_test.countDocuments({ big: true })"), json!({ "$numberInt": "2" }));
        e.eval("db.humon_console_test.insertOne({ _id: 1 })");
        let dup = e.eval("db.humon_console_test.insertOne({ _id: 1 })");
        assert!(matches!(&dup[..], [Output::Error(m)] if m.contains("duplicate key")), "a write error must not look like success: {dup:?}");
        assert_eq!(value(&mut e, "db.humon_console_test.drop()"), json!(true));
    }
}
