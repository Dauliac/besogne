use super::types::{BesogneIR, ContentId, ResolvedNode};
use crate::manifest::Phase;
use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::HashMap;

/// Build the exec-phase DAG from resolved inputs
pub fn build_exec_dag(
    ir: &BesogneIR,
) -> Result<(DiGraph<ContentId, ()>, HashMap<ContentId, NodeIndex>), String> {
    let mut graph = DiGraph::new();
    let mut node_map: HashMap<ContentId, NodeIndex> = HashMap::new();

    // Add all exec-phase inputs as nodes
    let exec_nodes: Vec<&ResolvedNode> = ir
        .nodes
        .iter()
        .filter(|i| i.phase == Phase::Exec)
        .collect();

    for input in &exec_nodes {
        let idx = graph.add_node(input.id.clone());
        node_map.insert(input.id.clone(), idx);
    }

    // Add ordering edges (parent → child)
    for input in &exec_nodes {
        if let Some(node_idx) = node_map.get(&input.id) {
            for parent_id in &input.parents {
                if let Some(parent_idx) = node_map.get(parent_id) {
                    graph.add_edge(*parent_idx, *node_idx, ());
                }
                // Skip parents not in exec DAG (cross-phase refs like source nodes)
            }
        }
    }

    // Check for cycles
    if petgraph::algo::is_cyclic_directed(&graph) {
        return Err("circular ordering in exec DAG".into());
    }

    Ok((graph, node_map))
}

/// Compute parallel execution tiers from topological sort
pub fn compute_tiers(
    graph: &DiGraph<ContentId, ()>,
) -> Result<Vec<Vec<NodeIndex>>, String> {
    let topo = petgraph::algo::toposort(graph, None)
        .map_err(|_| "circular ordering detected".to_string())?;

    let mut depth: HashMap<NodeIndex, usize> = HashMap::new();
    let mut max_depth = 0;

    for &node in &topo {
        let d = graph
            .neighbors_directed(node, petgraph::Direction::Incoming)
            .map(|pred| depth.get(&pred).copied().unwrap_or(0) + 1)
            .max()
            .unwrap_or(0);
        depth.insert(node, d);
        max_depth = max_depth.max(d);
    }

    let mut tiers: Vec<Vec<NodeIndex>> = vec![vec![]; max_depth + 1];
    for (node, d) in &depth {
        tiers[*d].push(*node);
    }

    Ok(tiers)
}
