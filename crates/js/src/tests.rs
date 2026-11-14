use super::*;
use craft_anything_testkit::electron as fx;

fn run(path: &str, src: &str) -> Analysis {
    analyze(path, src, &Limits::default())
}

#[test]
fn main_process_facts() {
    let a = run("main.js", fx::MAIN_JS);
    let f = &a.facts;
    assert_eq!(f.mode, ParseMode::Parsed, "{:?}", f.mode_reason);
    let w = &f.windows[0];
    assert_eq!(w.class, "BrowserWindow");
    assert_eq!(w.preload.as_deref(), Some("preload.js"));
    assert_eq!(w.web_preferences.get("contextIsolation"), Some(&PrefValue::Bool(true)));
    assert_eq!(w.web_preferences.get("nodeIntegration"), Some(&PrefValue::Bool(false)));
    assert_eq!(w.binding.as_deref(), Some("mainWindow"));
    let h = f.ipc.iter().find(|i| i.channel.as_deref() == Some("export-notes")).unwrap();
    assert_eq!((h.side.as_str(), h.method.as_str()), ("main", "handle"));
    assert_eq!(h.handler.as_deref(), Some("ipcMain.handle(export-notes)"));
    let handler = f.functions.iter().find(|x| x.name == "ipcMain.handle(export-notes)").unwrap();
    assert!(handler.calls.contains(&"fs.writeFileSync".to_string()), "{:?}", handler.calls);
    assert!(handler.calls.contains(&"dialog.showSaveDialog".to_string()));
    assert!(handler.calls.contains(&"toCsv".to_string()));
    assert!(f.effects.iter().any(|e| e.api == "fs.writeFileSync" && e.function.as_deref() == Some("ipcMain.handle(export-notes)")));
    assert_eq!(f.loads[0].method, "loadFile");
    assert_eq!(f.loads[0].receiver.as_deref(), Some("mainWindow"));
    assert_eq!(f.loads[0].target.as_deref(), Some("renderer/index.html"));
    assert!(f.imports.iter().any(|i| i.specifier == "electron" && i.kind == "require"));
    assert!(f.events.iter().any(|e| e.target == "app" && e.event == "window-all-closed"));
    assert_eq!(f.source_kind, "script");
}

#[test]
fn preload_and_renderer_facts() {
    let p = run("preload.js", fx::PRELOAD_JS).facts;
    let e = &p.exposed[0];
    assert_eq!(e.key.as_deref(), Some("api"));
    assert_eq!(e.members.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), vec!["exportNotes", "log"]);
    let inv = p.ipc.iter().find(|i| i.method == "invoke").unwrap_or_else(|| panic!("{:#?}", p.ipc));
    assert_eq!(inv.channel.as_deref(), Some("export-notes"));
    assert_eq!(inv.function.as_deref(), Some("api.exportNotes"));

    let r = run("renderer/app.js", fx::RENDERER_JS).facts;
    let f = r
        .functions
        .iter()
        .find(|f| f.name == "exportNotes")
        .unwrap_or_else(|| panic!("{:?} {:?}", r.mode_reason, r.functions.iter().map(|f| &f.name).collect::<Vec<_>>()));
    assert!(f.calls.contains(&"api.exportNotes".to_string()), "{:?}", f.calls);
    let ev = r.events.iter().find(|e| e.event == "click").unwrap();
    assert_eq!(ev.handler.as_deref(), Some("exportNotes"));
    let ep = r.endpoints.iter().find(|e| e.api == "fetch").unwrap();
    assert_eq!(ep.url, "https://api.notes.example/v1/sync");
    assert_eq!(ep.method.as_deref(), Some("POST"));
    assert!(r.storage.iter().any(|s| s.api == "localStorage.setItem" && s.arg.as_deref() == Some("draft")));
}

#[test]
fn esm_imports_exports_and_routes() {
    let src = r#"
import React, { useState as useS } from 'react';
import * as api from './api.js';
export { helper as default2 } from './helper.js';
export * from './all.js';
export const a = 1, b = 2;
export function go() { return import('./lazy.js'); }
export default class App {}
const routes = [{ path: '/notes/:id', component: 1 }];
const BASE = 'https://x.test';
fetch(`${BASE}/notes/${id}`);
axios.post('/api/save', {});
const w = new Worker(new URL('./worker.js', import.meta.url));
const ws = new WebSocket('wss://live.x.test/socket');
export const El = () => <Route path="/settings" element={<div onClick={() => save()} />} />;
"#;
    let f = run("app.jsx", src).facts;
    assert_eq!(f.source_kind, "module");
    for n in ["default2", "*", "a", "b", "go", "default", "El"] {
        assert!(f.exports.esm.contains(&n.to_string()), "missing export {n}: {:?}", f.exports.esm);
    }
    let kinds: Vec<(&str, &str)> = f.imports.iter().map(|i| (i.kind.as_str(), i.specifier.as_str())).collect();
    assert!(kinds.contains(&("import", "react")));
    assert!(kinds.contains(&("re-export", "./helper.js")));
    assert!(kinds.contains(&("dynamic-import", "./lazy.js")));
    assert!(kinds.contains(&("worker", "./worker.js")), "{kinds:?}");
    assert!(f.routes.iter().any(|r| r.path == "/notes/:id"));
    assert!(f.routes.iter().any(|r| r.path == "/settings" && r.source == "jsx"));
    assert!(f.endpoints.iter().any(|e| e.url == "https://x.test/notes/{id}" && !e.literal), "{:?}", f.endpoints);
    assert!(f.endpoints.iter().any(|e| e.api == "axios" && e.method.as_deref() == Some("POST") && e.url == "/api/save"));
    assert!(f.endpoints.iter().any(|e| e.api == "websocket"));
    assert!(f.jsx);
}

#[test]
fn commonjs_exports() {
    let f = run("x.js", "exports.a = 1; module.exports.b = () => 1; Object.defineProperty(exports, 'c', {}); ").facts;
    assert_eq!(f.exports.cjs, vec!["a", "b", "c"]);
    let g = run("y.js", "module.exports = { load, save: function () {} };").facts;
    assert_eq!(g.exports.cjs_whole.as_deref(), Some("object"));
    assert_eq!(g.exports.cjs, vec!["load", "save"]);
}

#[test]
fn webpack_bundle_is_split() {
    let a = run("main.js", fx::WEBPACK_BUNDLE);
    assert_eq!(a.bundle.bundlers, vec!["webpack"]);
    let ids: Vec<&str> = a.bundle.modules.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, vec!["./src/api.js", "./src/util.js", "./src/index.js"]);
    let index = &a.bundle.modules[2];
    assert_eq!(index.deps, vec!["./src/api.js", "./src/util.js"]);
    assert_eq!(a.facts.source_map_url.as_deref(), Some("main.js.map"));
    // Literal concatenation through a constant.
    assert!(a.facts.endpoints.iter().any(|e| e.url == "https://api.example.test/items"), "{:?}", a.facts.endpoints);

    let c = run("179.js", fx::WEBPACK_CHUNK);
    assert_eq!(c.bundle.chunk_ids, vec!["179"]);
    let m = c.bundle.modules.iter().find(|m| m.id == "512").unwrap();
    assert_eq!(m.deps, vec!["87"]);
    assert_eq!(m.format, "webpack-chunk");
}

#[test]
fn esbuild_bundle_is_split() {
    let a = run("out.js", fx::ESBUILD_BUNDLE);
    assert!(a.bundle.bundlers.contains(&"esbuild".to_string()));
    let main = a.bundle.modules.iter().find(|m| m.id == "src/main.js").unwrap();
    assert_eq!(main.format, "esbuild-cjs");
    assert_eq!(main.deps, vec!["src/util.js"]);
}

#[test]
fn minified_esbuild_wrappers_keep_dependencies() {
    let minified = fx::ESBUILD_BUNDLE.replace("require_util", "a").replace("require_main", "b");
    let a = run("out.js", &minified);
    assert_eq!(a.facts.parse_errors, 0);
    assert_eq!(a.bundle.modules.len(), 2);
    let main = a.bundle.modules.iter().find(|m| m.id == "src/main.js").unwrap();
    assert_eq!(main.format, "esbuild-cjs");
    assert_eq!(main.deps, ["src/util.js"]);

    // Resolve after visiting the whole file: an initializer may be declared later.
    let source = r#"
var m = h({"src/main.js"(exports, module) { u(); i(); }});
var u = h({"src/util.js"(exports, module) { module.exports = 7; }});
var i = h({"src/setup.js"() { globalThis.ready = true; }});
"#;
    let a = run("out.js", source);
    assert_eq!(a.facts.parse_errors, 0);
    assert_eq!(a.bundle.modules.len(), 3);
    assert_eq!(a.bundle.modules.iter().find(|m| m.id == "src/main.js").unwrap().deps, ["src/util.js", "src/setup.js"]);
    assert_eq!(a.bundle.modules.iter().find(|m| m.id == "src/setup.js").unwrap().format, "esbuild-esm");
}

#[test]
fn esbuild_wrapper_dependencies_follow_bindings() {
    let source = r#"
var a = h({"src/outer.js"(exports, module) { module.exports = 1; }});
var b = h({"src/main.js"(exports, module) { a(); }});
function anotherBundle() {
    var a = h({"src/inner.js"(exports, module) { module.exports = 2; }});
    var b = h({"src/other.js"(exports, module) { a(); }});
}
var s = h({"src/shadow.js"(exports, module) { function local(a) { a(); } }});
"#;
    let a = run("out.js", source);
    assert_eq!(a.facts.parse_errors, 0);
    let deps = |id| &a.bundle.modules.iter().find(|m| m.id == id).unwrap().deps;
    assert_eq!(deps("src/main.js"), &["src/outer.js"]);
    assert_eq!(deps("src/other.js"), &["src/inner.js"]);
    assert!(deps("src/shadow.js").is_empty(), "a local parameter is not a wrapper call");
}

#[test]
fn esbuild_dependencies_ignore_unrecognised_and_repeated_calls() {
    let source = r#"
var a = h({"src/util.js"(exports, module) { module.exports = 1; }});
var b = h({"src/main.js"(exports, module) { a(); a(); unknown(); require_external(); a(1); }});
"#;
    let a = run("out.js", source);
    assert_eq!(a.facts.parse_errors, 0);
    assert_eq!(a.bundle.modules.iter().find(|m| m.id == "src/main.js").unwrap().deps, ["src/util.js"]);
}

#[test]
fn fingerprints_ignore_names() {
    let a = run("a.js", "function f(a, b) { return a + b * 2; }").facts.fingerprint;
    let b = run("b.js", "function g(x, y) { return x + y * 2; }").facts.fingerprint;
    let c = run("c.js", "function g(x, y) { return x - y * 2; }").facts.fingerprint;
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn deep_and_broken_sources_fall_back() {
    let deep = format!("x = {}1{};", "(".repeat(50_000), ")".repeat(50_000));
    let f = run("deep.js", &deep).facts;
    assert_eq!(f.mode, ParseMode::Lexer);
    assert!(f.mode_reason.unwrap().contains("nests"));
    let not = format!("x = {}y;", "!".repeat(100_000));
    assert_eq!(run("n.js", &not).facts.mode, ParseMode::Lexer);
    let chain = format!("x = {}1;", "a+".repeat(100_000));
    assert_eq!(run("c.js", &chain).facts.mode, ParseMode::Lexer);
    // Just under the limit still parses (on the big-stack thread) without crashing.
    let ok = format!("x = {}1{};", "(".repeat(2000), ")".repeat(2000));
    assert_ne!(run("ok.js", &ok).facts.mode, ParseMode::Lexer);
    let broken = run("b.js", "const x = require('electron'); ipcMain.handle('ch', () => { ) } }}}");
    assert!(broken.facts.ipc.iter().any(|i| i.channel.as_deref() == Some("ch")) || broken.facts.imports.iter().any(|i| i.specifier == "electron"));
    // Shapes the estimator used to admit, which then overflowed the parser's stack.
    let n = 6_000;
    for (what, src) in [
        ("else-if chain", format!("if(a);{}", "else if(a);".repeat(n))),
        ("class extends chain", format!("x = class{};", " extends class".repeat(n) + " {}")),
        ("prefix ++ run", format!("x = {}a;", "++".repeat(n))),
        ("brackets after an update operator", format!("x = a++ /{} / 1;", "[".repeat(n) + &"]".repeat(n))),
        ("else-if chain with comma expressions", "if(a)b,c;else ".repeat(n) + "if(a)b,c;"),
        ("block and unbraced else-if consequents", "if(a){b;}else if(a)c;else ".repeat(n) + "if(a){d;}"),
        ("brackets in a regex after yield", format!("x = yield /{} /;", "[".repeat(n) + &"]".repeat(n))),
        ("prefix operators in a regex after yield", format!("x = yield /{}a / 1;", "!".repeat(n))),
        ("addition chain in a regex after yield", format!("x = yield /{}a / 1;", "a+".repeat(n))),
        ("separated prefix operators in a regex after yield", format!("x = yield /{}a / 1;", "!a".repeat(n))),
        ("conditional chain in a regex after yield", format!("x = yield /{}a / 1;", "a?b:".repeat(n))),
    ] {
        let f = run("deep.js", &src).facts;
        assert_eq!(f.mode, ParseMode::Lexer, "{what}");
        assert!(f.mode_reason.as_deref().unwrap_or("").contains("nests"), "{what}: {:?}", f.mode_reason);
    }
}

/// Exhausted ambiguous-regex lookahead fails closed instead of guessing division, and the
/// configured (wasm-size) nesting limit still applies to sources the lookahead can judge.
#[test]
fn exhausted_yield_lookahead_fails_closed() {
    let hostile = "yield /[".repeat(50);
    let limits = Limits { max_nesting: 160, ..Limits::default() };
    assert!(admit(&hostile, &limits).is_err(), "admit fails closed when the lookahead budget runs out");
    let f = analyze("h.js", &hostile, &limits).facts;
    assert_eq!(f.mode, ParseMode::Lexer, "{:?}", f.mode_reason);
    let deep = format!("x = {}1{};", "(".repeat(200), ")".repeat(200));
    assert!(admit(&deep, &limits).is_err(), "the 160-level limit still refuses a deep source");
    assert!(admit("yield /a[0]/g;", &limits).is_ok(), "a compact regex after yield is still admitted");
}

/// A `yield /…/` regex whose following token differs between the regex and division readings
/// fails admission closed; a compact regex with a plain follow is still admitted.
#[test]
fn ambiguous_regex_with_a_diverging_follow_fails_closed() {
    let limits = Limits { max_nesting: 160, ..Limits::default() };
    for src in ["yield /a[0]/ /b[1]/;", "yield /a[0]/ ++b;"] {
        assert!(admit(src, &limits).is_err(), "{src}");
        assert_eq!(analyze("d.js", src, &limits).facts.mode, ParseMode::Lexer, "{src}");
    }
    assert!(admit("yield /a[0]/g;", &limits).is_ok(), "the compact regex control");
}

/// A quote inside the ambiguous token starts a string in the division reading, where deeply
/// nested code follows; the regex reading hides it, so admission fails closed.
#[test]
fn ambiguous_regex_with_a_quote_hides_division_boundaries() {
    let quoted = format!("yield /\"a/b\"/ {};", "(".repeat(6_000));
    assert!(admit(&quoted, &Limits::default()).is_err(), "a quote inside the ambiguous token");
    assert_eq!(analyze("q.js", &quoted, &Limits::default()).facts.mode, ParseMode::Lexer);
}

/// A conditional chain in the division reading counts one level per `?`, so the wasm-size limit
/// refuses it; the compact generator regex stays admitted.
#[test]
fn ambiguous_regex_conditional_chain_is_counted() {
    let limits = Limits { max_nesting: 160, ..Limits::default() };
    let cond = format!("x = yield /{}a / 1;", "a?b:".repeat(200));
    assert!(admit(&cond, &limits).is_err(), "a conditional chain in an ambiguous regex");
    assert!(admit("yield /a[0]/g;", &limits).is_ok(), "the compact regex control");
}

/// Bitwise chains in the division reading count one level per operator at the configured limit.
#[test]
fn ambiguous_regex_bitwise_chains_are_counted() {
    let limits = Limits { max_nesting: 160, ..Limits::default() };
    for op in ['&', '|', '^'] {
        let src = format!("x = yield /{}a / 1;", format!("a{op}").repeat(200));
        assert!(admit(&src, &limits).is_err(), "bitwise {op}");
    }
    assert!(admit("yield /a[0]/g;", &limits).is_ok(), "the compact regex control");
}

/// A closing slash immediately followed by `*` opens a block comment in sloppy code while the
/// regex reading passes it, so admission fails closed; the balanced case is the regression.
#[test]
fn ambiguous_regex_before_a_block_comment_fails_closed() {
    let src = "yield /a/*b*/;";
    assert!(admit(src, &Limits::default()).is_err());
    assert_eq!(analyze("c.js", src, &Limits::default()).facts.mode, ParseMode::Lexer);
}

/// The division reading's depth stays in the expression, so following brackets add to it.
#[test]
fn ambiguous_regex_depth_carries_into_following_expression() {
    let limits = Limits { max_nesting: 160, ..Limits::default() };
    let src = format!("x = yield /{}a / {}1{};", "a+".repeat(100), "(".repeat(100), ")".repeat(100));
    assert!(admit(&src, &limits).is_err(), "100 operators plus 100 brackets should exceed 160");
}

/// An unbalanced bracket inside the ambiguous token leaves the two readings with different
/// bracket scopes, so admission fails closed.
#[test]
fn ambiguous_regex_with_unbalanced_brackets_fails_closed() {
    let src = "x = yield /a(/ 1;";
    assert!(admit(src, &Limits::default()).is_err());
    assert_eq!(analyze("u.js", src, &Limits::default()).facts.mode, ParseMode::Lexer);
}

/// Balanced brackets inside the ambiguous token keep their depth, so 100 of them plus 100
/// following brackets exceed the 160-level limit.
#[test]
fn ambiguous_regex_balanced_brackets_count_toward_the_limit() {
    let limits = Limits { max_nesting: 160, ..Limits::default() };
    let src = format!("x = yield /{}/ {}1{};", "(".repeat(100) + &")".repeat(100), "(".repeat(100), ")".repeat(100));
    assert!(admit(&src, &limits).is_err(), "balanced brackets plus following nesting should exceed 160");
}

#[test]
fn junk_never_panics() {
    let samples = [
        "",
        "\u{0}",
        "`${",
        "/*",
        "<div",
        "<<<<",
        "require(",
        "import(",
        "new BrowserWindow({webPreferences: x})",
        "contextBridge.exposeInMainWorld()",
        "ipcMain.handle()",
        "fetch()",
        "a.b.c.d.e.f.g.h.i.j.k.l.m.n.o.p.q.r.s.t.u.v.w.x.y.z.a.b.c.d.e.f.g()",
        "é = 'é'; ü.ö('ä')",
    ];
    for s in samples {
        let _ = run("j.js", s);
        let _ = run("j.ts", s);
        let _ = run("j.mjs", s);
    }
    let long_member = format!("x{}();", ".y".repeat(5000));
    let _ = run("m.js", &long_member);
}

#[test]
fn aliases_follow_scopes_not_names() {
    // Minified code reuses short names: a parameter `e` must not inherit the module-level alias.
    let src = "const { ipcRenderer: e } = require('electron');\nfunction f(e) { e.on('not-ipc', () => {}); }\ne.on('real', () => {});\nconst API = 'https://a.test'; function g(API) { fetch(API + '/x'); }";
    let f = run("r.js", src).facts;
    let channels: Vec<&str> = f.ipc.iter().filter_map(|i| i.channel.as_deref()).collect();
    assert_eq!(channels, vec!["real"]);
    assert!(f.endpoints.iter().any(|e| e.url == "{API}/x" && !e.literal), "{:?}", f.endpoints);
    let long = format!("a{}.on('x', function () {{}})", ".b()".repeat(200));
    let g = run("l.js", &long).facts;
    assert!(g.functions.iter().all(|f| f.name.chars().count() < 140), "{:?}", g.functions.iter().map(|f| &f.name).collect::<Vec<_>>());
}

#[test]
fn window_options_through_named_objects() {
    let dynamic = Some(PrefValue::Text("dynamic".into()));
    let window = |path: &str, src: &str| run(path, src).facts.windows;
    // options in a variable, with a preload path
    let w = window(
        "main.js",
        "const path = require('path');\nconst opts = { width: 800, webPreferences: { contextIsolation: true, preload: path.join(__dirname, 'preload.js') } };\nconst win = new BrowserWindow(opts);",
    );
    assert_eq!(w[0].web_preferences.get("contextIsolation"), Some(&PrefValue::Bool(true)));
    assert_eq!(w[0].preload.as_deref(), Some("preload.js"));
    assert_eq!(w[0].options_from, Some(NamedObjectRef { name: "opts".into(), line: 2 }));
    assert_eq!(w[0].binding.as_deref(), Some("win"));
    // webPreferences in their own variable, inside named or inline options; two windows share one
    let src = "const prefs = { sandbox: true };\nconst opts = { webPreferences: prefs };\nnew BrowserWindow(opts);\nnew BrowserWindow(opts);\nnew BrowserView({ webPreferences: prefs });";
    let w = window("main.js", src);
    assert!(w.iter().all(|x| x.web_preferences.get("sandbox") == Some(&PrefValue::Bool(true))), "{w:?}");
    assert_eq!(w.iter().map(|x| x.options_from.as_ref().map(|o| o.name.as_str())).collect::<Vec<_>>(), vec![Some("opts"), Some("opts"), Some("prefs")]);
    // an object declared inside an exported function is still local
    let w = window("main.mjs", "export function make() { const opts = { webPreferences: { sandbox: true } }; return new BrowserWindow(opts); }");
    assert_eq!(w[0].web_preferences.get("sandbox"), Some(&PrefValue::Bool(true)));

    // Anything that could change the object keeps the window dynamic, whatever the order.
    for src in [
        "const opts = { webPreferences: { sandbox: true } };\nopts.webPreferences.sandbox = false;\nnew BrowserWindow(opts);",
        "const opts = { webPreferences: { sandbox: true } };\ntweak(opts);\nnew BrowserWindow(opts);",
        "const opts = { webPreferences: { sandbox: true } };\nfunction make() { return new BrowserWindow(opts); }\nopts.webPreferences = { sandbox: false };",
        "let opts = { webPreferences: { sandbox: true } };\nopts = other;\nnew BrowserWindow(opts);",
        "const opts = { webPreferences: { sandbox: true } };\nmodule.exports = opts;\nnew BrowserWindow(opts);",
        "const prefs = { sandbox: true };\nObject.assign(prefs, extra);\nnew BrowserWindow({ webPreferences: prefs });",
        // options that escape take their webPreferences object with them
        "const prefs = { sandbox: true };\nconst opts = { webPreferences: prefs };\nleak(opts);\nnew BrowserWindow({ webPreferences: prefs });",
        // a parameter of the same name is not the object
        "const opts = { webPreferences: { sandbox: true } };\nfunction make(opts) { return new BrowserWindow(opts); }",
    ] {
        let w = window("main.js", src);
        assert_eq!(w[0].web_preferences.get("webPreferences"), dynamic.as_ref(), "{src}\n{w:?}");
        assert_eq!(w[0].web_preferences.get("sandbox"), None, "{src}");
        assert_eq!(w[0].options_from, None, "{src}");
    }
    // A spread may set or replace any preference: only what is written after it is known.
    let w = window(
        "main.js",
        "new BrowserWindow({ ...base, webPreferences: { sandbox: true } });\nnew BrowserWindow({ webPreferences: { sandbox: true }, ...base });\nnew BrowserWindow({ webPreferences: { preload: 'p.js', sandbox: true, ...extra } });\nnew BrowserWindow({ webPreferences: { ...extra, contextIsolation: false } });\nnew BrowserWindow({ [key]: {}, show: false });",
    );
    assert_eq!(w[0].web_preferences, std::collections::BTreeMap::from([("sandbox".to_string(), PrefValue::Bool(true))]));
    assert_eq!(w[1].web_preferences.get("webPreferences"), dynamic.as_ref());
    assert_eq!((w[1].web_preferences.len(), w[2].web_preferences.len(), w[2].preload.as_deref()), (1, 1, None));
    assert_eq!(w[3].web_preferences.get("contextIsolation"), Some(&PrefValue::Bool(false)));
    assert_eq!(w[3].web_preferences.get("webPreferences"), dynamic.as_ref());
    assert_eq!(w[4].web_preferences.get("webPreferences"), dynamic.as_ref());
    // the same through a named object (a minified app's shape)
    let w = window(
        "main.js",
        "function make() { let e = { ...A, width: 1, webPreferences: { preload: P.join(__dirname, 'p.js'), ...A.webPreferences } }; return new BrowserWindow(e); }",
    );
    assert_eq!((w[0].web_preferences.get("webPreferences"), w[0].preload.as_deref()), (dynamic.as_ref(), None));
    assert_eq!(w[0].options_from.as_ref().map(|o| o.name.as_str()), Some("e"));

    for src in [
        "export const opts = { webPreferences: { sandbox: true } };\nnew BrowserWindow(opts);",
        "const opts = { webPreferences: { sandbox: true } };\nnew BrowserWindow(opts);\nexport { opts };",
    ] {
        let w = window("main.mjs", src);
        assert_eq!(w[0].web_preferences.get("webPreferences"), dynamic.as_ref(), "{src}\n{w:?}");
    }
}

// ---- GraphQL and tRPC ----

fn ops(src: &str) -> Vec<(String, String, Option<String>, bool)> {
    let a = analyze("app.tsx", src, &Limits::default());
    a.facts.api_operations.iter().map(|o| (o.protocol.clone(), o.kind.clone(), o.name.clone(), o.literal)).collect()
}

fn op(protocol: &str, kind: &str, name: &str) -> (String, String, Option<String>, bool) {
    (protocol.into(), kind.into(), Some(name.into()), true)
}

fn has(list: &[(String, String, Option<String>, bool)], protocol: &str, kind: &str, name: &str) -> bool {
    list.contains(&op(protocol, kind, name))
}

fn unknowns(list: &[(String, String, Option<String>, bool)]) -> usize {
    list.iter().filter(|o| o.1 == "unknown" && !o.3).count()
}

#[test]
fn graphql_tagged_templates() {
    let l = ops(r#"import { gql } from "@apollo/client";
        export const Q = gql`query GetUser($id: ID!) { user(id: $id) { name } }`;
        const M = gql`mutation SaveUser { save { id } }`;
        const S = graphql`subscription Live { tick }`;"#);
    assert!(has(&l, "graphql", "query", "GetUser") && has(&l, "graphql", "mutation", "SaveUser") && has(&l, "graphql", "subscription", "Live"), "{l:?}");
    assert_eq!(l.len(), 3, "each once, though the template is also read as text: {l:?}");
}

#[test]
fn graphql_interpolation_means_unknown() {
    let l = ops("const Q = gql`query Known { a ...F } ${FRAGMENT}`;");
    assert!(has(&l, "graphql", "query", "Known"), "{l:?}");
    assert_eq!(unknowns(&l), 1, "what's in the fragment isn't known: {l:?}");
    let l = ops("const Q = gql`query ${name} { a }`;");
    assert!(l.iter().all(|o| o.2.is_none() && !o.3) && !l.is_empty(), "the name isn't guessed: {l:?}");
    let l = ops("const Q = gql`query Get${suffix} { a }`;");
    assert!(l.iter().all(|o| o.2.is_none() && !o.3) && !l.is_empty(), "nor half of it: {l:?}");
    assert_eq!(unknowns(&ops("const Q = gql`${whole}`;")), 1);
    assert_eq!(unknowns(&ops("const Q = gql(text);")), 1);
}

#[test]
fn graphql_documents_in_plain_strings_and_minified_shapes() {
    let l = ops(r#"fetch("/graphql",{method:"POST",body:JSON.stringify({query:"query Feed{items{id}}",variables:{}})})"#);
    assert!(has(&l, "graphql", "query", "Feed"), "{l:?}");
    assert!(l.iter().any(|o| o.0 == "graphql" && o.1 == "endpoint"), "{l:?}");
    let min = ops(r#"const a=(0,n.gql)`query A{b}`,c=(0,n.gql)("mutation D{e}");"#);
    assert!(has(&min, "graphql", "query", "A") && has(&min, "graphql", "mutation", "D"), "{min:?}");
    assert_eq!(min.len(), 2, "{min:?}");
    // A compiled document, as bundler plugins write them.
    let compiled =
        ops(r#"var d={kind:"Document",definitions:[{kind:"OperationDefinition",operation:"mutation",name:{kind:"Name",value:"Pay"},selectionSet:{}}]};"#);
    assert_eq!(compiled, [op("graphql", "mutation", "Pay")]);
    let anon = ops(r#"var d={kind:"OperationDefinition",operation:"query",name:void 0};"#);
    assert_eq!(anon, [("graphql".to_string(), "query".to_string(), None, true)], "known to be anonymous");
}

#[test]
fn graphql_documents_that_begin_with_a_comment() {
    // A comment is valid at the start of a GraphQL document, and common in generated clients.
    let l = ops(r##"const q="# Client document\nquery Feed{items{id}}";"##);
    assert!(has(&l, "graphql", "query", "Feed"), "{l:?}");
    let l = ops("const q=`# Client document\nquery Feed{items{id}}`;");
    assert!(has(&l, "graphql", "query", "Feed"), "{l:?}");
    // A lone CR ends a comment in GraphQL, as a CRLF does. A string escape keeps the CR; a
    // template literal would normalize it to LF.
    let l = ops(r##"const q="# Client document\rquery Feed{items{id}}";"##);
    assert!(has(&l, "graphql", "query", "Feed"), "{l:?}");
    // A template with run-time text goes through the other call site.
    let l = ops("const q=`# Client document\nquery Feed{items{id}} ${x}`;");
    assert!(has(&l, "graphql", "query", "Feed"), "{l:?}");
    // The same document in a request body.
    let l = ops(r##"fetch("/graphql",{method:"POST",body:JSON.stringify({query:"# Client document\nquery Feed{items{id}}",variables:{}})})"##);
    assert!(has(&l, "graphql", "query", "Feed"), "{l:?}");
    // A comment doesn't make any other text a document.
    let l = ops(r##"const s="# note\nconst a = {b:1}";"##);
    assert!(!l.iter().any(|o| o.0 == "graphql"), "{l:?}");
}

#[test]
fn graphql_comment_inside_a_selection_set() {
    // A comment may sit between fields, and a lone CR ends it as a newline does.
    let l = ops(r##"const q="query Feed{items # note\r id}";"##);
    assert!(has(&l, "graphql", "query", "Feed"), "{l:?}");
}

#[test]
fn graphql_spreads_and_computed_values_mean_unknown() {
    let l = ops(r#"var d={...base,kind:"OperationDefinition",operation:"query",name:{value:"Hidden"}};"#);
    assert_eq!(unknowns(&l), 1, "{l:?}");
    assert!(!has(&l, "graphql", "query", "Hidden"), "a spread may replace it: {l:?}");
    let l = ops(r#"var d={kind:"OperationDefinition",operation:op,name:{value:"X"}};"#);
    assert_eq!(unknowns(&l), 1, "{l:?}");
    let l = ops(r#"var d={kind:"OperationDefinition",operation:"query",name:{...n,value:"X"}};"#);
    assert_eq!(unknowns(&l), 1, "{l:?}");
    let l = ops(r#"post({operationName:"Named",query:q,variables:v})"#);
    assert_eq!(l, [op("graphql", "operation", "Named")]);
    assert_eq!(unknowns(&ops(r#"post({operationName:name,query:q})"#)), 1);
    assert_eq!(unknowns(&ops(r#"post({...x,operationName:"Named",query:q})"#)), 1);
}

#[test]
fn not_graphql() {
    for src in [
        r#"const a = "query the database";"#,
        r#"const a = "mutation observer";"#,
        r#"const a = "query { unfinished";"#,
        r#"const a = "{ a }";"#,
        "const a = `query ${x}`;",
        r#"const o = {kind: "Other", operation: "query"};"#,
        r#"const o = {operationName: "NotARequest"};"#,
        r#"db.query("select 1");"#,
        "const c = css`query { color: ${x}`;",
        r#"const u = "/docs/graphql/intro";"#,
        r#"const u = "graphql";"#,
    ] {
        assert_eq!(ops(src), [], "{src}");
    }
}

#[test]
fn trpc_routers_with_nesting() {
    let l = ops(r#"export const appRouter = createTRPCRouter({
        hello: publicProcedure.query(() => "hi"),
        post: createTRPCRouter({
            byId: publicProcedure.input(z.string()).query(({input}) => input),
            create: protectedProcedure.input(z.object({})).mutation(async () => {}),
            onAdd: t.procedure.subscription(() => stream()),
        }),
        plain: { inner: t.procedure.query(fn) },
    });"#);
    for (kind, name) in [("query", "hello"), ("query", "post.byId"), ("mutation", "post.create"), ("subscription", "post.onAdd"), ("query", "plain.inner")] {
        assert!(has(&l, "trpc", kind, name), "{kind} {name}: {l:?}");
    }
    assert_eq!(l.len(), 5, "the nested router isn't read again by itself: {l:?}");
}

#[test]
fn trpc_router_parts_we_cannot_read_are_unknown() {
    let l = ops(r#"const r = t.router({ a: t.procedure.query(f), post: postRouter, ...others, [key]: t.procedure.mutation(g) });"#);
    assert!(has(&l, "trpc", "query", "a"), "{l:?}");
    assert!(l.contains(&("trpc".into(), "unknown".into(), Some("post".into()), false)), "the key is known, what's under it isn't: {l:?}");
    assert_eq!(l.iter().filter(|o| o.1 == "unknown" && o.2.is_none()).count(), 2, "the spread and the computed key: {l:?}");
    assert!(l.iter().all(|o| o.2.as_deref() != Some("key")), "{l:?}");
    let named = ops("const r = createTRPCRouter({ post: postRouter });");
    assert_eq!(named, [("trpc".to_string(), "unknown".to_string(), Some("post".to_string()), false)], "the name alone says it's a router");
    assert_eq!(unknowns(&ops("const r = mergeRouters(a, b);")), 1);
}

#[test]
fn trpc_minified_routers_are_recognised_by_shape() {
    let l = ops("const r=a({get:b.input(c).query(async d=>e),add:b.input(c).mutation(f),sub:a({x:b.use(g).query(h)})});");
    assert!(has(&l, "trpc", "query", "get") && has(&l, "trpc", "mutation", "add") && has(&l, "trpc", "query", "sub.x"), "{l:?}");
    assert_eq!(l.len(), 3, "{l:?}");
}

#[test]
fn trpc_client_calls() {
    let l = ops(r#"import { trpc } from "./trpc";
        const a = trpc.post.byId.useQuery({id: 1});
        const b = trpc.post.create.useMutation();
        const c = trpc.hello.useQuery();
        const d = trpc.feed.list.useInfiniteQuery({});
        const e = trpc.tick.useSubscription();
        const f = trpcClient.user.get.query({});
        const g = trpcClient.user.save.mutate({});
        const h = trpc.useQuery(["old.style", {}]);"#);
    for (kind, name) in [
        ("query", "post.byId"),
        ("mutation", "post.create"),
        ("query", "hello"),
        ("query", "feed.list"),
        ("subscription", "tick"),
        ("query", "user.get"),
        ("mutation", "user.save"),
        ("query", "old.style"),
    ] {
        assert!(has(&l, "trpc", kind, name), "{kind} {name}: {l:?}");
    }
    let min = ops("const a=t.post.byId.useQuery({id:1}),b=(0,o.useState)(0);");
    assert_eq!(min, [op("trpc", "query", "post.byId")]);
    assert_eq!(unknowns(&ops("const a=trpc[name].useQuery();")), 1, "a path chosen at run time");
    assert_eq!(unknowns(&ops("const a=trpc.useQuery([name]);")), 1);
}

#[test]
fn trpc_urls() {
    let l = ops(r#"const a = httpBatchLink({url: "/api/trpc"}); fetch("https://x.test/trpc/a.b,c.d?batch=1");"#);
    assert!(l.iter().any(|o| o.0 == "trpc" && o.1 == "endpoint"), "{l:?}");
    assert!(has(&l, "trpc", "operation", "a.b") && has(&l, "trpc", "operation", "c.d"), "{l:?}");
    let dynamic = analyze("a.js", "fetch(`/api/trpc/${path}`); fetch('/api/trpc/' + other);", &Limits::default()).facts.api_operations;
    assert!(dynamic.iter().all(|o| o.protocol == "trpc" && o.name.is_none()), "no procedure name is guessed: {dynamic:?}");
    assert_eq!(dynamic.iter().filter(|o| o.kind == "unknown" && !o.literal).count(), 3, "{dynamic:?}");
}

#[test]
fn not_trpc() {
    for src in [
        "const a = useQuery({queryKey: []});",
        "const a = client.useQuery(q);",
        "const a = api.endpoints.getPosts.useQuery();",
        r#"import { useQuery } from "@tanstack/react-query"; useQuery({});"#,
        r#"const q = require("@tanstack/react-query"); q.useQuery({});"#,
        r#"const q = require("@tanstack/react-query"); q.foo.useQuery({});"#,
        "const a = db.users.query(sql);",
        "const a = pool.query(fn);",
        "const a = wrap({list: db.query(fn), item: db.mutation(fn)});",
        "const a = router({routes: []});",
        "const a = createRouter({history: h, routes: r});",
        "app.use(router({a: 1}));",
        r#"const u = "https://trpc.io/docs";"#,
    ] {
        assert_eq!(ops(src), [], "{src}");
    }
}

#[test]
fn the_token_scan_finds_the_same_documents_hooks_and_urls() {
    let limits = Limits { max_parse_bytes: 1, ..Limits::default() };
    let src = r#"const q=gql`query Tagged{a}`,p=gql`query ${n}{a}`,s="mutation Plain{b}",u="/api/trpc/x.y",g="/graphql",h=t.post.byId.useQuery(),i=useQuery(),j=a.useQuery(),k=e.endpoints.g.useQuery(),l=`/trpc/${z}`,w="query the db";"#;
    let a = analyze("big.js", src, &limits);
    assert_eq!(a.facts.mode, ParseMode::Lexer);
    let l: Vec<_> = a.facts.api_operations.iter().map(|o| (o.protocol.clone(), o.kind.clone(), o.name.clone(), o.literal)).collect();
    for (p, k, n) in [("graphql", "query", "Tagged"), ("graphql", "mutation", "Plain"), ("trpc", "operation", "x.y"), ("trpc", "query", "post.byId")] {
        assert!(has(&l, p, k, n), "{p} {k} {n}: {l:?}");
    }
    assert!(l.iter().any(|o| o.0 == "graphql" && o.1 == "endpoint"), "{l:?}");
    assert!(l.iter().any(|o| o.0 == "graphql" && o.1 == "unknown"), "the document with a hole: {l:?}");
    assert!(l.iter().any(|o| o.0 == "trpc" && o.1 == "unknown"), "the URL with a hole: {l:?}");
    assert_eq!(l.len(), 7, "{l:?}");
}

#[test]
fn api_rows_are_capped_on_hostile_sources() {
    let src: String = (0..(crate::facts::MAX_LIST + 50)).map(|i| format!("const a{i}=gql`query Q{i}{{a}}`;\n")).collect();
    let a = analyze("many.js", &src, &Limits::default());
    assert_eq!(a.facts.api_operations.len(), crate::facts::MAX_LIST);
    assert!(a.facts.truncated.iter().any(|t| t == "api_operations"));
    let deep = format!("const r=createTRPCRouter({}1{});", "{a:".repeat(40), "}".repeat(40));
    let _ = analyze("deep.js", &deep, &Limits::default());
}

#[test]
fn sources_without_api_rows_serialise_as_before() {
    let a = analyze("a.js", "const x = 1;", &Limits::default());
    assert!(a.facts.api_operations.is_empty());
    assert!(serde_json::to_value(&a.facts).unwrap().get("api_operations").is_none(), "no new key where there's nothing to say");
}

/// The lexer fallback caps endpoints and exposed objects like the syntax-tree pass, so a huge or
/// unparsable bundle can't grow them without limit.
#[test]
fn lexer_fallback_caps_endpoints_and_exposed() {
    let limits = Limits { max_parse_bytes: 1, ..Limits::default() };
    let mut src = "fetch('x');\n".repeat(crate::facts::MAX_LIST + 10);
    // Endpoints are already at the cap, so a URL literal no longer adds one.
    src.push_str(&"('https://x.test/a');\n".repeat(8));
    src.push_str(&"window.exposeInMainWorld('k');\n".repeat(crate::facts::MAX_LIST * 2));
    let a = analyze("big.js", &src, &limits);
    assert_eq!(a.facts.mode, ParseMode::Lexer, "{:?}", a.facts.mode_reason);
    assert_eq!(a.facts.endpoints.len(), crate::facts::MAX_LIST, "endpoints are capped, literals included");
    assert!(a.facts.endpoints.iter().all(|e| e.api == "fetch"), "{:?}", a.facts.endpoints.first());
    assert!(a.facts.truncated.iter().any(|t| t == "endpoints"), "{:?}", a.facts.truncated);
    assert_eq!(a.facts.exposed.len(), crate::facts::MAX_LIST);
}
