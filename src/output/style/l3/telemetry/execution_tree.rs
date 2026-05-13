//! ExecutionTree: unified trace of a besogne execution.
//!
//! One tree with three zoom levels:
//!   Phase (build/seal/exec) → Node (command/probe) → Process (pid/child)
//! Metrics appear inline at every depth. Command output is inline under each command node.
//!
//! Uses L3 atoms: status_badge, node_badge, exit_code, binary_ref
//! Uses L2 tokens: phase, status, node, badge, label, telemetry, ptree, diagnostic, weight, message
//!
//! ```text
//! ├── build (44 nodes: 32 system, 1 nix)
//! ├── seal (3 nodes)
//! │   ├── ∎ sealed  env  HOME
//! │   └── ∎ sealed  file go.mod
//! └── exec (2 nodes)  ⏱ 0.088s  🧠 7.6MB
//!     ├── tier 0
//!     │   └── ∎ cached  cmd  compile  ✓ 0.003s  🧠 3.8MB
//!     │       │  echo → /usr/bin/echo v9.4
//!     │       │  compiling project
//!     │       │  ✓ idempotent
//!     │       ├─ echo compiling project [0]  ⏱ 0.001s
//!     │       └─ echo (subshell) [0]  ⏱ 0.000s
//!     └── tier 1
//!         └── ∎ cached  cmd  link  ✓ 0.001s  ← compile
//! ```

use std::collections::{HashMap, HashSet};
use termtree::Tree;

use crate::ir::{BesogneIR, ContentId, ResolvedNode, ResolvedNativeNode};
use crate::ir::dag;
use crate::manifest::Phase;
use crate::runtime::cache::ContextCache;
use crate::tracer::ProcessMetrics;

use crate::output::style::{styled, dim};
use crate::output::style::palette::RESET;
use crate::output::style::{
    phase, status, outcome, label, message,
    telemetry, ptree, diagnostic, icon,
};
use crate::output::style::l3::atoms;

/// Render the unified execution tree and print to stderr.
/// Includes last run info at the bottom.
pub fn render(ir: &BesogneIR, cache: &ContextCache) {
    let tree = build_tree(ir, cache);
    eprintln!("{tree}");

    if let Some(lr) = cache.get_last_run() {
        let status_str = if lr.exit_code == 0 {
            styled(outcome::OK, label::PASSED)
        } else {
            styled(outcome::FAIL, label::FAILED)
        };
        eprintln!("  {} {status_str} {}",
            dim(message::LAST_RUN),
            dim(&format!("{} ({:.3}s)",
                crate::output::format_relative_time(&lr.ran_at),
                lr.duration_ms as f64 / 1000.0)));
    }
}

/// Build the execution tree as a `termtree::Tree<String>`.
fn build_tree(ir: &BesogneIR, cache: &ContextCache) -> Tree<String> {
    let build_nodes: Vec<&ResolvedNode> = ir.nodes.iter().filter(|n| n.phase == Phase::Build).collect();
    let seal_nodes: Vec<&ResolvedNode> = ir.nodes.iter().filter(|n| n.phase == Phase::Seal).collect();
    let exec_nodes: Vec<&ResolvedNode> = ir.nodes.iter().filter(|n| n.phase == Phase::Exec).collect();

    let mut root = Tree::new(String::new());

    if !build_nodes.is_empty() {
        root.push(build_phase_node(&build_nodes));
    }
    if !seal_nodes.is_empty() {
        root.push(seal_phase_node(&seal_nodes, cache));
    }
    if !exec_nodes.is_empty() {
        root.push(exec_phase_node(ir, &exec_nodes, cache));
    }

    root
}

// ── Build phase: collapsed summary ──────────────────────────────────────

fn build_phase_node(nodes: &[&ResolvedNode]) -> Tree<String> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for n in nodes {
        if let ResolvedNativeNode::Binary { source, .. } = &n.node {
            let src = match source {
                Some(crate::ir::types::BinarySourceResolved::Nix { .. }) => "nix",
                Some(crate::ir::types::BinarySourceResolved::Mise { .. }) => "mise",
                Some(crate::ir::types::BinarySourceResolved::System) => "system",
                None => "other",
            };
            *counts.entry(src).or_insert(0) += 1;
        }
    }
    let sources: Vec<String> = counts.iter().map(|(k, v)| format!("{v} {k}")).collect();
    Tree::new(format!("{}build{RESET} {}",
        phase::BUILD_DIM,
        dim(&format!("({} nodes: {})", nodes.len(), sources.join(", ")))))
}

// ── Seal phase: list all probes ─────────────────────────────────────────

fn seal_phase_node(nodes: &[&ResolvedNode], cache: &ContextCache) -> Tree<String> {
    let mut tree = Tree::new(format!("{}seal{RESET} {}",
        phase::SEAL_DIM,
        dim(&format!("({} nodes)", nodes.len()))));

    for n in nodes {
        let (st_color, st_label) = if cache.get_probe(&n.id.0).is_some() {
            (status::SEALED, label::SEALED)
        } else {
            (status::PENDING, label::PENDING)
        };
        tree.push(Tree::new(format!("{} {} {}",
            atoms::status_badge::render(st_label, st_color),
            crate::output::node_type_badge(n),
            dim(&crate::output::node_short_label(n)))));
    }
    tree
}

// ── Exec phase: tiers → commands (output + verify + processes) ──────────

fn exec_phase_node(
    ir: &BesogneIR,
    exec_nodes: &[&ResolvedNode],
    cache: &ContextCache,
) -> Tree<String> {
    // Aggregate metrics across all cached commands
    let agg = aggregate_exec_metrics(exec_nodes, cache);

    let mut tree = Tree::new(format!("{}exec{RESET} {}{}",
        phase::EXEC_DIM,
        dim(&format!("({} nodes)", exec_nodes.len())),
        format_inline_metrics_full(&agg)));

    let node_by_id: HashMap<&ContentId, &ResolvedNode> = exec_nodes.iter()
        .map(|n| (&n.id, *n)).collect();
    let exec_ids: HashSet<&ContentId> = exec_nodes.iter().map(|n| &n.id).collect();

    // Binary paths map for binary_ref rendering
    let binary_map: HashMap<&str, (&str, Option<&str>)> = ir.nodes.iter()
        .filter_map(|n| match &n.node {
            ResolvedNativeNode::Binary { name, resolved_path: Some(path), resolved_version, .. } =>
                Some((name.as_str(), (path.as_str(), resolved_version.as_deref()))),
            _ => None,
        }).collect();

    if let Ok((graph, _)) = dag::build_exec_dag(ir) {
        if let Ok(tiers) = dag::compute_tiers(&graph) {
            for (tier_idx, tier) in tiers.iter().enumerate() {
                let parallel = if tier.len() > 1 {
                    format!(" {}", dim(&format!("({} parallel)", tier.len())))
                } else { String::new() };
                let mut tier_tree = Tree::new(format!("{}tier {tier_idx}{RESET}{parallel}",
                    phase::EXEC_DIM));

                for &node_idx in tier {
                    let content_id = &graph[node_idx];
                    let Some(n) = node_by_id.get(content_id) else { continue };

                    let node_tree = exec_node_tree(
                        n, cache, &node_by_id, &exec_ids, &binary_map, tier_idx, ir);
                    tier_tree.push(node_tree);
                }
                tree.push(tier_tree);
            }
        }
    }
    tree
}

/// Build a tree node for a single exec-phase node.
/// For commands: includes binary refs, cached output, verify, process children.
fn exec_node_tree(
    n: &ResolvedNode,
    cache: &ContextCache,
    node_by_id: &HashMap<&ContentId, &ResolvedNode>,
    exec_ids: &HashSet<&ContentId>,
    binary_map: &HashMap<&str, (&str, Option<&str>)>,
    tier_idx: usize,
    ir: &BesogneIR,
) -> Tree<String> {
    let node_status = crate::output::get_node_status(n, cache);
    let status_badge = crate::output::node_status_badge(&node_status);
    let type_badge = crate::output::node_type_badge(n);
    let short_label = crate::output::node_short_label(n);

    // Parent edges
    let parent_names: Vec<String> = n.parents.iter()
        .filter(|p| exec_ids.contains(p))
        .filter_map(|p| node_by_id.get(p))
        .filter_map(|p| match &p.node {
            ResolvedNativeNode::Command { name, .. } => Some(name.clone()),
            _ => Some(crate::output::input_label(p)),
        })
        .collect();
    let parents_tag = if parent_names.len() > 1 {
        format!("  {}", dim(&format!("\u{2190} {}", parent_names.join(", "))))
    } else if !parent_names.is_empty() && tier_idx > 0 {
        format!("  {}", dim(&format!("\u{2190} {}", parent_names[0])))
    } else { String::new() };

    // Command detail (exit + all metrics, no thresholds)
    let detail = match &n.node {
        ResolvedNativeNode::Command { name, .. } => {
            cache.get_command(name).map(|c| {
                let metrics = format_inline_metrics_full(&NodeMetrics {
                    wall_ms: c.wall_ms,
                    user_ms: c.user_ms,
                    sys_ms: c.sys_ms,
                    max_rss_kb: c.max_rss_kb,
                    disk_read_bytes: c.disk_read_bytes,
                    disk_write_bytes: c.disk_write_bytes,
                    net_read_bytes: c.net_read_bytes,
                    net_write_bytes: c.net_write_bytes,
                    processes_spawned: c.processes_spawned,
                });
                format!("  {}{metrics}", atoms::exit_code::render(c.exit_code))
            }).unwrap_or_default()
        }
        _ => String::new(),
    };

    let mut tree = Tree::new(format!("{status_badge} {type_badge} {short_label}{detail}{parents_tag}"));

    // Command-specific children: binary refs, output, verify, process tree
    if let ResolvedNativeNode::Command { name, run, .. } = &n.node {
        if let Some(cached) = cache.get_command(name) {
            // Binary refs (L3) — show resolved paths for args that match declared binaries
            for arg in run {
                if let Some(&(path, ver)) = binary_map.get(arg.as_str()) {
                    tree.push(Tree::new(
                        atoms::binary_ref::render(arg, &crate::output::crop_path(path, 60), ver)));
                }
            }

            // Env vars (L3 dim — show values, mask secrets)
            if !cached.stdout.is_empty() || !cached.stderr.is_empty() {
                let secret_vars: HashSet<&str> = ir.nodes.iter()
                    .filter_map(|n| match &n.node {
                        ResolvedNativeNode::Env { name, secret: true, .. } => Some(name.as_str()),
                        _ => None,
                    }).collect();

                let mut env_display: Vec<String> = Vec::new();
                for probe_node in &ir.nodes {
                    if let ResolvedNativeNode::Env { name, secret: _, .. } = &probe_node.node {
                        if let Some(probe) = cache.get_probe(&probe_node.id.0) {
                            for (k, v) in &probe.variables {
                                if secret_vars.contains(name.as_str()) || secret_vars.contains(k.as_str()) {
                                    env_display.push(format!("{k}=*****"));
                                } else {
                                    let display_v = if v.len() > 50 { format!("{}...", &v[..47]) } else { v.clone() };
                                    env_display.push(format!("{k}={display_v}"));
                                }
                            }
                        }
                    }
                }
                if !env_display.is_empty() {
                    env_display.sort();
                    tree.push(Tree::new(dim(&env_display.join("  "))));
                }
            }

            // Cached output (L3 dim — leaf nodes, filtered for non-empty)
            let stdout_lines: Vec<&str> = cached.stdout.lines()
                .filter(|l| !l.is_empty()).collect();
            let stderr_lines: Vec<&str> = cached.stderr.lines()
                .filter(|l| !l.is_empty()).collect();
            for line in stdout_lines.iter().take(10) {
                tree.push(Tree::new(dim(line)));
            }
            if stdout_lines.len() > 10 {
                tree.push(Tree::new(dim(&format!("...{} more lines", stdout_lines.len() - 10))));
            }
            for line in stderr_lines.iter().take(5) {
                tree.push(Tree::new(dim(line)));
            }
            if stderr_lines.len() > 5 {
                tree.push(Tree::new(dim(&format!("...{} more lines", stderr_lines.len() - 5))));
            }

            // Verification result
            if cache.verified_hash.is_some() {
                let is_bad = cache.non_idempotent.contains(name);
                if is_bad {
                    tree.push(Tree::new(styled(diagnostic::NOT_IDEMPOTENT,
                        &format!("{} {}", icon::FAIL, message::NOT_IDEMPOTENT))));
                } else {
                    tree.push(Tree::new(styled(diagnostic::IDEMPOTENT,
                        &format!("{} {}", icon::OK, message::IDEMPOTENT))));
                }
            }

            // Process tree children (L3) — after output
            render_process_children(&mut tree, &cached.process_tree);
        }
    }

    tree
}

// ── Process tree rendering ──────────────────────────────────────────────

fn render_process_children(parent: &mut Tree<String>, procs: &[ProcessMetrics]) {
    if procs.len() <= 1 { return; }

    // Build pid→children index
    let mut children_map: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, p) in procs.iter().enumerate() {
        if i > 0 {
            children_map.entry(p.ppid).or_default().push(i);
        }
    }

    // Render children of root process
    let root = &procs[0];
    if let Some(kids) = children_map.get(&root.pid) {
        for &idx in kids {
            render_process_node(parent, procs, &children_map, idx, &root.cmdline);
        }
    }
}

fn render_process_node(
    parent: &mut Tree<String>,
    procs: &[ProcessMetrics],
    children_map: &HashMap<u32, Vec<usize>>,
    idx: usize,
    parent_cmdline: &str,
) {
    let p = &procs[idx];
    let plabel = crate::output::process_label(p);

    // Subshell detection
    let ss = if !p.cmdline.is_empty() && !parent_cmdline.is_empty() && p.cmdline == parent_cmdline {
        format!(" {}", dim("(subshell)"))
    } else { String::new() };

    // Exit code: dim green for 0, red for non-zero (escalated)
    let exit = atoms::exit_code::render(p.exit_code);

    // Per-process metrics (all metrics, no thresholds)
    let metrics = format_inline_metrics_full(&NodeMetrics {
        wall_ms: p.wall_ms,
        user_ms: p.user_ms,
        sys_ms: p.sys_ms,
        max_rss_kb: p.max_rss_kb,
        disk_read_bytes: p.read_bytes,
        disk_write_bytes: p.write_bytes,
        net_read_bytes: 0,
        net_write_bytes: 0,
        processes_spawned: 0,
    });

    let label = format!("{}{plabel}{RESET}{ss} [{exit}]{metrics}", ptree::CHILD);
    let mut node = Tree::new(label);

    // Recurse into grandchildren
    if let Some(kids) = children_map.get(&p.pid) {
        for &kid_idx in kids {
            render_process_node(&mut node, procs, children_map, kid_idx, &p.cmdline);
        }
    }

    parent.push(node);
}

// ── Inline metrics (compact, L3) ────────────────────────────────────────

struct NodeMetrics {
    wall_ms: u64,
    user_ms: u64,
    sys_ms: u64,
    max_rss_kb: u64,
    disk_read_bytes: u64,
    disk_write_bytes: u64,
    net_read_bytes: u64,
    net_write_bytes: u64,
    processes_spawned: u64,
}

impl NodeMetrics {
    fn zero() -> Self {
        Self { wall_ms: 0, user_ms: 0, sys_ms: 0, max_rss_kb: 0,
               disk_read_bytes: 0, disk_write_bytes: 0,
               net_read_bytes: 0, net_write_bytes: 0, processes_spawned: 0 }
    }
}

fn aggregate_exec_metrics(exec_nodes: &[&ResolvedNode], cache: &ContextCache) -> NodeMetrics {
    let mut m = NodeMetrics::zero();
    for n in exec_nodes {
        if let ResolvedNativeNode::Command { name, .. } = &n.node {
            if let Some(c) = cache.get_command(name) {
                m.wall_ms += c.wall_ms;
                m.user_ms += c.user_ms;
                m.sys_ms += c.sys_ms;
                m.max_rss_kb = m.max_rss_kb.max(c.max_rss_kb);
                m.disk_read_bytes += c.disk_read_bytes;
                m.disk_write_bytes += c.disk_write_bytes;
                m.net_read_bytes += c.net_read_bytes;
                m.net_write_bytes += c.net_write_bytes;
                m.processes_spawned += c.processes_spawned;
            }
        }
    }
    m
}

/// Full metrics for --status view: show everything, no thresholds.
fn format_inline_metrics_full(m: &NodeMetrics) -> String {
    use crate::output::style::metric_label as ml;

    if m.wall_ms == 0 && m.max_rss_kb == 0 {
        return String::new();
    }
    let mut parts = Vec::new();
    parts.push(format!("{}{}:{:.3}s{RESET}",
        telemetry::TIME, telemetry::TIME_ICON, m.wall_ms as f64 / 1000.0));
    if m.user_ms > 0 || m.sys_ms > 0 {
        let cores = if m.wall_ms > 0 { (m.user_ms + m.sys_ms) as f64 / m.wall_ms as f64 } else { 0.0 };
        parts.push(format!("{}{}:{:.2}s {} + {:.2}s {} ({:.1} {}){RESET}",
            telemetry::CPU, telemetry::CPU_ICON,
            m.user_ms as f64 / 1000.0, ml::USER,
            m.sys_ms as f64 / 1000.0, ml::KERNEL,
            cores, ml::CORES));
    }
    if m.max_rss_kb > 0 {
        parts.push(format!("{}{}:{}{RESET}",
            telemetry::MEMORY, telemetry::MEMORY_ICON,
            format_bytes(m.max_rss_kb * 1024)));
    }
    if m.disk_read_bytes > 0 || m.disk_write_bytes > 0 {
        parts.push(format!("{}{} \u{2b07} {}:{} \u{2b06} {}:{}{RESET}",
            telemetry::DISK, telemetry::DISK_ICON,
            ml::READ, format_bytes(m.disk_read_bytes),
            ml::WRITE, format_bytes(m.disk_write_bytes)));
    }
    if m.net_read_bytes > 0 || m.net_write_bytes > 0 {
        parts.push(format!("{}{} \u{2b07} {}:{} \u{2b06} {}:{}{RESET}",
            telemetry::NETWORK, telemetry::NETWORK_ICON,
            ml::DOWNLOAD, format_bytes(m.net_read_bytes),
            ml::UPLOAD, format_bytes(m.net_write_bytes)));
    }
    if m.processes_spawned > 0 {
        parts.push(format!("{}{} {}:{}{RESET}",
            telemetry::PROCESS, telemetry::PROCESS_ICON,
            ml::PROCESSES, m.processes_spawned + 1));
    }
    format!("  {}", parts.join("  "))
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1}GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1}MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{}KB", bytes / 1024)
    } else {
        format!("{bytes}B")
    }
}
