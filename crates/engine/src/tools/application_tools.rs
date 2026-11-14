//! Application-graph workflows: feature and semantic traces, version and export-outline
//! comparisons, reference sources versus the shipped bundle, reconstruction expectation ledgers
//! and coverage, and platform projections of artifact inventories. Pure computation over
//! evidence (or over a path, analysed on the spot).

use super::artifact_tools::{
    WalkOptions, abs_path, env, inventory_path, is_path_input, recorded_inventory, resolve_evidence_input, skipped_gitignores_limitation, walk_dir,
};
use super::js_tools::{load_graph, load_graph_with_limitations};
use crate::tool::{Ctx, Effects, Requires, ToolOutput, ToolSpec, Toolset, parse_params, schema_of};
use craft_anything_analysis::js_inventory::{InvLimits, Inventory};
use craft_anything_analysis::js_ledger::{Binding, Review, build_ledger, evaluate_coverage};
use craft_anything_analysis::js_source::{SourceGraph, build_source_graph, diff_source_and_bundle};
use craft_anything_analysis::js_trace::{Direction, SeedKind, find_seeds, follow_feature, trace_semantics};
use craft_anything_analysis::{js_compare, js_projection};
use craft_anything_evidence::{Confidence, Coverage, CoverageStatus, EvidenceBuilder, Limitation, Location, Subject, ToolError};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

pub fn specs() -> Vec<ToolSpec> {
    let app =
        |id: &'static str, title: &'static str, summary: &'static str, beginner: &'static str, schema: fn() -> Value, run: crate::tool::ToolFn| ToolSpec {
            id,
            title,
            summary,
            beginner,
            toolset: Toolset::Application,
            effects: Effects::EVIDENCE,
            requires: Requires::Nothing,
            input_schema: schema,
            run,
        };
    vec![
        app(
            "app_feature_follow",
            "Trace a feature",
            "Follow a feature through an application graph from a seed (module, function, string, IPC channel, endpoint, route, bridge API or node id): what leads to it and what it leads to, across renderer ↔ preload ↔ main, with readable chains and a one-line story.",
            "Follow one feature (like \"Export\") from the button the user clicks to what the app finally does.",
            schema_of::<TraceFeatureParams>,
            app_feature_follow,
        ),
        app(
            "js_function_explain",
            "Explain a function",
            "Function-level slice of an application graph: for the seeded functions, their calls (resolved where possible), callers (resolved or same-name inferred), strings, the IPC/effects/endpoints/storage they touch, and a depth-limited callee tree. Unresolved dynamic calls are listed.",
            "See what a piece of code does: what it calls, who calls it and what it touches.",
            schema_of::<TraceSemanticsParams>,
            js_function_explain,
        ),
        app(
            "app_graphs_compare",
            "Compare two app versions",
            "Compare two application graphs: modules (unchanged, changed, moved by content, renamed by structure, added, removed), bundle modules (content → structure → id), IPC channels and handlers, bridge members, endpoints, routes, window webPreferences, native modules and vendor versions.",
            "See what changed between two versions of an app.",
            schema_of::<PairParams>,
            app_graphs_compare,
        ),
        app(
            "js_exports_compare",
            "Compare module exports",
            "Compare what each module exports (ES module names, CommonJS names, the kind of module.exports) between two application graphs, matching modules by path then by structure.",
            "Check whether the pieces of an app still offer the same functions to each other.",
            schema_of::<PairParams>,
            js_exports_compare,
        ),
        app(
            "app_source_import",
            "Import a source folder",
            "Read a reference source tree (honouring .gitignore, skipping .git and hidden files) into a source graph: every file with sha256 and language; JavaScript/TypeScript files with exports, imports, function names, strings and a structural fingerprint. Use with js_source_bundle_compare.",
            "Load the original source code of an app so it can be lined up with what was shipped.",
            schema_of::<ImportSourceParams>,
            app_source_import,
        ),
        app(
            "js_source_bundle_compare",
            "Line up source with the bundle",
            "Match reference source files (from app_source_import, or a folder path) to the shipped app's files and bundle modules by path, source-map listing, shared strings, function names and export names; reports matches with scores and reasons, unmatched sources and bundle code without a source.",
            "Find which original source file each part of a shipped app came from.",
            schema_of::<SourceBundleParams>,
            compare_source_to_bundle_tool,
        ),
        app(
            "app_ledger_build",
            "Build a rebuild ledger",
            "Derive the expectations a clean-room rebuild must meet from an application graph (IPC handlers, bridge members, windows, endpoints, routes, storage, file/process effects, native modules, entry point) and mark each one proven only when the manifest gives it one owner, a parser type where needed, every case it must cover with strong enough evidence, and a passing check with its result. Ledger status: ready, open, failing or unclear.",
            "Make a checklist of everything a rebuilt app must do, and tick items off only with proof.",
            schema_of::<LedgerParams>,
            app_ledger_build,
        ),
        app(
            "app_coverage_evaluate",
            "How complete is the rebuild",
            "Summarise a rebuild ledger: the share of in-scope expectations proven, counts by layer and status, and the open gaps with what each still needs.",
            "See how much of a rebuild is proven and what's still missing.",
            schema_of::<CoverageParams>,
            app_coverage_evaluate,
        ),
        app(
            "android_app_graph_build",
            "Map an Android app",
            "Project an Android package's inventory (core_archive_show evidence or a path) into an application graph: manifest, DEX code, native libraries by ABI, split APKs, signing files, resources and embedded runtimes (Flutter, React Native/Hermes, Cordova/Capacitor, .NET, Unity…). Runtime hints are inferred from file names.",
            "Get an overview of how an Android app is put together.",
            schema_of::<ProjectionParams>,
            android_app_graph_build,
        ),
        app(
            "apple_app_graph_build",
            "Map an Apple app",
            "Project an iOS/macOS app's inventory (core_archive_show evidence or a path) into an application graph: bundles and extensions, executables, frameworks, dylibs, signing material, interface files, asset catalogs and embedded runtimes (Flutter, React Native, Electron, Unity…). Runtime hints are inferred from file names.",
            "Get an overview of how an iPhone or Mac app is put together.",
            schema_of::<ProjectionParams>,
            apple_app_graph_build,
        ),
        app(
            "harmony_app_graph_build",
            "Map a HarmonyOS app",
            "Project a HarmonyOS package's inventory (core_archive_show evidence or a path) into an application graph: module manifests, HAP/HSP packages, Ark bytecode, native libraries by ABI and resources.",
            "Get an overview of how a HarmonyOS app is put together.",
            schema_of::<ProjectionParams>,
            harmony_app_graph_build,
        ),
    ]
}

/// Link ids into a builder.
fn with_links(mut b: EvidenceBuilder, ids: impl IntoIterator<Item = Option<String>>) -> EvidenceBuilder {
    for id in ids.into_iter().flatten() {
        b = b.link(id);
    }
    b
}

/// A side's analysis limitations, labelled so a pair's reader knows which side they belong to.
fn side_limitations(side: &str, lims: Vec<Limitation>) -> Vec<Limitation> {
    lims.into_iter().map(|l| Limitation::new(l.code, format!("{side}: {}", l.message))).collect()
}

fn app_subject(label: &str) -> Subject {
    Subject { kind: "js-app".into(), sha256: None, name: Some(label.to_string()) }
}

// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum MatchMode {
    /// Case-insensitive substring.
    #[default]
    Contains,
    Exact,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SeedKindParam {
    Node,
    Module,
    Function,
    String,
    IpcChannel,
    Endpoint,
    Route,
    Api,
    #[default]
    Any,
}

impl From<SeedKindParam> for SeedKind {
    fn from(s: SeedKindParam) -> Self {
        match s {
            SeedKindParam::Node => SeedKind::Node,
            SeedKindParam::Module => SeedKind::Module,
            SeedKindParam::Function => SeedKind::Function,
            SeedKindParam::String => SeedKind::String,
            SeedKindParam::IpcChannel => SeedKind::IpcChannel,
            SeedKindParam::Endpoint => SeedKind::Endpoint,
            SeedKindParam::Route => SeedKind::Route,
            SeedKindParam::Api => SeedKind::Api,
            SeedKindParam::Any => SeedKind::Any,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Seed {
    #[serde(default)]
    pub kind: SeedKindParam,
    /// What to look for: a channel name, function name, string, module path, URL fragment or node id.
    pub value: String,
    #[serde(default, rename = "match")]
    pub match_mode: MatchMode,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DirectionParam {
    /// What the seed leads to.
    Outgoing,
    /// What leads to the seed.
    Incoming,
    #[default]
    Both,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceFeatureParams {
    /// js_app_map evidence (inline or {"kind":"saved","evidence_id":…}) or an absolute path to analyse.
    pub application: Value,
    pub seed: Seed,
    #[serde(default)]
    pub direction: DirectionParam,
    /// Most hops from the seed (1–32).
    #[serde(default = "default_depth")]
    pub max_depth: usize,
    /// Most nodes in the trace.
    #[serde(default = "default_trace_nodes")]
    pub max_nodes: usize,
}

fn default_depth() -> usize {
    8
}
fn default_trace_nodes() -> usize {
    500
}

fn no_seed(value: &str) -> ToolError {
    ToolError::invalid(format!("nothing in the application graph matches {value:?}"))
        .with_remediation("Try seed.kind=any with match=contains, or look at the graph's ipc, endpoints and modules lists for exact names.")
}

fn app_feature_follow(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: TraceFeatureParams = parse_params(params)?;
    let (g, id) = load_graph(ctx, &p.application)?;
    let seeds = find_seeds(&g, p.seed.kind.into(), &p.seed.value, p.seed.match_mode == MatchMode::Exact);
    if seeds.is_empty() {
        return Err(no_seed(&p.seed.value));
    }
    let dir = match p.direction {
        DirectionParam::Outgoing => Direction::Outgoing,
        DirectionParam::Incoming => Direction::Incoming,
        DirectionParam::Both => Direction::Both,
    };
    let t = follow_feature(&g, &seeds, dir, p.max_depth.clamp(1, 32), p.max_nodes.clamp(1, 20_000));
    let summary = if t.story.len() > 1 { t.story.join(" → ") } else { format!("{} node(s) connected to {:?}.", t.nodes.len(), p.seed.value) };
    let mut b = EvidenceBuilder::new("app_feature_follow", app_subject(&g.label), env(ctx))
        .params(json!({"graph_id": g.graph_id, "seed": {"kind": format!("{:?}", p.seed.kind), "value": p.seed.value, "exact": p.seed.match_mode == MatchMode::Exact}, "direction": format!("{dir:?}"), "max_depth": p.max_depth}))
        .result(serde_json::to_value(&t).map_err(|e| ToolError::internal(e.to_string()))?)
        .summary(summary.chars().take(2000).collect::<String>())
        .confidence(Confidence::Derived)
        .limitation(Limitation::new("static_edges", "Edges come from literal names in code (channels, bridge members, function names); dynamic dispatch is not followed."));
    if t.truncated {
        b = b.limitation(Limitation::new("truncated", "The trace hit max_nodes; raise it to see more."));
    }
    for n in t.nodes.iter().filter(|n| t.seeds.contains(&n.id)) {
        if let (Some(m), Some(l)) = (&n.module, n.line) {
            b = b.location(Location::new("line", format!("{m}:{l}")).with_label(n.label.clone()));
        }
    }
    Ok(ToolOutput::evidence(with_links(b, [id]).build()?))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceSemanticsParams {
    /// js_app_map evidence or an absolute path to analyse.
    pub application: Value,
    pub seed: Seed,
    /// Depth of the callee tree (1–16).
    #[serde(default = "default_sem_depth")]
    pub max_depth: usize,
    /// Most functions reported.
    #[serde(default = "default_sem_functions")]
    pub max_functions: usize,
}

fn default_sem_depth() -> usize {
    4
}
fn default_sem_functions() -> usize {
    32
}

fn js_function_explain(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: TraceSemanticsParams = parse_params(params)?;
    let (g, id) = load_graph(ctx, &p.application)?;
    let seeds = find_seeds(&g, p.seed.kind.into(), &p.seed.value, p.seed.match_mode == MatchMode::Exact);
    if seeds.is_empty() {
        return Err(no_seed(&p.seed.value));
    }
    let r = trace_semantics(&g, &seeds, p.max_depth.clamp(1, 16), p.max_functions.clamp(1, 1000));
    let n = r["functions"].as_array().map_or(0, Vec::len);
    let first = r["functions"].get(0);
    let summary = match first {
        Some(f) => format!(
            "{} ({}:{}) calls {} function(s), has {} caller(s) and touches {} thing(s){}.",
            f["name"].as_str().unwrap_or("?"),
            f["module"].as_str().unwrap_or("?"),
            f["line"],
            f["calls"].as_array().map_or(0, Vec::len),
            f["callers"].as_array().map_or(0, Vec::len),
            f["touches"].as_array().map_or(0, Vec::len),
            if n > 1 { format!("; {} more function(s) matched", n - 1) } else { String::new() }
        ),
        None => "The seed matched no functions.".to_string(),
    };
    let b = EvidenceBuilder::new("js_function_explain", app_subject(&g.label), env(ctx))
        .params(json!({"graph_id": g.graph_id, "seed": p.seed.value, "max_depth": p.max_depth}))
        .result(r)
        .summary(summary)
        .confidence(Confidence::Derived)
        .limitation(Limitation::new("no_dataflow", "Calls and touches are read from syntax; values flowing between functions are not tracked."))
        .limitation(Limitation::new("inferred_callers", "Callers marked 'same name (inferred)' match by name only and may be wrong."));
    Ok(ToolOutput::evidence(with_links(b, [id]).build()?))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PairParams {
    /// The older version: js_app_map evidence or an absolute path.
    pub left: Value,
    /// The newer version, in the same forms.
    pub right: Value,
}

fn app_graphs_compare(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: PairParams = parse_params(params)?;
    let (l, lid, llims) = load_graph_with_limitations(ctx, &p.left)?;
    ctx.cancel.check()?;
    let (r, rid, rlims) = load_graph_with_limitations(ctx, &p.right)?;
    let d = js_compare::compare_versions(&l, &r);
    let rows_capped = d["rows_capped"].as_bool().unwrap_or(false);
    let summary = format!(
        "{} {} → {}: {} module(s) changed, {} added, {} removed; IPC channels +{} −{}; endpoints +{} −{}.",
        l.label,
        d["left"]["version"].as_str().unwrap_or("?"),
        d["right"]["version"].as_str().unwrap_or("?"),
        d["modules"]["changed"].as_array().map_or(0, Vec::len),
        d["modules"]["added"].as_array().map_or(0, Vec::len),
        d["modules"]["removed"].as_array().map_or(0, Vec::len),
        d["ipc_channels"]["added_total"].as_u64().unwrap_or(0),
        d["ipc_channels"]["removed_total"].as_u64().unwrap_or(0),
        d["endpoints"]["added_total"].as_u64().unwrap_or(0),
        d["endpoints"]["removed_total"].as_u64().unwrap_or(0),
    );
    let mut b = EvidenceBuilder::new(
        "app_graphs_compare",
        Subject { kind: "js-app-pair".into(), sha256: None, name: Some(format!("{} vs {}", l.label, r.label)) },
        env(ctx),
    )
    .params(json!({"left": l.graph_id, "right": r.graph_id}))
    .result(d)
    .summary(summary)
    .confidence(Confidence::Derived)
    .limitation(Limitation::new(
        "tiered_matching",
        "Modules match by path, then identical content, then the same name apart from a bundler's build hash (one on each side; `was` gives the old name), then identical structure; a heavily rewritten module shows as removed + added.",
    ))
    .limitations(side_limitations("left", llims))
    .limitations(side_limitations("right", rlims));
    if rows_capped {
        b = b
            .limitation(Limitation::new(
                "rows_capped",
                "One or more comparison lists stop at 2,000 rows; the totals beside each list and the summary count every change.",
            ))
            .coverage(Coverage { status: CoverageStatus::Partial, ..Default::default() });
    }
    Ok(ToolOutput::evidence(with_links(b, [lid, rid]).build()?))
}

fn js_exports_compare(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: PairParams = parse_params(params)?;
    let (l, lid, llims) = load_graph_with_limitations(ctx, &p.left)?;
    let (r, rid, rlims) = load_graph_with_limitations(ctx, &p.right)?;
    let d = js_compare::compare_export_outlines(&l, &r);
    let rows_capped = d["rows_capped"].as_bool().unwrap_or(false);
    let summary = format!("{} module(s) compared; {} changed their exports.", d["modules"].as_array().map_or(0, Vec::len), d["changed"]);
    let mut b = EvidenceBuilder::new(
        "js_exports_compare",
        Subject { kind: "js-app-pair".into(), sha256: None, name: Some(format!("{} vs {}", l.label, r.label)) },
        env(ctx),
    )
    .params(json!({"left": l.graph_id, "right": r.graph_id}))
    .result(d)
    .summary(summary)
    .confidence(Confidence::Derived)
    .limitation(Limitation::new("names_only", "Export names and module.exports kind are compared, not function signatures or types."))
    .limitations(side_limitations("left", llims))
    .limitations(side_limitations("right", rlims));
    if rows_capped {
        b = b
            .limitation(Limitation::new("rows_capped", "A module's export diff stops at 2,000 added or removed rows; the totals beside it are complete."))
            .coverage(Coverage { status: CoverageStatus::Partial, ..Default::default() });
    }
    Ok(ToolOutput::evidence(with_links(b, [lid, rid]).build()?))
}

// ---------------------------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportSourceParams {
    /// Absolute path of the source folder.
    pub path: String,
    /// Skip files matched by .gitignore files.
    #[serde(default = "yes")]
    pub respect_gitignore: bool,
    /// Include dot-files and dot-folders (other than .git, always skipped).
    #[serde(default)]
    pub include_hidden: bool,
    #[serde(default = "default_src_files")]
    pub max_files: usize,
    #[serde(default = "default_src_file_bytes")]
    pub max_file_bytes: u64,
    #[serde(default = "default_src_total")]
    pub max_total_bytes: u64,
}

fn yes() -> bool {
    true
}
fn default_src_files() -> usize {
    20_000
}
fn default_src_file_bytes() -> u64 {
    4 * 1024 * 1024
}
fn default_src_total() -> u64 {
    512 * 1024 * 1024
}

fn source_graph_from_path(ctx: &Ctx<'_>, p: &ImportSourceParams) -> Result<(SourceGraph, Vec<(String, String)>), ToolError> {
    let root = abs_path(&p.path, "path")?;
    if !root.is_dir() {
        return Err(ToolError::invalid(format!("{} is not a folder", root.display())));
    }
    let skip = |rel: &str| rel.ends_with("node_modules") && !p.respect_gitignore;
    let w = walk_dir(
        &root,
        &WalkOptions { max_files: p.max_files.max(1), max_depth: 64, skip_dir: &skip, gitignore: p.respect_gitignore, include_hidden: p.include_hidden },
        &ctx.cancel,
    )?;
    let mut files = Vec::new();
    let mut total = 0u64;
    let mut over = false;
    for f in &w.files {
        if f.rel.ends_with(".gitignore") {
            continue;
        }
        if f.size > p.max_file_bytes || total.saturating_add(f.size) > p.max_total_bytes {
            over = true;
            continue;
        }
        if let Ok(b) = std::fs::read(&f.abs) {
            total += b.len() as u64;
            files.push((f.rel.clone(), b));
        }
    }
    let label = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let cancel = ctx.cancel.clone();
    let graph = build_source_graph(&label, &files, w.ignored, w.truncated || over, &craft_anything_js::Limits::default(), &move || cancel.is_cancelled())
        .map_err(|_| ToolError::cancelled())?;
    Ok((graph, w.skipped_gitignores))
}

fn app_source_import(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: ImportSourceParams = parse_params(params)?;
    let (g, skipped_gitignores) = source_graph_from_path(ctx, &p)?;
    let scripts = g.files.iter().filter(|f| f.parse_mode.is_some()).count();
    let summary = format!("{}: {} file(s), {} script(s) analysed, {} ignored.", g.label, g.files.len(), scripts, g.ignored);
    let mut b = EvidenceBuilder::new(
        "app_source_import",
        Subject { kind: "source-tree".into(), sha256: Some(g.source_graph_id.trim_start_matches("sg_").to_string()), name: Some(g.label.clone()) },
        env(ctx),
    )
    .params(json!({"path": p.path, "respect_gitignore": p.respect_gitignore, "include_hidden": p.include_hidden}))
    .result(serde_json::to_value(&g).map_err(|e| ToolError::internal(e.to_string()))?)
    .summary(summary)
    .confidence(Confidence::Observed)
    .authority(craft_anything_evidence::Authority::Reasoning)
    .coverage(Coverage {
        status: if g.truncated { CoverageStatus::Partial } else { CoverageStatus::Complete },
        examined: Some(g.files.len() as u64),
        total: None,
        next_offset: None,
    })
    .location(Location::new("path", p.path.clone()));
    if g.truncated {
        b = b.limitation(Limitation::new("truncated", "Some files were skipped by the size or count limits."));
    }
    if let Some(l) = skipped_gitignores_limitation(&skipped_gitignores) {
        b = b.limitation(l);
    }
    b = b.limitation(Limitation::new(
        "reference_claim",
        "A source tree is someone's claim about what was built; js_source_bundle_compare checks it against the shipped code.",
    ));
    Ok(ToolOutput::evidence(b.build()?))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceBundleParams {
    /// app_source_import evidence (inline or retained) or an absolute source folder path.
    pub source: Value,
    /// js_app_map evidence or an absolute path to analyse.
    pub application: Value,
    #[serde(default = "default_rows")]
    pub max_rows: usize,
}

fn default_rows() -> usize {
    2000
}

/// Compare a reference source tree with a shipped bundle, carrying a source map's unread external
/// sections and the row cap into the evidence as uncertainty rather than absent sources.
fn compare_source_to_bundle_tool(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: SourceBundleParams = parse_params(params)?;
    let (src, sid, skipped_gitignores) = if let Some(path) = is_path_input(&p.source) {
        let ip = ImportSourceParams {
            path: path.into(),
            respect_gitignore: true,
            include_hidden: false,
            max_files: default_src_files(),
            max_file_bytes: default_src_file_bytes(),
            max_total_bytes: default_src_total(),
        };
        let (g, skipped) = source_graph_from_path(ctx, &ip)?;
        (g, None, skipped)
    } else {
        let ev = resolve_evidence_input(ctx, &p.source, &["app_source_import"])?;
        let g: SourceGraph =
            serde_json::from_value(ev.result.clone()).map_err(|e| ToolError::invalid(format!("evidence {} has no source graph: {e}", ev.id)))?;
        (g, Some(ev.id), Vec::new())
    };
    let (app, aid) = load_graph(ctx, &p.application)?;
    let max_rows = p.max_rows.clamp(1, 100_000);
    let d = diff_source_and_bundle(&src, &app, max_rows);
    let s = &d["summary"];
    let summary = format!(
        "{} of {} source script(s) examined; {} matched; {} unmatched; {} uncertain; {} bundle unit(s) with no observed source match.",
        s["scripts_examined"], s["source_scripts"], s["matched"], s["unmatched_sources"], s["uncertain_sources"], s["bundle_units_without_source"]
    );
    let rows_capped = d["rows_capped"].as_bool().unwrap_or(false);
    let inventory_truncated = d["source_map_inventory_truncated"].as_bool().unwrap_or(false);
    let map_external = d["source_map_external_sections"].as_bool().unwrap_or(false);
    let map_unreadable = d["source_map_unreadable"].as_bool().unwrap_or(false);
    let mut b = EvidenceBuilder::new(
        "js_source_bundle_compare",
        Subject { kind: "source-bundle-pair".into(), sha256: None, name: Some(format!("{} vs {}", src.label, app.label)) },
        env(ctx),
    )
    .params(json!({"source_graph_id": src.source_graph_id, "graph_id": app.graph_id}))
    .result(d)
    .summary(summary)
    .confidence(Confidence::Inferred)
    .limitation(Limitation::new(
        "heuristic_matching",
        "Beyond identical paths and source-map listings, matches rest on shared strings and names; 'weak' matches need a human look.",
    ));
    if inventory_truncated {
        b = b
            .limitation(Limitation::new(
                "source_map_inventory_truncated",
                "Some source-map paths were omitted by inventory or source-map limits; sources without an observed match are uncertain, not confirmed absent.",
            ))
            .coverage(Coverage { status: CoverageStatus::Partial, ..Default::default() });
    }
    if map_external {
        b = b
            .limitation(Limitation::new(
                "source_map_external_sections",
                "A source map points at an external map that wasn't read; sources without an observed match are uncertain, not confirmed absent.",
            ))
            .coverage(Coverage { status: CoverageStatus::Partial, ..Default::default() });
    }
    if map_unreadable {
        b = b
            .limitation(Limitation::new(
                "source_map_unreadable",
                "A source map was malformed or not loaded; sources without an observed match are uncertain, not confirmed absent.",
            ))
            .coverage(Coverage { status: CoverageStatus::Partial, ..Default::default() });
    }
    if rows_capped {
        b = b
            .limitation(Limitation::new(
                "rows_capped",
                format!(
                    "At most {max_rows} source scripts are compared and at most {max_rows} rows are listed; the match counts cover the examined scripts, and bundle units without an observed source are counted in full."
                ),
            ))
            .coverage(Coverage { status: CoverageStatus::Partial, ..Default::default() });
    }
    if let Some(l) = skipped_gitignores_limitation(&skipped_gitignores) {
        b = b.limitation(l);
    }
    Ok(ToolOutput::evidence(with_links(b, [sid, aid]).build()?))
}

// ---------------------------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OwnerParam {
    pub module_path: String,
    pub symbol: String,
    #[serde(default)]
    pub impl_sha256: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaseParam {
    /// works, refuses, bad-input, cancel or shutdown.
    pub kind: String,
    /// Evidence id proving this case.
    pub evidence_id: String,
    /// unit-test, module-test, network-test, page-test, built-app, native-call, live-run or outside-check.
    pub strength: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerifierParam {
    pub command: String,
    pub strength: String,
    /// pass or fail.
    pub status: String,
    pub result_evidence: String,
    /// Expectations this verifier checks.
    pub expectation_ids: Vec<String>,
    #[serde(default)]
    pub nondeterminism: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BindingParam {
    pub expectation_id: String,
    pub owner: OwnerParam,
    #[serde(default)]
    pub parser_type: Option<String>,
    #[serde(default)]
    pub cases: Vec<CaseParam>,
    #[serde(default)]
    pub verifier: Option<VerifierParam>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ManifestParam {
    #[serde(default)]
    pub bindings: Vec<BindingParam>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewParam {
    pub expectation_id: String,
    /// required, skipped or waiting.
    pub decision: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LedgerParams {
    /// js_app_map evidence or an absolute path to analyse.
    pub application: Value,
    /// How the rebuild binds expectations to owners, cases and verifiers.
    #[serde(default)]
    pub manifest: ManifestParam,
    /// Expectations reviewed as out of scope or blocked.
    #[serde(default)]
    pub reviewed_expectations: Vec<ReviewParam>,
}

fn app_ledger_build(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: LedgerParams = parse_params(params)?;
    let (g, id) = load_graph(ctx, &p.application)?;
    for r in &p.reviewed_expectations {
        if !matches!(r.decision.as_str(), "required" | "skipped" | "waiting") {
            return Err(ToolError::invalid(format!("decision {:?} must be required, skipped or waiting", r.decision)));
        }
    }
    let bindings: Vec<Binding> = p
        .manifest
        .bindings
        .iter()
        .map(|b| Binding {
            expectation_id: b.expectation_id.clone(),
            owner: craft_anything_analysis::js_ledger::Owner {
                module_path: b.owner.module_path.clone(),
                symbol: b.owner.symbol.clone(),
                impl_sha256: b.owner.impl_sha256.clone(),
            },
            parser_type: b.parser_type.clone(),
            cases: b
                .cases
                .iter()
                .map(|c| craft_anything_analysis::js_ledger::Case { kind: c.kind.clone(), evidence_id: c.evidence_id.clone(), strength: c.strength.clone() })
                .collect(),
            verifier: b.verifier.as_ref().map(|v| craft_anything_analysis::js_ledger::Verifier {
                command: v.command.clone(),
                strength: v.strength.clone(),
                status: v.status.clone(),
                result_evidence: v.result_evidence.clone(),
                expectation_ids: v.expectation_ids.clone(),
                nondeterminism: v.nondeterminism.clone(),
            }),
        })
        .collect();
    let reviews: Vec<Review> = p
        .reviewed_expectations
        .iter()
        .map(|r| Review { expectation_id: r.expectation_id.clone(), decision: r.decision.clone(), reason: r.reason.clone() })
        .collect();
    let session = &*ctx.session;
    let ok = |eid: &str| session.resolve_evidence(eid).is_ok_and(|e| e.verify());
    let l = build_ledger(&g, &bindings, &reviews, &ok);
    let summary = format!(
        "Ledger {}: {} expectation(s), status {} ({}).",
        l["ledger_id"].as_str().unwrap_or(""),
        l["expectations"].as_array().map_or(0, Vec::len),
        l["status"].as_str().unwrap_or("?"),
        l["counts"].as_object().map(|m| m.iter().map(|(k, v)| format!("{v} {k}")).collect::<Vec<_>>().join(", ")).unwrap_or_default()
    );
    let mut b = EvidenceBuilder::new("app_ledger_build", app_subject(&g.label), env(ctx))
        .params(json!({"graph_id": g.graph_id, "bindings": bindings.len(), "reviews": reviews.len()}))
        .result(l)
        .summary(summary)
        .confidence(Confidence::Derived)
        .limitation(Limitation::new(
            "candidate_expectations",
            "Expectations are derived from what static analysis found; behaviour it could not see needs expectations added by review.",
        ));
    for bnd in &bindings {
        for c in &bnd.cases {
            b = b.link(c.evidence_id.clone());
        }
        if let Some(v) = &bnd.verifier {
            b = b.link(v.result_evidence.clone());
        }
    }
    Ok(ToolOutput::evidence(with_links(b, [id]).build()?))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CoverageParams {
    /// app_ledger_build evidence (inline or retained).
    pub ledger: Value,
}

fn app_coverage_evaluate(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: CoverageParams = parse_params(params)?;
    let ev = resolve_evidence_input(ctx, &p.ledger, &["app_ledger_build"])?;
    let c = evaluate_coverage(&ev.result);
    let summary =
        format!("{}% of {} in-scope expectation(s) proven; ledger {}.", c["proven_percent"], c["in_scope"], c["ledger_status"].as_str().unwrap_or("?"));
    let b = EvidenceBuilder::new("app_coverage_evaluate", ev.subject.clone(), env(ctx))
        .params(json!({"ledger_id": c["ledger_id"]}))
        .result(c)
        .summary(summary)
        .confidence(Confidence::Derived)
        .link(ev.id);
    Ok(ToolOutput::evidence(b.build()?))
}

// ---------------------------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectionParams {
    /// core_archive_show evidence (inline or retained). Its recorded path is re-read for the full inventory.
    #[serde(default)]
    pub inventory: Option<Value>,
    /// Or an absolute path to inventory now. Defaults to the active target when neither is given.
    #[serde(default)]
    pub path: Option<String>,
}

fn projection_inventory(ctx: &Ctx<'_>, p: &ProjectionParams) -> Result<(Inventory, Subject, Option<String>), ToolError> {
    let limits = InvLimits { max_depth: 2, ..InvLimits::default() };
    if let Some(v) = &p.inventory {
        let ev = resolve_evidence_input(ctx, v, &["core_archive_show"])?;
        let (inv, subject) = recorded_inventory(&ev, limits, &ctx.cancel)?;
        return Ok((inv, subject, Some(ev.id)));
    }
    let path = match &p.path {
        Some(x) => abs_path(x, "path")?,
        None => ctx.session.active()?.target.path.clone(),
    };
    let (inv, subject) = inventory_path(&path, limits, &ctx.cancel)?;
    Ok((inv, subject, None))
}

fn project(ctx: &mut Ctx<'_>, params: Value, op: &'static str, f: fn(&Inventory) -> Value, platform: &str) -> Result<ToolOutput, ToolError> {
    let p: ProjectionParams = parse_params(params)?;
    let (inv, subject, id) = projection_inventory(ctx, &p)?;
    let r = f(&inv);
    let recognised = r["recognised"].as_bool().unwrap_or(false);
    let lims: Vec<Limitation> = r["limitations"].as_array().into_iter().flatten().filter_map(Value::as_str).map(|m| Limitation::new("projection", m)).collect();
    let runtimes: Vec<&str> = r["runtimes"].as_array().into_iter().flatten().filter_map(|x| x["runtime"].as_str()).collect();
    let summary = if recognised {
        format!("{} looks like a {platform} app{}.", inv.root, if runtimes.is_empty() { String::new() } else { format!(" (hints: {})", runtimes.join(", ")) })
    } else if r["shape"] == "bare-executable" {
        format!("{} is a bare program, not a {platform} app bundle; apple_macho_show and the native tools fit it better.", inv.root)
    } else {
        format!("{} does not look like a {platform} app; the projection is mostly empty.", inv.root)
    };
    let mut b = EvidenceBuilder::new(op, subject, env(ctx))
        .params(json!({"content_id": inv.content_id()}))
        .result(r)
        .summary(summary)
        .confidence(Confidence::Inferred)
        .limitations(lims);
    if inv.truncated {
        b = b.limitation(Limitation::new("truncated", "The inventory was cut short by a limit."));
    }
    Ok(ToolOutput::evidence(with_links(b, [id]).build()?))
}

fn android_app_graph_build(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    project(ctx, params, "android_app_graph_build", js_projection::project_android, "Android")
}

fn apple_app_graph_build(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    project(ctx, params, "apple_app_graph_build", js_projection::project_apple, "Apple")
}

fn harmony_app_graph_build(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    project(ctx, params, "harmony_app_graph_build", js_projection::project_harmony, "HarmonyOS")
}

#[cfg(test)]
mod tests {
    use crate::{CallOptions, Caller, Engine};
    use craft_anything_evidence::ToolError;
    use craft_anything_testkit::electron as fx;
    use serde_json::{Value, json};

    fn call(e: &Engine, id: &str, p: Value) -> Result<Value, ToolError> {
        e.call(id, p, CallOptions::new(Caller::Agent)).map(|o| o.to_json())
    }

    fn retained(v: &Value) -> Value {
        json!({"kind": "saved", "evidence_id": v["id"]})
    }

    #[test]
    fn notes_export_feature_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let root = fx::write_notes_app(dir.path()).unwrap();
        let e = Engine::new(crate::testing::session());
        let a = call(&e, "js_app_map", json!({"input_path": root})).unwrap();
        let t =
            call(&e, "app_feature_follow", json!({"application": retained(&a), "seed": {"kind": "ipc-channel", "value": "export-notes", "match": "exact"}}))
                .unwrap();
        let story = t["summary"].as_str().unwrap();
        assert!(story.contains("renderer/app.js") && story.contains("window.api.exportNotes") && story.contains("ipcMain.handle(export-notes)"), "{story}");
        let all = t["result"].to_string();
        assert!(all.contains("fs.writeFileSync") && all.contains("dialog.showSaveDialog"), "{all}");
        // By string, from a path input (analysed on the spot).
        let s =
            call(&e, "app_feature_follow", json!({"application": root, "seed": {"kind": "string", "value": "notes.csv"}, "direction": "incoming"})).unwrap();
        assert!(s["result"]["nodes"].as_array().unwrap().len() >= 2);
        let sem =
            call(&e, "js_function_explain", json!({"application": retained(&a), "seed": {"kind": "function", "value": "toCsv", "match": "exact"}})).unwrap();
        assert!(sem["summary"].as_str().unwrap().starts_with("toCsv (main.js:"), "{}", sem["summary"]);
        assert!(call(&e, "app_feature_follow", json!({"application": retained(&a), "seed": {"value": "no-such-thing-xyz"}})).is_err());
    }

    #[test]
    fn versions_exports_and_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let v1 = fx::write_files(&dir.path().join("v1"), &fx::notes_app_files()).unwrap();
        let v2 = fx::write_files(&dir.path().join("v2"), &fx::notes_app_v2_files()).unwrap();
        let e = Engine::new(crate::testing::session());
        let d = call(&e, "app_graphs_compare", json!({"left": v1, "right": v2})).unwrap();
        assert_eq!(d["result"]["ipc_channels"]["added"], json!(["import-notes"]));
        let x = call(&e, "js_exports_compare", json!({"left": v1, "right": v2})).unwrap();
        assert_eq!(x["result"]["changed"], 0);
        let a = call(&e, "js_app_map", json!({"input_path": v1})).unwrap();
        let l = call(&e, "app_ledger_build", json!({"application": retained(&a)})).unwrap();
        assert_eq!(l["result"]["status"], "open");
        let ipc = l["result"]["expectations"].as_array().unwrap().iter().find(|o| o["target"] == "ipc:export-notes").unwrap()["expectation_id"]
            .as_str()
            .unwrap()
            .to_string();
        // Bind it with real evidence ids from this session.
        let proof = a["id"].as_str().unwrap();
        let manifest = json!({"bindings": [{
            "expectation_id": ipc, "owner": {"module_path": "src/main.rs", "symbol": "export_notes"}, "parser_type": "Notes",
            "cases": [{"kind": "works", "evidence_id": proof, "strength": "module-test"}, {"kind": "refuses", "evidence_id": proof, "strength": "module-test"}, {"kind": "bad-input", "evidence_id": proof, "strength": "network-test"}],
            "verifier": {"command": "cargo test", "strength": "module-test", "status": "pass", "result_evidence": proof, "expectation_ids": [ipc]}
        }]});
        let l2 = call(&e, "app_ledger_build", json!({"application": retained(&a), "manifest": manifest})).unwrap();
        assert_eq!(l2["result"]["counts"]["proven"], 1);
        let c = call(&e, "app_coverage_evaluate", json!({"ledger": retained(&l2)})).unwrap();
        assert_eq!(c["result"]["proven"], 1);
        assert!(
            call(&e, "app_ledger_build", json!({"application": retained(&a), "reviewed_expectations": [{"expectation_id": "x", "decision": "maybe"}]}))
                .is_err()
        );
        assert!(call(&e, "app_coverage_evaluate", json!({"ledger": retained(&a)})).is_err(), "wrong evidence kind");
    }

    #[test]
    fn reference_source_against_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src-tree");
        fx::write_files(
            &src,
            &[
                ("src/api.js", "const base = 'https://api.example.test'; module.exports = { list: () => fetch(base + '/items') };".to_string()),
                ("src/util.js", "exports.sum = (a, b) => a + b;".to_string()),
                ("generated/skip.js", "nope".to_string()),
                (".gitignore", "generated/\n".to_string()),
            ],
        )
        .unwrap();
        let dist = dir.path().join("dist");
        std::fs::create_dir(&dist).unwrap();
        std::fs::write(dist.join("main.js"), fx::WEBPACK_BUNDLE).unwrap();
        let e = Engine::new(crate::testing::session());
        let s = call(&e, "app_source_import", json!({"path": src})).unwrap();
        assert!(!s["result"]["files"].as_array().unwrap().iter().any(|f| f["path"] == "generated/skip.js"));
        let c = call(&e, "js_source_bundle_compare", json!({"source": retained(&s), "application": dist})).unwrap();
        assert_eq!(c["result"]["summary"]["matched"], 2, "{}", c["result"]);
        assert_eq!(c["confidence"], "inferred");
    }

    /// A `.gitignore` too large to read whole is skipped, and the evidence says so: its rules no
    /// longer hide files. The same applies to a nested one.
    #[test]
    fn oversized_gitignores_are_skipped_and_disclosed() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source");
        fx::write_files(
            &src,
            &[
                ("src/api.js", "exports.a = 1;\n".to_string()),
                ("generated/skip.js", "nope".to_string()),
                ("sub/secret.js", "nope".to_string()),
                (".gitignore", "generated/\n".to_string()),
                ("sub/.gitignore", "secret.js\n".to_string()),
            ],
        )
        .unwrap();
        for rel in [".gitignore", "sub/.gitignore"] {
            let f = std::fs::OpenOptions::new().write(true).open(src.join(rel)).unwrap();
            f.set_len(8 * 1024 * 1024).unwrap();
        }
        let e = Engine::new(crate::testing::session());
        let s = call(&e, "app_source_import", json!({"path": src})).unwrap();
        let paths: Vec<&str> = s["result"]["files"].as_array().unwrap().iter().filter_map(|f| f["path"].as_str()).collect();
        assert!(paths.contains(&"generated/skip.js"), "{paths:?}");
        assert!(paths.contains(&"sub/secret.js"), "{paths:?}");
        let limitations = s["limitations"].as_array().unwrap();
        let message = limitations.iter().find(|l| l["code"] == "gitignore_skipped").and_then(|l| l["message"].as_str()).unwrap_or("");
        assert!(message.starts_with("2 .gitignore"), "{message}");
        assert!(message.contains("sub/.gitignore"), "{message}");
    }

    /// A `.gitignore` that isn't a regular file can't block the walk; it is skipped and disclosed.
    #[test]
    fn a_non_regular_gitignore_is_disclosed() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source");
        fx::write_files(&src, &[("src/api.js", String::new())]).unwrap();
        std::fs::create_dir(src.join(".gitignore")).unwrap();
        let e = Engine::new(crate::testing::session());
        let s = call(&e, "app_source_import", json!({"path": src})).unwrap();
        let limitations = s["limitations"].as_array().unwrap();
        assert!(
            limitations.iter().any(|l| l["code"] == "gitignore_skipped" && l["message"].as_str().is_some_and(|m| m.contains("not a regular file"))),
            "{limitations:?}"
        );
    }

    /// A `.gitignore` that isn't UTF-8 is skipped and disclosed instead of vanishing.
    #[test]
    fn a_non_utf8_gitignore_is_disclosed() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source");
        fx::write_files(&src, &[("src/api.js", String::new())]).unwrap();
        std::fs::write(src.join(".gitignore"), [0xff, 0xfe]).unwrap();
        let e = Engine::new(crate::testing::session());
        let s = call(&e, "app_source_import", json!({"path": src})).unwrap();
        let limitations = s["limitations"].as_array().unwrap();
        assert!(
            limitations.iter().any(|l| l["code"] == "gitignore_skipped" && l["message"].as_str().is_some_and(|m| m.contains("not UTF-8"))),
            "{limitations:?}"
        );
    }

    /// A `.gitignore` that is a symlink to a regular file still applies: the cap must not reject
    /// ordinary symlinked ignore files.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_gitignore_still_applies() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source");
        fx::write_files(&src, &[("src/api.js", String::new()), ("generated/skip.js", "nope".to_string()), ("ignore.rules", "generated/\n".to_string())])
            .unwrap();
        std::os::unix::fs::symlink("ignore.rules", src.join(".gitignore")).unwrap();
        let e = Engine::new(crate::testing::session());
        let s = call(&e, "app_source_import", json!({"path": src})).unwrap();
        let paths: Vec<&str> = s["result"]["files"].as_array().unwrap().iter().filter_map(|f| f["path"].as_str()).collect();
        assert!(!paths.contains(&"generated/skip.js"), "{paths:?}");
        assert!(!s["limitations"].as_array().unwrap().iter().any(|l| l["code"] == "gitignore_skipped"), "{s}");
    }

    /// A comparison that examines one of two scripts says so, marks the rows capped, and carries
    /// partial coverage with the limitation.
    #[test]
    fn source_comparison_discloses_capped_rows_and_examined_scripts() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("reference");
        fx::write_files(&src, &[("src/a.js", "const a = 1;\n".to_string()), ("src/b.js", "const b = 2;\n".to_string())]).unwrap();
        let dist = dir.path().join("dist");
        fx::write_files(&dist, &[("main.js", "const main = 3;\n".to_string())]).unwrap();
        let e = Engine::new(crate::testing::session());
        let comparison = call(&e, "js_source_bundle_compare", json!({"source": src, "application": dist, "max_rows": 1})).unwrap();
        assert_eq!(comparison["result"]["summary"]["scripts_examined"], 1, "{}", comparison["result"]);
        assert_eq!(comparison["result"]["summary"]["source_scripts"], 2);
        assert_eq!(comparison["result"]["summary"]["bundle_units_without_source"], 1);
        assert_eq!(comparison["result"]["rows_capped"], true);
        let limitations = comparison["limitations"].as_array().unwrap();
        assert!(limitations.iter().any(|l| l["code"] == "rows_capped"), "{limitations:?}");
        assert_eq!(comparison["coverage"]["status"], "partial");
        let summary = comparison["summary"].as_str().unwrap();
        assert!(summary.contains("1 of 2 source script(s) examined"), "{summary}");
    }

    #[test]
    fn source_comparison_discloses_an_unreadable_map() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("reference");
        fx::write_files(&src, &[("src/kept.js", String::new())]).unwrap();
        let dist = dir.path().join("dist");
        fx::write_files(&dist, &[("main.js", "//# sourceMappingURL=main.js.map\n".to_string()), ("main.js.map", "{\"version\":2}".to_string())]).unwrap();
        let e = Engine::new(crate::testing::session());
        let source = call(&e, "app_source_import", json!({"path": src})).unwrap();
        let comparison = call(&e, "js_source_bundle_compare", json!({"source": retained(&source), "application": dist})).unwrap();
        assert_eq!(comparison["result"]["source_map_unreadable"], true);
        assert_eq!(comparison["result"]["unmatched_sources"], json!([]));
        assert_eq!(comparison["result"]["uncertain_sources"], json!([{"source": "src/kept.js", "reason": "source_map_unreadable"}]));
        assert!(comparison["limitations"].as_array().unwrap().iter().any(|l| l["code"] == "source_map_unreadable"));
        assert_eq!(comparison["coverage"]["status"], "partial");
    }

    #[test]
    fn source_comparison_discloses_incomplete_map_inventories() {
        for truncated in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let src = dir.path().join("reference");
            fx::write_files(&src, &[("src/kept.js", String::new()), ("src/omitted.js", String::new())]).unwrap();
            let dist = dir.path().join("dist");
            let mut sources = vec!["kept.js".to_string()];
            if truncated {
                sources.extend((0..4999).map(|i| format!("padding-{i}.js")));
                sources.push("omitted.js".to_string());
            }
            fx::write_files(
                &dist,
                &[
                    ("main.js", "//# sourceMappingURL=main.js.map\n".to_string()),
                    ("main.js.map", json!({"version": 3, "sourceRoot": "src", "sources": sources, "mappings": ""}).to_string()),
                ],
            )
            .unwrap();
            let e = Engine::new(crate::testing::session());
            let source = call(&e, "app_source_import", json!({"path": src})).unwrap();
            let comparison = call(&e, "js_source_bundle_compare", json!({"source": retained(&source), "application": dist})).unwrap();
            assert_eq!(comparison["result"]["summary"]["matched"], 1);
            assert_eq!(comparison["result"]["source_map_inventory_truncated"], truncated);
            let limitations = comparison["limitations"].as_array().unwrap();
            assert_eq!(limitations.iter().any(|l| l["code"] == "source_map_inventory_truncated"), truncated);
            if truncated {
                assert_eq!(comparison["result"]["unmatched_sources"], json!([]));
                assert_eq!(
                    comparison["result"]["uncertain_sources"],
                    json!([
                        {"source": "src/omitted.js", "reason": "source_map_inventory_truncated"}
                    ])
                );
                assert_eq!(comparison["coverage"]["status"], "partial");
                let summary = comparison["summary"].as_str().unwrap();
                assert!(summary.contains("1 uncertain"), "{summary}");
                assert!(summary.contains("no observed source match"), "{summary}");
                let first_row = call(&e, "js_source_bundle_compare", json!({"source": retained(&source), "application": dist, "max_rows": 1})).unwrap();
                assert_eq!(first_row["result"]["summary"]["matched"], 1);
                assert_eq!(first_row["result"]["uncertain_sources"], json!([]));
                assert_eq!(first_row["coverage"]["status"], "partial");
            } else {
                assert_eq!(comparison["result"]["unmatched_sources"], json!(["src/omitted.js"]));
                assert_eq!(comparison["result"]["uncertain_sources"], json!([]));
                assert_eq!(comparison["coverage"]["status"], "unknown");
            }
        }
    }

    /// A comparison with a map whose external section was never read names that section in its
    /// limitation and reports an otherwise unmatched source as uncertain.
    #[test]
    fn source_comparison_discloses_unread_external_sections() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("reference");
        fx::write_files(&src, &[("src/kept.js", String::new()), ("src/vendor.js", String::new())]).unwrap();
        let dist = dir.path().join("dist");
        fx::write_files(
            &dist,
            &[
                ("main.js", "//# sourceMappingURL=main.js.map\n".to_string()),
                (
                    "main.js.map",
                    json!({
                        "version": 3,
                        "sections": [
                            {"offset": {"line": 0, "column": 0}, "map": {"version": 3, "sourceRoot": "src", "sources": ["kept.js"], "mappings": ""}},
                            {"offset": {"line": 10, "column": 0}, "url": "vendor.js.map"}
                        ]
                    })
                    .to_string(),
                ),
            ],
        )
        .unwrap();
        let e = Engine::new(crate::testing::session());
        let source = call(&e, "app_source_import", json!({"path": src})).unwrap();
        let comparison = call(&e, "js_source_bundle_compare", json!({"source": retained(&source), "application": dist})).unwrap();
        assert_eq!(comparison["result"]["summary"]["matched"], 1, "{}", comparison["result"]);
        assert_eq!(comparison["result"]["source_map_external_sections"], true);
        assert_eq!(comparison["result"]["unmatched_sources"], json!([]));
        assert_eq!(comparison["result"]["uncertain_sources"], json!([{"source": "src/vendor.js", "reason": "source_map_external_sections"}]));
        assert!(comparison["limitations"].as_array().unwrap().iter().any(|l| l["code"] == "source_map_external_sections"));
        assert_eq!(comparison["coverage"]["status"], "partial");
    }

    #[test]
    fn projections_from_paths_and_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let apk = dir.path().join("app.apk");
        std::fs::write(
            &apk,
            fx::zip_stored(&[
                ("AndroidManifest.xml", &[3, 0, 8, 0]),
                ("classes.dex", b"dex\n035\0"),
                ("lib/arm64-v8a/libhermes.so", b"\x7fELF"),
                ("assets/index.android.bundle", b"x"),
            ]),
        )
        .unwrap();
        let e = Engine::new(crate::testing::session());
        let p = call(&e, "android_app_graph_build", json!({"path": apk})).unwrap();
        assert_eq!(p["result"]["recognised"], true);
        assert!(p["summary"].as_str().unwrap().contains("react-native"));
        let inv = call(&e, "core_archive_show", json!({"path": apk})).unwrap();
        let q = call(&e, "apple_app_graph_build", json!({"inventory": retained(&inv)})).unwrap();
        assert_eq!(q["result"]["recognised"], false);
        assert_eq!(q["links"][0], inv["id"]);
        let h = call(&e, "harmony_app_graph_build", json!({"path": apk})).unwrap();
        assert_eq!(h["confidence"], "inferred");
    }

    /// A malformed source map on either side stays visible in both comparisons, for path inputs and
    /// for saved `js_app_map` evidence.
    #[test]
    fn comparisons_keep_each_sides_analysis_limitations() {
        let dir = tempfile::tempdir().unwrap();
        let package = r#"{"name":"maps","main":"main.js"}"#;
        let write = |root: &std::path::Path| {
            fx::write_files(
                root,
                &[
                    ("package.json", package.to_string()),
                    ("main.js", "const x = 1;\n//# sourceMappingURL=main.js.map\n".to_string()),
                    ("main.js.map", "not a source map".to_string()),
                ],
            )
            .unwrap()
        };
        let left = write(&dir.path().join("left"));
        let right = write(&dir.path().join("right"));
        let e = Engine::new(crate::testing::session());
        let assert_sides = |tool: &str, d: &Value| {
            let lims = d["limitations"].as_array().unwrap();
            for side in ["left:", "right:"] {
                assert!(
                    lims.iter().any(|l| l["code"] == "source_map_unreadable" && l["message"].as_str().unwrap().starts_with(side)),
                    "{tool} keeps the {side} source-map limitation: {d}"
                );
            }
        };
        for tool in ["app_graphs_compare", "js_exports_compare"] {
            let d = call(&e, tool, json!({"left": left.clone(), "right": right.clone()})).unwrap();
            assert_sides(tool, &d);
        }
        let l = call(&e, "js_app_map", json!({"input_path": left})).unwrap();
        let r = call(&e, "js_app_map", json!({"input_path": right})).unwrap();
        for tool in ["app_graphs_compare", "js_exports_compare"] {
            let d = call(&e, tool, json!({"left": retained(&l), "right": retained(&r)})).unwrap();
            assert_sides(tool, &d);
        }
    }

    /// A clean path pair gains no source-map limitation.
    #[test]
    fn clean_comparisons_have_no_analysis_limitations() {
        let dir = tempfile::tempdir().unwrap();
        let package = r#"{"name":"clean","main":"main.js"}"#;
        let write = |root: &std::path::Path| {
            fx::write_files(root, &[("package.json", package.to_string()), ("main.js", "module.exports = { x: 1 };\n".to_string())]).unwrap()
        };
        let left = write(&dir.path().join("left"));
        let right = write(&dir.path().join("right"));
        let e = Engine::new(crate::testing::session());
        for tool in ["app_graphs_compare", "js_exports_compare"] {
            let path = call(&e, tool, json!({"left": left.clone(), "right": right.clone()})).unwrap();
            assert!(!path["limitations"].as_array().unwrap().iter().any(|l| l["code"] == "source_map_unreadable"), "{tool} path inputs: {path}");
        }
    }

    /// A comparison whose channel list is cut reports complete totals and partial coverage.
    #[test]
    fn app_graph_comparison_discloses_capped_rows() {
        let dir = tempfile::tempdir().unwrap();
        let package = r#"{"name":"channels","main":"main.js"}"#.to_string();
        let channels =
            format!("const {{ ipcMain }} = require('electron');\n{}", (0..2001).map(|i| format!("ipcMain.handle('c{i}', () => {{}});\n")).collect::<String>());
        let left = fx::write_files(&dir.path().join("left"), &[("package.json", package.clone()), ("main.js", String::new())]).unwrap();
        let right = fx::write_files(&dir.path().join("right"), &[("package.json", package), ("main.js", channels)]).unwrap();
        let e = Engine::new(crate::testing::session());
        let d = call(&e, "app_graphs_compare", json!({"left": left, "right": right})).unwrap();
        assert_eq!(d["result"]["ipc_channels"]["added_total"], 2001);
        assert_eq!(d["result"]["ipc_channels"]["added"].as_array().unwrap().len(), 2000);
        assert_eq!(d["result"]["rows_capped"], true);
        assert!(d["limitations"].as_array().unwrap().iter().any(|l| l["code"] == "rows_capped"), "{d}");
        assert_eq!(d["coverage"]["status"], "partial");
        let summary = d["summary"].as_str().unwrap();
        assert!(summary.contains("IPC channels +2001 −0"), "{summary}");
    }

    /// Caps inside a changed module's function and export diffs mark the comparison too.
    #[test]
    fn app_graph_comparison_discloses_nested_module_caps() {
        let dir = tempfile::tempdir().unwrap();
        let package = r#"{"name":"exports","main":"main.js"}"#.to_string();
        let exports = |prefix: &str| (0..2001).map(|i| format!("{prefix}{i}: {i}")).collect::<Vec<_>>().join(", ");
        let left =
            fx::write_files(&dir.path().join("left"), &[("package.json", package.clone()), ("main.js", format!("module.exports = {{ {} }};\n", exports("l")))])
                .unwrap();
        let right =
            fx::write_files(&dir.path().join("right"), &[("package.json", package), ("main.js", format!("module.exports = {{ {} }};\n", exports("r")))])
                .unwrap();
        let e = Engine::new(crate::testing::session());
        let d = call(&e, "app_graphs_compare", json!({"left": left, "right": right})).unwrap();
        assert_eq!(d["result"]["modules"]["rows_capped"], true, "{d}");
        assert_eq!(d["result"]["rows_capped"], true, "{d}");
        assert!(d["limitations"].as_array().unwrap().iter().any(|l| l["code"] == "rows_capped"), "{d}");
        assert_eq!(d["coverage"]["status"], "partial");
    }

    /// A capped export diff marks the export comparison and its coverage.
    #[test]
    fn export_comparison_discloses_capped_rows() {
        let dir = tempfile::tempdir().unwrap();
        let package = r#"{"name":"exports","main":"main.js"}"#.to_string();
        let exports = (0..2001).map(|i| format!("e{i}: {i}")).collect::<Vec<_>>().join(", ");
        let left = fx::write_files(&dir.path().join("left"), &[("package.json", package.clone()), ("main.js", "module.exports = { base: 1 };\n".to_string())])
            .unwrap();
        let right =
            fx::write_files(&dir.path().join("right"), &[("package.json", package), ("main.js", format!("module.exports = {{ base: 1, {exports} }};\n"))])
                .unwrap();
        let e = Engine::new(crate::testing::session());
        let d = call(&e, "js_exports_compare", json!({"left": left, "right": right})).unwrap();
        assert_eq!(d["result"]["rows_capped"], true);
        assert!(d["limitations"].as_array().unwrap().iter().any(|l| l["code"] == "rows_capped"), "{d}");
        assert_eq!(d["coverage"]["status"], "partial");
        let row = d["result"]["modules"].as_array().unwrap().iter().find(|r| r["right"] == "main.js").unwrap();
        assert_eq!(row["diff"]["added_total"], 2001);
        assert_eq!(row["diff"]["added"].as_array().unwrap().len(), 2000);
    }
}
